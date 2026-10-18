-- 认领任务按 (priority, task_id) 排序：索引和排序一致，积压很多时也只读开头几行，不必全量排序
DROP INDEX ops.task_claim_idx;
CREATE INDEX task_claim_idx ON ops.task (priority, task_id) WHERE status IN ('pending', 'failed');

-- 接续下一个波次时会清空前一个运行的 pending_waves，执行器据此找出没接上的运行。
-- 这之前结束的运行都已经处理过（接上了或早已错过），清空，免得被当成没接上的再规划一次
UPDATE ops.run SET params = jsonb_set(params, '{pending_waves}', '[]'::jsonb)
WHERE status <> 'running' AND jsonb_array_length(coalesce(params->'pending_waves', '[]'::jsonb)) > 0;
