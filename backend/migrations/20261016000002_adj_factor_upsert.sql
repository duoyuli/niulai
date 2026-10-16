-- 等比复权因子改为增量写入：原来的 TRUNCATE + INSERT 在重算的约 40 秒里持有排他锁，
-- 期间所有复权价查询（K 线、均线、全市场扫描）都会被阻塞。现在只写入值有变化的行，读取不受影响。
-- 回补期间执行器会定期重算（见 docs/architecture/sync_pipeline.md#postprocess），这一点更重要。
CREATE OR REPLACE PROCEDURE mkt.rebuild_adj_factor(p_run_id BIGINT)
LANGUAGE sql AS $$
  -- 运行结束时的重算和回补期间的定期重算可能同时发生：串行执行，避免两次大批量写入互相死锁
  SELECT pg_advisory_xact_lock(hashtext('mkt.rebuild_adj_factor'));
  INSERT INTO mkt.adj_factor_1d AS t (thscode, trade_date, factor, run_id)
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
  FROM k LEFT JOIN ratio r USING (thscode, trade_date)
  ON CONFLICT (thscode, trade_date) DO UPDATE SET factor = EXCLUDED.factor, run_id = EXCLUDED.run_id
  WHERE t.factor IS DISTINCT FROM EXCLUDED.factor;

  -- 日线已不存在的因子（例如证券被删除）一并删除
  DELETE FROM mkt.adj_factor_1d f
  WHERE NOT EXISTS (SELECT 1 FROM mkt.stock_bar_1d b WHERE b.thscode = f.thscode AND b.trade_date = f.trade_date);

  INSERT INTO mkt.adj_factor_latest AS t
  SELECT DISTINCT ON (thscode) thscode, trade_date, factor
  FROM mkt.adj_factor_1d ORDER BY thscode, trade_date DESC
  ON CONFLICT (thscode) DO UPDATE SET trade_date = EXCLUDED.trade_date, factor = EXCLUDED.factor
  WHERE (t.trade_date, t.factor) IS DISTINCT FROM (EXCLUDED.trade_date, EXCLUDED.factor);
  DELETE FROM mkt.adj_factor_latest l WHERE NOT EXISTS (SELECT 1 FROM mkt.adj_factor_1d f WHERE f.thscode = l.thscode);
$$;
