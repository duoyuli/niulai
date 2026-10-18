# ADR：限速在进程内，同一时刻只有一个进程执行同步任务

- 状态：已采纳（accepted），2026-10-16。取代 [ADR：技术栈](technology_stack.md) §8.1 第 4 条（启用 Redis）、[ADR：单进程服务](single_process_service.md) 中「扩容」一条，以及 [ADR：采用同花顺金融数据服务](hithink_data_source.md) 后果中「限速必须跨进程共享」。
- 相关文档：[限速与单一执行进程](../architecture/sync_pipeline.md#rate_limit)、[配置](../interfaces/configuration.md)。

## 背景

引入 Redis 的理由是多个进程共用一个供应商 API Key：扩容出来的 `niulai serve --no-api`，以及前台的 `niulai work`，都要遵守同一个每秒请求上限，所以用 Redis 跨进程计数。

2026-10-16 架构审查时重新核对了这个前提：免费 Key 全局每秒只能请求约 2 次，实测单个进程、3 个并发已经达到每秒 1.7 次。多开进程只是让它们分享同一个上限，不会更快，扩容在这里没有意义。而为了它，部署要多依赖一个 Redis，代码里还要维护 Redis 连接失败时退回进程内限速的分支。退回之后多个进程合计会超过上限，这个分支本身也不安全。

## 决策

1. 去掉 Redis：删除 `redis` 依赖和 `REDIS_URL` 配置，限速只用进程内的 1 秒滑动窗口。
2. 同一时刻只允许一个进程执行同步任务。执行循环启动前，用一个独立的数据库连接取得会话级咨询锁 `pg_try_advisory_lock(hashtext('niulai.executor'))`（`Engine::lock_executor`）。取不到时 `niulai work` 直接退出并提示已有进程在执行；`niulai serve` 起初也是启动失败，2026-10-18 起改为先只提供 API、定期再试，并定期确认持锁的连接还在（见 [限速与单一执行进程](../architecture/sync_pipeline.md#rate_limit)）。连接关闭（进程退出）时锁自动释放。
3. 只提供 API 的进程（`niulai serve --no-worker`）不受限制，可以另外启动。

## 取舍

- **限速放在 PostgreSQL 中：** 不采用。可以跨进程，但多进程本身没有收益，加锁更简单，也不会出现两个进程互相等待的情况。
- **运维命令的请求：** `niulai plan`、`niulai daily` 规划时可能请求几次供应商接口（例如获取可选日期），它们不取锁，会与执行循环短暂叠加。次数很少，可以接受。

## 后果

- 部署只依赖 PostgreSQL。
- 想在 `serve` 运行时用 `niulai work` 手动执行任务，需要先停止 `serve`，或让 `serve` 以 `--no-worker` 运行。
- 将来换成更高额度的 Key、确实需要多进程执行时，再评估跨进程限速。
