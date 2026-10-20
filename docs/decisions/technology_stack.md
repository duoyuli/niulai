# ADR：技术栈（Rust REST API 与 PostgreSQL）

- 状态：已采纳（accepted），2026-10-15。
- 当前实现：[项目概览](../project_overview.md)、[前端](../architecture/frontend.md)、[REST API](../interfaces/rest_api.md) 和 [运维手册](../operations/runbook.md)。
- 后续细化：[前端技术选型](frontend_stack.md)、[单进程服务](single_process_service.md) 和 [限速在进程内](in_process_rate_limit.md)。

本文记录仍然适用于当前仓库的技术边界和选择原因。历史上被取代的进程拆分、Redis 和工具链比较不在本文重复；需要追溯时看 Git 历史和对应 ADR。

<a id="stack_choices"></a>
## 技术栈

| 层面 | 当前选择 | 作用 |
| --- | --- | --- |
| 语言与工具链 | Rust Edition 2024，版本由 `backend/rust-toolchain.toml` 固定 | 后端编译和运行 |
| 异步与 HTTP | Tokio、Axum、Tower、`tower-http` | REST API、任务执行、超时、请求标识和静态文件 |
| 数据库 | PostgreSQL（Docker 基线为 18.6） | 业务数据、行情、原始记录、任务状态和运行结果 |
| 数据库访问 | SQLx 0.9 | 连接池、事务、参数化 SQL、迁移和批量写入 |
| 数据格式 | serde、serde_json、serde-saphyr、Parquet | JSON、YAML 数据集注册表和公司行动导出 |
| 外部 HTTP | reqwest 0.13 + rustls | 供应商请求、超时、重试和限速 |
| 数值与时间 | `rust_decimal`、chrono、PostgreSQL `DATE` / `TIMESTAMPTZ` | 金额、价格、交易日和事件时间 |
| API 契约 | utoipa、OpenAPI、orval | 从 Rust 接口生成规范，再生成前端客户端 |
| 前端 | React、TypeScript、Vite、React Router、TanStack Query、Tailwind CSS、Lightweight Charts | 网页、数据请求和 K 线展示 |

具体依赖以 `backend/Cargo.toml`、`frontend/package.json` 和锁文件为准。版本号会变化，本文不记录发布日期、下载量或当时的维护排名。

<a id="code_organization"></a>
## 架构与代码组织

项目采用模块化单体。唯一的常驻程序是 `niulai serve`，同一进程提供 REST API、页面、同步任务执行和定时调度；一次只允许一个进程执行同步任务。模块职责见 [项目概览](../project_overview.md#repository_layout) 和 [同步流水线](../architecture/sync_pipeline.md)。

```text
浏览器 → Axum API → 共享业务模块 → SQLx → PostgreSQL
                         │
                         └─ 同步执行器 → 供应商接口
```

PostgreSQL 保存任务、租约、原始数据、类型化数据和业务结果，使服务重启后可以继续处理。后台任务按至少一次执行设计，依靠租约、幂等写入和失败重试处理重复执行；这些机制以同步文档和测试为准。

<a id="api_conventions"></a>
## API 与数据约定

- API 使用 `/api/v1` 版本路径，错误响应包含稳定错误码和 `x-request-id`。
- 数值字段按接口约定使用十进制字符串或安全整数，金额和价格在数据库中使用明确精度的 `NUMERIC`。
- 交易日使用业务时区解释，系统事件时间使用 `TIMESTAMPTZ`；供应商原始单位、采集时间和修订信息保留在数据层。
- Rust DTO 通过 utoipa 生成 OpenAPI，前端 `frontend/src/api/generated/` 由 orval 生成，生成文件不手改。
- 凭据通过环境变量或首次登录引导注入，不进入代码、日志和文档。

## 取舍

- 直接使用 SQL 保留批量写入、分区、窗口和查询计划的控制力，代价是需要维护 SQL、迁移和结果类型。
- 使用 PostgreSQL 任务机制减少额外基础设施；只有测量确认吞吐或资源隔离不足时，才评估消息队列或独立进程。
- 使用单进程降低部署复杂度，代价是 API、同步和计算共享进程资源；出现明确的扩容或故障隔离需求后再拆分。

<a id="extension_conditions"></a>
## 扩展条件

引入 Redis、专用消息队列、独立分析存储或独立服务前，先提供可复现的吞吐、延迟、故障恢复或资源隔离证据，并同步更新对应 ADR、现状文档和运维流程。没有证据时保持当前单进程、PostgreSQL 和本地计算边界。

## 历史变更

早期方案曾计划将 API 和 Worker 拆成多个进程，并用 Redis 共享供应商限速；这两项已分别由 [单进程服务](single_process_service.md) 和 [限速在进程内](in_process_rate_limit.md) 取代。早期的依赖维护状况核查属于时间点资料，当前依赖状态以锁文件和构建结果为准。
