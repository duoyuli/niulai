# 同花顺金融数据服务接口

本文说明供应商接口的通用约定、错误处理、实测发现的限制和问题，以及不可用的接口。内容来自官方文档 <https://fuyao.aicubes.cn/docs/>（机器可读版本 `https://fuyao.aicubes.cn/llms-full.txt`）和 2026-10-15 用免费 Key 的实测。各数据集能取到什么、取多深见 [数据目录](dataset_catalog.md)，客户端实现在 `backend/src/vendor/client.rs`。

<a id="envelope_and_auth"></a>
## 通用约定

| 项目 | 约定 |
| --- | --- |
| Base URL | `https://fuyao.aicubes.cn`，可用 `NIULAI_VENDOR_BASE_URL` 覆盖 |
| 方法 | 公开数据接口都是 `GET` |
| 认证 | 请求头 `X-api-key: <API_KEY>`。Key 在 <https://fuyao.aicubes.cn/admin> 管理，本项目从 `HITHINK_FINANCE_API_KEY` 读取 |
| 响应信封 | `{code, message, request_id, data}`；成功时 `data` 承载数据，业务错误时 `data` 为 null |
| 成功判断 | HTTP 200 且 `code == 0`。HTTP 200 不代表成功 |
| 证券代码 | 完整 thscode，例如 `600519.SH`；不要猜交易所后缀 |
| 时间 | 毫秒 Unix 时间戳，表示 Asia/Shanghai 当地零点，换算见 [时间戳](../domains/market_data_rules.md#timestamps)；少数接口用 `yyyy-MM-dd` 或 `yyyyMMdd` 字符串 |
| 空值 | null 表示未披露或上游无值，不能补零 |
| 调用次数 | 官方说明不限制累计调用次数，但会动态限流 |

API Key 不能写进代码、日志、文档或提交。

<a id="error_handling"></a>
## 错误处理

客户端把每次调用的结果分成三类，执行器据此决定是否重试（见 [任务生命周期](../architecture/sync_pipeline.md#task_lifecycle)）：

| 结果 | 条件 | 处理 |
| --- | --- | --- |
| 成功 | HTTP 200 且 `code == 0` | 使用 `data` |
| 可重试 | `code` 为 3002（数据未就绪）、4001（限流）、4040（导出文件未就绪）、5001、5002、5003；HTTP 429；HTTP 5xx；网络错误、超时、读取中断；响应不是合法 JSON | 退避后重试 |
| 不可重试 | 其他情况，包括 1001–1004（参数错误）、2001、2003、2004（未认证、无权限、客户端专用）、3001（标的不存在）、3004（类型不支持） | 任务直接记为 `dead`，错误码写入 `ops.task.last_api_code` |

每次尝试都写入 `ops.api_call`，含 HTTP 状态、`code`、`request_id`、延迟。向供应商报障时需要提供 `request_id`。

实测中出现过两种可恢复的异常，重试一次即成功：一是约 15–23 秒后返回非 JSON 内容；二是传输中途断开（`transfer closed with outstanding read data remaining`）。

<a id="rate_limits"></a>
## 请求频率

官方没有公布具体的限流阈值。实测每秒 2 次、并发 3 的强度下，全市场日线回补的 9,504 次请求没有触发限流，平均延迟约 1 秒。本项目默认限速每秒 2 次，见 [限速与单一执行进程](../architecture/sync_pipeline.md#rate_limit)。

<a id="endpoint_quirks"></a>
## 接口限制与已知问题

| 接口 | 限制或问题 |
| --- | --- |
| `/api/a-share/prices/historical` | 每次只能查 1 只证券；`[start, end]` 不超过 10 年；最早到 2005-01-04；`adjust` 默认是 `forward`，本项目固定传 `none` |
| `/api/a-share/prices/snapshot` | 不传代码时分页取全市场，`limit` 最大 200；成交量和成交额只保留约 9 位有效数字，见 [快照与历史接口](../domains/market_data_rules.md#snapshot_precision) |
| 快照类接口（行情快照、估值、集合竞价、异动原因、指数快照、指数成分） | `data.timestamp` 是请求时刻，不是数据所属的交易日；判断数据日期的方法见 [交易日校验](../domains/market_data_rules.md#trade_date_guard) |
| `/api/a-share-index/prices/snapshot` | 不接受中证指数代码（`.CSI`），批量请求中含有这类代码时整批返回 1002 |
| `/api/a-share/corporate-actions/adjustment-factors` | 每次 1 只；**不返回配股字段**（`allotment_ratio`、`allotment_price`），文档的响应字段表里也没有。完整数据只能从全市场导出文件获取 |
| `/api/a-share/valuations/snapshot`、`/api/a-share/auction/snapshot` | 每次最多 100 个代码，超过返回 1003 |
| `/api/a-share-index/prices/historical` | 只有 2022-01-04 以后的数据；**窗口起点早于这一天时返回空列表，不报错**，所以窗口起点必须不早于 2022-01-01 |
| `/api/a-share/financials/indicators` | 每次 1 只证券 × 1 个报告期；2014 年起可用。某个报告期没有数据时返回 5003（例如上市前、北交所股票改代码之前），本项目按无数据处理 |
| `/api/a-share/special-data/limit-up-pool` | 最早有数据的交易日是 2020-07-22，更早的日期返回空 |
| `/api/a-share/special-data/limit-down-pool`、`limit-break-pool` | **只支持最近一年**，更早的日期返回 1002 `Invalid parameter format` |
| `/api/a-share/special-data/dragon-tiger-list`、`hot-stock-list-history`、`limit-up-statistics` | 只能查一年内，超出返回 1003 `date must be within one year` |
| `/api/a-share/special-data/dragon-tiger-stock-institution` | 只接受 A 股代码。龙虎榜中的可转债、ETF、REITs、B 股代码返回 1002 `Unknown thscode` |
| `abnormal-monitor-list`、`dragon-tiger-department-history` | 每页最多 30 条 |
| `auction-stock-opportunities` | 每页最多 20 条 |
| `/api/meta/tickers/list` | 只含在市证券；场外基金和期权都超过 1 万条，需要翻页（`limit` 最大 10000） |
| 已退市证券 | 所有接口都查不到，返回 1002 `Unknown thscode` 或 3001 |

同一股票同一除权日有多条公司行动、已公告未除权的事件，这两种情况的处理见 [公司行动规则](../domains/price_adjustment.md#corp_action_rules)。

<a id="market_dumps"></a>
## 全市场导出

`GET /api/dump/market-dumps/{daily-k,daily-k-10d,adjustment-factors}/download-url` 返回 `data.presigned_url`（S3 预签名链接，约 5 分钟有效），再 GET 该链接下载 Parquet 文件（ZSTD 压缩）：

| 导出 | 内容 | 实测规模 |
| --- | --- | --- |
| `daily-k` | 全市场约 10 年未复权日线 | 1,034 万行，182 MB |
| `daily-k-10d` | 最近 10 个交易日日线 | 5.6 万行 |
| `adjustment-factors` | 全部历史公司行动，含配股字段和未来事件 | 5.8 万行，约 300 KB |

本项目只用 `adjustment-factors`（数据集 `a_share.corp_action_dump`），原因见 [ADR：采用同花顺金融数据服务](../decisions/hithink_data_source.md)。导出的日线与 REST 历史接口逐行一致（968 万行重叠，0 差异），可作为对账和灾备的备选。

<a id="unavailable"></a>
## 不可用的接口

- **同花顺 AI 客户端专用**（免费 Key 返回 2004）：资金流向（`capital-flow/*`）、高频与分时（`high-frequency/*`）、筹码形态（`chip-shape-*`）、新增概念（`baseinfo/new-concepts`）、成交额预测、宽基 ETF 申购净流入、期货 F10 宏观指标、资讯事件库（`news/events/search`）。
- **尚未上线**：股票基础信息（行业、市值、股本）、个股所属同花顺指数反查、指数概况与权重。
- 期货和期权的 K 线，免费 Key 只能取日 K（`time_period=day_1`）；分时只能取当日（`trade_date=0`）。
