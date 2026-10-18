-- 调用日志按时间清理：没有这个索引，每天的清理都要扫整张表
CREATE INDEX api_call_called_at_idx ON ops.api_call (called_at);
-- 个股页「哪些策略选过它」按股票查巡检结果
CREATE INDEX screen_hit_thscode_idx ON app.screen_hit (thscode);
