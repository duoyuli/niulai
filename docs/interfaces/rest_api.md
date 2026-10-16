# REST API

本文说明 `niulai serve` 对外提供的 REST API：通用约定、接口清单、复权参数和错误格式。字段级的定义以 OpenAPI 规范为准（`GET /api/v1/openapi.json`，或运行 `niulai openapi`），本文只记录跨接口的约定和不容易从规范看出的行为。实现在 `backend/src/api/`。

<a id="conventions"></a>
## 通用约定

- 所有接口都在 `/api/v1` 下，响应是 JSON。默认监听 `127.0.0.1:8080`（`NIULAI_API_BIND`）。同一端口的其他路径是前端页面（配置了 `NIULAI_WEB_DIR` 时），未匹配的 `/api/` 路径返回 JSON 404，见 [由 niulai serve 提供](../architecture/frontend.md#serving)。
- **程序本身没有登录和访问控制**。默认只监听本机端口（`127.0.0.1:8080`），见 [部署](../operations/runbook.md#deployment)。
- 请求标识：每个响应都带 `x-request-id` 头。请求中带了这个头时沿用，否则生成一个 UUID；日志中的 `request_id` 与它一致。
- 超时：单个请求超过 120 秒返回 408。
- 数值格式：
  - 未复权价格和成交额是十进制字符串，保留数据库中的精度，例如 `"1312.9700"`、`"7537706223.71"`。
  - 复权价格也是字符串，保留 6 位小数。
  - 成交量是整数（股）。
  - 均线、复权因子是 JSON 数字（浮点）。
- 日期是 `yyyy-MM-dd`，按 Asia/Shanghai 解释；时间（如 `started_at`）是 UTC 的 RFC 3339 字符串。
- 列表接口都有返回上限，超出上限的参数会被截到上限。

<a id="endpoints"></a>
## 接口清单

| 方法与路径 | 说明 | 主要参数与上限 |
| --- | --- | --- |
| `GET /health` | 存活检查 | — |
| `GET /ready` | 就绪检查，能连上数据库时返回 200 | — |
| `GET /setup` | 首次登录引导的状态：API Key 是否已配置、来源、末尾 4 位、生成 Key 的网址、是否已规划回补 | 见 [首次登录引导](#setup) |
| `PUT /setup/api-key` | 验证并保存 API Key，立即生效 | 请求体 `{api_key}`；见 [首次登录引导](#setup) |
| `GET /securities` | 证券列表 | `q`（代码或名称片段）、`board`；`limit` 默认 50、最大 500；`offset` |
| `GET /securities/{thscode}` | 证券详情：简称历史、本地日线的范围和根数 | 不存在时返回 404 |
| `GET /securities/{thscode}/bars` | 个股日线 | `adjust`（见 [复权参数](#adjust_param)）、`start`、`end`；`limit` 默认 500、最大 10,000，返回范围内**最近**的 N 根，按日期升序 |
| `GET /securities/{thscode}/ma` | 均线与交叉信号 | `fast` 默认 15、`slow` 默认 30、`start`、`end`、`signals_only`；规则见 [均线与交叉信号](../domains/ma_signals.md) |
| `GET /signals/ma-cross` | 全市场某个交易日的交叉扫描，结果缓存到下一次复权重算 | `date` 默认本地最新交易日、`fast`、`slow`、`signal`（`golden` / `death`） |
| `GET /backtest/ma-cross` | 单只股票的均线交叉策略回测：汇总指标、交易记录、每日账户总值 | `thscode`（必填）、`fast`、`slow`、`start`、`end`、`capital` 默认 100,000；`commission_rate`、`min_commission`、`stamp_tax_rate` 默认万 2.5、5 元、千分之 0.5；规则见 [回测](../domains/backtest.md) |
| `GET /watchlist` | 自选股及诊断：最新收盘、近 1 日和 20 日涨跌、均线状态、最近一次交叉；按收藏时间倒序 | `fast`、`slow`，同均线接口；诊断规则见 [自选股诊断](../domains/ma_signals.md#watchlist) |
| `PUT /watchlist/{thscode}` | 收藏一只股票，重复收藏不报错；返回 204 | 证券不存在时返回 404 |
| `DELETE /watchlist/{thscode}` | 取消收藏，本来没有收藏也返回 204 | — |
| `GET /indices` | 指数与板块列表，含当前成分数 | `category`、`q`；最多 2,000 条 |
| `GET /indices/{thscode}/bars` | 指数日线（没有复权） | `start`、`end`、`limit`，同个股日线 |
| `GET /indices/{thscode}/members` | 指数成分 | `date` 默认今天；历史成分从开始采集那天起才有 |
| `GET /datasets` | 数据集列表：优先级、开关、最近完成时间、未完成和 `dead` 任务数 | — |
| `GET /datasets/{dataset_id}/records` | 通用记录查询，返回供应商原始字段 | `thscode`、`start`、`end`（按 `obs_date`）；`limit` 默认 100、最大 1,000；按 `obs_date` 倒序 |
| `GET /sync/runs` | 最近的运行 | `limit` 默认 20、最大 200 |
| `POST /sync/runs` | 发起运行，只规划任务，由执行循环执行；返回 202 | 请求体 `{kind, datasets, priorities, thscodes}`；`kind` 是 `backfill` 或 `incremental`；`datasets` 为空时按 `priorities` 选择已开启的数据集 |
| `GET /sync/runs/{run_id}` | 运行详情：各数据集进度，最多 50 条失败任务 | — |

`POST /sync/runs` 返回的 `run_id` 是第一个波次。后续波次是新的运行，`params.root_run` 指向它，见 [波次规划](../architecture/sync_pipeline.md#waves)。

<a id="setup"></a>
## 首次登录引导

同花顺金融数据服务的 API Key 一般不在部署时提供，而是第一次打开网页时配置，实现在 `backend/src/setup.rs`：

- `GET /setup` 的 `api_key_configured` 为 false 时，执行循环和定时调度暂停，前端把所有页面跳转到引导页。`api_key_url` 是生成 Key 的页面 <https://fuyao.aicubes.cn/admin/>。
- `PUT /setup/api-key` 先用这个 Key 请求一次交易日历接口：被拒绝时返回 400，连不上时返回 503，都不保存；成功后写入 `app.setting` 并立即生效。数据库里还没有任何回补运行时，同时规划 P0、P1 的全量回补，响应中的 `backfill_run_id` 是第一个波次的运行 ID。
- Key 由环境变量 `HITHINK_FINANCE_API_KEY` 提供时，`api_key_source` 为 `env`，`PUT` 返回 409。
- 响应中只有末尾 4 位（`api_key_hint`），不返回完整 Key。

为什么这样设计，见 [ADR：API Key 在首次登录引导中配置](../decisions/api_key_setup.md)。

<a id="adjust_param"></a>
## 复权参数

`/securities/{thscode}/bars` 的 `adjust` 取值为 `none`、`qfq`（默认）、`hfq`、`qfq_ths`、`hfq_ths`，各自对应的视图和用途见 [复权视图与 API 参数](../domains/price_adjustment.md#adjust_views)。只有 `qfq`、`hfq` 返回 `factor`。

<a id="errors"></a>
## 错误格式

错误响应体是 `{"code": "...", "message": "..."}`：

| HTTP 状态 | `code` | 含义 |
| --- | --- | --- |
| 400 | `bad_request` | 参数不合法，`message` 说明原因，例如 `adjust` 取值不对、`fast` 不小于 `slow`、回测区间内没有日线 |
| 404 | `not_found` | 证券、数据集或运行不存在 |
| 409 | `conflict` | 与当前状态冲突，例如 API Key 由环境变量提供时在网页上修改 |
| 500 | `internal` | 服务器内部错误；详细原因只写日志，响应中不包含 |
| 503 | `unavailable` | 依赖的外部服务暂时不可用，例如验证 API Key 时连不上同花顺接口 |

查询参数格式错误（例如日期不是 `yyyy-MM-dd`）由框架直接返回 400，响应体是纯文本。

<a id="openapi"></a>
## OpenAPI 与前端类型

- 规范版本是 OpenAPI 3.1，由 utoipa 从 Rust 代码生成，路径相对于 `/api/v1`。
- 新增或修改接口时，同时更新 `#[utoipa::path]` 注解，并在所属模块的 `routes()` 中用 `routes!` 注册。新增接口模块时，还要在 `backend/src/api/mod.rs` 的 `routes()` 中合并它；HTTP 路由和 OpenAPI 规范都从这一处生成。只用 `Router::route` 注册的路由不会出现在规范中。
- 每个响应都要写 `description`（OpenAPI 3.1 的要求），否则 orval 校验规范时失败。
- 前端用 orval 根据规范生成 TypeScript 类型和 TanStack Query hooks，更新步骤见 [接口客户端](../architecture/frontend.md#api_client)。
