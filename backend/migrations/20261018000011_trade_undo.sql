-- 撤销（红冲）一条手工流水：把流水标记为已撤销，现金和这只股票的持仓、成本、已实现盈亏
-- 在同一个事务里按剩下的流水重算。已撤销的流水保留下来可查，但不再参与持仓回放。
-- reversed_at 为空表示有效流水。口径见 docs/domains/accounts.md#undo。
ALTER TABLE app.account_trade ADD COLUMN reversed_at TIMESTAMPTZ;

-- 回放持仓和已实现盈亏时按账户取有效流水（新的在后），避免把已撤销的算进去
CREATE INDEX account_trade_effective_idx ON app.account_trade (account_id, trade_id) WHERE reversed_at IS NULL;
