# 同步流水线

本文说明数据从供应商接口到 PostgreSQL 的完整流程：注册表加载、任务规划、任务执行、写入、运行结束后的计算、定时调度，以及限速与单一执行进程。数据集配置的字段含义见 [数据集注册表格式](../interfaces/dataset_registry.md)，各张表的职责见 [数据库分层](data_model.md)，供应商接口的约定见 [同花顺接口](../interfaces/hithink_api.md)。

## 章节导航

- [总体流程](#pipeline_flow)：先读这一节。
- [代码结构](#code_structure)：新增或移动 `backend/src/sync/` 中的代码时。
- [波次规划](#waves)：修改规划器，或数据集之间有依赖时。
- [任务生命周期](#task_lifecycle)：修改认领、重试、失败处理时。
- [写入](#record_writes)：修改记录提取、写库、游标时。
- [运行结束后的计算](#postprocess)：修改复权重算、回补期间的定期重算或波次衔接时。
- [定时调度](#scheduling)：修改每日增量的触发规则时。
- [限速与单一执行进程](#rate_limit)：修改请求频率，或想同时运行多个执行任务的进程时。

<a id="pipeline_flow"></a>
## 总体流程

```
backend/config/datasets.yaml ─加载─► ops.dataset
        │ 规划器（按波次）
        ▼
   ops.run ─► ops.task ──执行器（SKIP LOCKED 认领，单进程多并发）──► 供应商 HTTP
                                    │                                   └─► ops.api_call（每次尝试一行）
                                    ▼
                 raw.record / raw.record_version（store_record 时）
                 ref.* / mkt.* / fin.*（sink）
                                    ▼
                  运行结束 ─► 复权重算 ─► 规划下一个波次
```

`niulai serve` 和运维子命令启动时都会创建同步引擎（`sync::Engine`）：读取并校验 `datasets.yaml`，把配置同步到 `ops.dataset`（配置中删掉的数据集标记为关闭），创建供应商客户端和限速器。配置在进程启动时读取一次，修改配置后需要重启进程。

还没有配置 API Key 时，执行循环不认领任务、定时调度不发起增量，直到在网页上配置，见 [首次登录引导](../interfaces/rest_api.md#setup)。

规划和执行是解耦的：命令行、API、调度器都可以规划运行，任务都写进 `ops.task`，由 `niulai serve` 的执行循环（或 `niulai work`）执行。`niulai serve` 在一个进程中同时运行 API、执行循环和调度器，三者共用同一个同步引擎；`--no-api`、`--no-worker`、`--no-scheduler` 可关闭其中一部分。同一时刻只有一个进程执行任务，见 [限速与单一执行进程](#rate_limit)。

<a id="code_structure"></a>
## 代码结构

`backend/src/sync/` 按职责分层，上层依赖下层，同层模块互不依赖：

| 层 | 模块 | 职责 |
| --- | --- | --- |
| 入口 | `scheduler.rs`、`executor.rs` | 定时触发增量；执行循环，以及单个任务各步骤的编排、运行结束后的衔接 |
| 规划 | `planner.rs` | 把分区展开为任务，按波次创建运行 |
| 执行步骤 | `fetch.rs`、`records.rs`、`sinks.rs`、`postprocess.rs` | HTTP 取数与调用日志；记录主键与 `raw.record`；类型化表投影；运行结束后的计算 |
| 状态 | `queue.rs`、`run.rs`、`cursor.rs`、`calendar.rs` | `ops.task` 的状态变更；`ops.run` 的创建与结束；`ops.cursor`；`ref.trading_day` 与交易日校验 |
| 配置 | `spec.rs`、`template.rs`、`registry.rs` | 注册表的类型化表示、模板展开、同步到 `ops.dataset` |

任务状态的 SQL 只写在 `queue.rs`，运行状态的 SQL 只写在 `run.rs`。重试、租约、推迟等时间策略是这两个模块和 `calendar.rs` 顶部的常量。

<a id="waves"></a>
## 波次规划

一次规划请求包含多个数据集。有些数据集依赖其他数据集的结果：按证券展开的数据集需要先有代码表，`from_dataset` 分区需要先有来源数据集的记录。规划器按依赖层级把数据集分成波次：

| 层级 | 分区类型 |
| --- | --- |
| 0 | `once`、`enum`、`trading_day`（不依赖本地数据） |
| 1 | `security`、`security_batch`、`security_window`、`report_period`（依赖证券范围） |
| 来源层级 + 1 | `from_dataset`（来源数据集也有 from_dataset 时继续往上算，最多 8 层） |

规划时只创建第一个波次的运行，其余波次写入该运行的 `params.pending_waves`。一个波次的全部任务结束后，执行器规划下一个波次，新运行的 `params.root_run` 指向第一个波次。某个波次规划出 0 个任务时，立即结束该运行并规划下一个波次。

规划一个波次时，先算出所有数据集的任务，再在一个事务里创建运行、写入全部任务和任务总数。所以执行者看到一个运行时，它的任务已经齐全；否则前几个数据集的任务做完、后面的还在规划时，运行会被误判为已结束，下一个波次会提前开始。某个数据集规划失败（例如 universe 查询出错）时只记录错误，不影响同一波次的其他数据集。

`from_dataset` 分区的增量规划只看上次规划之后变化的来源记录。读取来源记录之前先记下数据库时刻，任务写入成功后再把这个时刻存为游标（`ops.cursor` 中 `part_key = '_plan'`）。读取期间变化的记录下次还会被规划，不会漏掉。

规划的选项有两个：
- `trade_date`：运行对应的交易日，默认今天，快照类数据集用它作主键的一部分。
- `only_thscodes`：只规划指定证券，作用于按证券展开的分区，用于重拉个别股票。

两个选项都写进运行的 params，在后续波次中保留。

任务按 `(dataset_id, part_key)` 去重：同一分区已有 `pending`、`running` 或 `failed` 状态的任务时，不会再创建一个。所以重复规划是安全的。主键里带 `{_run.trade_date}` 的数据集（按天积累的快照），`part_key` 末尾会加上 `|day=<运行交易日>`，不同交易日的任务互不去重；否则排队中的旧任务会挡住当天的增量。每种分区如何展开为任务，见 [分区类型](../interfaces/dataset_registry.md#partitions)。

<a id="task_lifecycle"></a>
## 任务生命周期

| 状态 | 含义 |
| --- | --- |
| `pending` | 等待执行 |
| `running` | 已被某个执行者认领，`locked_by` 记录执行者 |
| `done` | 成功，并写入了记录 |
| `empty` | 成功，但接口返回 0 条记录（例如停牌期间的窗口、非交易日），或返回了数据集 `empty_codes` 中声明的错误码（原因写在 `last_error`） |
| `skipped` | 主动跳过，原因写在 `last_error`：交易日校验判定这一天不是交易日、在未来或已经错过（见 [交易日校验](../domains/market_data_rules.md#trade_date_guard)），或人工标记 |
| `failed` | 可重试的失败，`not_before` 之后会被重新认领 |
| `dead` | 不再重试：不可重试的错误，或尝试次数达到 `max_attempts`（默认 4） |

交易日校验判定数据尚未就绪时，任务回到 `pending`，`not_before` 推迟 10 分钟，`attempts` 不增加，所以可以一直等到当天数据就绪。

执行者用一个短事务认领任务（实现在 `queue.rs`）：按 `priority, task_id` 取第一条 `pending` 或 `failed` 且 `not_before` 已到的任务，用 `FOR UPDATE SKIP LOCKED` 锁定，同时把 `attempts` 加 1。HTTP 调用在认领事务提交之后执行。完成、失败的更新都带 `locked_by = 当前执行者` 条件，超时被回收的旧执行者无法覆盖新结果。任务的 `priority` 由运行类型和数据集优先级决定：增量任务是 P0 10、P1 20、P2 30、P3 40，回补任务在此基础上加 100。所以增量任务永远先于回补任务执行，积压的历史回补不会挡住每日同步；同类任务中 P0 先执行。

失败处理分两层：
1. **单次请求内**：可重试的错误按 `defaults.retry.backoff_s`（默认 2、8、30 秒）退避后重试，每次尝试都写一行 `ops.api_call`。哪些错误可重试见 [错误处理](../interfaces/hithink_api.md#error_handling)。
2. **任务级**：重试用尽后，任务标为 `failed`，`not_before` 推迟 5 分钟 × 已尝试次数；尝试次数达到 `max_attempts`（默认 4，配置项 `defaults.retry.max_attempts`）后标为 `dead`。不可重试的错误（1xxx、2xxx、3001 等）、配置或解析错误直接标为 `dead`。

认领的任务带 10 分钟的租约。执行期间，执行者每分钟续租一次（刷新 `locked_at`），所以下载导出文件、长时间翻页这类超过 10 分钟的任务不会被当成崩溃。执行者崩溃后，任务停留在 `running`、不再续租。执行循环的 0 号槽位每分钟做一次维护，不论队列是否空闲：
1. 调用 `ops.requeue_stale('10 minutes')`，把租约过期的任务放回 `failed`；如果尝试次数已达上限，则标为 `dead`。
2. 找出状态仍是 `running`、但已经没有未完成任务的运行，把它结束。例如最后一个任务被回收后直接标为 `dead`，没有执行者处理过它，运行不会自己结束。
3. 回补期间定期重算复权因子，见 [运行结束后的计算](#postprocess)。

运行的状态：所有任务都结束（没有 `pending`、`running`、`failed`）时，有 `dead` 任务记为 `partial`，否则记为 `succeeded`。每个任务结束后，执行者先用部分索引 `task_run_open_idx` 判断所在运行是否还有未完成的任务；回补运行有二十多万个任务，这样不必每次统计整个运行。没有未完成的任务时，用带条件的 UPDATE 结束运行，只有一个执行者能成功，因此运行结束后的计算不会重复执行。

运行的 `tasks_done`、`tasks_failed` 在任务结束时累加，供查看进度；运行结束时再按任务表重新统计一次，纠正人工改动任务等原因造成的偏差。

<a id="record_writes"></a>
## 写入

一个任务的执行步骤：
1. 配置了 `trade_date_guard` 的数据集，在请求之前先做交易日校验，见 [交易日校验](../domains/market_data_rules.md#trade_date_guard)。
2. 取出任务的 `params.query`（已展开的请求参数），按配置的分页方式逐页请求。翻页在以下任一条件满足时停止：响应中 `data.pagination.has_next` 为 false；已到 `data.pagination.pages`；返回条数少于页大小；已翻满 10,000 页。全市场导出（`records: parquet`）先取预签名链接再下载 Parquet，下载失败时重新签名，最多 3 次。
3. 按 `records` 路径取出记录，按主键模板生成 `rkey`、`thscode`、`obs_date`，见 [主键与模板](../interfaces/dataset_registry.md#keys_and_templates)。
4. `store_record` 为 true（默认）时，每 2,000 条调用一次 `raw.upsert_records`：值未变化只刷新 `last_seen_at`，值变化时先把旧值写入 `raw.record_version`。
5. 依次执行配置中的每个 `sink`，写入类型化表。所有 sink 都是幂等的 UPSERT。
6. 推进进度游标 `ops.cursor`：`security_window` 分区按证券记录已同步到的窗口末日，`trading_day` 分区记录已同步到的日期；只前进不后退。增量规划从游标往后算。

任务的 `records` 是返回的记录数，`changed` 是新增或值变化的记录数（`store_record` 为 false 时是 sink 写入的行数）。

<a id="postprocess"></a>
## 运行结束后的计算

运行结束时，如果日线（`a_share.bar_1d`、`a_share.snapshot`）或公司行动（`a_share.corp_action`、`a_share.corp_action_dump`）有变化（成功任务的 `changed` 合计大于 0），依次执行：
1. 把行情中出现过的日期补进 `ref.trading_day`（来源记为 `derived`）。供应商日历只有最近一年。
2. `CALL mkt.rebuild_adj_segment(run_id)`：全量重算复权分段，包括等比复权因子和同花顺减法复权的系数，约 2 秒。
3. 更新 `ops.derived_version` 的 `adj_factor` 行，使依赖复权价的缓存（全市场扫描）失效。

重算完成后，用开启了巡检的选股策略在最新交易日上执行一次 [每日巡检](../domains/strategies.md#screening)，约 1 到 2 秒；巡检失败只写日志，不影响运行的状态。之后规划下一个波次（如果有）。复权的计算规则见 [复权](../domains/price_adjustment.md)。

新的一天的日线不用重算就有复权价（落在最后一个分段里）。但一个回补运行可能要执行一两天，期间补回来的历史日线和公司行动会改变分段，只在运行结束时重算的话，这段时间的复权价是旧的。所以执行循环的维护（每分钟一次）还会检查：自上次重算以来有价格类任务写入了变化，并且距上次重算超过 5 分钟，就重算一次。上次重算的时刻记在 `ops.scheduler` 的 `adj_rebuild` 作业上，用条件 UPDATE 抢占；运行结束时的重算也会刷新这个时刻。重算不阻塞查询，见 [等比复权因子](../domains/price_adjustment.md#equal_ratio_factor)。

<a id="scheduling"></a>
## 定时调度

`niulai serve` 默认启动调度器（`--no-scheduler` 或 `NIULAI_SCHEDULER_ENABLED=false` 可关闭）。调度器每 30 秒检查一次，满足以下条件时发起一次增量运行：
1. 上海时间已到 `NIULAI_SCHEDULE_AFTER_CLOSE`（默认 18:00），且当天是周一到周五。
2. 成功抢占 `ops.scheduler` 中的 `after_close` 作业：用条件 UPDATE 把 `last_fired` 推进到今天，每天只有一个进程能成功，多个进程并存时也只触发一次。
3. 现取交易日历并写入 `ref.trading_day`，今天是交易日。日历取不到时退回本地记录；本地日历还没有覆盖到今天时按交易日处理，快照的交易日校验会兜底。

触发时还会尝试抢占 `weekly`（每个自然周一次）和 `monthly`（每个自然月一次）作业。到期的数据集是：已开启，并且 `schedule` 满足下表条件。

| schedule | 何时到期 |
| --- | --- |
| `after_close`、`daily` | 每个交易日 |
| `weekly` | 本周第一次触发时 |
| `monthly` | 本月第一次触发时 |
| `report_season` | 1、4、7、8、10 月每个交易日；其余月份每周一次 |
| `after_open` | 目前不调度 |

增量运行的交易日是今天。手动触发用 `niulai daily`，它不抢占每周和每月作业，所以不包括 `weekly`、`monthly` 数据集；`report_season` 数据集只在 1、4、7、8、10 月包括。

<a id="rate_limit"></a>
## 限速与单一执行进程

供应商 API Key 有全局请求频率上限，多开进程不会更快，所以同一时刻只允许一个进程执行同步任务：`niulai serve` 的执行循环或 `niulai work` 启动前，用一个独立的数据库连接取得咨询锁 `pg_try_advisory_lock(hashtext('niulai.executor'))`，取不到就报错退出；进程退出、连接关闭时锁自动释放。`niulai serve --no-worker` 不取锁，可以另外启动。

限速在进程内：每次请求前检查最近 1 秒内的请求数，达到 `NIULAI_RATE_LIMIT_RPS`（默认 2）时等到最早那次请求满 1 秒。`niulai plan`、`niulai daily` 规划时偶尔请求几次接口，不取锁，会与执行循环短暂叠加。为什么不用 Redis 跨进程限速，见 [ADR：限速在进程内](../decisions/in_process_rate_limit.md)。

单个进程同时执行的任务数由 `NIULAI_WORKER_CONCURRENCY`（默认 3）控制。实测每秒 2 次、并发 3 的强度下没有触发供应商限流，实际吞吐约每秒 1.7 次。
