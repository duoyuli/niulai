-- 账户不再选风格：每个账户直接存自己的选股和风控规则（app.account.rules，JSON，缺的字段按稳健的参考值补齐），
-- 保守、稳健、进取只是高级设置里的参考值。规则不绑定具体策略，见 docs/domains/accounts.md#rules。
ALTER TABLE app.account ADD COLUMN rules JSONB NOT NULL DEFAULT '{}';

-- 已有账户：原来风格的参考值，加上改过的高级设置（去掉不再有的「参考哪些模板」「只采纳好于随便买」）
UPDATE app.account SET rules = CASE style
    WHEN 'conservative' THEN '{"min_picks": 2, "max_positions": 5, "max_weight_pct": 20, "min_cash_pct": 20, "stop_loss_pct": 6, "trend_ma": 20, "weak_market": "pause", "chase_limit_pct": 3, "sell_st": true}'::jsonb
    WHEN 'aggressive' THEN '{"min_picks": 1, "max_positions": 8, "max_weight_pct": 30, "min_cash_pct": 0, "stop_loss_pct": 10, "trend_ma": 60, "weak_market": "normal", "chase_limit_pct": 7, "sell_st": false}'::jsonb
    ELSE '{}'::jsonb
  END || (overrides - 'strategies' - 'require_edge');

ALTER TABLE app.account DROP COLUMN style, DROP COLUMN overrides;

-- 参考值的回测按新的规则重算
DELETE FROM app.style_stat;
ALTER TABLE app.style_stat RENAME TO preset_stat;
ALTER TABLE app.preset_stat RENAME COLUMN style TO preset;
ALTER TABLE app.preset_stat RENAME COLUMN params_hash TO rules_hash;
