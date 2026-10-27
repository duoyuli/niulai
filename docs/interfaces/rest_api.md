# REST API

本文说明 `niulai serve` 对外提供的 REST API：通用约定、登录与权限、接口清单、复权参数和错误格式。字段级的定义以 OpenAPI 规范为准（`GET /api/v1/openapi.json`，或运行 `niulai openapi`），本文只记录跨接口的约定和不容易从规范看出的行为。实现在 `backend/src/api/`。

## 章节导航

- [通用约定](#conventions)：路径、超时、数值类型和研究 GET 的副作用。
- [接口清单](#endpoints)：查找方法、参数和范围；实际登录调用见 [运维手册](../operations/runbook.md#api_smoke)。
- [登录与权限](#auth)、[首次登录引导](#setup)：确认 Cookie、角色和供应商 Key 的不同用途。
- [复权参数](#adjust_param)、[错误格式](#errors)、[OpenAPI](#openapi)：实现客户端和排错。

<a id="conventions"></a>
## 通用约定

- 所有接口都在 `/api/v1` 下，业务成功响应通常是 JSON；204 没有正文，`/metrics` 是文本，其他例外见 [错误格式](#errors)。默认监听 `127.0.0.1:8080`（`NIULAI_API_BIND`）。配置 `NIULAI_WEB_DIR` 后，同一端口还提供页面，未匹配的 `/api/` 路径由页面 fallback 返回 JSON 404；未配置页面目录时使用框架默认 404，见 [由 niulai serve 提供](../architecture/frontend.md#serving)。
- **要先登录**：除健康检查、登录/退出、OpenAPI 和监控指标外，所有接口都要带登录 Cookie，否则返回 401；普通用户调用管理员接口返回 403，见 [登录与权限](#auth)。默认只监听本机端口（`127.0.0.1:8080`），见 [部署](../operations/runbook.md#deployment)。
- 请求标识：每个响应都带 `x-request-id` 头。请求中带了这个头时沿用，否则生成一个 UUID；日志中的 `request_id` 与它一致。
- 超时层使用 `NIULAI_API_TIMEOUT_S`，默认 300 秒，返回 408；请求排队时间也在其中。同步 CPU 段没有立即中止保证；前端取消只是取消请求，不保证服务端计算立即停止或已写入记录回滚，见 [研究排错](../domains/factor_research.md#troubleshooting)。
- 数值格式：
  - 未复权价格和成交额是十进制字符串，保留数据库中的精度，例如 `"1312.9700"`、`"7537706223.71"`。
  - 复权价格也是字符串，保留 6 位小数。
  - 成交量是整数（股）。
  - 均线、复权因子是 JSON 数字（浮点）。
  - 因子研究结果不采用价格字符串约定：`summary`、`equity`、`trades` 中的价格、金额、费用和比率是 JSON 数字；不可计算的可选统计量为 `null`，比率 0.1 表示 10%。
- 日期是 `yyyy-MM-dd`，按 Asia/Shanghai 解释；时间（如 `started_at`）是 UTC 的 RFC 3339 字符串。
- 分页查询的 `limit` 通常会截到上限；不要据此推断所有数值参数都截断。例如板块 `days`、研究的 `top_n`、日期范围越界会返回 400。自选股等用户维护的列表按各自契约返回。
- 研究计算虽然使用 GET，成功返回前会写入 `ops.backtest_run`，不是无副作用查询。不要用于健康检查、定时刷新或无条件自动重试；完整调用和持久化边界见 [因子研究](../domains/factor_research.md#usage)。

<a id="endpoints"></a>
## 接口清单

| 方法与路径 | 说明 | 主要参数与上限 |
| --- | --- | --- |
| `GET /health` | 存活检查 | — |
| `GET /ready` | 执行 `SELECT 1` 成功时返回 200 和 `{"status":"ready"}` | 不检查数据覆盖、供应商、Redis 或研究是否可运行；数据库查询失败通常为 500 |
| `GET /metrics` | Prometheus 文本格式的监控指标（进程、请求、队列、连接池）；默认关闭 | 设置 `NIULAI_METRICS_TOKEN` 后启用，请求要带 `Authorization: Bearer <令牌>`（不接受写在网址里）；没有设置时返回 404，令牌不对返回 401；见 [监控指标](../operations/runbook.md#metrics) |
| `POST /auth/login` | 登录，成功时设置登录 Cookie | 请求体 `{username, password}`，用户名不区分大小写；连续输错 5 次后返回 429；见 [登录与权限](#auth) |
| `POST /auth/logout` | 退出登录，清除 Cookie；返回 204 | — |
| `GET /auth/me` | 当前用户：编号、用户名、角色（`admin` / `member`）、是否要提醒修改密码（`must_change_password`） | 没有登录时返回 401 |
| `PUT /auth/password` | 改自己的密码，响应带新的登录 Cookie，其他浏览器上的登录失效 | 请求体 `{current_password, new_password}`，新密码至少 8 个字符；现在的密码不对时返回 400 |
| `GET /operation-logs` | 最近的成功登录日志（管理员） | `limit` 默认 100、最大 200；`offset` 默认 0 |
| `GET /users` | 全部用户（管理员），不含密码哈希 | — |
| `POST /users` | 添加用户（管理员），返回 201 | 请求体 `{username, password?, role?}`：不填密码时生成一个，只在这次的 `generated_password` 中返回；不填角色时是 `member`；用户名已存在返回 409 |
| `PUT /users/{user_id}` | 修改角色、停用或启用（管理员） | 请求体 `{role, disabled}`；不能修改自己，至少留一个可以登录的管理员，否则返回 400 |
| `POST /users/{user_id}/password` | 重置别人的密码（管理员），这个用户之前的登录都失效 | 请求体 `{password?}`，不填时生成一个并在 `generated_password` 中返回；不能重置自己的 |
| `GET /system/data` | 数据准备得怎么样：核心数据（个股日线、公司行动）的首次回补还剩多少、全部回补的进度、一句话说明 | 普通用户也能看；首页据此提示「数据准备中」 |
| `GET /system/version` | 当前版本、可以升级到的最新版本（GitHub 上的标签，并且镜像已经能从 GHCR 拉取）、一键升级的状态（upgrader 是否在运行、进度、失败时的日志）、每日备份的情况（最近一份、几份、失败原因、超过 36 小时没有成功时 `stale`） | `refresh=true` 时不用 6 小时的缓存，立即查询；见 [一键升级](../operations/runbook.md#one_click_upgrade) |
| `POST /system/upgrade` | 一键升级：把版本号交给 upgrader 容器，返回 202 | 请求体 `{version}`，为空时升级到最新版本；只接受正式版本号（`YYYY.MM.DD`、`YYYY.MM.DD.N`）且高于当前版本；upgrader 没有运行或正在升级时返回 409 |
| `GET /setup` | 首次登录引导的状态：API Key 是否已配置、来源、末尾 4 位、生成 Key 的网址、是否已规划回补 | 见 [首次登录引导](#setup) |
| `PUT /setup/api-key` | 验证并保存 API Key，立即生效 | 请求体 `{api_key}`；见 [首次登录引导](#setup) |
| `GET /securities` | 证券列表（股票和交易所 ETF，结果带 `asset_type`，取值是 `stock` 或 `etf`，各接口一致） | `q`（代码或名称片段）、`board`；`limit` 默认 50、最大 500；`offset` |
| `GET /securities/{thscode}` | 证券详情：简称历史、本地日线的范围和根数 | 不存在时返回 404 |
| `GET /securities/{thscode}/fundamentals` | 股票的最近估值历史、三张报表和财务指标；ETF 返回行情范围、供应商前复权标记和基金资料；股票报表同时返回披露日期（按上海时间换算，和因子研究推断财报可用日的口径一致）。读取和供应商字段解析在 `analysis/fundamentals.rs` | `limit` 默认 90、最大 365（ETF 忽略） |
| `GET /securities/{thscode}/bars` | 个股或 ETF 日线 | `adjust`（见 [复权参数](#adjust_param)；ETF 只支持 `none` / `qfq` / `hfq`）、`start`、`end`；`limit` 默认 500、最大 10,000，返回范围内**最近**的 N 根，按日期升序 |
| `GET /research/factors` | 因子目录：口径、类别（价量或财务）、预期方向和依据 | 见 [因子目录](../domains/factor_research.md#catalog) |
| `GET /research/factor-test` | 因子检验：RankIC、分组收益、多空、换手、衰减、因子相关性、PBO 和 Deflated Sharpe；两个以上因子时额外检验等权合成 | `factors`（必填，逗号分隔的因子 ID）、`start` 默认结束日前 5 年、`end`，区间 60 天到 8 年；`horizon` 5、10 或 20（默认 5）；`quantiles` 3 到 10（默认 5）；`neutralize` 默认 true；`exclude_small_pct` 0 到 50；股票池参数 `boards`、`exclude_st`、`min_price`、`min_amount`、`min_listed_days`；规则见 [因子检验](../domains/factor_research.md#test) |
| `GET /research/factor-portfolio` | 多因子组合回测：定期按合成得分持有前若干只，返回净值、超额、信息比率、换手、Deflated Sharpe、每一笔成交和最新得分前列 | `factors`（必填，可带权重，例如 `reversal:1,idio_vol:0.5`）、`start` 默认结束日前 3 年、`end`，区间 60 天到 5 年；`rebalance` 5 到 60（默认 20）；`top_n` 5 到 100（默认 30）；`buffer` 0 到 2（默认 0.5）；`capital` 默认 100 万、范围 1000 到 100 亿；成本 `commission_rate`（默认 0.00025）、`min_commission`（默认 5，0 到 100）、`stamp_tax_rate`（默认 0.0005）、`transfer_fee_rate`（默认 0），费率都在 0 到 0.01 之间，`slippage_bps` 默认 10、最大 500；股票池和预处理参数同因子检验；规则见 [多因子组合回测](../domains/factor_research.md#portfolio) |
| `GET /research/presets`、`PUT /research/presets`、`DELETE /research/presets/{preset_id}` | 当前用户的因子预设：列出（最近改过的在前）、保存（同名覆盖）、删除；当前中间件仅放行管理员的保存和删除，普通用户这两项返回 403 | 保存的请求体 `name`（1 到 30 个字）、`factors`（`[{factor, weight}]`，同账户规则）、`top_n`（5 到 50）、`buffer`（0 到 2）；每个用户最多 50 个；权限意图待确认，见 [因子预设](../domains/accounts.md#presets) |
| `GET /market/overview` | 市场概况：涨跌家数、涨跌停数、涨跌幅中位数、站上 20 日均线的比例 | `date` 默认最新交易日；规则见 [市场环境](../domains/market_data_rules.md#market) |
| `GET /market/sectors` | 板块强弱排名 | `category`（`industry` 默认 / `cn_concept`）、`days` 默认 5、范围 1 到 60、`date`；规则见 [市场环境](../domains/market_data_rules.md#market) |
| `GET /market/hot` | 今日热点：权威涨停池按连板高度分组、跌停、炸板、封板率等统计；普通用户可看 | `date` 是截止日期，默认最新交易日；每组和每个列表最多 50 只，全部家数在 `*_total`；数据按交易日同步，这一天的数据还没同步时 `available` 为 `false`、`note` 用白话说明原因；口径和空态见 [今日热点](../domains/hotspots.md) |
| `GET /watchlist` | 当前用户的自选股（每个用户各自一份）：最新收盘、近 1 日和 20 日涨跌（按前复权价）；按收藏时间倒序 | — |
| `PUT /watchlist/{thscode}` | 收藏一只股票，重复收藏不报错；返回 204 | 证券不存在时返回 404 |
| `DELETE /watchlist/{thscode}` | 取消收藏，本来没有收藏也返回 204 | — |
| `GET /accounts` | 当前用户的账户：总资产、当日盈亏、已实现盈亏、持仓只数、最新建议的摘要、核对提醒 | 只返回自己的；已实现盈亏见 [撤销流水和已实现盈亏](../domains/accounts.md#undo) |
| `POST /accounts` | 新建账户（规则用默认值，见 [调仓规则](../domains/accounts.md#rules)），返回 201 和详情，并生成第一份建议 | 请求体 `{name, cash?, note?}`；同一用户下重名返回 409，超过 10 个返回 400 |
| `GET` / `PUT` / `DELETE /accounts/{id}` | 账户详情（持仓带现价、市值、占比和盈亏）；修改名字、规则 `rules`、费率 `fees`、备注；删除（连同持仓、流水和建议） | 别人的账户返回 404（管理员也一样）；`PUT` 的参数范围见 [调仓规则](../domains/accounts.md#rules)，之后在后台重新生成建议 |
| `POST /accounts/{id}/positions/parse` | 识别粘贴的持仓文字，只返回预览 | 请求体 `{text}`，最多 20,000 字；规则见 [持仓](../domains/accounts.md#positions) |
| `PUT /accounts/{id}/positions` | 校准：整体替换持仓和现金，差异记为校准流水，之后在后台重新生成建议 | 请求体 `{cash, positions: [{thscode, shares, cost_price?, opened_on?}]}`；代码不存在、股数不大于 0、重复时返回 400 |
| `GET` / `POST /accounts/{id}/trades` | 流水（新的在前，`limit` 默认 100、最多 500，已撤销的也列出并带撤销时间）；手动记一笔买入、卖出、转入、转出，之后重新生成建议 | 请求体 `{kind, thscode?, shares?, price?, amount?, fee?, trade_date?, note?}`；现金不够、卖出超过持有时返回 400；卖出流水带这一笔的 `realized_pnl`；配股认购（`kind` 为 `rights`）也在流水里 |
| `DELETE /accounts/{id}/trades/{trade_id}` | 撤销一条手动流水（红冲）：标成已撤销，现金和这只股票的持仓、成本、已实现盈亏在同一个事务里重算，之后重新生成建议 | 只能撤销手动记的买入、卖出、转入、转出；按建议执行、分红送转、校准的流水、已经撤销过的流水返回 400；撤销后现金会变成负数、或后面的卖出/分红送转会前后对不上时返回 400；口径见 [撤销流水和已实现盈亏](../domains/accounts.md#undo) |
| `GET /accounts/{id}/corp-actions` | 待处理的公司行动（目前是配股）：股票、除权日、每 10 股配几股、配股价、可认购的股数、要付的配股款和费用 | 只看最近 180 天、除权日已过、还没处理过的；口径见 [分红送转自动记账与配股提醒](../domains/accounts.md#corp_actions) |
| `POST /accounts/{id}/corp-actions/rights/{thscode}/{ex_date}/subscribe` | 记一笔配股认购：扣配股款和费用，持仓按平均成本更新，提醒消失，之后重新生成建议 | 没有这次配股、不足 1 股、已经处理过或现金不够时返回 400；口径同上 |
| `POST /accounts/{id}/corp-actions/rights/{thscode}/{ex_date}/ignore` | 暂不处理这次配股（不买、不扣钱），返回 204，之后不再提醒 | 已经处理过或不足 1 股返回 400 |
| `GET /accounts/{id}/equity` | 每日净值曲线：每个交易日的现金、持仓市值、总资产、当日盈亏，以及剔除转入转出后的区间收益率 | `days` 默认 180、最多 1,825 个交易日；口径见 [每日净值](../domains/accounts.md#equity) |
| `POST /accounts/{id}/checked` | 确认持仓和券商一致（只记时间），返回 204 | — |
| `GET /accounts/{id}/advice` | 最新一份建议（`date` 指定交易日）和每条的处理状态；没有时 `waiting` 说明原因；改了持仓、现金或设置之后还在后台重新生成时 `regenerating` 为 true；上一次重新生成失败时 `regenerating` 为 false、`regenerate_error` 是原因（后台自动重试） | 结构和规则见 [生成建议](../domains/accounts.md#generation) |
| `POST /accounts/{id}/advice/refresh` | 按现在的持仓和最新的因子排名重新生成 | — |
| `GET /accounts/{id}/advice/history` | 每个交易日的最终一份建议的摘要 | `limit` 默认 60、最多 250 |
| `POST /accounts/{id}/advice/{advice_id}/items/{key}/execute` | 已执行：按实际成交记一笔，持仓和现金更新，这份建议不重新生成 | 请求体 `{shares?, price?, fee?, trade_date?}`，默认按建议；同一条执行两次、这条不需要操作时返回 400 |
| `POST` / `DELETE /accounts/{id}/advice/{advice_id}/items/{key}/skip` | 不做这一条 / 撤销「不做」；影子订单随之记为已取消或回到计划中 | 已执行的不能改成不做（400），也不能撤销；条目不存在时 404 |
| `GET /accounts/{id}/review` | 最近 180 天的建议之后 5、10、20 个交易日的涨跌和汇总 | 口径见 [建议回看](../domains/accounts.md#review) |
| `GET /accounts/{id}/shadow-orders` | 查看理论订单与实际成交之间的价差、部分成交和取消状态 | `limit` 默认 250、最大 1,000；建议生成、执行或跳过后自动更新 |
| `GET /indices` | 指数与板块列表，含当前成分数 | `category`、`q`；最多 2,000 条 |
| `GET /indices/{thscode}/bars` | 指数日线（没有复权） | `start`、`end`、`limit`，同个股日线；指数没有复权，`adjust` 参数会被忽略 |
| `GET /indices/{thscode}/members` | 指数成分 | `date` 默认今天；历史成分从开始采集那天起才有 |
| `GET /datasets` | 数据集列表（只含配置中现有的数据集）：中文名、分类、一句话说明、更新频率和所属同步计划（`schedule`：`daily`、`weekly`、`monthly`、`report_season`、`manual`）、是否只有当天值、开关、最近完成时间、未完成任务数、没有解决的失败任务数、历史回补的进度；`category_order` 是分类的先后（分类在 `datasets.yaml` 中第一次出现的顺序），`etf_market` 标记 ETF 行情和资料的数据集 | — |
| `GET /sync/health` | 同步是否正常：最近一个交易日的数据是否就绪、最近 20 个交易日的缺口（能否补回）、没有解决的失败（白话原因）、历史回补的进度和剩余时间 | 规则见 [完整性核对与兜底补偿](../architecture/sync_pipeline.md#audit) |
| `POST /sync/repair` | 立即兜底补偿：能补的缺口规划补偿运行，供应商暂时不可用导致的失败重新排队；返回 202 | 每天每日同步结束后也会自动执行一次 |
| `GET /sync/check` | 数据核检：是否正在核检、最近一次核检（来源、查到哪个交易日、每个数据集查了多少、新发现、已补回、确认停牌、仍缺、重取的运行）、还缺数据的证券（缺的天数多的在前，最多 50 只，每只列最早 10 天） | 规则见 [数据核检](../architecture/sync_pipeline.md#data_check) |
| `POST /sync/check` | 立即在后台核检，返回 202；正在核检时返回 409 | 要几十秒，用 `GET /sync/check` 看结果 |
| `GET /sync/schedule` | 同步计划：时间设置，每日、每周、每月、财报季、不自动同步各自的规则、包含的已开启数据集、下次同步时刻、上次同步的运行和状态 | 规则见 [定时调度](../architecture/sync_pipeline.md#scheduling) |
| `PUT /sync/schedule` | 修改同步时间（上海时间），返回新的同步计划 | 请求体 `{daily_time, weekly_weekday, weekly_time}`；时间是 `HH:MM`，星期 1（周一）到 7（周日），不合法时返回 400 |
| `GET /datasets/{dataset_id}/records` | 通用记录查询，返回供应商原始字段 | `thscode`、`start`、`end`（按 `obs_date`）；`limit` 默认 100、最大 1,000；按 `obs_date` 倒序 |
| `GET /sync/runs` | 最近的运行 | `limit` 默认 20、最大 200 |
| `POST /sync/runs` | 发起运行，立即返回 202 和第一个波次的 `run_id`；任务规划在后台完成，详情接口轮询任务数 | 请求体 `{kind, datasets, priorities, thscodes}`；`kind` 是 `backfill` 或 `incremental`；`datasets` 为空时按 `priorities` 选择已开启的数据集，两者都为空时是全部已开启的数据集；运行的 `params.trigger` 记为 `manual` |
| `GET /sync/runs/{run_id}` | 一次同步的详情，传任何一批的编号都返回整次同步：第一批的字段、已经开始的各批（`waves`）、还没开始的批次的数据集（`waiting`）、各批合计的各数据集进度、这次没有新建任务的数据集及原因（`without_tasks`，`queued_in` 是同样内容正在排队的那次同步）、最多 50 条失败任务 | 进度中 `done` 包括成功、没有数据和跳过，`pending` 包括等待重试的，`failed` 只算不再重试的，与运行的 `tasks_done`、`tasks_failed` 口径一致 |

`POST /sync/runs` 返回的 `run_id` 是第一个波次。后续波次是新的运行，`params.root_run` 指向它，见 [波次规划](../architecture/sync_pipeline.md#waves)。

<a id="auth"></a>
## 登录与权限

用户保存在 `app.user`，**不开放注册**：管理员在「用户」页（`POST /users`）添加用户、设置或生成初始密码。第一个管理员由迁移创建，启动时如果它还没有密码，用 `NIULAI_ADMIN_USER`、`NIULAI_ADMIN_PASSWORD`（默认都是 `niulai`）设置，之后这两个变量不再起作用。用户、密码、会话签名在 `backend/src/auth.rs`，接口和中间件在 `backend/src/api/auth.rs`、`users.rs`。

- **哪些接口要登录：** 除 `/health`、`/ready`、`/metrics`、`/auth/login`、`/auth/logout`、`/openapi.json` 外的全部 `/api/v1/*` 接口。其中 `/metrics` 默认关闭，设置 `NIULAI_METRICS_TOKEN` 后改为校验令牌，见 [监控指标](../operations/runbook.md#metrics)。前端页面本身（`/` 和静态文件）不需要登录，页面在收到 401 时转到登录页。
- **角色：** 管理员（`admin`）什么都能做。普通用户（`member`）：`/sync`、`/datasets`、`/setup`、`/users`、`/operation-logs`、`/system/upgrade` 开头的接口连查看都不行；其余接口可以查看（因子研究都是 GET，普通用户也能算）；写操作只有改自己的密码、收藏和取消收藏、管理自己的账户（`/accounts` 下的新建、修改、删除、校准、记账、撤销流水、配股认购、建议的重新生成、执行和不做）。名单在 `api/auth.rs` 的 `ADMIN_PREFIXES`、`MEMBER_WRITES`，新增的写接口默认只有管理员能用。不允许时返回 403。
- **我的数据：** 自选股、账户（含持仓、流水、建议）按当前用户保存，接口从 `CurrentUser` 取用户编号过滤；管理员也看不到别人的，别人的账户返回 404。
- **密码：** Argon2id 哈希，至少 8 个字符。管理员添加、重置的密码和默认密码都标记 `must_change_password`，本人登录后页面顶部提醒，本人改过后清除。
- **登录状态：** 登录成功后设置 Cookie `niulai_session`，`HttpOnly`、`SameSite=Lax`、`Path=/`，保存 30 天（`Max-Age=2592000`），期间使用不会延长。内容是 `用户编号.到期时间.签名`，HMAC-SHA256 签名覆盖用户编号、到期时间和密码哈希；签名密钥第一次用到时随机生成，保存在 `app.setting` 的 `session_secret`，重启后仍然有效。改密码、被重置密码后这个用户之前的登录全部失效；停用的用户每次请求都被拒绝。服务端不保存会话列表，退出登录只是清除这台浏览器的 Cookie。
- **操作日志：** 登录成功后写入 `app.operation_log`，动作固定为 `login`，保存用户名、用户编号、来源 IP、登录请求的 `User-Agent` 和时间；登录失败不写入。来源地址优先取服务端注入的 TCP 地址，直接测试或未注入时兼容 `X-Forwarded-For`、`X-Real-IP`，都没有时记为 `unknown`。日志只能由管理员通过 `GET /operation-logs` 查看。
- **防止锁在外面：** 不能修改自己的角色、停用自己、用重置接口改自己的密码；至少保留一个可以登录的管理员。都忘了密码时用命令行重置，见 [用户与密码](../operations/runbook.md#users)。
- **猜密码：** 同一用户名（不区分大小写）连续输错 5 次后暂停 5 分钟，返回 429；用户不存在时也做一次同样耗时的校验。计数在进程内，重启后清零。
- Cookie 没有 `Secure` 属性，通过 HTTP 访问也能登录。直接暴露在公网时，密码会以明文在网络上传输。

为什么这样设计，见 [ADR：多用户登录与角色](../decisions/multi_user_login.md)。

<a id="setup"></a>
## 首次登录引导

同花顺金融数据服务的 API Key 一般不在部署时提供，而是第一次打开网页时配置，实现在 `backend/src/setup.rs`：

- `GET /setup` 的 `api_key_configured` 为 false 时，执行循环和定时调度暂停；管理员登录后的页面会跳转到引导页，普通用户不执行这项前端检查。`api_key_url` 是生成 Key 的页面 <https://fuyao.aicubes.cn/admin/>。
- `PUT /setup/api-key` 先用这个 Key 请求一次交易日历接口：被拒绝时返回 400，连不上时返回 503，都不保存；成功后写入 `app.setting` 并立即生效。数据库里还没有任何回补运行时，同时规划 P0、P1 的全量回补，响应中的 `backfill_run_id` 是第一个波次的运行 ID。
- Key 由环境变量 `HITHINK_FINANCE_API_KEY` 提供时，`api_key_source` 为 `env`，`PUT` 返回 409。
- 响应中只有末尾 4 位（`api_key_hint`），不返回完整 Key。

为什么这样设计，见 [ADR：API Key 在首次登录引导中配置](../decisions/api_key_setup.md)。

<a id="adjust_param"></a>
## 复权参数

`/securities/{thscode}/bars` 的 `adjust` 取值为 `none`、`qfq`（默认）、`hfq`、`qfq_ths`、`hfq_ths`，各自对应的视图和用途见 [复权视图与 API 参数](../domains/price_adjustment.md#adjust_views)。只有 `qfq`、`hfq` 返回 `factor`。

<a id="errors"></a>
## 错误格式

业务错误由 `ApiError` 返回 `{"code": "...", "message": "..."}`：

| HTTP 状态 | `code` | 含义 |
| --- | --- | --- |
| 400 | `bad_request` | 参数不合法，`message` 说明原因，例如 `adjust` 取值不对、回测区间内没有日线、账户规则超出范围 |
| 401 | `unauthorized` | 没有登录、登录已过期、账号密码不对，或用户已停用 |
| 403 | `forbidden` | 已登录，但角色不允许，例如普通用户打开数据同步 |
| 404 | `not_found` | 证券、数据集、运行或账户不存在 |
| 409 | `conflict` | 与当前状态冲突，例如 API Key 由环境变量提供时在网页上修改、添加已有的用户名、账户重名 |
| 429 | `too_many_requests` | 同一用户名连续输错密码 5 次，暂停 5 分钟 |
| 500 | `internal` | 服务器内部错误；详细原因只写日志，响应中不包含 |
| 503 | `unavailable` | 依赖的外部服务暂时不可用，例如验证 API Key 时连不上同花顺接口 |

查询参数格式错误（例如日期不是 `yyyy-MM-dd`）由框架直接返回 400，请求体 JSON 的结构不对（例如账户规则里有未知的因子）返回 422，响应体都是纯文本。

超时层返回 408，不经过 `ApiError`，不保证有 JSON 错误正文；`/metrics` 的 404、401 也是纯文本。客户端应先读取 HTTP 状态，再兼容 JSON、纯文本和空响应，不应对全部响应无条件解析 JSON。`x-request-id` 用于关联服务日志，不包含登录凭据。

<a id="openapi"></a>
## OpenAPI 与前端类型

- 规范版本是 OpenAPI 3.1，由 utoipa 从 Rust 代码生成，路径相对于 `/api/v1`。
- 新增或修改接口时，同时更新 `#[utoipa::path]` 注解，并在所属模块的 `routes()` 中用 `routes!` 注册。新增接口模块时，还要在 `backend/src/api/mod.rs` 的 `routes()` 中合并它；HTTP 路由和 OpenAPI 规范都从这一处生成。只用 `Router::route` 注册的路由不会出现在规范中。
- 当前 `/metrics` 和 `/openapi.json` 就是直接注册的例外，不列入导出的业务路径清单。认证、角色拒绝和超时来自中间件，各业务注解也不一定枚举它们；客户端还需遵守本文的跨接口约定。研究响应的 `summary`、`trades` 等字段因 `serde(flatten)` 位于顶层，不在 `outcome` 子对象里。
- 每个响应都要写 `description`（OpenAPI 3.1 的要求），否则 orval 校验规范时失败。
- 前端用 orval 根据规范生成 TypeScript 类型和 TanStack Query hooks，更新步骤见 [接口客户端](../architecture/frontend.md#api_client)。
