-- 重取 2026-10-18 热修报告中已经失败、但写入逻辑已经修正的日线窗口。
-- 同一分区只重新排队最新的一条，避免历史重试记录重复创建任务。
UPDATE ops.task t SET status = 'pending', attempts = 0, not_before = now(), finished_at = NULL, last_error = NULL
WHERE t.task_id IN (
  SELECT max(x.task_id)
  FROM ops.task x
  WHERE x.status = 'dead'
    AND x.dataset_id IN ('a_share.bar_1d', 'index.bar_1d')
    AND x.last_error LIKE '写入%'
    AND NOT EXISTS (
      SELECT 1 FROM ops.task o
      WHERE o.dataset_id = x.dataset_id AND o.part_key = x.part_key
        AND o.status IN ('pending', 'running', 'failed')
    )
  GROUP BY x.dataset_id, x.part_key
);

-- 历史日期的热股接口明确返回 5003 时按没有数据处理，不再每天被兜底补偿重开。
UPDATE ops.task
SET status = 'empty', last_error = '供应商返回 5003，按没有数据处理'
WHERE status = 'dead' AND dataset_id = 'special.hot_stock_daily' AND last_error LIKE '%5003%';

-- 被重新排队的任务所在运行重新打开，任务结束后照常结束。
UPDATE ops.run r SET status = 'running', finished_at = NULL
WHERE r.status <> 'running' AND EXISTS (SELECT 1 FROM ops.task t WHERE t.run_id = r.run_id AND t.status = 'pending');
