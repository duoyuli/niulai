-- 每日同步的调度作业改名：after_close → daily，与同步计划的名字一致
UPDATE ops.scheduler SET job = 'daily' WHERE job = 'after_close';

-- 「这个分区之后有没有成功过」是失败统计、完整性核对和数据核检的常用查询，按分区建索引
CREATE INDEX task_part_idx ON ops.task (dataset_id, part_key, task_id);
