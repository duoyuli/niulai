-- 账户规则的 strategies：原来空数组表示「全部参加巡检的策略」，改为不写（null）表示全部，空数组不再合法（一个都不勾）
UPDATE app.account SET rules = rules - 'strategies' WHERE rules->'strategies' = '[]'::jsonb;
