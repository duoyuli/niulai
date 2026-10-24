-- 「失败还没解决」的唯一口径：任务是 dead，而且同一分区（dataset_id, part_key）在它之后没有任务成功（done 或 empty）。
-- 同步页的失败计数、完整性核对的失败列表、兜底补偿的重排和定期清理都从这个视图取，不再各写一份子查询。
-- 用视图而不是 SQL 函数：视图会被展开进外层查询，执行计划和手写的 NOT EXISTS 相同。
CREATE VIEW ops.v_unresolved_dead AS
SELECT t.*
FROM ops.task t
WHERE t.status = 'dead'
  AND NOT EXISTS (
    SELECT 1 FROM ops.task o
    WHERE o.dataset_id = t.dataset_id AND o.part_key = t.part_key AND o.task_id > t.task_id
      AND o.status IN ('done', 'empty')
  );
