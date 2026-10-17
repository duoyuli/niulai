# ADR：技术栈（Rust REST API 与 PostgreSQL）

- 状态：已采纳（accepted），2026-10-15。§4 中「API 与 Worker 使用独立进程」部分已被 [ADR：单进程服务](single_process_service.md) 取代；§8.1 第 4 条（启用 Redis）已被 [ADR：限速在进程内](in_process_rate_limit.md) 取代。
- 复核：同日完成维护状况复核，调整见 §2.1 和附录 A。
- 实施：前后端均已实现，与计划不同的地方见 §8.1；前端选型见 [ADR：前端技术选型](frontend_stack.md)。
- 相关文档：[同步流水线](../architecture/sync_pipeline.md)、[REST API](../interfaces/rest_api.md)、[配置](../interfaces/configuration.md)、[运维手册](../operations/runbook.md)。

本文记录从零建设 Niulai 前后端分离应用的技术选型，供后续开发和评审使用。后端使用 Rust，提供 RESTful API，连接 PostgreSQL；前端组合记录为推荐基线，界面工具尚待细化。

## 章节导航

- [§2 技术栈](#stack_choices)：查某个组件选了什么、为什么不用另一个时。
- [§4 架构与代码组织](#code_organization)、[§5 API 与数据约定](#api_conventions)：调整模块边界、接口或数值类型约定时。
- [§8 后续扩展条件](#extension_conditions)：考虑引入登录、消息队列、分析存储等组件时。
- [附录 A](#maintenance_check)：升级依赖前，查当时的维护状况。

## 1. 背景与目标

项目面向小团队的数据共享、日线研究和中短期回测。当前优先完成数据采集、存储、查询和管理，后续逐步增加研究、回测及协作功能。

选型优先考虑生态延续性、类型与接口契约、SQL 可控性、任务恢复和部署维护成本。前瞻性通过清晰的边界和可验证的演进条件实现。

本文确定技术组合和工程边界，不承诺尚未测试的吞吐量、查询延迟或故障恢复能力。

<a id="stack_choices"></a>
## 2. 技术栈

| 层面 | 决策 | 用途与约束 |
| --- | --- | --- |
| 语言与工具链 | Rust stable，Edition 2024 | 初始化时固定具体工具链版本 |
| 异步运行时 | Tokio | 统一处理网络、数据库和异步 I/O |
| HTTP 框架 | Axum，优先采用 0.8 系列的兼容版本 | 提供路由、请求提取、响应和应用状态管理 |
| HTTP 中间件 | Tower 0.5 + `tower-http` **0.6.x** | 按需配置请求追踪、超时、请求体限制和 CORS；`tower-http` 与 Axum 0.8 依赖的 0.6 系列保持一致 |
| 数据库 | PostgreSQL 18 作为新部署基线 | 保存业务数据、行情数据、任务状态与实验元数据；现有 SQL 已在 18.6 上验证 |
| 数据库访问 | SQLx 0.9 | 使用 PostgreSQL 驱动、内置连接池、事务、参数化 SQL 和 `COPY`（批量导入日线） |
| 数据库迁移 | SQLx migrations / sqlx-cli 0.9 | 使用版本化 SQL 管理表结构变更 |
| API 契约 | utoipa 6 + utoipa-axum 0.3 + OpenAPI | 从 Rust DTO 与接口注解生成接口规范 |
| JSON | serde + serde_json | 请求、响应和配置的序列化与反序列化 |
| YAML（数据集注册表） | **serde-saphyr** | 读取 `backend/config/datasets.yaml`；不使用已废弃的 serde_yaml 和有安全公告的 serde_yml |
| Parquet | **parquet**（Apache arrow-rs） | 读取供应商公司行动导出文件 |
| 错误处理 | thiserror + anyhow | 定义可分类的业务错误，保留基础设施与任务错误上下文 |
| 日志 | tracing + tracing-subscriber | 记录结构化日志，关联请求、任务和外部调用 |
| 外部 HTTP | reqwest 0.13，使用 rustls | 连接供应商 API，明确配置超时、并发和重试 |
| 日期与数值 | chrono + rust_decimal | 处理业务日期、时间戳和十进制数据；不引入 chrono-tz，交易所时区换算在 PostgreSQL 中完成 |

这些组件的具体版本和 feature 在工程初始化时验证并锁定。分别存在稳定版本，不代表任意版本组合都兼容；尤其需要核对 Axum、Tower、utoipa 及其集成组件的依赖关系。已确认的兼容关系见附录 A。

前端推荐使用 **React 19 + TypeScript（暂锁 6.x）+ Vite + TanStack Query**：Vite 负责开发与构建，TanStack Query 管理服务端数据的获取、缓存和刷新。路由、组件库、表格和 K 线图工具根据首批页面需求另行确定。

### 2.1 复核后的调整（2026-10-15）

| 调整 | 原因 |
| --- | --- |
| `tower-http` 使用 0.6.x，而不是最新的 0.7 | Axum 0.8.9 依赖 `tower-http ^0.6.8`。两条版本线依赖相同的 `http 1`、`tower 0.5`，混用也能工作，但会编译两份。0.6 系列仍有补丁（0.6.11，2026-05）。Axum 升级到新版本时再一起切换 |
| 新增 serde-saphyr 解析 YAML | `serde_yaml` 已由作者标为废弃（0.9.34+deprecated，2024 年后不再更新）；分支 `serde_yml` 有 RustSec 公告 RUSTSEC-2025-0068（unsound，unmaintained）。serde-saphyr 近 12 个月有 19 位作者、1,000 多次提交，2026-10 发布 2.0 |
| 新增 parquet | 同步方案中公司行动数据来自 Parquet 导出文件。arrow-rs 是 Apache 项目，近 12 个月有 252 位贡献者 |
| 不使用 chrono-tz | chrono-tz 近 12 个月没有提交。供应商毫秒时间戳转交易日，已由 PostgreSQL 的 `ops.ms_to_cn_date()` 处理，并验证了 1986–1991 年的夏令时。jiff 时区支持更好，但 SQLx 0.9 没有 jiff 特性，所以 Rust 侧仍用 chrono |
| TypeScript 暂锁 6.x（当前 6.0.3） | TypeScript 最新版是 7.0（Go 重写的版本），而 openapi-typescript 7.13 的 peer 依赖是 `typescript ^5.x`，`@hey-api/openapi-ts` 只声明支持到 6。初始化时要验证 orval 能在 6.x 下工作；如果退回 openapi-typescript，就需要锁 5.9。等代码生成工具支持 7 以后再升级 |
| 前端类型生成首选 **orval**，openapi-typescript 作为备选 | openapi-typescript 最近一次发布是 2026-02，近 12 个月人工提交只有 39 次，其余是依赖更新机器人。orval 近 12 个月人工提交 992 次，2026-10-14 发布 8.43，可以直接从 OpenAPI 生成 TanStack Query hooks，正好匹配前端选型。§3.3 的链路相应调整 |

## 3. 主要决策依据

### 3.1 使用 Axum 与 Tokio

Axum 与 Tokio、Hyper 配合，并使用 Tower 的中间件体系。选择这一组合，可以在统一的运行时和中间件模型下组织 HTTP、数据库及外部网络调用。[Axum 官方文档][axum]

路由处理函数负责协议转换、输入校验和调用业务服务。业务规则放在可由 API 与 Worker 共同调用的模块中。

Actix Web 是可行的替代方案；本次选择 Axum 的依据是组件组合方式及与 Tokio、Tower 的一致性。当前没有运行框架性能对比测试。

### 3.2 使用 SQLx

数据采集与研究查询需要明确控制批量写入、聚合、窗口函数、索引和查询计划，因此采用直接编写 SQL 的方式。SQLx 提供异步访问、连接池和事务，并允许使用 PostgreSQL 的原生能力。[SQLx 官方仓库][sqlx]

SQLx 的编译期检查适用于 `query!`、`query_as!` 等宏覆盖的静态查询；动态查询仍需通过执行测试验证。查询参数使用绑定机制，动态标识符及排序字段使用白名单。

使用查询宏时，需要开发数据库或离线查询元数据。计划将 `.sqlx` 纳入版本控制，并在 CI 中使用与迁移一致的 PostgreSQL 检查元数据，避免构建依赖个人开发数据库。[SQLx CLI 文档][sqlx-cli]

代价是需要自行维护 SQL、查询结果类型和迁移。SeaORM 更适合围绕实体关系展开的大量 CRUD；如果未来业务形态发生明显变化，再评估是否调整访问方式。

### 3.3 使用 OpenAPI 连接前后端

接口契约按以下链路维护：

```text
Rust 请求／响应 DTO
    → utoipa 生成 OpenAPI（提交到仓库，作为契约的唯一来源）
    → orval 生成 TypeScript 类型和 TanStack Query hooks
    → 前端直接使用生成的 hooks
```

utoipa 支持从代码生成 OpenAPI；orval 根据 OpenAPI 生成类型和请求函数。具体规范版本以两端工具的兼容结果为准。如果 orval 生成的代码不符合需要，可以退回 openapi-typescript 加 openapi-fetch，契约文件不变。[utoipa 文档][utoipa]、[orval 文档][orval]、[openapi-typescript 文档][openapi-typescript]

utoipa 的提交主要来自一位维护者（近 12 个月 juhaku 20 次，其余贡献者共约 20 次），存在单点风险，但 Rust 侧没有更活跃的替代：aide 最近一次发布在 2025-08。因为 OpenAPI 文件提交在仓库里，即使以后更换生成工具，前端和契约也不受影响。

接口变更需要同步更新规范和生成类型，并通过前端类型检查。API DTO 与数据库行类型分开，避免表结构变更直接成为接口变更。生成的 TypeScript 类型提供编译期约束，服务端仍须执行输入校验和权限检查。

<a id="code_organization"></a>
## 4. 架构与代码组织

采用模块化单体：同一仓库内维护共享业务模块，API 与 Worker 使用独立启动入口、独立进程，可以先部署在同一台机器上。

> 2026-10-15 更新：独立进程的部分已被 [ADR：单进程服务](single_process_service.md) 取代。现在只有一个程序 `niulai`，`niulai serve` 在一个进程中运行 API、任务执行和定时调度；下文的职责划分仍然适用，只是不再分成多个进程。

```text
前端 → Axum API → 共享业务模块 → SQLx → PostgreSQL
                      ↑
                后台 Worker → 外部数据源
```

职责划分如下：

- API：处理查询、创建任务、读取任务进度和结果。
- Worker：执行采集、补数和后续的回测等长任务。
- 共享业务模块：维护业务规则、数据转换、事务边界和数据库访问。
- PostgreSQL：保存任务状态、业务数据和结果，使状态可以在进程重启后恢复。

建议的初始后端结构如下，是决策时的计划布局。实际结构见 [项目概览](../project_overview.md#repository_layout)：

```text
backend/
├── Cargo.toml
├── migrations/
├── src/
│   ├── lib.rs
│   ├── bin/
│   │   ├── api.rs
│   │   └── worker.rs
│   ├── modules/
│   │   ├── securities/
│   │   ├── market_data/
│   │   └── ingestion/
│   ├── config.rs
│   └── telemetry.rs
└── tests/
```

每个业务模块内部按需要组织 HTTP、业务逻辑和数据库访问。先使用一个后端 Cargo 包，模块稳定且出现独立复用或编译边界需求后，再拆分 workspace crates。

### 后台任务的可靠性边界

首版使用 PostgreSQL 保存任务及执行状态。任务实现需要包含领取、租约、重试、幂等和取消语义；`tokio::spawn` 只负责进程内并发，任务恢复依赖持久化状态和恢复逻辑。

- 使用短事务领取任务；供应商 HTTP 调用在领取事务提交后执行。
- 按至少一次执行设计，使用任务标识、业务唯一键和幂等写入处理重复执行。
- 租约续期及任务完成更新需要校验当前执行者，避免过期 Worker 覆盖新执行结果。
- 明确结果写入与任务完成之间的事务关系，覆盖崩溃后重新执行的情况。
- CPU 密集计算使用受控的计算线程池或计算进程，并限制并发，保护 API 的 I/O 处理能力。

这些是实施要求，需要通过重启、重试和租约失效测试验证。已有任务表和 SQL 是否满足要求，要在实现阶段逐项核对。

<a id="api_conventions"></a>
## 5. API 与数据约定

### 5.1 REST API

- 使用版本化路径，例如 `/api/v1/securities`。
- 查询、创建、修改和删除使用对应的 HTTP 方法与状态码；错误响应保留稳定错误码和请求关联标识。
- 长任务创建后返回任务标识及状态查询地址；任务状态、进度和结果可单独查询。
- 列表接口明确排序、分页、日期范围和最大返回量，避免一次返回全部历史数据。
- 日志与响应之间使用请求标识关联；内部错误保留排障上下文，对外响应不暴露连接信息或凭据。
- 在配置中统一设置连接池、数据库查询及外部 HTTP 的超时和并发限制。

### 5.2 数值、时间与版本

| 数据 | 约定 |
| --- | --- |
| 需要十进制精度的价格、金额等 | PostgreSQL 使用有明确精度与标度的 `NUMERIC`，Rust 使用 `Decimal`；核对取值范围能被 `rust_decimal` 表达 |
| 精确十进制 API 字段 | 明确序列化为字符串，OpenAPI 与实际 JSON 保持一致 |
| 整数成交量等 | 按字段范围使用整数类型；可能超出 JavaScript 安全整数范围的值在 API 中使用字符串 |
| 技术指标与统计计算 | 根据算法要求使用 `f64`，明确精度、舍入和类型转换位置 |
| 交易日、报告期等业务日期 | 使用 PostgreSQL `DATE`；按业务日历及交易所时区解释 |
| 采集、任务与系统事件时间 | 使用 PostgreSQL `TIMESTAMPTZ`，接口统一输出 UTC 时间戳 |
| 原始数据与修订 | 保留来源、原始单位、采集时间及修订信息 |
| 实验结果 | 记录输入范围、数据版本或快照、参数、代码版本和执行假设 |

API 使用十进制字符串并不能恢复供应商已丢失的精度。采集时需明确 JSON 数值解析、单位换算和落库舍入规则，避免经过不必要的 `f64` 转换。

## 6. 工程基线

1. 使用 `rust-toolchain.toml` 固定工具链，提交应用的 `Cargo.lock`，构建时使用锁定依赖。
2. 配置通过强类型结构读取并在启动时校验；开发配置提供示例文件，凭据通过环境变量或部署环境注入。
3. 使用 `rustfmt`、`clippy` 和 `cargo test`；数据库集成测试使用与部署目标一致的 PostgreSQL。
4. CI 验证空库迁移、关键查询、SQLx 查询元数据、OpenAPI 生成结果及前端类型检查。
5. 日志至少关联请求标识、任务标识、操作、耗时和结果；数据供应商凭据不进入日志。
6. 提供存活与就绪检查、优雅停机；停机时停止领取新任务，并按约定结束或释放正在执行的任务。
7. 首版使用 Docker Compose 组织部署，提供数据库备份与恢复步骤，并实际验证恢复。

## 7. 与现有数据方案的关系

配套数据方案为：

- [数据目录](../interfaces/dataset_catalog.md) 与 [同步流水线](../architecture/sync_pipeline.md)
- [数据库分层](../architecture/data_model.md) 与 [复权](../domains/price_adjustment.md)
- [ADR：采用同花顺金融数据服务](hithink_data_source.md)
- 迁移文件：`backend/migrations/20261015000001_sync.sql`、`backend/migrations/20261015000002_market.sql`

已有文档与 SQL 标注 PostgreSQL 16，该版本作为当时数据方案的环境记录保留。本决策选择 PostgreSQL 18 作为新部署目标（当前 18.6，社区支持到 2030-11；19 仍为 beta）。2026-10-15 在 PostgreSQL 18.6 上验证了以下内容：
- `001_sync.sql` 和 `002_market.sql` 能在空库建成；
- `rebuild_adj_factor`、`rebuild_adj_segment`、`raw.upsert_records` 和 `ops.ms_to_cn_date` 都能正常执行；
- 3 只样本股的前复权结果与 PostgreSQL 16 逐位一致。

全量数据在 18 上的性能尚未测试。

`backend/migrations/` 中的 SQL 是后续整理迁移的输入。初始化工程时需核对现有数据库状态，将结构纳入 SQLx migrations，并明确新库初始化与已有库接管方式。历史上已执行的 SQL 不能仅靠改名就视为已完成迁移登记。

本文不重新确认供应商权限、覆盖范围或数据完整性，也不替代数据方案中的核查结论。

<a id="extension_conditions"></a>
## 8. 后续扩展条件与实施顺序

| 事项 | 何时细化或引入 |
| --- | --- |
| 登录与权限 | 团队共享入口交付前确定身份、会话和资源归属；根据部署与用户体系选择具体组件 |
| 前端路由、组件库和图表 | 已选定，见 [ADR：前端技术选型](frontend_stack.md) |
| 参数校验、配置加载组件 | 初始化工程时按 DTO、配置来源和兼容性选择，保持各模块使用方式一致。候选：配置用 config-rs（活跃，2026-09 有发布；figment 自 2024-05 以后没有发布，不建议）；校验用 validator 或 garde（validator 近 12 个月人工提交只有 11 次，初始化时再比较） |
| 定时调度 | 需要 cron 表达式时，在执行循环中用 croner 解析，调度状态存 PostgreSQL；也可以用部署环境的 cron 或 systemd timer 触发 |
| Redis | 实际出现跨实例缓存、限流或高频共享状态需求时评估 |
| 专用消息队列 | PostgreSQL 任务机制在吞吐、分发或运维上出现经测量确认的限制时评估 |
| 分析存储、Parquet 或 DuckDB | 批量扫描、导出或回测数据访问测试显示明确收益时评估 |
| OpenTelemetry 与集中监控 | 跨进程追踪和集中排障需求出现时，在 tracing 基础上接入 |
| 独立服务 | 模块出现独立扩缩容、部署或资源隔离需求时拆分 |

建议按以下顺序落地：

1. 初始化 Rust 工程、PostgreSQL 连接、配置、日志和健康检查。
2. 整理数据库迁移，实现一个数据集的写入与 REST 查询。
3. 生成 OpenAPI 与 TypeScript 类型，完成前端展示闭环。
4. 接入持久化采集任务，验证幂等、失败重试、重启恢复和任务归属。
5. 根据首版实际使用情况增加管理、研究与回测功能。

### 8.1 实施记录（2026-10-15）

第 1 至 4 步已于 2026-10-15、16 完成。当前的代码结构、进程和部署见 [项目概览](../project_overview.md)，此处只保留实施时与本文计划不同的地方。

与本文计划不同的地方：

1. **SQL 写法**：没有使用 SQLx 的编译期检查宏（`query!`），因此暂时没有 `.sqlx` 离线元数据。大多数 SQL 需要按配置动态拼接，例如证券范围、复权视图名、均线窗口。拼接处都用 SQLx 0.9 的 `AssertSqlSafe` 标记，并在注释里说明安全理由；静态 SQL 由集成测试覆盖。如果以后静态查询增多，再引入宏和离线元数据。
2. **没有用 dotenvy**：它最近一次发布是 2023 年，改用约 20 行代码读取 `.env`，再交给 config-rs 处理。
3. **没有用 croner**：调度只有两个时间点（每个交易日收盘后、每周一次，2026-10-17 起每周的时间单独设置），每月跟着每周走，用几行日期计算和 PostgreSQL 的 `date_trunc` 就能判断，没有用到 cron 表达式。
4. **启用了 Redis**：多个进程（扩容的 `niulai serve`、前台的 `niulai work`）共用一个供应商 API Key，需要全局限速，满足 §8 中「跨实例限流」的引入条件。Redis 不可用时自动退回进程内限速。（已被 [ADR：限速在进程内](in_process_rate_limit.md) 取代，Redis 已去掉。）
5. **前端**（2026-10-16 补充）：已用 orval 从 `niulai openapi` 生成类型和 TanStack Query hooks，完成第 3 步的前端部分，见 [前端](../architecture/frontend.md)。

## 9. 官方资料

- [Rust Edition 2024][rust-2024]
- [Axum][axum]
- [Tokio][tokio]
- [SQLx][sqlx] 与 [SQLx CLI][sqlx-cli]
- [PostgreSQL 版本支持政策][postgresql]
- [utoipa][utoipa]
- [openapi-typescript][openapi-typescript]
- [Vite][vite]
- [TanStack Query][tanstack-query]

<a id="maintenance_check"></a>
## 附录 A：维护状况核查（2026-10-15）

数据来源：GitHub API（star 数、最近推送、最新 release、近 12 个月的提交）、crates.io API（最新稳定版、近 90 天下载量）、npm registry（最新版、周下载量）。各项数字只反映当天的情况。

### A.1 后端（Rust）

| 组件 | GitHub | Star | 最新稳定版（日期） | 近 90 天下载 | 结论 |
| --- | --- | --- | --- | --- | --- |
| Rust 工具链 | rust-lang/rust | 120k | 1.99.0（2026-10-01） | — | 保留，Edition 2024 |
| Tokio | tokio-rs/tokio | 33.4k | 1.53.3（2026-10-13） | 2.6 亿 | 保留 |
| Axum | tokio-rs/axum | 27.5k | 0.8.9（2026-04-14），主分支当天有提交 | 1.3 亿 | 保留；对比 Actix Web（24.9k，4.17.0），Axum 的下载量更高，并且与 Tokio、Tower 同属一个生态 |
| Tower / `tower-http` | tower-rs | 4.3k / 0.9k | 0.5.3 / 0.6.11 与 0.7.1 | 2.0 亿 / 1.8 亿 | 保留，`tower-http` 用 0.6.x |
| SQLx | transact-rs/sqlx（原 launchbadge） | 17.6k | 0.9.0（2026-05-21） | 4,600 万 | 保留；SeaORM（9.9k，2.0.4）适合以 CRUD 为主的场景，不适合本项目 |
| utoipa / utoipa-axum | juhaku/utoipa | 4.1k | 6.0.0 / 0.3.0（2026-09-22） | 1,800 万 | 保留，注意单点维护风险；aide（0.7k，最近发布在 2025-08）更不活跃 |
| serde / serde_json | serde-rs | 10.9k / 5.7k | 1.0.229 / 1.0.154 | 3.6 亿 | 保留 |
| thiserror / anyhow | dtolnay | 5.6k / 6.7k | 2.0.21 / 1.0.104 | 4.3 亿 / 2.4 亿 | 保留 |
| tracing | tokio-rs/tracing | 6.9k | 0.1.44 / subscriber 0.3.23 | 2.2 亿 | 保留 |
| reqwest / rustls | seanmonstar / rustls | 11.9k / 7.7k | 0.13.5 / 0.23.46 | 2.2 亿 / 2.3 亿 | 保留 |
| chrono | chronotope/chrono | 3.9k | 0.4.45（2026-06-04） | 2.0 亿 | 保留；维护节奏较慢但稳定，是 SQLx 原生支持的类型 |
| rust_decimal | paupino/rust-decimal | 1.3k | 1.43.0（2026-09-02） | 3,900 万 | 保留；近 12 个月有两位主要维护者；28 位有效数字，能覆盖现有 `NUMERIC(24,x)` 字段 |
| serde-saphyr（新增） | bourumir-wyngs/serde-saphyr | 0.2k | 2.0.0（2026-10-13） | 780 万 | 新增；star 不多，但提交活跃、贡献者多。备选方案是把注册表改为 TOML，使用 toml-rs（Cargo 本身在用） |
| parquet（新增） | apache/arrow-rs | 3.6k | 60.0.0（2026-10-15） | 2,000 万 | 新增 |

被排除的：`serde_yaml`（已废弃）、`serde_yml`（RUSTSEC-2025-0068）、`chrono-tz`（近 12 个月无提交）、`figment`（2024-05 之后没有发布）。

### A.2 数据库

PostgreSQL 18.6 是当前最新的正式版本，社区支持到 2030-11-14；PostgreSQL 19 截至 2026-09-24 是 Beta 4。保留 18。

### A.3 前端

| 组件 | 最新版（日期） | 周下载 | 结论 |
| --- | --- | --- | --- |
| React | 19.3.1（2026-10-14） | 1.9 亿 | 保留；仓库已迁到 react/react |
| TypeScript | 7.0.2（2026-07-08） | 3.1 亿 | 暂锁 6.x，等代码生成工具支持 7 |
| Vite | 8.3.4（2026-10-08） | 2.0 亿 | 保留 |
| TanStack Query | 5.104.3（2026-10-15） | 6,500 万 | 保留 |
| orval | 8.43.0（2026-10-14） | 245 万 | **改为首选**，近 12 个月人工提交 992 次 |
| openapi-typescript / openapi-fetch | 7.13.0 / 0.17.0（2026-02-11） | 950 万 / 1,100 万 | 改为备选；下载量大，但近 12 个月人工提交只有 39 次 |
| `@hey-api/openapi-ts` | 0.99.0（2026-06-22） | 540 万 | 不选；很活跃，但仍是 0.x 版本，接口变化频繁 |

[orval]: https://orval.dev/
[rust-2024]: https://doc.rust-lang.org/edition-guide/rust-2024/index.html
[axum]: https://docs.rs/axum/latest/axum/
[tokio]: https://tokio.rs/
[sqlx]: https://github.com/transact-rs/sqlx
[sqlx-cli]: https://github.com/transact-rs/sqlx/blob/main/sqlx-cli/README.md
[postgresql]: https://www.postgresql.org/support/versioning/
[utoipa]: https://docs.rs/utoipa/latest/utoipa/
[openapi-typescript]: https://openapi-ts.dev/introduction
[vite]: https://vite.dev/guide/
[tanstack-query]: https://tanstack.com/query/latest/docs/framework/react/overview
