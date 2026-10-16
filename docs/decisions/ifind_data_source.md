# ADR：使用同花顺 iFinD 作为数据源（已被取代）

- 状态：已被取代（superseded），2026-10-15。由 [ADR：采用同花顺金融数据服务](hithink_data_source.md) 取代。
- 取代原因：用户实际持有的是同花顺金融数据服务（fuyao.aicubes.cn）的免费 API Key，不是 iFinD 账号。两者是不同的产品，Key 和接口都不通用。
- 保留原因：记录 iFinD 免费版的额度限制。以后如果考虑换用 iFinD，可以从这里开始评估。

以下是当时的草案原文，没有按后来的决定修改；其中的技术栈假设（Python、PostgreSQL 16）也已被 [技术栈 ADR](technology_stack.md) 取代。

---

日期：2026-10-15。状态：待讨论。依据：`docs/research/` 下三份调研，以及 iFinD 官方帮助中心（[FAQ][faq]、[权限说明][perm]）。尚未用真实账号调用接口，字段单位和复权因子的取法均需实测。

## 1. 先决条件：账号额度决定方案

| 事实（官方页面） | 对设计的影响 |
| --- | --- |
| Mac 和国产系统只能用 **HTTP 接口**，SDK（iFinDPy）不可用 | 直接写 HTTP 客户端，不依赖 iFinDPy |
| 认证：`refresh_token`（与账号同期有效）换 `access_token`（7 天有效），最多绑定 20 个 IP | Token 缓存在本地，过期（错误码 -1302）自动刷新重试一次 |
| 单函数 QPS ≤ 10，账号总并发 ≤ 20；单次请求 ≤ 200 万单元 | 客户端自带限流和请求拆分 |
| 免费版：历史行情 **100 万单元/月**，只能取最近 5 年 | 见下面估算 |
| 试用/正式版：行情数据 **1.5 亿单元/周**；正式版历史从上市至今 | 全市场回补一周内可以完成 |

「单元」指一个字段值，一根 10 个字段的日线消耗 10 个单元。

**按全市场约 5,400 只股票、每根日线 10 个字段估算：**

| 任务 | 消耗 | 免费版（100 万/月） | 正式版（1.5 亿/周） |
| --- | --- | --- | --- |
| 回补 5 年日线 | ≈ 6,600 万 | 需要约 66 个月，不可行 | 一次完成 |
| 每日增量 | ≈ 5.4 万/天，≈ 120 万/月 | 略微超额 | 很宽裕 |
| 自选池 300 只回补 5 年 | ≈ 360 万 | 约 4 个月 | 一次完成 |

结论：免费版只够跟踪自选池，无法支撑全市场。全市场诊断需要试用或正式账号；另一条路是用 BaoStock 等免费源回补历史，用 iFinD 做增量和抽样对账（BaoStock 的情况见 `free-a-share-data-sources.md`）。

## 2. 范围（首版）

入库：股票档案、交易日历、未复权日线、复权因子、交易状态。
不入库：均线等技术指标。MA15/MA30 由本地从原始价格和复权因子计算，原因是：
1. 不消耗额度；
2. 窗口和价格口径（前复权、后复权或原始价）由我们控制；
3. 改参数不需要重新拉数据。

## 3. 架构

```
iFinD HTTP API ──► IfindClient（token / 限流 / 拆分 / 额度检查）
                        │
                  Provider 接口（以后可接 BaoStock / Tushare）
                        │
                  Ingest 任务（回补 / 每日增量 / 校验）──► PostgreSQL
                                                          │
                                         视图：前复权价格、MA、交叉信号
```

- 语言：Python 3.12，用 uv 管理；依赖 `httpx`、`psycopg[binary]` v3、`pydantic-settings`、`typer`。
- 数据库：PostgreSQL 16，本地用 Docker Compose 跑。数据量约 5,400 只 × 20 年 × 243 天 ≈ 2,600 万行，普通 PG 表加索引就够用，暂时不引入 TimescaleDB。按查询实测结果再决定是否分区。
- 配置：`.env` 存 `IFIND_REFRESH_TOKEN` 和 `DATABASE_URL`，不提交到仓库。

## 4. 表结构

```sql
-- 股票档案（包含已退市股票）
security (code PK, name, exchange, board, list_date, delist_date, status, updated_at)

-- 交易日历
trade_calendar (exchange, cal_date, is_open, PK(exchange, cal_date))

-- 未复权日线：只存供应商原值，单位统一后入库
daily_bar (
  code, trade_date,
  open, high, low, close, pre_close  NUMERIC(12,4),
  volume BIGINT,          -- 股（入库时换算单位，原始单位记录在 source 配置中）
  amount NUMERIC(20,2),   -- 元
  turnover_rate NUMERIC(10,4),
  trade_status TEXT,      -- 正常 / 停牌等，停牌行要保留，不能因为没有数据就补零
  source TEXT, batch_id BIGINT, fetched_at TIMESTAMPTZ,
  PRIMARY KEY (code, trade_date)
)

-- 复权因子（后复权累计因子）
adj_factor (code, trade_date, factor NUMERIC(20,10), source, batch_id, PK(code, trade_date))

-- 采集任务与水位线：支持断点续传、追溯来源、统计额度消耗
ingest_batch (id, dataset, provider, params JSONB, status, rows, cells_used, error, started_at, finished_at)
sync_watermark (dataset, code, last_date, PK(dataset, code))
```

复权因子的取法：iFinD 是否直接提供复权因子指标还需要确认。如果不提供，就在同一请求里分别取不复权收盘价（`CPS=1`）和后复权收盘价（`CPS=3`），用两者之比得到因子。每根日线多消耗约 1 个单元。前复权价格在查询时用 `close × factor / 最新 factor` 计算，避免供应商的前复权结果随截止日变化而难以解释。

指标先做成视图，用窗口函数计算：

```sql
CREATE VIEW v_ma AS
SELECT code, trade_date, qfq_close,
       AVG(qfq_close) OVER w15 AS ma15,
       AVG(qfq_close) OVER w30 AS ma30
FROM v_qfq_bar
WINDOW w15 AS (PARTITION BY code ORDER BY trade_date ROWS 14 PRECEDING),
       w30 AS (PARTITION BY code ORDER BY trade_date ROWS 29 PRECEDING);
-- 交叉信号：用 LAG 比较前一天和当天的 (ma15 - ma30) 符号
```

如果全市场扫描太慢，再改成物化表，并记录计算参数和版本。

## 5. 采集流程

1. **Token**：读取 `refresh_token`，调用 `get_access_token`，结果缓存到本地文件，7 天内复用。
2. **额度检查**：每次请求前估算 `代码数 × 天数 × 字段数`。超过单次上限就拆分，默认每次 50 只 × 1 年。剩余额度不足时停止任务，不跑一半。会定期调用 `get_data_volume` 对账。
3. **限流**：默认并发 4、QPS 5，低于官方上限，给其他使用者留余量。遇到 -4400（频率超限）指数退避，遇到 -4304~-4306（单次数据量超限）对半拆分后重试。
4. **写入**：在事务内批量 `INSERT … ON CONFLICT DO UPDATE`，同时更新 `sync_watermark` 和 `ingest_batch`。重复执行结果不变。
5. **每日增量**：交易日 18:00 后执行（由 launchd 或 cron 触发），只取水位线之后的数据。
6. **校验**：日线数量和交易日历对得上（停牌日除外）；价格满足 high ≥ max(open, close)；抽样和第二数据源对比。

命令行：

```
niulai sync securities | calendar
niulai backfill daily --since 2021-01-01 [--codes 600519.SH,...]
niulai sync daily               # 增量
niulai quota                    # 查询已用额度
```

## 6. 第一个里程碑（spike）

用真实 `refresh_token` 拉 3 只股票 1 个月的数据，选正常、停牌过、近期除权各一只。目的：
- 确认 `cmd_history_quotation` 的指标名、成交量单位（股还是手）、停牌行怎么返回；
- 确认复权因子能否直接取；
- 测量实际消耗的单元数，修正第 1 节的估算。

确认后再建表并写采集任务。

## 7. 待确认

1. 账号是免费版、试用版还是正式版？
2. 股票范围是全市场还是自选池？历史要多少年？
3. 指标统一用前复权吗？同花顺界面默认显示前复权。
4. PostgreSQL 先在本机用 Docker 跑，还是已有服务器？

[faq]: https://quantapi.10jqka.com.cn/gwstatic/static/ds_web/quantapi-web/help-center/faq.html
[perm]: https://quantapi.10jqka.com.cn/gwstatic/static/ds_web/quantapi-web/help-center/permission.html
