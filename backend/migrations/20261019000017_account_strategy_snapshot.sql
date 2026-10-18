-- 旧账户的 null 表示「全部策略」；固定为当前参加巡检的策略，后续新增策略需要用户主动勾选。
UPDATE app.account a
SET rules = jsonb_set(
  a.rules,
  '{strategies}',
  COALESCE(
    (SELECT jsonb_agg(s.strategy_id ORDER BY s.sort_order, s.strategy_id)
     FROM app.strategy s
     WHERE s.screening),
    '[]'::jsonb
  ),
  true
)
WHERE NOT (a.rules ? 'strategies')
   OR jsonb_typeof(a.rules->'strategies') = 'null';
