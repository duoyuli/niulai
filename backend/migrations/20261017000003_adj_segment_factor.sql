-- 等比复权因子改为按区间存储，与同花顺分段系数合在 mkt.adj_segment 中（见 docs/decisions/adj_factor_segments.md）。
-- 因子只在除权日变化，两次除权之间是常数。原来的 mkt.adj_factor_1d 每根 K 线存一行（约 1,500 万行、1.4 GB），
-- 全量重算约 75 秒；按区间存储约 6 万行，与同花顺分段一起重算约 1 秒。
-- 没有任何事件的股票没有分段行，视图按因子 1 处理：新上市的股票不用等重算就有复权价。

ALTER TABLE mkt.adj_segment
  ADD COLUMN factor     DOUBLE PRECISION NOT NULL DEFAULT 1,   -- 等比后复权因子：本库第一根 K 线为 1，之后每个除权日乘上当天的比例
  ADD COLUMN qfq_factor DOUBLE PRECISION NOT NULL DEFAULT 1;   -- 等比前复权因子 = factor / 最新一段的 factor

-- 全量重算分段系数：同花顺口径（fwd_*、bwd_*）和等比因子（factor、qfq_factor）。
-- 分段按除权日切分：[valid_from, valid_to)，第一段从 -infinity 开始，最后一段到 infinity。
-- 等比因子在每段内是常数：除权日落在停牌期间时，事件作用在除权日之后的第一根 K 线上，
-- 这根 K 线一定在同一段或更后的段中，所以一段的因子 = 生效日早于本段结束日的所有比例之积。
CREATE OR REPLACE PROCEDURE mkt.rebuild_adj_segment(p_run_id BIGINT)
LANGUAGE sql AS $$
  -- 运行结束时的重算和回补期间的定期重算可能同时发生：串行执行
  SELECT pg_advisory_xact_lock(hashtext('mkt.rebuild_adj_segment'));
  -- 只有约 6 万行：在一个事务内删除再插入，查询在提交前一直看到旧数据，不会被阻塞
  DELETE FROM mkt.adj_segment;
  INSERT INTO mkt.adj_segment (thscode, valid_from, valid_to, fwd_a, fwd_b, bwd_a, bwd_b, as_of_date, run_id, factor, qfq_factor)
  WITH asof AS (SELECT max(trade_date) AS d FROM mkt.stock_bar_1d),
  e AS (
    SELECT a.*,
           1 + bonus_ratio + rights_ratio            AS u,   -- 送配后股本倍数
           cash_div - rights_ratio * rights_price    AS h    -- 每股现金净流出
    FROM mkt.adj_event_effective a, asof
    WHERE a.ex_date <= asof.d
  ),
  -- ── 同花顺口径（减法）：每个除权日对原价做仿射变换 P → (P − h) / u ──
  k AS (
    SELECT e.*,
           -- 后复权（前缀）：A_i = Π_{j≤i} u_j，B_i = Σ_{k≤i} h_k · Π_{j<k} u_j
           exp(sum(ln(u)) OVER w_pre)                                AS bwd_a,
           sum(h * exp(sum_prev_ln_u)) OVER w_pre                    AS bwd_b,
           -- 前复权（后缀，只取 ex_date 之后的事件）：A = Π_{j>i} 1/u_j，B = Σ_{k>i} (−h_k) · Π_{j≥k} 1/u_j
           coalesce(exp(-sum(ln(u)) OVER w_suf), 1)                  AS fwd_a,
           coalesce(sum(-h * exp(-suf_ln_u_incl)) OVER w_suf, 0)     AS fwd_b,
           lead(ex_date) OVER (PARTITION BY thscode ORDER BY ex_date) AS next_ex
    FROM (
      SELECT e.*,
             coalesce(sum(ln(u)) OVER (PARTITION BY thscode ORDER BY ex_date
                        ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING), 0) AS sum_prev_ln_u,
             sum(ln(u)) OVER (PARTITION BY thscode ORDER BY ex_date
                        ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING)     AS suf_ln_u_incl
      FROM e
    ) e
    WINDOW w_pre AS (PARTITION BY thscode ORDER BY ex_date ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW),
           w_suf AS (PARTITION BY thscode ORDER BY ex_date ROWS BETWEEN 1 FOLLOWING AND UNBOUNDED FOLLOWING)
  ),
  first_seg AS (            -- 首个事件之前：后复权为恒等；前复权包含全部事件
    SELECT DISTINCT ON (thscode) thscode, ex_date,
           exp(-sum(ln(u)) OVER w_all)                          AS fwd_a,
           sum(-h * exp(-suf_ln_u_incl)) OVER w_all             AS fwd_b
    FROM (SELECT e.*, sum(ln(u)) OVER (PARTITION BY thscode ORDER BY ex_date
                        ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) AS suf_ln_u_incl FROM e) e
    WINDOW w_all AS (PARTITION BY thscode)
    ORDER BY thscode, ex_date
  ),
  seg AS (
    SELECT thscode, '-infinity'::date AS valid_from, ex_date AS valid_to, fwd_a, fwd_b, 1::float8 AS bwd_a, 0::float8 AS bwd_b FROM first_seg
    UNION ALL
    SELECT thscode, ex_date, coalesce(next_ex, 'infinity'), fwd_a, fwd_b, bwd_a, bwd_b FROM k
  ),
  -- ── 等比复权：每个除权日的比例 = 前一交易日收盘价 / 除权参考价 ──
  ev AS (                   -- 事件落在除权日当天或之后的第一根 K 线上（生效日）
    SELECT e.thscode, e.ex_date, e.u, e.h, x.eff_date
    FROM e CROSS JOIN LATERAL (SELECT min(b.trade_date) AS eff_date FROM mkt.stock_bar_1d b
                               WHERE b.thscode = e.thscode AND b.trade_date >= e.ex_date) x
    WHERE x.eff_date IS NOT NULL
  ),
  ev_comp AS (              -- 停牌期间跨了多个除权日时，同一根 K 线上的多个事件依次作用
    SELECT thscode, eff_date, ex_date, u, h,
           sum(ln(u)) OVER (PARTITION BY thscode, eff_date ORDER BY ex_date
                            ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) AS suf_ln_u
    FROM ev
  ),
  day_map AS (              -- 合并后的仿射变换：除权参考价 = a × 前收 + b
    SELECT thscode, eff_date, exp(-sum(ln(u))) AS a, sum(-h * exp(-suf_ln_u)) AS b
    FROM ev_comp GROUP BY thscode, eff_date
  ),
  ratio AS (                -- 生效日之前没有 K 线（事件早于本库第一根 K 线）或参考价不为正时不调整
    SELECT d.thscode, d.eff_date, p.prev_close / (d.a * p.prev_close + d.b) AS r
    FROM day_map d
    CROSS JOIN LATERAL (SELECT close::float8 AS prev_close FROM mkt.stock_bar_1d b
                        WHERE b.thscode = d.thscode AND b.trade_date < d.eff_date
                        ORDER BY b.trade_date DESC LIMIT 1) p
    WHERE d.a * p.prev_close + d.b > 0
  ),
  -- 分段和比例按日期排在一起：分段行以结束日排序，比例行以生效日排序，同一天时分段行在前（结束日不含当天）
  pts AS (
    SELECT thscode, valid_to AS d, 0 AS kind, valid_from, valid_to, fwd_a, fwd_b, bwd_a, bwd_b, NULL::float8 AS r FROM seg
    UNION ALL
    SELECT thscode, eff_date, 1, NULL, NULL, NULL, NULL, NULL, NULL, r FROM ratio
  ),
  cum AS (
    SELECT *, exp(coalesce(sum(ln(r)) OVER (PARTITION BY thscode ORDER BY d, kind ROWS UNBOUNDED PRECEDING), 0)) AS factor
    FROM pts
  )
  SELECT thscode, valid_from, valid_to, fwd_a, fwd_b, bwd_a, bwd_b, (SELECT d FROM asof), p_run_id, factor,
         factor / first_value(factor) OVER (PARTITION BY thscode ORDER BY valid_from DESC)
  FROM cum WHERE kind = 0;
$$;

-- 标准复权视图（等比）：计算指标、判断信号、显示收益率都用这两个视图
CREATE OR REPLACE VIEW mkt.v_stock_bar_hfq AS
SELECT b.thscode, b.trade_date,
       b.open * coalesce(s.factor, 1) AS open, b.high * coalesce(s.factor, 1) AS high,
       b.low  * coalesce(s.factor, 1) AS low,  b.close * coalesce(s.factor, 1) AS close,
       b.volume, b.amount, coalesce(s.factor, 1) AS factor
FROM mkt.stock_bar_1d b
LEFT JOIN mkt.adj_segment s
  ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to;

CREATE OR REPLACE VIEW mkt.v_stock_bar_qfq AS
SELECT b.thscode, b.trade_date,
       b.open * coalesce(s.qfq_factor, 1) AS open, b.high * coalesce(s.qfq_factor, 1) AS high,
       b.low  * coalesce(s.qfq_factor, 1) AS low,  b.close * coalesce(s.qfq_factor, 1) AS close,
       b.volume, b.amount, coalesce(s.qfq_factor, 1) AS factor
FROM mkt.stock_bar_1d b
LEFT JOIN mkt.adj_segment s
  ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to;

DROP PROCEDURE mkt.rebuild_adj_factor(BIGINT);
DROP TABLE mkt.adj_factor_latest;
DROP TABLE mkt.adj_factor_1d;     -- 连同 20261017000002 中的日期索引

-- 立即按新口径算一遍，否则在下一次重算之前等比因子都是默认值 1
CALL mkt.rebuild_adj_segment(0);
UPDATE ops.derived_version SET updated_at = now() WHERE name = 'adj_factor';
