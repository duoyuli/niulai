-- 一次性修正已经失败（dead）的任务，见 docs/architecture/sync_pipeline.md#audit：
-- 1. 个股、指数日线因供应商返回的错误行写入失败：写入已修正，重新排队。
-- 2. 龙虎榜机构明细：不支持的代码（1002 Unknown thscode）按无数据处理；超过一年的日期接口不再提供，记为跳过。
-- 同一个分区可能有多个失败的任务：只重新排队最新的一个，而且这个分区没有正在排队的任务（每个分区同时只能有一个未完成的任务）
UPDATE ops.task t SET status = 'pending', attempts = 0, not_before = now(), finished_at = NULL, last_error = NULL
WHERE t.task_id IN (
  SELECT max(x.task_id) FROM ops.task x
  WHERE x.status = 'dead' AND x.dataset_id IN ('a_share.bar_1d', 'index.bar_1d') AND x.last_error LIKE '写入%'
    AND NOT EXISTS (SELECT 1 FROM ops.task o WHERE o.dataset_id = x.dataset_id AND o.part_key = x.part_key
                      AND o.status IN ('pending', 'running', 'failed'))
  GROUP BY x.dataset_id, x.part_key);

UPDATE ops.task SET status = 'empty', last_error = '接口不支持这个代码，按无数据处理（1002）'
WHERE status = 'dead' AND dataset_id = 'special.dragon_tiger_institution' AND last_error LIKE '%Unknown thscode%';

UPDATE ops.task SET status = 'skipped', last_error = '日期已超过一年，接口不再提供'
WHERE status = 'dead' AND dataset_id = 'special.dragon_tiger_institution' AND last_error LIKE '%within one year%';

-- 重新排队的任务所在的运行重新打开，任务结束后照常结束
UPDATE ops.run r SET status = 'running', finished_at = NULL
WHERE r.status <> 'running' AND EXISTS (SELECT 1 FROM ops.task t WHERE t.run_id = r.run_id AND t.status = 'pending');
