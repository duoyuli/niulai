-- 权威涨跌停池（special.limit_up_pool、special.limit_down_pool、special.limit_break_pool）的版本号。
-- 这三个池只写在 raw.record、没有类型化表，不能靠 PRICE_TABLES 判断是否变化；同步运行结束后
-- 按任务的变化条数更新这一行，让依赖「当天涨停名单」的进程内缓存（市场概况、信号过去表现）失效。
-- 与 adj_factor 分开是因为涨停名单可以在行情没有变化时补上（例如池的同步滞后一天），
-- 此时市场概况的涨跌家数不变、涨停数却变了。
INSERT INTO ops.derived_version (name) VALUES ('limit_pool') ON CONFLICT DO NOTHING;
