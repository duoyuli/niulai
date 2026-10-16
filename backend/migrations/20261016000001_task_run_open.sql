-- 每个任务结束后，执行器都要判断所在运行是否还有未完成的任务。
-- 只含未完成任务的部分索引让这个判断不必扫描整个运行（回补运行有二十多万个任务）。
CREATE INDEX task_run_open_idx ON ops.task (run_id) WHERE status IN ('pending', 'running', 'failed');
