-- 允许旧的直接写入路径省略显示顺序；新策略仍会按当前最大值追加。
WITH missing AS (
  SELECT strategy_id,
         COALESCE((SELECT max(sort_order) + 1 FROM app.strategy), 0)
           + row_number() OVER (ORDER BY strategy_id) - 1 AS next_order
  FROM app.strategy
  WHERE sort_order IS NULL
)
UPDATE app.strategy s
SET sort_order = m.next_order
FROM missing m
WHERE s.strategy_id = m.strategy_id;

ALTER TABLE app.strategy ALTER COLUMN sort_order SET DEFAULT 0;
ALTER TABLE app.strategy ALTER COLUMN sort_order SET NOT NULL;
