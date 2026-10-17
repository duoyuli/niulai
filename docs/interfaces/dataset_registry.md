# 数据集注册表格式

本文说明 `backend/config/datasets.yaml` 的格式：每个字段的含义、分区如何展开为任务、主键模板的写法，以及新增数据集的步骤。同步流程见 [同步流水线](../architecture/sync_pipeline.md)，现有数据集的内容和历史深度见 [数据目录](dataset_catalog.md)。解析代码在 `backend/src/sync/spec.rs`，展开代码在 `backend/src/sync/planner.rs`。

## 章节导航

- [字段](#fields)：查字段含义时阅读。
- [分区类型](#partitions)：新增数据集，或任务数量与预期不符时阅读。
- [主键与模板](#keys_and_templates)：编写 `key`、`params` 模板时阅读。
- [写入目标](#write_targets)：决定数据写进哪里时阅读。
- [新增数据集](#adding_dataset)：按步骤操作。

<a id="fields"></a>
## 字段

文件有两个顶层键：`defaults`（默认值）和 `datasets`（数据集列表）。程序启动时解析并校验整个文件：未知的分区类型、分页方式、调度类型、重复的 ID 都会让启动失败，不会等到执行时才发现。

`defaults` 中被程序读取的键：`paging`、`incremental`、`schedule`，以及 `retry.max_attempts`（任务最大尝试次数，默认 4）和 `retry.backoff_s`（单次请求的退避秒数，默认 2、8、30）。

数据集条目的字段：

| 字段 | 必填 | 含义 |
| --- | --- | --- |
| `id` | 是 | `<领域>.<名称>`，例如 `a_share.bar_1d`。领域取第一个点之前的部分 |
| `title`、`category`、`summary` | 是 | 中文名、分类、一句话说明（是什么数据、用在哪里），数据同步页按分类展示。分类的顺序在 `frontend/src/lib/labels.ts` 的 `DATASET_CATEGORIES` 中 |
| `endpoint` | 是 | 供应商接口路径，只支持 GET |
| `priority` | 是 | `P0`–`P3`，决定默认开关，以及同一层任务中的先后（P0 先执行）；分层见 [任务生命周期](../architecture/sync_pipeline.md#task_lifecycle) |
| `enabled` | 否 | 是否开启。未配置时 P0、P1 开启，P2、P3 关闭 |
| `partition` | 是 | 任务切分方式，见 [分区类型](#partitions) |
| `params` | 否 | 请求参数模板，值中的 `{变量}` 由分区变量填充 |
| `paging` | 否 | `none`、`{offset: {limit_param, offset_param, size}}` 或 `{page: {page_param, size_param, size}}`。翻页停止条件见 [写入](../architecture/sync_pipeline.md#record_writes) |
| `records` | 是 | 记录在响应中的位置：一个路径（如 `data.item`）、路径列表（多个数组合并），或 `parquet`（全市场导出：先取预签名链接再下载） |
| `key` | 是 | 主键模板，生成 `raw.record.rkey` |
| `thscode`、`obs_date` | 否 | 记录关联的证券和业务日期的模板，写入 `raw.record` 的同名列 |
| `incremental` | 否 | 增量规则：`rerun`（与回补相同的分区，整份重取）、`{since_cursor: {overlap_days: N}}`（从游标往后，重叠 N 天）、`{latest_n: {params: {...}}}` 或 `{latest_n: {n: N}}`（只取最近 N 期） |
| `schedule` | 否 | `daily`（默认）、`weekly`、`monthly`、`report_season`、`manual`（不自动同步），到期规则见 [定时调度](../architecture/sync_pipeline.md#scheduling) |
| `store_record` | 否 | 是否写入通用记录库 `raw.record`，默认 true |
| `sink` | 否 | 要写入的类型化表列表，必须是 `sinks.rs` 中已实现的组合 |
| `trade_date_guard` | 否 | 快照类数据集设为 true：执行前做交易日校验，未收盘或数据未就绪时推迟，已经错过的交易日记为失败，见 [交易日校验](../domains/market_data_rules.md#trade_date_guard)。默认 false |
| `recover_with` | 否 | 这个数据集取不到时，由哪个有历史的数据集补回同样的数据，例如收盘快照由个股日线补。兜底补偿按它规划，见 [完整性核对与兜底补偿](../architecture/sync_pipeline.md#audit)。没有配置的只有当天值的数据集，错过就补不回来 |
| `empty_codes` | 否 | 按「没有数据」处理的供应商错误码列表，例如 `a_share.fin_indicator` 的 `[5003]`。命中时不重试，任务记为 `empty`，原因写在 `last_error` |

`notes` 是给人看的说明，程序不读取。回补范围由分区决定，覆盖还是追加由 `key` 决定，不另设字段。

<a id="partitions"></a>
## 分区类型

规划器按分区类型把一个数据集展开为一组任务。每个任务有一个 `part_key`，同一数据集内唯一，用于去重和进度游标。

| 分区 | 写法 | 展开方式 | 分区变量 |
| --- | --- | --- | --- |
| `once` | `partition: once` | 一个任务，`part_key = *` | `today`、`today+60d` |
| `enum` | `{enum: {参数: [值, ...]}}` | 各参数取值的笛卡尔积 | 各参数名 |
| `security` | `{security: {universe}}` | 每只证券一个任务 | `thscode` |
| `security_batch` | `{security_batch: {universe, size}}` | 每 `size` 只证券一个任务 | `thscodes`（逗号拼接） |
| `security_window` | `{security_window: {universe, window_days, floor, start_from}}` | 每只证券 × 连续的时间窗口 | `thscode`、`start`、`end`（日期）、`start_ms`、`end_ms` |
| `trading_day` | `{trading_day: {from, calendar, dates_from}}` | 每个交易日一个任务 | `date`、`date_ms` |
| `report_period` | `{report_period: {universe, from, quarters}}` | 每只证券 × 已结束的报告期 | `thscode`、`report`（如 `2025-4`） |
| `from_dataset` | `{from_dataset: {dataset, fields, filter}}` | 另一个数据集记录中字段的去重组合 | `fields` 中的各个键 |

所有分区都有 `today`（今天）和 `today+60d` 两个变量。各分区的细节：

- **universe**：作用于视图 `ops.v_universe` 的 SQL 条件，可用列有 `dataset_id`、`rkey`、`thscode`、`asset_type`、`list_date`、`obs_date`、`payload`、`last_seen_at`、`active`。例：`dataset_id = 'meta.tickers' AND asset_type = 'a-share' AND active`。配置文件属于受信任的代码，条件会直接拼进 SQL。规划时指定 `only_thscodes` 会进一步限定证券范围。
- **security_window**：
  - 起点是 `floor`（绝对日期，或 `-5y`、`-30d` 这样相对今天的偏移）；配置了 `start_from: list_date` 时，起点取 `floor` 和上市日中较晚的那个。
  - 每个窗口 `window_days` 天，最后一个窗口到今天为止，窗口的起止时刻都是当天零点，见 [时间戳](../domains/market_data_rules.md#timestamps)。
  - 上市日在未来的证券跳过。
  - 增量规则为 `since_cursor` 时，从该证券的游标往前 `overlap_days` 天开始。
  - 增量规则为带 `params` 的 `latest_n` 时，每只证券只生成一个任务：去掉引用时间窗口变量的参数，换成 `latest_n.params`。
- **trading_day**：
  - `from` 是起点，写法同 `floor`。
  - 默认用 `ref.trading_day` 中的交易日；`calendar: natural` 改用自然日；`dates_from` 指定一个接口，在规划时调用，从 `data.dates` 读取可选日期。
  - 增量规则为 `since_cursor` 时，从全局游标往前 `overlap_days` 天开始。
- **report_period**：
  - `from` 写成 `YYYY-Q`，例如 `2014-4`；`quarters` 限定季度，例如 `[4]` 只取年报。
  - 只生成季度末早于今天的报告期；跳过上市前一年以前的报告期（科创板等新股在上市前一年就有财务数据）。
  - 增量规则 `latest_n: {n: N}` 只取最近 N 期。
- **from_dataset**：
  - `fields` 的值可以是 `rkey`、`thscode`、`obs_date`，或以 `$` 开头的 JSONPath（作用在来源记录的 `payload` 上，可以返回多个值）。
  - `filter` 是作用于来源记录（别名 `r`）的额外 SQL 条件。例：龙虎榜里有可转债、ETF 等非 A 股代码，机构明细接口只接受 A 股，所以配置了 `filter: "r.thscode IN (SELECT thscode FROM ref.security)"`。
  - 增量规划只取上次规划之后变化过的来源记录，时间水位存在 `ops.cursor` 的 `part_key = '_plan'` 行。
- 配置中出现但规划器不支持的分区（目前是 `cross`）会让该数据集规划失败，运行中该数据集的任务数为 0，日志记录错误。

数据集之间的依赖决定规划的波次，见 [波次规划](../architecture/sync_pipeline.md#waves)。

<a id="keys_and_templates"></a>
## 主键与模板

`params`、`key`、`thscode`、`obs_date` 都是模板，用 `{名称}` 引用变量，用 `{名称:date}` 把毫秒时间戳（或 `yyyyMMdd`、`yyyy-MM-dd` 字符串）转换为 `yyyy-MM-dd` 日期。

变量的查找顺序：
1. `_part.x`：任务的分区变量 `x`。
2. `_run.trade_date`：运行对应的交易日。
3. `_seq`：同一批记录中，除 `_seq` 外主键相同的记录按源顺序编号，从 1 开始。例如同一股票同一除权日有两条事件，`key: "{thscode}|{ex_date_ms:date}|{_seq}"` 会生成 `…|1` 和 `…|2`。
4. `_array`：`records` 是路径列表时，记录所在路径的最后一段（例如 `stock_items`）。
5. 其他名称：先查记录字段（支持 `a.b` 嵌套路径），再查分区变量。

变量不存在时报错，任务记为 `dead`，避免生成错误的主键或请求。`params` 只能引用分区变量（规划时展开，结果存在 `ops.task.params.query`）。

主键是否包含日期决定了数据的语义：参考类数据用自然键，例如 `{thscode}`；只有当前值、需要每天积累的数据，在主键里加 `{_run.trade_date}`。主键包含 `{_run.trade_date}` 时，规划器还会在任务的 `part_key` 末尾加 `|day=<运行交易日>`，使不同交易日的任务互不去重，见 [波次规划](../architecture/sync_pipeline.md#waves)。见 [通用记录库](../architecture/data_model.md#generic_records)。

<a id="write_targets"></a>
## 写入目标

| 情况 | 配置 |
| --- | --- |
| 只需要保存，以后再用 | 默认即可：写入 `raw.record`，可以用 `GET /api/v1/datasets/{id}/records` 查询 |
| 需要类型化表（有类型、有索引、给 API 或指标用） | 在 `sinks.rs` 实现投影，在 `sink` 中引用 |
| 数据量很大、已有类型化表 | `store_record: false`，只写类型化表 |

目前 `store_record: false` 的数据集是 `a_share.bar_1d`、`a_share.snapshot`、`index.bar_1d`、`index.snapshot`。已实现的 sink 组合见 [类型化表](../architecture/data_model.md#typed_tables)。

数据集在流程中的角色由 `sink` 和分区推出，代码里不写死数据集 ID（`spec.rs` 的 `Registry`）：`sink` 中有 `mkt.stock_bar_1d` 或 `mkt.corp_action` 的，有变化时运行结束后重算复权；按证券和时间窗口取、写日线表的，参加数据核检，其中写个股日线的那个用于兜底补偿时按股票补。

<a id="adding_dataset"></a>
## 新增数据集

1. 在 [同花顺接口](hithink_api.md) 和官方文档中确认接口路径、参数、分页上限、历史深度，用真实 Key 调一次，看响应结构。
2. 在 `datasets.yaml` 中新增条目：写中文名、分类和一句话说明，选择分区类型，写 `params`、`records`、`key`，有业务日期时写 `obs_date`，有证券时写 `thscode`。把实测结论写进 `notes`。
3. 需要类型化表时：新增迁移文件建表，在 `sinks.rs` 的 `apply` 中增加 `(sink, 数据集)` 分支。
4. 运行 `cargo test`。`spec::tests::project_registry_parses` 会解析真实的配置文件。
5. 用少量证券试跑：`niulai plan backfill -d <数据集> --thscode 600519.SH`。`niulai serve` 在运行时会自动执行；没有运行时用 `niulai work --until-idle`。检查 `ops.task` 中没有 `dead` 任务，并抽查 `raw.record`。
6. 更新 [数据目录](dataset_catalog.md)。
