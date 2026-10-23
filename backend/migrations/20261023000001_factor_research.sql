-- 因子研究：因子检验和多因子组合回测也保存到回测运行记录，用于统计试过多少组配置（Deflated Sharpe 的试验次数）
ALTER TABLE ops.backtest_run DROP CONSTRAINT backtest_run_mode_check;
ALTER TABLE ops.backtest_run ADD CONSTRAINT backtest_run_mode_check
  CHECK (mode IN ('single', 'portfolio', 'account', 'factor_test', 'factor_portfolio'));
CREATE INDEX backtest_run_mode_idx ON ops.backtest_run (mode, definition_hash);
