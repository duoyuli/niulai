-- 兜底补偿把重试用尽的临时失败重新排队的次数。供应商一直给不了的分区（例如某个历史日期一直返回 5003）
-- 补偿几次后就不再重排，不会每天重开一次所属的运行
ALTER TABLE ops.task ADD COLUMN repairs SMALLINT NOT NULL DEFAULT 0;

-- 2026-10-18 线上热修时临时加的触发器（运行被重开时清空已经接上的 pending_waves）。
-- 现在接续本身是幂等的（接续时在同一个事务里清空 pending_waves），不再需要它；没有这个触发器的部署上什么都不做
DROP TRIGGER IF EXISTS run_reopen_clears_consumed_waves ON ops.run;
DROP FUNCTION IF EXISTS ops.clear_consumed_waves_on_reopen();
