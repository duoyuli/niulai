# 项目概览

本文说明 niulai 的目标、核心术语、仓库结构和进程，是阅读其他文档的公共前提。同步框架见 [同步流水线](architecture/sync_pipeline.md)，表的职责见 [数据库分层](architecture/data_model.md)。

## 目标

niulai 是面向 A 股股票和交易所 ETF 的研究和调仓工作台。数据来自同花顺金融数据服务，同步到本地 PostgreSQL：首次拉取全部历史，之后按同步计划增量。在此基础上提供复权行情、市场环境与热点、横截面因子检验与多因子组合回测，以及按因子排名每月调仓的账户建议，通过网页和 REST API 使用。数据源和技术栈的选择理由见 [ADR：采用同花顺金融数据服务](decisions/hithink_data_source.md) 和 [ADR：技术栈](decisions/technology_stack.md)。

<a id="glossary"></a>
## 核心术语

| 术语 | 含义 |
| --- | --- |
| thscode | 同花顺证券代码，带交易所后缀，例如 `600519.SH`、`920022.BJ`、`881101.TI`（同花顺板块指数）。所有表和接口都用它标识证券 |
| 数据集（dataset） | `backend/config/datasets.yaml` 中的一个条目：供应商的一个接口和一套同步规则，ID 形如 `a_share.bar_1d` |
| 优先级 P0–P3 | P0 当前功能必需；P1 A 股相关、建议积累；P2 其他资产；P3 低价值或按需。未显式配置时 P0、P1 开启，P2、P3 关闭 |
| 同步计划 | 数据集多久同步一次：每日收盘后、每周、每月、财报季、不自动同步，见 [定时调度](architecture/sync_pipeline.md#scheduling) |
| 运行（run）/ 任务（task） | 运行是一次同步批次（`ops.run`），有依赖的数据集分几批（波次）执行，每批一个运行；任务是一个请求单元（`ops.task`），例如某只股票的一个 10 年窗口 |
| 通用记录库 | `raw.record`：默认保留数据集的供应商原始字段（JSONB）；`store_record: false` 的大体量日线和快照只写类型化表 |
| 类型化表 / sink | `ref`、`mkt`、`fin` 下有明确列类型的表，例如日线、公司行动；数据集通过 `sink` 投影到这些表 |
| 公司行动 / 复权 | 分红、送转股、配股会让除权除息日的价格断崖；复权消除断崖，本项目的标准口径是等比复权，见 [复权](domains/price_adjustment.md) |
| 快照 | 全市场当前行情的一次截取（`a_share.snapshot`），收盘后取快照最快得到当天日线 |
| 因子 | 每只股票每天的一个数，用来在同一天的全市场股票之间排名（例如过去 5 日涨跌幅的相反数）；先检验排名能否预测之后的收益，再合成打分做组合，见 [因子研究](domains/factor_research.md) |
| 调仓建议 | 每个账户每个交易日一份「明天怎么操作」：按因子排名，每月最后一个交易日整体调仓，没有持满时补仓，见 [账户与调仓建议](domains/accounts.md) |

<a id="repository_layout"></a>
## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `backend/` | Rust 后端：`src/` 源码，`migrations/` 数据库迁移（表结构的唯一来源），`config/datasets.yaml` 数据集注册表，`tests/` 集成测试 |
| `frontend/` | React 单页应用，见 [前端](architecture/frontend.md) |
| `deploy/` | `Dockerfile`（同时构建前端和后端）、`compose.yml` 和 `.env.example`（Docker 部署），见 [部署](operations/runbook.md#deployment) |
| `docs/` | 本文档库；`docs/research/` 是调研笔记，不属于文档库 |
| `.github/workflows/ci.yml` | CI：检查代码；推送 `v*` 标签时构建多架构镜像 |
| `.agents/skills/` | 智能体技能目录（`.claude/skills` 是指向它的符号链接） |
| `.agents/skills/project-doc/` | 维护本文档库的 skill |
| `.agents/skills/architecture-review/` | 架构评审 skill：按 SOLID、组件耦合与内聚原则等通用架构原则输出只读评审报告 |
| `AGENTS.md` | 仓库说明与工作约定（`CLAUDE.md` 是指向它的符号链接） |

后端模块：`sync`（同步框架和领域事件）、`bootstrap`（应用初始化与事件编排）、`vendor`（供应商客户端与限速）、`analysis`（证券信息、分析价格、全市场计算的资源预算、市场环境、今日热点、个股基本面、派生数据版本、因子与因子研究、研究运行记录、交易成本和股票池）、`account`（账户、持仓、流水、估值、规则和建议重算的待办）、`advice`（因子排名、调仓建议、建议的处理、建议重算和建议回看）、`api`（REST API 和页面）、`auth`（用户、密码和登录会话）、`instrument`（股票与 ETF 的数据表和交易规则）、`operation_log`（操作日志）、`upgrader`（一键升级助手）、`setup`（首次登录引导）、`server`（常驻服务的组件编排）、`cache`（Redis 共享读取缓存）、`precompute`（首页结果预热）、`readiness`（核心数据是否就绪、增量同步是否在运行），以及 `config`、`db`、`telemetry`、`time`、`version` 等基础模块。

<a id="dependency_rules"></a>
依赖方向：`server` / CLI → `api` → `advice` → `account` → `analysis` → `instrument` 等基础模块；`bootstrap` 编排同步事件和建议；`sync` 只发布事实（领域事件），不依赖 `analysis`、`account`、`advice`、`api` 和 `cache`。`backend/src/lib.rs` 的架构适应度测试检查这些方向（`sync`、`analysis`、`account`、`advice` 不能引用上层模块，`error`、`cache`、`instrument`、`readiness` 等基础模块不能引用业务模块），同步状态表（`ops.task`、`ops.run`、`ops.dataset`）只在 `sync`、`readiness` 和同步管理接口 `api/sync.rs` 里查询，并检查每个 `AssertSqlSafe` 上一行都有「审计：」注释，违反时 `cargo test` 失败。业务模块要知道「数据齐没齐、同步在不在跑」时问 `readiness`，应用层要知道「这些运行写了什么」时调 `sync::run`。

<a id="processes"></a>
## 进程

后端只有一个程序 `niulai`：

| 子命令 | 职责 |
| --- | --- |
| `niulai serve` | 常驻服务：一个进程同时运行 REST API、页面、同步任务执行和定时调度，启动时执行数据库迁移 |
| `niulai upgrader` | 一键升级助手，在 Docker 部署的 upgrader 容器中运行，不连数据库，见 [一键升级](operations/runbook.md#one_click_upgrade) |
| 其他子命令 | 运维命令，执行完就退出：迁移（`migrate`）、校验并同步数据集注册表（`registry`）、规划运行（`plan`）、立即发起每日增量（`daily`）、在前台执行任务（`work`）、数据核检（`check`）、重算复权（`recompute`）、查看状态（`status`）、列出用户和重置密码（`user`）、导出 OpenAPI（`openapi`），见 `niulai --help` |

PostgreSQL 保存业务数据、任务队列、运行状态和进度，是服务的持久化事实来源。同步还依赖供应商 HTTP 接口；Redis 是可选的页面读取缓存，失效时可回退到 PostgreSQL。数据库不可用时服务不能就绪，同一时刻只有一个进程执行同步任务，限速在进程内，见 [ADR：单进程服务](decisions/single_process_service.md)。
