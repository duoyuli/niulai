# 数据库分层

本文说明 PostgreSQL 中各 schema 和表的职责、通用记录库的语义，以及哪些数据集写入哪些类型化表。写入流程见 [同步流水线](sync_pipeline.md#record_writes)，复权相关的表和视图见 [复权](../domains/price_adjustment.md)。表结构的唯一来源是 `backend/migrations/`，连数据库的子命令（除 `niulai openapi`、`niulai upgrader` 外）启动时都会先执行迁移。

<a id="schemas"></a>
## Schema 一览

| Schema | 职责 | 主要对象 |
| --- | --- | --- |
| `ops` | 同步框架的运行状态 | `dataset`（注册表）、`run`（运行）、`task`（任务队列）、`cursor`（进度游标）、`api_call`（调用日志）、`scheduler`（调度状态）、`derived_version`（派生数据版本），视图 `v_progress`、`v_universe`，函数 `ms_to_cn_date`、`requeue_stale` |
| `raw` | 供应商原始数据 | `record`（通用记录库）、`record_version`（修订历史），函数 `upsert_records` |
| `ref` | 参考数据 | `security`、`security_name_hist`、`trading_day`、`index_info`、`index_member` |
| `mkt` | 行情与公司行动 | `stock_bar_1d`、`index_bar_1d`、`corp_action`、`adj_segment`（复权分段），复权视图和重算存储过程 |
| `fin` | 估值等基本面 | `valuation_1d` |
| `app` | 用户在网页上维护的数据和由它产生的结果，不由同步写入 | `user`（网页用户：用户名不区分大小写，Argon2id 密码哈希为空表示还没有设置，角色 `admin` / `member`，`disabled` 停用，`must_change_password` 提醒本人改密码，见 [登录与权限](../interfaces/rest_api.md#auth)）、`watchlist`（自选股，按用户保存，主键 `(user_id, thscode)`；删除证券时一并删除）、`setting`（网页上的设置：API Key、登录 Cookie 的签名密钥 `session_secret`、同步时间 `sync_schedule`）、`strategy`（选股策略，`definition` 是 JSON 定义，`screening` 是否巡检，`show_on_today` 是否在今日页展示，`builtin_key` 非空时是由模板同步的内置策略）、`screen_run`（每个策略每个交易日一次巡检，`scanned` 是当天有日线的股票数）、`screen_hit`（巡检结果，`kind` 是 `buy` 今日买点、`sell` 今日卖点或 `holding` 持有中）、`signal`（自定义信号，`definition` 是条件的 JSON，策略用 `my_signal` 条件按编号引用）；删除策略时一并删除它的巡检记录。规则见 [选股策略与诊断](../domains/strategies.md#screening)。账户：`account`（属于一个用户，`rules` 是选股和风控规则的 JSON，现金、费率）、`position`（持仓，`since` 是股数最近变动的日期）、`account_trade`（流水，持仓和现金每一次变动一行）、`advice`（每个账户每个交易日一份建议，`input` 是生成时的全部输入，`result` 是建议，`superseded_by` 指向重新生成的一份）、`advice_item_state`（每条建议已执行或不做）、`strategy_stat`、`preset_stat`（策略过去表现和三组参考值的回测，每周重算）；删除账户时一并删除。规则见 [账户与调仓建议](../domains/accounts.md) |

日期约定：业务日期（交易日、报告期、除权日）一律存为 `DATE`，按 Asia/Shanghai 解释；采集和系统事件时间存为 `TIMESTAMPTZ`。供应商毫秒时间戳的换算规则见 [时间戳](../domains/market_data_rules.md#timestamps)。

<a id="generic_records"></a>
## 通用记录库

`raw.record` 保存所有 `store_record` 为 true 的数据集的记录，每个数据集的每个主键只有一行：

| 列 | 含义 |
| --- | --- |
| `dataset_id`、`rkey` | 主键。`rkey` 由数据集配置的 `key` 模板生成 |
| `obs_date` | 记录的业务日期，可为空 |
| `thscode` | 记录关联的证券，可为空 |
| `payload` | 单条记录，字段保持供应商原样 |
| `payload_hash` | `md5(payload::text)`，用于判断值是否变化 |
| `first_seen_at`、`last_seen_at`、`changed_at` | 首次出现、最近一次出现、最近一次值变化的时间 |

写入统一通过 `raw.upsert_records(dataset_id, task_id, records_jsonb)`，返回新增数和变化数：
- 新主键：插入。
- 已有主键、值未变化：只刷新 `last_seen_at`。
- 已有主键、值变化：旧值先写入 `raw.record_version`（记录有效区间），再覆盖。
- 同一批中重复的 `rkey`：只保留最后一条。

主键是否包含日期，决定了记录的语义：
- **参考类**（如 `meta.tickers` 的 `{thscode}`）：每个对象一行，值会被覆盖，变化历史在 `record_version` 里。
- **观察类**（如 `a_share.valuation` 的 `{_run.trade_date}|{thscode}`）：每天一行，用于积累只有当前值的数据。

`raw.record` 声明为按 `dataset_id` 的 LIST 分区，但只有一个默认分区。另有 `(dataset_id, thscode, obs_date)` 和 `(dataset_id, obs_date)` 两个索引，服务于按证券和按日期的查询。`ops.v_universe` 基于 `raw.record` 提供规划器用的证券范围，`active` 列表示该记录在所属数据集最近一次同步时仍然出现（与该数据集最新的 `last_seen_at` 相差不超过 1 天）。

不保存完整的原始响应：记录原文在 `raw.record.payload` 中，请求的参数、状态码和耗时在 `ops.api_call` 中。

选择这种存储方式的理由见 [ADR：通用记录库加类型化投影](../decisions/generic_record_store.md)。

<a id="typed_tables"></a>
## 类型化表

类型化表由数据集配置中的 `sink` 写入，实现在 `backend/src/sync/sinks.rs`：

| 表 | 写入它的数据集 | 说明 |
| --- | --- | --- |
| `ref.security` | `meta.tickers`（只取 `asset_type = 'a-share'`） | 证券主表。`board` 按代码段推导：BJ 为 `bse`，300、301 开头为 `chinext`，688、689 开头为 `star`，其余为 `main`。退市推断见 [证券生命周期](../domains/market_data_rules.md#security_lifecycle) |
| `ref.security_name_hist` | 同上 | 简称变化区间；`is_st` 由简称是否包含 `ST` 生成 |
| `ref.trading_day` | `a_share.calendar`；运行结束后的计算 | `source` 为 `api`（供应商日历）或 `derived`（从行情推导） |
| `ref.index_info` | `index.ths_catalog`（带分类）、`meta.tickers`（`a-share-index`，只补缺少的条目） | `category`：industry、cn_concept、region、tszs 来自目录；`ths`、`exchange` 是按代码后缀推断的；`unknown` 表示只在成分查询中出现过 |
| `ref.index_member` | `index.members` | 每天与当前成分对比：退出的关闭区间（`valid_to`），新进入的打开区间 |
| `mkt.stock_bar_1d` | `a_share.bar_1d`（`source = hist`）、`a_share.snapshot`（`source = snapshot`） | 未复权日线。两种来源的覆盖规则见 [快照精度](../domains/market_data_rules.md#snapshot_precision) |
| `mkt.index_bar_1d` | `index.bar_1d`、`index.snapshot` | 指数日线，没有复权；快照只补 `pre_close` |
| `mkt.corp_action` | `a_share.corp_action_dump` | 公司行动原样保存，`seq` 是同一除权日内的源顺序。覆盖规则见 [公司行动规则](../domains/price_adjustment.md#corp_action_rules) |
| `fin.valuation_1d` | `a_share.valuation`（PE、PB、PS、PCF）、`a_share.auction_final`（`float_market_cap`） | 每个交易日一行，两个数据集分别更新各自的列 |

`mkt.adj_segment` 不由 sink 写入，由运行结束后的存储过程全量重算，见 [复权](../domains/price_adjustment.md)。

<a id="ops_tables"></a>
## 运行状态表

- `ops.dataset`：`datasets.yaml` 的数据库副本。`spec` 保存完整配置，`spec_hash` 用于判断配置是否变化。
- `ops.run`：运行。`kind` 是 `backfill`（历史回补）、`incremental`（定时同步和手动增量）或 `repair`（兜底补偿，见 [完整性核对与兜底补偿](sync_pipeline.md#audit)）。`params` 中有 `datasets`、`pending_waves`、`root_run`、`only_thscodes`、`trigger`（谁发起的，见 [定时调度](sync_pipeline.md#scheduling)）。
- `ops.task`：任务队列，状态机见 [任务生命周期](sync_pipeline.md#task_lifecycle)。`params` 包含 `vars`（分区变量）和 `query`（已展开的请求参数）。部分唯一索引 `task_open_uniq` 保证同一分区同时只有一个未完成任务；部分索引 `task_run_open_idx` 只含未完成任务，用于判断运行是否结束；索引 `task_part_idx`（`dataset_id, part_key, task_id`）用于按分区查最近的任务。结束超过 60 天的任务定期清理，见 [定期清理](sync_pipeline.md#retention)。
- `ops.cursor`：每个数据集、每个分区的进度，`state` 是 JSON，例如 `{"synced_to": "2026-10-15"}`。`part_key = '_plan'` 的行是 `from_dataset` 增量规划的时间水位。
- `ops.api_call`：每次 HTTP 尝试一行，含状态码、供应商 `code`、`request_id`、延迟。表声明为按 `called_at` 的范围分区，但只有默认分区；超过 30 天的行定期清理，见 [定期清理](sync_pipeline.md#retention)。
- `ops.bar_gap`、`ops.data_check`：数据核检发现的日线缺口（`open` 还缺、`suspended` 确认停牌，补上了就删掉）和每次核检的结果，见 [数据核检](sync_pipeline.md#data_check)。
- `ops.scheduler`：定时作业最近一次触发的日期和时刻：`daily`（每日同步，也用于判断要不要补发）、`weekly`（按触发时刻 `fired_at` 判断每周时刻有没有触发过）、`monthly`、`repair`（每个交易日一次的兜底补偿）、`data_check`（每周一次的数据核检）、`adj_rebuild`（回补期间定期重算复权）、`prune`（每天一次的 [定期清理](sync_pipeline.md#retention)），见 [定时调度](sync_pipeline.md#scheduling)。
- `ops.derived_version`：派生数据最近一次重算完成的时刻。目前只有 `adj_factor` 一行，复权因子重算完成后更新，进程内依赖复权价的缓存以它为版本，见 [ADR：指标在 Rust 中计算](../decisions/indicator_compute.md)。
- `ops.v_progress`：按运行和数据集汇总任务状态，API 的运行详情使用它。

<a id="migrations"></a>
## 迁移规则

- 迁移文件放在 `backend/migrations/`，文件名带时间戳前缀，由 SQLx 按顺序执行，执行记录在 `_sqlx_migrations`。
- 已执行过的迁移文件不再修改；表结构变更一律新增迁移文件。修改已执行的文件会导致校验和不一致，迁移会报错。
- 新增迁移文件只写结构和一次性的数据修正；对应的现状说明写在本库的相关文档中，不在迁移文件里加 `@project-doc` 锚点。
