# 今日热点

本文说明「今日热点」页面的数据来源、口径和页面结构。它是权威涨跌停池对普通用户的出口：数据来自同花顺整理的交易所口径涨跌停池，已经同步在 `raw.record`，之前只有管理员能通过 `GET /datasets/{id}/records` 看到。接口在 `backend/src/api/market.rs` 的 `GET /market/hot`，页面在 `frontend/src/pages/TodayPage.tsx`（首页卡片）和 `frontend/src/pages/HotPage.tsx`（`/hot`）。数据目录和同步方式见 [数据目录](../interfaces/dataset_catalog.md#special)，接口清单见 [REST API](../interfaces/rest_api.md#endpoints)。

<a id="data"></a>
## 数据来源

四个只读数据集，都不落到业务表，查询时直接从 `raw.record` 按 `dataset_id` + `obs_date` 取：

| 数据集 | 内容 | 历史范围 |
| --- | --- | --- |
| `special.limit_up_pool` | 当天涨停的股票：涨停原因、首次涨停时间、连板天数、封单额、是否 ST | 2020-07-22 起每个交易日 |
| `special.limit_down_pool` | 当天跌停的股票：首次和最后一次跌停时间、成交额、换手率 | 最近一年（2025-10-16 起） |
| `special.limit_break_pool` | 炸板股：盘中涨停、收盘没封住，带炸板次数 | 最近一年（2025-10-16 起） |
| `special.limit_up_stats` | 封板率、收盘涨停家数、盘中触及涨停家数、开板家数 | 最近一年（2025-10-17 起） |

- 连板梯队（`special.limit_up_ladder`）不用：涨停池每只股票自带 `continue_day_cnt`，本身就是完整的分组来源；梯队只有最近 30 个交易日，而且和池子是两套独立数据，一起用会对不上。
- 涨停、跌停、炸板池按交易日同步；热点接口仍需处理当天任务尚未完成、供应商名单滞后的情况。连板高度来自涨停池的 `continue_day_cnt`。
- 供应商偶尔有一行 `thscode` 为空：查询时用 `ticker` 到 `ref.security` 里找到完整代码；还找不到时按 6 位代码段补 `.SH` / `.SZ` / `.BJ` 后缀，保证页面上的链接可用。

<a id="rules"></a>
## 口径

- **交易日**：`date` 是截止日期，和 `/market/overview` 一样取本地日线里不晚于它的最近一天，「10 月 17 日（周六）」会显示 10 月 16 日（周五）的数据。不传 `date` 时取本地日线的最新一天；日线还没同步时退回涨停池自己的最新 `obs_date`，先把已经同步的数据显示出来。
- **涨停分组**：按 `continue_day_cnt` 从高到低分组，1 表示首板。`label` 是「首板」「2 板」「3 板」……，`count` 是这一档的全部家数，`stocks` 最多列 50 只。组内按封单额从大到小排（封单越大，说明想买没买到的人越多）。
- **总数和截断**：`limit_up_total`、`limit_down_total`、`broken_total` 都是全部家数，不受列出的 50 只限制；页面显示「共 N 家，列出前 50 只」。
- **跌停和炸板**：跌停按当天跌幅从大到小排，炸板按炸板次数从多到少排，都只列前 50 只。
- **统计**：`stats` 来自 `special.limit_up_stats`，`seal_rate` 是封板率（0.55 表示 55%），`limit_up_count` 是收盘涨停家数，`touched_limit_up_count` 是盘中触及过涨停的家数，`open_count` 是盘中打开、收盘没封住的家数。这个数据集 2025-10-17 起才有，更早的日期 `stats` 为空。
- **价格和金额**：`last_price`、`seal_money`、`turnover` 都是十进制字符串（接口约定见 [REST API](../interfaces/rest_api.md#conventions)），涨跌幅、换手率、封板率是数字。
- 样本里涨停池的行数可能和 `stats.limit_up_count` 不完全一样（供应商两个接口的口径有差异），接口以池子为准展示明细，统计数字单独展示，不互相校正。

<a id="empty"></a>
## 没有数据时

每个响应都带 `available` 和 `note`：

- 这一天的四个数据集都还没有记录时 `available` 为 `false`，`note` 是一句白话说明：当天的话是「今天的涨停数据还没同步，稍后再看」，其他日期是「2026-10-16 的涨停、跌停、炸板数据还没同步，稍后再看」；比涨停池开始采集（2020-07-22）还早的日期，说明「2020 年 7 月 22 日才开始有」。
- 本地连行情数据都还没有时 `trade_date` 为空，`note` 说明等第一次同步完成。
- 页面在 `available` 为 `false` 时只显示 `note`，不渲染空表格；首页卡片同理，不会报错。

<a id="pages"></a>
## 页面

- **首页卡片**（`TodayPage.tsx`）：在大盘概况下面显示涨停、跌停、炸板家数，最高连板，封板率，以及每个连板高度前 6 只股票（点进个股 K 线）和「全部热点 →」。只列前 4 个连板高度。
- **`/hot` 页面**（`HotPage.tsx`）：顶部是当天的家数、最高连板、封板率和「盘中触及过涨停」的拆分；下面按连板高度分组，每档一张表列出全部涨停股（名称、代码、连板说明、涨停原因、最新价、封单额、首次涨停时间）；再下面分别是跌停和炸板的表。日期框默认最新，可以往回看，日期写在 URL 的 `date` 参数里。
- 按 [界面按任务组织](../decisions/ui_by_task.md)，`/hot` 不加主导航项，从首页卡片进入，和指数、板块页一样。
- 每只股票都链接到 `/securities/{thscode}`，和首页其他位置一致。

<a id="tests"></a>
## 测试

`backend/tests/integration.rs` 的 `market_hot_groups_truncates_and_explains_missing_data` 写 `raw.record` 行验证：连板分组和组内排序、`thscode` 为空时按 `ticker` 补、每组和列表截断到 50 只、总数不受截断影响、日期是截止日期、没有数据时 `available` 为 `false` 且 `note` 说明原因、日期格式不对时返回 400、普通用户可以访问。这些测试只在设置了 `NIULAI_TEST_DATABASE_URL` 时运行，见 [本地运行](../operations/runbook.md#testing)。
