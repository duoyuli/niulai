# 项目概览

本文说明 niulai 的目标、核心术语、仓库结构和进程，是阅读其他文档的公共前提。同步框架的工作方式见 [同步流水线](architecture/sync_pipeline.md)，数据库表的职责见 [数据库分层](architecture/data_model.md)。

## 目标

niulai 是面向 A 股的个股诊断工作台。数据来自同花顺金融数据服务，同步到本地 PostgreSQL：首次拉取全部历史，之后每个交易日收盘后（T+1）增量同步。在此基础上提供复权行情、均线交叉信号（例如 15 日均线是否上穿 30 日均线）、全市场扫描、自选股诊断和均线策略回测，通过 REST API 和网页使用。

数据源和技术栈的选择理由分别见 [采用同花顺金融数据服务](decisions/hithink_data_source.md) 和 [技术栈决策](decisions/technology_stack.md)。

<a id="glossary"></a>
## 核心术语

| 术语 | 含义 |
| --- | --- |
| thscode | 同花顺证券代码，带交易所后缀，例如 `600519.SH`、`000001.SZ`、`920022.BJ`、`881101.TI`（同花顺板块指数）。本项目所有表和接口都用 thscode 标识证券 |
| 数据集（dataset） | `backend/config/datasets.yaml` 中的一个条目，对应供应商的一个接口和一套同步规则，ID 形如 `a_share.bar_1d` |
| 优先级 P0–P3 | P0：当前功能必需；P1：A 股相关、建议从现在开始积累；P2：其他资产；P3：低价值或按需。未显式配置时，P0 和 P1 默认开启，P2 和 P3 默认关闭 |
| 运行（run） | 一次同步批次，记录在 `ops.run`。一次请求的多个数据集按依赖关系拆成几个波次，每个波次是一个运行 |
| 任务（task） | 一个请求单元，例如某只股票的某个 10 年窗口、某个交易日，记录在 `ops.task` |
| 通用记录库 | `raw.record`：所有数据集的记录都以供应商原始字段（JSONB）保存在这里 |
| 类型化表 / sink | `ref`、`mkt`、`fin` 下有明确列类型的表，例如日线、公司行动。数据集通过配置中的 `sink` 投影到这些表 |
| 公司行动 / 除权除息 | 分红、送转股、配股等事件。事件发生日（除权除息日）股价会按规则下调，详见 [复权](domains/price_adjustment.md) |
| 复权 | 消除除权除息造成的价格断崖，使价格序列连续。本项目的标准口径是等比复权 |
| 快照 | 全市场当前行情的一次性截取（`a_share.snapshot`）。收盘后取快照，可以最快得到当天日线 |
| T+1 | 交易日收盘后同步当天数据 |

<a id="repository_layout"></a>
## 仓库结构

| 路径 | 内容 |
| --- | --- |
| `backend/` | Rust 后端。`src/` 源码，`migrations/` 数据库迁移（表结构的唯一来源），`config/datasets.yaml` 数据集注册表，`tests/` 集成测试，`.env` 本地配置（不提交） |
| `frontend/` | 前端：React 单页应用，`src/` 源码，`openapi.json` 是生成接口客户端用的后端规范，见 [前端](architecture/frontend.md) |
| `docs/` | 本文档库；`docs/research/` 是早期调研笔记，不属于文档库 |
| `deploy/` | CD：`Dockerfile`（同时构建后端和前端）、`compose.yml`（Docker 部署，内置 PostgreSQL 和 niulai）、`.env.example`；源码部署不用 Docker，直接编译运行，见 [部署](operations/runbook.md#deployment) |
| `.github/workflows/ci.yml` | CI：检查代码，构建多架构镜像并推送到 GitHub 镜像仓库 |
| `.agents/skills/project-doc/` | 维护本文档库所用的 skill；`.claude/skills/project-doc` 是指向它的符号链接 |
| `AGENTS.md` | 仓库说明与工作约定；`CLAUDE.md` 是指向它的符号链接 |
| `README.md` | 项目介绍和快速开始 |

后端代码按职责分为几个模块：`sync`（同步框架）、`vendor`（供应商客户端与限速）、`analysis`（均线、扫描、回测的计算）、`api`（REST API 和前端页面），以及 `config`、`db`、`time` 等基础模块。

<a id="processes"></a>
## 进程

后端只有一个程序 `niulai`：

| 子命令 | 职责 |
| --- | --- |
| `niulai serve` | 常驻服务。一个进程同时运行 REST API、同步任务执行和定时调度，启动时执行数据库迁移；配置了 `NIULAI_WEB_DIR` 时还提供前端页面 |
| 其他子命令 | 运维命令，执行完就退出：迁移、规划运行、在前台执行任务、查看状态、重算复权、导出 OpenAPI |

数据、任务队列和进度都在 PostgreSQL 中，不依赖其他服务；同一时刻只有一个进程执行同步任务，限速在进程内。为什么合并为一个进程见 [ADR：单进程服务](decisions/single_process_service.md)，命令用法见 [运维手册](operations/runbook.md)。
