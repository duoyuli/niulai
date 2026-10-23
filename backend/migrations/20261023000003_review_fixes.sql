-- 架构评审后的收敛：分析价格和交易日的唯一出口、因子输入的数据版本、建议重算的持久化待办、研究运行的完整配置。

-- 因子排名依赖的非价格输入（财报、行业成分、证券名称和 ST）的版本；价格变化仍由 adj_factor 表示
INSERT INTO ops.derived_version (name) VALUES ('factor_input') ON CONFLICT DO NOTHING;

-- 本地有日线的日期：个股和 ETF 的并集。最新交易日、净值曲线的交易日、从行情推导交易日历都读它
CREATE VIEW mkt.v_bar_date AS
SELECT trade_date FROM mkt.stock_bar_1d
UNION ALL
SELECT trade_date FROM mkt.etf_bar_1d;

-- 一只证券的分析价格：未复权价用于成交、手数和涨跌停，q_* 用于收益和因子。
-- 股票用等比前复权（表达式与 mkt.v_stock_bar_qfq 相同，集成测试逐根比对两者），ETF 用供应商前复权价
CREATE VIEW mkt.v_analysis_bar AS
SELECT b.thscode, b.trade_date, 'stock'::text AS asset_type,
       b.open::float8 AS open, b.high::float8 AS high, b.low::float8 AS low, b.close::float8 AS close,
       (b.open * coalesce(s.qfq_factor, 1))::float8 AS q_open, (b.high * coalesce(s.qfq_factor, 1))::float8 AS q_high,
       (b.low * coalesce(s.qfq_factor, 1))::float8 AS q_low, (b.close * coalesce(s.qfq_factor, 1))::float8 AS q_close,
       b.volume::float8 AS volume, b.amount::float8 AS amount
FROM mkt.stock_bar_1d b
LEFT JOIN mkt.adj_segment s ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to
UNION ALL
SELECT b.thscode, b.trade_date, 'etf'::text AS asset_type,
       b.open::float8, b.high::float8, b.low::float8, b.close::float8,
       b.open::float8, b.high::float8, b.low::float8, b.close::float8,
       b.volume::float8, b.amount::float8
FROM mkt.etf_bar_1d b;

-- 建议重算的待办：账户写操作在同一个事务里标记，后台消费者生成成功后按标记时刻确认；失败保留标记并推迟重试
ALTER TABLE app.account
  ADD COLUMN advice_dirty_at TIMESTAMPTZ,
  ADD COLUMN advice_retry_at TIMESTAMPTZ,
  ADD COLUMN advice_error    TEXT;
CREATE INDEX account_advice_dirty_idx ON app.account (advice_dirty_at) WHERE advice_dirty_at IS NOT NULL;

-- 研究运行的完整配置：配置哈希由它派生，离开当次响应也能还原参数。之前的运行没有保存，保持为空
ALTER TABLE ops.backtest_run ADD COLUMN config JSONB;
