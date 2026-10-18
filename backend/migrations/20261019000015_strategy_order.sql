-- 策略页的持久化显示顺序；拖动策略卡片后所有用户看到同一顺序
ALTER TABLE app.strategy ADD COLUMN sort_order INTEGER;

WITH ordered AS (
  SELECT strategy_id,
         row_number() OVER (
           ORDER BY CASE WHEN builtin_key = 'ma_cross_15_30' THEN 0 WHEN builtin_key IS NOT NULL THEN 1 ELSE 2 END,
                    strategy_id
         ) - 1 AS n
  FROM app.strategy
)
UPDATE app.strategy s
SET sort_order = o.n
FROM ordered o
WHERE s.strategy_id = o.strategy_id;

ALTER TABLE app.strategy ALTER COLUMN sort_order SET NOT NULL;
CREATE INDEX strategy_sort_order_idx ON app.strategy (sort_order, strategy_id);
