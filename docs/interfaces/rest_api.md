# REST API

本文说明 `niulai serve` 对外提供的 REST API：通用约定、登录、接口清单、复权参数和错误格式。字段级的定义以 OpenAPI 规范为准（`GET /api/v1/openapi.json`，或运行 `niulai openapi`），本文只记录跨接口的约定和不容易从规范看出的行为。实现在 `backend/src/api/`。

<a id="conventions"></a>
## 通用约定

- 所有接口都在 `/api/v1` 下，响应是 JSON。默认监听 `127.0.0.1:8080`（`NIULAI_API_BIND`）。同一端口的其他路径是前端页面（配置了 `NIULAI_WEB_DIR` 时），未匹配的 `/api/` 路径返回 JSON 404，见 [由 niulai serve 提供](../architecture/frontend.md#serving)。
- **要先登录**：除健康检查和登录本身，所有接口都要带登录 Cookie，否则返回 401，见 [登录](#auth)。默认只监听本机端口（`127.0.0.1:8080`），见 [部署](../operations/runbook.md#deployment)。
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
| `POST /auth/login` | 登录，成功时设置登录 Cookie | 请求体 `{username, password}`；见 [登录](#auth) |
| `POST /auth/logout` | 退出登录，清除 Cookie；返回 204 | — |
| `GET /auth/me` | 当前登录的账号，以及是否仍在使用默认账号密码 | 没有登录时返回 401 |
| `GET /system/version` | 当前版本、GitHub 上的最新版本、一键升级的状态（upgrader 是否在运行、进度、失败时的日志） | `refresh=true` 时不用 6 小时的缓存，立即查询；见 [一键升级](../operations/runbook.md#one_click_upgrade) |
| `POST /system/upgrade` | 一键升级：把版本号交给 upgrader 容器，返回 202 | 请求体 `{version}`，为空时升级到最新版本；只接受 `X.Y.Z` 且高于当前版本；upgrader 没有运行或正在升级时返回 409 |
| `GET /setup` | 首次登录引导的状态：API Key 是否已配置、来源、末尾 4 位、生成 Key 的网址、是否已规划回补 | 见 [首次登录引导](#setup) |
| `PUT /setup/api-key` | 验证并保存 API Key，立即生效 | 请求体 `{api_key}`；见 [首次登录引导](#setup) |
| `GET /securities` | 证券列表 | `q`（代码或名称片段）、`board`；`limit` 默认 50、最大 500；`offset` |
| `GET /securities/{thscode}` | 证券详情：简称历史、本地日线的范围和根数 | 不存在时返回 404 |
| `GET /securities/{thscode}/bars` | 个股日线 | `adjust`（见 [复权参数](#adjust_param)）、`start`、`end`；`limit` 默认 500、最大 10,000，返回范围内**最近**的 N 根，按日期升序 |
| `GET /securities/{thscode}/ma` | 均线与交叉信号 | `fast` 默认 15、`slow` 默认 30、`start`、`end`、`signals_only`；规则见 [均线与交叉信号](../domains/ma_signals.md) |
| `GET /signals/ma-cross` | 全市场某个交易日的交叉扫描，结果缓存到下一次复权重算 | `date` 默认本地最新交易日、`fast`、`slow`、`signal`（`golden` / `death`） |
| `GET /backtest/ma-cross` | 单只股票的均线交叉策略回测：汇总指标、交易记录、每日账户总值 | `thscode`（必填）、`fast`、`slow`、`start`、`end`、`capital` 默认 100,000；`commission_rate`、`min_commission`、`stamp_tax_rate` 默认万 2.5、5 元、千分之 0.5；规则见 [回测](../domains/backtest.md) |
| `GET /backtest/strategy` | 单只股票的选股策略回测，响应结构同均线交叉回测，交易记录带卖出原因 | `strategy_id`、`thscode`（必填），其余同均线交叉回测；规则见 [回测](../domains/backtest.md#rules) |
| `GET /backtest/portfolio` | 全市场组合回测：每天用策略选股，最多同时持有若干只 | `strategy_id`（必填）、`start` 默认结束日前 3 年、`end` 默认最新交易日，区间最长 5 年；`max_positions` 默认 5、范围 1 到 20；资金和成本同上；最多返回最近 500 笔交易；规则见 [全市场组合回测](../domains/backtest.md#portfolio) |
| `GET /strategies/catalog` | 指标、信号（含白话说明）、模板目录和默认股票池，前端据此生成策略编辑器和「指标与信号」页 | — |
| `GET /strategies` | 全部策略，含白话说明（`explained`）和最近一次巡检的数量 | — |
| `POST /strategies` | 新建策略，返回 201 | 请求体 `{name, description, definition, screening, show_on_today, template}`，`screening`、`show_on_today` 默认 true；名称 1 到 40 字；定义的校验见 [策略定义](../domains/strategies.md#definition) |
| `GET` / `PUT` / `DELETE /strategies/{id}` | 读取、修改（请求体同新建）、删除策略；删除返回 204 | 不存在时返回 404 |
| `POST /strategies/preview` | 试算：不保存，在某个交易日上执行一个策略定义 | 请求体 `{definition, date}`，`date` 默认最新交易日 |
| `GET /strategies/{id}/results` | 某次巡检的结果：入选股票、命中条件、是否新入选、上次入选这次不在的股票，以及有结果的交易日 | `date` 默认最近一次巡检 |
| `POST /screening/run` | 立即巡检 | `strategy_id` 默认全部开启巡检的策略、`date` 默认最新交易日 |
| `GET /securities/{thscode}/diagnosis` | 个股诊断：四个维度的结论、最近 20 个交易日的信号、关键价位、当天符合的策略 | 规则见 [个股诊断](../domains/strategies.md#diagnosis) |
| `GET /market/overview` | 市场概况：涨跌家数、涨跌停数、涨跌幅中位数、站上 20 日均线的比例 | `date` 默认最新交易日 |
| `GET /market/sectors` | 板块强弱排名 | `category`（`industry` 默认 / `cn_concept`）、`days` 默认 5、范围 1 到 60、`date`；规则见 [市场环境](../domains/strategies.md#market) |
| `GET /watchlist` | 自选股及诊断：最新收盘、近 1 日和 20 日涨跌、均线状态、最近一次交叉、最近 5 个交易日的信号；按收藏时间倒序 | `fast`、`slow`，同均线接口；诊断规则见 [自选股诊断](../domains/ma_signals.md#watchlist) |
| `PUT /watchlist/{thscode}` | 收藏一只股票，重复收藏不报错；返回 204 | 证券不存在时返回 404 |
| `DELETE /watchlist/{thscode}` | 取消收藏，本来没有收藏也返回 204 | — |
| `GET /indices` | 指数与板块列表，含当前成分数 | `category`、`q`；最多 2,000 条 |
| `GET /indices/{thscode}/bars` | 指数日线（没有复权） | `start`、`end`、`limit`，同个股日线 |
| `GET /indices/{thscode}/members` | 指数成分 | `date` 默认今天；历史成分从开始采集那天起才有 |
| `GET /datasets` | 数据集列表：中文名、分类、一句话说明、更新频率、是否只有当天值、开关、最近完成时间、未完成任务数、没有解决的失败任务数、历史回补的进度 | — |
| `GET /sync/health` | 同步是否正常：最近一个交易日的数据是否就绪、最近 20 个交易日的缺口（能否补回）、没有解决的失败（白话原因）、历史回补的进度和剩余时间 | 规则见 [完整性核对与兜底补偿](../architecture/sync_pipeline.md#audit) |
| `POST /sync/repair` | 立即兜底补偿：能补的缺口规划补偿运行，供应商暂时不可用导致的失败重新排队；返回 202 | 每天每日同步结束后也会自动执行一次 |
| `GET /datasets/{dataset_id}/records` | 通用记录查询，返回供应商原始字段 | `thscode`、`start`、`end`（按 `obs_date`）；`limit` 默认 100、最大 1,000；按 `obs_date` 倒序 |
| `GET /sync/runs` | 最近的运行 | `limit` 默认 20、最大 200 |
| `POST /sync/runs` | 发起运行，只规划任务，由执行循环执行；返回 202 | 请求体 `{kind, datasets, priorities, thscodes}`；`kind` 是 `backfill` 或 `incremental`；`datasets` 为空时按 `priorities` 选择已开启的数据集 |
| `GET /sync/runs/{run_id}` | 运行详情：各数据集进度，最多 50 条失败任务 | — |

`POST /sync/runs` 返回的 `run_id` 是第一个波次。后续波次是新的运行，`params.root_run` 指向它，见 [波次规划](../architecture/sync_pipeline.md#waves)。

<a id="auth"></a>
## 登录

网页只有一个管理员账户，账号和密码来自环境变量 `NIULAI_ADMIN_USER`、`NIULAI_ADMIN_PASSWORD`，没有设置时都是 `niulai`。实现在 `backend/src/api/auth.rs`。

- **哪些接口要登录：** 除 `/health`、`/ready`、`/auth/login`、`/auth/logout`、`/openapi.json` 外的全部 `/api/v1/*` 接口。前端页面本身（`/` 和静态文件）不需要登录，页面在收到 401 时转到登录页。
- **登录状态：** 登录成功后设置 Cookie `niulai_session`，`HttpOnly`、`SameSite=Lax`、`Path=/`，保存 30 天（`Max-Age=2592000`）。到期后要重新登录，期间使用不会延长。
- **Cookie 的内容：** 到期时间加上 HMAC-SHA256 签名，签名内容包括账号、密码和到期时间。签名密钥第一次用到时随机生成，保存在 `app.setting` 的 `session_secret`，所以重启后登录状态仍然有效；修改账号或密码后，之前的登录状态全部失效。服务端不保存会话列表，退出登录只是清除这台浏览器的 Cookie。
- **账号密码不对：** 等待约 0.8 秒后返回 401，放慢猜密码的速度。
- **默认账号密码：** 仍在使用 `niulai` / `niulai` 时，启动日志有一条警告，`GET /auth/me` 的 `default_credentials` 为 true，页面顶部提示修改。
- Cookie 没有 `Secure` 属性，通过 HTTP 访问也能登录。直接暴露在公网时，密码会以明文在网络上传输。

为什么这样设计，见 [ADR：网页登录](../decisions/web_login.md)。

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
| 400 | `bad_request` | 参数不合法，`message` 说明原因，例如 `adjust` 取值不对、`fast` 不小于 `slow`、回测区间内没有日线、策略定义没有通过校验 |
| 401 | `unauthorized` | 没有登录、登录已过期，或账号密码不对 |
| 404 | `not_found` | 证券、数据集、运行或策略不存在 |
| 409 | `conflict` | 与当前状态冲突，例如 API Key 由环境变量提供时在网页上修改 |
| 500 | `internal` | 服务器内部错误；详细原因只写日志，响应中不包含 |
| 503 | `unavailable` | 依赖的外部服务暂时不可用，例如验证 API Key 时连不上同花顺接口 |

查询参数格式错误（例如日期不是 `yyyy-MM-dd`）由框架直接返回 400，请求体 JSON 的结构不对（例如策略定义里有未知的信号）返回 422，响应体都是纯文本。

<a id="openapi"></a>
## OpenAPI 与前端类型

- 规范版本是 OpenAPI 3.1，由 utoipa 从 Rust 代码生成，路径相对于 `/api/v1`。
- 新增或修改接口时，同时更新 `#[utoipa::path]` 注解，并在所属模块的 `routes()` 中用 `routes!` 注册。新增接口模块时，还要在 `backend/src/api/mod.rs` 的 `routes()` 中合并它；HTTP 路由和 OpenAPI 规范都从这一处生成。只用 `Router::route` 注册的路由不会出现在规范中。
- 每个响应都要写 `description`（OpenAPI 3.1 的要求），否则 orval 校验规范时失败。
- 前端用 orval 根据规范生成 TypeScript 类型和 TanStack Query hooks，更新步骤见 [接口客户端](../architecture/frontend.md#api_client)。
