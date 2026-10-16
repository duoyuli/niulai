-- niulai: 常用数据的类型化表（数据源：同花顺金融数据服务 fuyao.aicubes.cn）
-- PostgreSQL 16。依赖 001_sync.sql（ops.run、ops.ms_to_cn_date）。
--   ref  证券、指数、交易日等参考数据
--   mkt  行情与公司行动（只存供应商原值）+ 可重算的复权系数
--   fin  估值等
-- 所有数据集都先进入通用记录库 raw.record（见 001_sync.sql）。
-- 这里的表是给高频使用、需要类型和索引的数据准备的投影，由同步任务直接写入，
-- 或从 raw.record 物化。run_id 指向 ops.run。

CREATE SCHEMA IF NOT EXISTS ref;
CREATE SCHEMA IF NOT EXISTS mkt;
CREATE SCHEMA IF NOT EXISTS fin;

-- ───────────────────────── ref ─────────────────────────

-- 证券主表。来源 /api/meta/tickers/list?asset_type=a-share。
-- 注意：供应商代码表只含在市证券，不含已退市股票。
-- delisted_on 由本地推断：某天起代码表中消失，就记为最后一次出现之后。
CREATE TABLE ref.security (
  thscode     TEXT PRIMARY KEY,              -- 600519.SH
  ticker      TEXT NOT NULL,                 -- 600519
  name        TEXT NOT NULL,
  exchange    TEXT NOT NULL CHECK (exchange IN ('SH', 'SZ', 'BJ')),
  board       TEXT NOT NULL,                 -- main / chinext / star / bse（按代码段推导）
  list_date   DATE,
  first_seen  DATE NOT NULL,
  last_seen   DATE NOT NULL,
  delisted_on DATE,
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 简称变化（含 ST / *ST 的戴帽、摘帽），每天和代码表对比后写入。
-- 供应商没有历史 ST 状态，这张表只能从开始采集那天起积累。
CREATE TABLE ref.security_name_hist (
  thscode    TEXT NOT NULL REFERENCES ref.security,
  name       TEXT NOT NULL,
  valid_from DATE NOT NULL,
  valid_to   DATE,                           -- NULL = 当前
  is_st      BOOLEAN GENERATED ALWAYS AS (name ~ 'ST') STORED,
  PRIMARY KEY (thscode, valid_from)
);

-- 交易日。供应商只给最近一年；更早的交易日由行情里出现过的日期补全（source = 'derived'）。
CREATE TABLE ref.trading_day (
  cal_date DATE PRIMARY KEY,
  source   TEXT NOT NULL CHECK (source IN ('api', 'derived'))
);

-- 指数与板块：交易所指数（000300.SH）、同花顺行业／概念／地区／特色指数（*.TI）
CREATE TABLE ref.index_info (
  thscode    TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  category   TEXT NOT NULL,                  -- exchange / industry / cn_concept / region / tszs
  first_seen DATE NOT NULL,
  last_seen  DATE NOT NULL
);

-- 指数成分。接口只返回当前成分，每日快照对比后记录区间，从开始采集那天起积累历史。
CREATE TABLE ref.index_member (
  index_code TEXT NOT NULL REFERENCES ref.index_info,
  thscode    TEXT NOT NULL,
  valid_from DATE NOT NULL,
  valid_to   DATE,
  PRIMARY KEY (index_code, thscode, valid_from)
);

-- ───────────────────────── mkt ─────────────────────────

-- 个股日线，未复权原值。
--   source = 'hist'     来自 /api/a-share/prices/historical?adjust=none，数值精确；最早到 2005-01-04
--   source = 'snapshot' 来自收盘后的 /api/a-share/prices/snapshot：价格精确，
--                       成交量和成交额只保留约 9 位有效数字，之后用 hist 覆盖
-- 停牌日不返回记录，这里也不补行。快照里成交量为 0 的停牌股不写入。
CREATE TABLE mkt.stock_bar_1d (
  thscode    TEXT NOT NULL,
  trade_date DATE NOT NULL,
  open       NUMERIC(12,4) NOT NULL,
  high       NUMERIC(12,4) NOT NULL,
  low        NUMERIC(12,4) NOT NULL,
  close      NUMERIC(12,4) NOT NULL,
  volume     BIGINT NOT NULL,                -- 股
  amount     NUMERIC(20,2) NOT NULL,         -- 元
  pre_close  NUMERIC(12,4),                  -- 交易所前收（除权日已调整），只有快照接口提供；历史数据为 NULL
  source     TEXT NOT NULL DEFAULT 'hist' CHECK (source IN ('hist', 'snapshot')),
  run_id     BIGINT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (thscode, trade_date)
);
CREATE INDEX stock_bar_1d_date_idx ON mkt.stock_bar_1d (trade_date);

-- 指数日线（无复权）。供应商只提供 2022-01-04 以后的数据；
-- 请求窗口的起点早于这一天时会返回空，所以起点要截到 2022-01-01。
CREATE TABLE mkt.index_bar_1d (
  thscode    TEXT NOT NULL,
  trade_date DATE NOT NULL,
  open       NUMERIC(14,4) NOT NULL,
  high       NUMERIC(14,4) NOT NULL,
  low        NUMERIC(14,4) NOT NULL,
  close      NUMERIC(14,4) NOT NULL,
  volume     NUMERIC(24,0) NOT NULL,
  amount     NUMERIC(24,2) NOT NULL,
  pre_close  NUMERIC(14,4),
  run_id     BIGINT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (thscode, trade_date)
);

-- 公司行动（除权除息事件），按供应商原样保存。
-- 同一天可能有多行（3 例），seq 保留源顺序；复权计算见 mkt.adj_event_effective。
-- 逐只 REST 接口不返回配股字段（rights_*），全市场导出文件才有。
CREATE TABLE mkt.corp_action (
  thscode      TEXT NOT NULL,
  ex_date      DATE NOT NULL,
  seq          SMALLINT NOT NULL DEFAULT 1,
  cash_div     NUMERIC(18,8) NOT NULL DEFAULT 0,   -- 每股现金分红（税前）
  bonus_ratio  NUMERIC(18,8) NOT NULL DEFAULT 0,   -- 每股送转股
  rights_ratio NUMERIC(18,8) NOT NULL DEFAULT 0,   -- 每股配股
  rights_price NUMERIC(18,8) NOT NULL DEFAULT 0,   -- 配股价
  source       TEXT NOT NULL,                      -- dump / rest
  run_id       BIGINT NOT NULL,
  updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (thscode, ex_date, seq)
);

-- 复权计算实际采用的事件：与同花顺价格口径一致，同一天多行时取最后一行。
-- 用 REST 的 forward/backward 价格逐日对比验证过，误差小于 1e-12。
CREATE VIEW mkt.adj_event_effective AS
SELECT DISTINCT ON (thscode, ex_date)
       thscode, ex_date, cash_div, bonus_ratio, rights_ratio, rights_price
FROM mkt.corp_action
ORDER BY thscode, ex_date, seq DESC;

-- 复权系数分段（可随时重算，不是事实数据）。在 [valid_from, valid_to) 区间内：
--   前复权价 = fwd_a * 原价 + fwd_b      （锚定最新：最新一段 fwd_a = 1、fwd_b = 0）
--   后复权价 = bwd_a * 原价 + bwd_b      （锚定上市：首次事件之前 bwd_a = 1、bwd_b = 0）
-- 每个除权日对原价做仿射变换 P → (P − 分红 + 配股比例 × 配股价) / (1 + 送转比例 + 配股比例)，
-- 与同花顺"精确前复权"一致，所以早年的前复权价可能为负。
-- 前复权只计入除权日不晚于 as_of_date（最新交易日）的事件。
CREATE TABLE mkt.adj_segment (
  thscode    TEXT NOT NULL,
  valid_from DATE NOT NULL,                  -- 第一段为 '-infinity'
  valid_to   DATE NOT NULL,                  -- 最后一段为 'infinity'
  fwd_a      DOUBLE PRECISION NOT NULL,
  fwd_b      DOUBLE PRECISION NOT NULL,
  bwd_a      DOUBLE PRECISION NOT NULL,
  bwd_b      DOUBLE PRECISION NOT NULL,
  as_of_date DATE NOT NULL,
  run_id     BIGINT NOT NULL,
  PRIMARY KEY (thscode, valid_from)
);

-- 全量重算分段系数。约 6 万个事件，重算一次在秒级。
CREATE OR REPLACE PROCEDURE mkt.rebuild_adj_segment(p_run_id BIGINT)
LANGUAGE sql AS $$
  DELETE FROM mkt.adj_segment;
  INSERT INTO mkt.adj_segment
  WITH asof AS (SELECT max(trade_date) AS d FROM mkt.stock_bar_1d),
  e AS (
    SELECT a.*,
           1 + bonus_ratio + rights_ratio            AS u,   -- 送配后股本倍数
           cash_div - rights_ratio * rights_price    AS h    -- 每股现金净流出
    FROM mkt.adj_event_effective a, asof
    WHERE a.ex_date <= asof.d
  ),
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
  )
  SELECT thscode, '-infinity'::date, ex_date, fwd_a, fwd_b, 1, 0, (SELECT d FROM asof), p_run_id FROM first_seg
  UNION ALL
  SELECT thscode, ex_date, coalesce(next_ex, 'infinity'), fwd_a, fwd_b, bwd_a, bwd_b, (SELECT d FROM asof), p_run_id FROM k;
$$;

-- ── 复权：业内通用做法（等比复权因子） ──────────────────────────────
-- 与 Tushare 等主流量化数据服务相同：存原价，另外存一份每日累计复权因子（后复权因子）。
--   后复权价 = 原价 × factor
--   前复权价 = 原价 × factor / 最新 factor
--   截至 D 日的动态前复权（回测用，不会用到 D 日之后的信息） = 原价 × factor / D 日的 factor
-- 每个除权日的比例 = 前一交易日收盘价 / 除权参考价，
--   除权参考价 = (前收 − 每股分红 + 配股比例 × 配股价) / (1 + 送转比例 + 配股比例)。
-- 等比复权不会出现负价，每天的涨跌幅与真实情况一致。由于前复权与后复权只差一个常数倍，
-- 均线交叉等形态信号的日期与基准日无关。
-- 因子从本库第一根 K 线（最早 2005 年）开始累计，早于第一根 K 线的事件不计入。
-- 因此本库的后复权价与供应商"从上市日起"的后复权价相差一个常数倍，涨跌幅与信号不受影响。
CREATE TABLE mkt.adj_factor_1d (
  thscode    TEXT NOT NULL,
  trade_date DATE NOT NULL,
  factor     DOUBLE PRECISION NOT NULL,     -- 累计后复权因子，第一根 K 线为 1
  run_id     BIGINT NOT NULL,
  PRIMARY KEY (thscode, trade_date)
);

-- 每只股票最新的因子，计算前复权时用（避免每次查询都扫一遍全表）
CREATE TABLE mkt.adj_factor_latest (
  thscode    TEXT PRIMARY KEY,
  trade_date DATE NOT NULL,
  factor     DOUBLE PRECISION NOT NULL
);

-- 全量重算等比复权因子。
-- 事件落在除权日当天或之后的第一根 K 线上；如果停牌期间跨了多个除权日，按除权日顺序合并成一次调整。
CREATE OR REPLACE PROCEDURE mkt.rebuild_adj_factor(p_run_id BIGINT)
LANGUAGE sql AS $$
  TRUNCATE mkt.adj_factor_1d;
  INSERT INTO mkt.adj_factor_1d (thscode, trade_date, factor, run_id)
  WITH k AS (
    SELECT thscode, trade_date, close,
           lag(close) OVER (PARTITION BY thscode ORDER BY trade_date) AS prev_close
    FROM mkt.stock_bar_1d
  ),
  ev AS (
    SELECT e.thscode, e.ex_date,
           1 + e.bonus_ratio + e.rights_ratio          AS u,
           e.cash_div - e.rights_ratio * e.rights_price AS h,
           (SELECT min(b.trade_date) FROM mkt.stock_bar_1d b
             WHERE b.thscode = e.thscode AND b.trade_date >= e.ex_date) AS eff_date
    FROM mkt.adj_event_effective e
  ),
  ev_comp AS (    -- 同一根 K 线上的多个事件依次作用：P → (P − h) / u
    SELECT thscode, eff_date, ex_date, u, h,
           sum(ln(u)) OVER (PARTITION BY thscode, eff_date ORDER BY ex_date
                            ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) AS suf_ln_u
    FROM ev WHERE eff_date IS NOT NULL
  ),
  day_map AS (    -- 合并后的仿射变换：参考价 = a × 前收 + b
    SELECT thscode, eff_date,
           exp(-sum(ln(u))) AS a,
           sum(-h * exp(-suf_ln_u)) AS b
    FROM ev_comp GROUP BY thscode, eff_date
  ),
  ratio AS (
    SELECT k.thscode, k.trade_date,
           k.prev_close / nullif(d.a * k.prev_close + d.b, 0) AS r
    FROM day_map d JOIN k ON k.thscode = d.thscode AND k.trade_date = d.eff_date
    WHERE k.prev_close IS NOT NULL AND d.a * k.prev_close + d.b > 0
  )
  SELECT k.thscode, k.trade_date,
         exp(sum(ln(coalesce(r.r, 1))) OVER (PARTITION BY k.thscode ORDER BY k.trade_date)),
         p_run_id
  FROM k LEFT JOIN ratio r USING (thscode, trade_date);

  TRUNCATE mkt.adj_factor_latest;
  INSERT INTO mkt.adj_factor_latest
  SELECT DISTINCT ON (thscode) thscode, trade_date, factor
  FROM mkt.adj_factor_1d ORDER BY thscode, trade_date DESC;
$$;

-- 标准复权视图（等比）：计算指标、判断信号、显示收益率都用这两个视图
CREATE VIEW mkt.v_stock_bar_hfq AS
SELECT b.thscode, b.trade_date,
       b.open * f.factor AS open, b.high * f.factor AS high,
       b.low  * f.factor AS low,  b.close * f.factor AS close,
       b.volume, b.amount, f.factor
FROM mkt.stock_bar_1d b
JOIN mkt.adj_factor_1d f USING (thscode, trade_date);

CREATE VIEW mkt.v_stock_bar_qfq AS
SELECT b.thscode, b.trade_date,
       b.open * f.factor / l.factor AS open, b.high  * f.factor / l.factor AS high,
       b.low  * f.factor / l.factor AS low,  b.close * f.factor / l.factor AS close,
       b.volume, b.amount, f.factor / l.factor AS factor
FROM mkt.stock_bar_1d b
JOIN mkt.adj_factor_1d f USING (thscode, trade_date)
JOIN mkt.adj_factor_latest l USING (thscode);

-- ── 同花顺口径（减法 / 精确复权） ─────────────────────────────────────
-- 只用于和同花顺软件、REST adjust=forward/backward 对数，或在 K 线图上显示与同花顺一致的价格。
-- 不要用它计算指标：早年价格可能为负，涨跌幅会失真。
CREATE VIEW mkt.v_stock_bar_qfq_ths AS
SELECT b.thscode, b.trade_date,
       coalesce(s.fwd_a, 1) * b.open  + coalesce(s.fwd_b, 0) AS open,
       coalesce(s.fwd_a, 1) * b.high  + coalesce(s.fwd_b, 0) AS high,
       coalesce(s.fwd_a, 1) * b.low   + coalesce(s.fwd_b, 0) AS low,
       coalesce(s.fwd_a, 1) * b.close + coalesce(s.fwd_b, 0) AS close,
       b.volume, b.amount
FROM mkt.stock_bar_1d b
LEFT JOIN mkt.adj_segment s
  ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to;

CREATE VIEW mkt.v_stock_bar_hfq_ths AS
SELECT b.thscode, b.trade_date,
       coalesce(s.bwd_a, 1) * b.open  + coalesce(s.bwd_b, 0) AS open,
       coalesce(s.bwd_a, 1) * b.high  + coalesce(s.bwd_b, 0) AS high,
       coalesce(s.bwd_a, 1) * b.low   + coalesce(s.bwd_b, 0) AS low,
       coalesce(s.bwd_a, 1) * b.close + coalesce(s.bwd_b, 0) AS close,
       b.volume, b.amount
FROM mkt.stock_bar_1d b
LEFT JOIN mkt.adj_segment s
  ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to;

-- ───────────────────────── fin（第二阶段） ─────────────────────────

-- 每日估值。供应商只提供最新值、没有历史，所以每个交易日存一份，从开始采集那天起积累。
--   pe_* / pb / ps / pcf  来自 a_share.valuation（/api/a-share/valuations/snapshot）
--   float_market_cap      来自 a_share.auction_final（/api/a-share/auction/snapshot?stage=final），
--                         按当日最新价计算；float_market_cap / close 可推算流通股本，再算换手率
CREATE TABLE fin.valuation_1d (
  thscode    TEXT NOT NULL,
  trade_date DATE NOT NULL,
  float_market_cap NUMERIC(24,2),
  pe_ttm     NUMERIC(20,6),
  pe_mrq     NUMERIC(20,6),
  pb_mrq     NUMERIC(20,6),
  ps_ttm     NUMERIC(20,6),
  pcf_ttm    NUMERIC(20,6),
  run_id     BIGINT NOT NULL,
  PRIMARY KEY (thscode, trade_date)
);
