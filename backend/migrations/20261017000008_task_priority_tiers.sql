-- 任务优先级改为按急迫程度分层：每日数据 0 层、兜底补偿 50 层、其他增量 100 层、历史回补 200 层（再加 P0–P3 的 10–40）。
-- 原来是增量 10–40、回补 110–140。把还没执行完的任务换到新的分层，见 docs/architecture/sync_pipeline.md#task_lifecycle
UPDATE ops.task t SET priority = t.priority + 100
FROM ops.run r
WHERE r.run_id = t.run_id AND r.kind = 'backfill' AND t.status IN ('pending', 'failed') AND t.priority < 200;

UPDATE ops.task t SET priority = t.priority + 100
FROM ops.run r, ops.dataset d
WHERE r.run_id = t.run_id AND d.dataset_id = t.dataset_id
  AND r.kind = 'incremental' AND t.status IN ('pending', 'failed') AND t.priority < 100
  AND coalesce(d.spec->>'schedule', 'after_close') NOT IN ('after_close', 'daily');
