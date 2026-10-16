# ADR：单进程服务

- 状态：已采纳（accepted），2026-10-15。取代 [ADR：技术栈](technology_stack.md) §4 中「API 与 Worker 使用独立启动入口、独立进程」的部分。「扩容」一条已被 [ADR：限速在进程内](in_process_rate_limit.md) 取代：同一时刻只有一个进程执行同步任务。
- 相关文档：[项目概览](../project_overview.md#processes)、[同步流水线](../architecture/sync_pipeline.md)、[运维手册](../operations/runbook.md)。

## 背景

后端首版有三个程序：`niulai-api`（REST API）、`niulai-worker`（执行同步任务和定时调度）、`niulai`（运维命令行）。拆分的理由是让长时间的采集任务不和查询请求争用资源，并且可以只扩容 worker。用户认为三个程序没有必要，要求合并为一个。

## 决策

1. 只保留一个程序 `niulai`。
2. 常驻服务是 `niulai serve`：一个进程同时运行 REST API、同步任务执行循环和定时调度，三者共用同一个同步引擎和数据库连接池。`--no-api`、`--no-worker`、`--no-scheduler` 可以关闭其中一部分。
3. 运维操作（迁移、规划、前台执行、查看状态、重算复权、导出 OpenAPI）是 `niulai` 的其他子命令，执行完就退出。
4. 部署时只有一个容器，运行 `niulai serve`。

## 取舍

- **资源争用**：同步任务主要是等待供应商响应的网络 I/O，吞吐受供应商限速（每秒 2 次）约束，与 API 查询争用的资源很少。CPU 较重的是运行结束后的复权重算（约 40 秒），它在数据库中执行，不占用 API 的处理线程。
- **扩容**：任务队列在 PostgreSQL 中，用 `FOR UPDATE SKIP LOCKED` 认领，限速通过 Redis 跨进程共享。需要扩容时再启动几个 `niulai serve --no-api` 即可，不需要拆分程序。
- **故障隔离**：API 和执行循环在同一个进程中，进程崩溃会同时影响两者。执行中的任务会在 10 分钟后被回收重试，API 由容器的 `restart: unless-stopped` 重启。目前接受这个风险。

## 后果

- 部署和运维只需要管理一个进程和一个镜像。
- 停机时，API 停止接受新请求，执行循环等正在执行的任务结束后退出。
- 以后如果出现需要隔离的重计算（例如回测），再评估是否在 `serve` 之外单独运行，或用 `--no-api` 的独立实例承担。
