-- 只保留多因子这一套：删除指标信号策略、每日巡检、策略统计和参考值回测；账户规则换成多因子调仓规则（组合 E），
-- 旧格式的建议（含处理状态和理论订单）一并删除。持仓、流水、净值和分红送转记录不变。见 docs/decisions/factor_only.md。
DELETE FROM app.shadow_order;
DELETE FROM app.advice_item_state;
DELETE FROM app.advice;
ALTER TABLE app.advice DROP COLUMN rules_version;
ALTER TABLE app.position DROP COLUMN strategy_id;

DROP TABLE app.strategy_stat, app.preset_stat, app.screen_hit, app.screen_run, app.signal, app.strategy;

UPDATE app.account SET rules = '{
  "factors": [
    {"factor": "volatility", "weight": 1.0},
    {"factor": "idio_vol", "weight": 1.0},
    {"factor": "profit_growth", "weight": 1.0},
    {"factor": "accruals", "weight": 1.0},
    {"factor": "liquidity", "weight": 1.0}
  ],
  "top_n": 30,
  "buffer": 1.5,
  "min_cash_pct": 0.0,
  "chase_limit_pct": 5.0,
  "sell_st": true
}'::jsonb, updated_at = now();

DELETE FROM ops.backtest_run WHERE mode NOT IN ('factor_test', 'factor_portfolio');
DROP INDEX ops.backtest_run_lookup_idx;
ALTER TABLE ops.backtest_run DROP COLUMN strategy_id, DROP COLUMN account_id;
ALTER TABLE ops.backtest_run DROP CONSTRAINT backtest_run_mode_check;
ALTER TABLE ops.backtest_run ADD CONSTRAINT backtest_run_mode_check CHECK (mode IN ('factor_test', 'factor_portfolio'));

DELETE FROM ops.scheduler WHERE job IN ('builtin_screen', 'strategy_stats', 'strategy_stats:v2-next-open-same-day');
