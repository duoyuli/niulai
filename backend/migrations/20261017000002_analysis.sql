-- 全市场扫描按日期范围读取复权因子。只有主键 (thscode, trade_date) 时，
-- 每次扫描都要顺序扫描整张因子表（约 1,500 万行），加上日期索引后两边都能按日期范围读取。
CREATE INDEX IF NOT EXISTS adj_factor_1d_date_idx ON mkt.adj_factor_1d (trade_date);

-- 派生数据的版本：每次重算完成后更新 updated_at。
-- 进程内的计算结果缓存（例如全市场扫描）以它为版本号，重算后自动失效。
CREATE TABLE ops.derived_version (
  name       TEXT PRIMARY KEY,               -- adj_factor
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO ops.derived_version (name) VALUES ('adj_factor');
