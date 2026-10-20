# niulai 架构 Review 报告

日期：2026-10-20
审查对象：`main` 分支 `4dd1618`
状态：已完成，只读静态审查
归档路径：`docs/research/2026-10-20-architecture-review-report.md`

> 本报告是过程材料，记录问题假设、证据与改进建议，不是已修复事实，也不替代 [工程文档索引](../index.md) 路由到的现状文档。引用前应重新核对代码。

## 1. 总体判断

局部工程质量很高：类型严格、文档库完整、测试覆盖关键路径、同步层与分析层已经用事件解耦。但架构上的核心毛病可以概括成一句话：**「单一事实来源」和「单一所有者」主要写在文档里，代码里仍有多套实现、多个写入者、多处泄漏。** 问题集中在五处：事件可靠性、状态机所有权、交易语义模型、跨资产抽象、配置与契约边界。

与 [2026-10-19 旧审计](2026-10-19-system-architecture-and-domain-audit-report.md) 相比：`sync` 直接调用 `analysis`/`advice` 的反向依赖已移除，`Engine::new` 的鉴权/模板副作用已挪到 `bootstrap`，这是实质进步。但连接池、`block_in_place`、非等值复权 JOIN、回测双轨制等旧问题仍在，并出现了「补偿重开终态运行」这一新路径。

## 2. 严重：会静默丢结果或产生不一致

### 2.1 同步完成事件不是事务性 outbox，存在永久丢失窗口

证据：`backend/src/sync/executor.rs:355-378`。`after_task` 先 `try_complete` 把运行置终态，再执行 `commit_planned_since`、`continue_chain`（两者都带 `?`），最后才写 `sync_event_pending` 标记。恢复机制只覆盖「仍是 running」或「还有 pending_waves」的运行（`backend/src/sync/run.rs:135-177`）；`backend/src/sync/events.rs:43-47` 在无订阅者时直接丢弃事件。最后一个波次结束后的进程崩溃、或 `continue_chain` 报错，会让复权后处理、每日巡检和调仓建议永远不触发，页面无告警。复权可能被每分钟维护顺手补上，巡检和建议不会。

改进方向：把「待消费」标记并入 `try_complete` 的条件 UPDATE；或改成事件表 + 租约领取。

### 2.2 运行和任务状态机没有单一所有者，补偿路径改写终态

证据：文档明确「任务状态 SQL 只写在 `queue.rs`，运行状态只写在 `run.rs`」（`docs/architecture/sync_pipeline.md:55`），但 `backend/src/sync/audit.rs:580-599` 的兜底补偿把 `dead` 任务重置为 `pending`、`attempts=0`，并把所有含未完成任务的非 running 运行重新置回 `running`；`backend/src/sync/audit.rs:467` 直接改 `skipped`；`backend/src/sync/planner.rs:259` 也写运行状态。后果是已 `succeeded`/`partial` 的运行会被「复完成」，`after_task` 再跑一次复权并再发一次事件，导致重复的全市场巡检和建议刷新；`attempts` 的单调语义失效。

改进方向：状态迁移集中到 `run.rs`/`queue.rs` 的显式函数；「重新打开运行」作为带原因和幂等键的专用迁移；事件带版本去重。

### 2.3 交易语义没有单一模型：卖出四份、回测三套、费率三套

证据：同一套卖出优先级（反向信号 → 止损 → 止盈 → 持有到期）分别实现在 `backend/src/analysis/points.rs:130-143`、`backend/src/analysis/backtest.rs:337-351`、`backend/src/advice/inputs.rs:264-298`、`backend/src/advice/backtest.rs:537-575`。更关键的是建议链路把决策压成预渲染文本 `strategy_exit: Option<String>`（`backend/src/advice/generate.rs:61`），生成层只能判断「有没有文本」；而且 `advice/inputs.rs` 用的是未复权收盘价和成本价，前两者用前复权价，口径已经分叉。费率、手数同样三套：`backend/src/analysis/backtest.rs:20-59`、`backend/src/account/lots.rs:9-67`、`backend/src/advice/backtest.rs:54-57`。结果是同一个持仓在「K 线买卖点」「组合回测」「每日建议」里可能在不同日期触发止损，账户设置的费率在规则回测路径不生效。

改进方向：`ExitRule`、`Fees`/`LotRule`、`Execution` 各自只保留一份实现；建议输入传结构化命中原因，文案在生成层生成。

### 2.4 跨资产抽象名不副实，ETF 路径已经有静默缺口

证据：`backend/src/instrument.rs:27-34` 把资产能力暴露成 `bar_table() -> &'static str`，调用方随即用字符串比较分派：`backend/src/analysis/prices.rs:30-41`、`:106`、`:152`、`:193`、`:239` 全是 `bar_table()=="mkt.etf_bar_1d"` 后复制一套 SQL；`for_each_stock` 与 `for_each_etf` 是两份几乎相同的扫描循环（`backend/src/analysis/prices.rs:297-407`）。绕过抽象的地方也已经出现：`backend/src/account/equity.rs:142-147` 直接 UNION 两张表，`backend/src/advice/review.rs:117` 只查 `mkt.v_stock_bar_qfq`，导致 ETF 建议回看永远取不到后续涨跌。此外每个 `instrument_*` 读函数都先查一次 `ref.instrument`，在持仓和候选循环里形成 N+1。

改进方向：按资产分派的 bar 仓储，扫描循环参数化唯一化，`equity`/`review` 走同一入口并补 ETF 用例。

### 2.5 分层仍有反向依赖

- `advice` 反过来依赖同步层内部：用 `backend/src/advice/mod.rs:166` 的 `sync::scheduler::claim` 做业务日锁，用 `backend/src/advice/inputs.rs:89` 的 `sync::audit::core_backfill_left` 判断能否生成建议。
- `analysis` 反向写应用聚合表：启动时 `backend/src/analysis/screener.rs:439-453` 会改 `app.position`、`app.account.rules`，删除 `app.strategy`，把数据迁移塞进了领域计算模块。
- `sync` 与 `setup` 双向依赖：`backend/src/sync/mod.rs:63` 读 `setup::stored_api_key`，`backend/src/setup.rs:6-8` 又依赖 `sync::Engine`/planner。
- `backend/src/bootstrap.rs:84-99` 重新硬编码三个涨跌停池数据集 ID 并直写 `ops.derived_version`，而 `spec.rs` 已经有一套「角色由 sink 决定」的元数据机制。

改进方向：调度决策上移到 `bootstrap`；`sync` 不再 import `setup`；`analysis` 不写 `app.*`。

## 3. 中等：数据、缓存、资源与配置边界

### 3.1 数据层把类型系统交给 JSONB，配置里又嵌裸 SQL

`ops.v_universe`（`backend/migrations/20261015000001_sync.sql:244-252`）用 `payload->>'asset_type'`、`payload->>'list_date'` 当列；`backend/src/sync/planner.rs:819-833` 把 `backend/config/datasets.yaml:76` 里的 `universe` 原样拼进 WHERE。payload 键改名不会报错，只会让规划器「成功」返回 0 只证券，形成空运行；配置和内部视图列名硬耦合，且启动校验覆盖不到这类语义错误。

改进方向：把 universe 依赖的字段固化为投影列或物化视图，启动时校验 universe 非空。

### 3.2 迁移和 schema 历史债，以及数据库里的业务逻辑

38 个迁移是纯 delta 链：建了又删的表和过程（`backend/migrations/20261017000003_adj_segment_factor.sql:123-125`）、加了又删的字段（`backend/migrations/20261018000009_notify.sql`、`backend/migrations/20261019000018_drop_retired_notify.sql`）、一个 `app.account.rules` JSON 列被连续改四次，结构迁移里还夹着 `DELETE FROM app.preset_stat`（`backend/migrations/20261019000021_account_exit_compat.sql:13`）。`ref.instrument` 同时有 sink 显式写入和兼容触发器两条写路径。`raw.record` 与 `ops.api_call` 声明了分区却只有 DEFAULT 分区，将来真正拆分区只能停机搬历史行。复权公式还在 PL/pgSQL 视图与 Rust SQL 片段中各一份（`backend/src/analysis/prices.rs:30-36` 对 `backend/migrations/20261017000003_adj_segment_factor.sql:114-121`），只能靠注释要求「逐位一致」。

改进方向：定期 squash 基线或由 CI 比对 schema 快照；兼容迁移标注删除期限；分区立即建真实分区或去掉声明；复权公式收敛到一处。

### 3.3 派生数据与缓存更新不原子，失败后不会重试

`backend/src/sync/postprocess.rs:25-45` 先 `mark_rebuilt` 推进 `fired_at`，再执行 `recompute_adjustments`；后者三步（补交易日、`CALL rebuild_adj_segment`、更新 `derived_version`）没有事务（`:81-104`）。重算失败时已推进的水位让 `refresh_if_stale` 认为没有待处理数据，不会重试；而缓存键依赖 `derived_version`（`backend/src/api/mod.rs:59-68`），页面会继续命中旧结果。缓存本身还有两层：Redis 的三个 cache-aside 实现各写一遍（`backend/src/api/insight.rs:81-105`），进程内另有 `OverviewCache`/`StatsCache`，预热模块又手工重建一份 `AppState`（`backend/src/api/precompute.rs:17-27`），预热填不到 API 的进程内缓存。

改进方向：重算与版本更新放同一事务并加锁；失败保留待重算标记；抽统一的 read-through 缓存入口；预热复用 router 的 `AppState`。

### 3.4 资源与并发模型写死在全局，读放大明显

默认连接池 10（`backend/src/config.rs:80-82`），只设 `acquire_timeout`，没有 `statement_timeout`（`backend/src/db.rs:18-33`），而 API、同步写入、全市场扫描共用同一池。全市场计算在请求内跑，准入靠进程级 `static Semaphore` + `task_local` + `block_in_place`（`backend/src/analysis/prices.rs:277-351`、`:432-439`），既不可配置也不可观测。读路径还有典型 N+1：`backend/src/api/watchlist.rs:147-174` 每只自选股调用两次 `recent_instrument_bars`，每次内部再查一次 `ref.instrument`；50 只自选股约 4N+7 条 SQL。

改进方向：拆只读池并设 `statement_timeout`；把并发准入换成注入的可配置调度器；列表接口改批量 JOIN。

### 3.5 事件消费者与周期维护寄生在执行槽里

`backend/src/bootstrap.rs:31-66` 的事件消费者是串行的，且每个 `SyncRunCompleted` 都同步跑一遍全市场巡检加全部账户建议；回补期间每个波次都触发，没有合并、去重，也不限制队列长度。维护逻辑又挂在 0 号执行槽「认领任务之前」（`backend/src/sync/executor.rs:95-100`、`:140-168`），0 号槽跑长任务时租约回收、波次接续、复权重算全都延迟。另外 `backend/src/api/mod.rs:135` 的 120 秒全局超时与「组合回测十几秒到几十秒、可取消」的长任务模型没有对齐。

改进方向：事件合并成可领取的作业；维护改成独立任务；消费者绑定执行者独占权；长任务走显式任务接口而不是普通请求超时。

### 3.6 「配置驱动」并未真正成立

`backend/src/sync/sinks.rs:46-98` 仍按 `(sink, dataset_id)` 硬编码 15 种组合，未命中直接运行期 `bail!`；新增一个复用已有表的数据集仍要改 Rust，与 `AGENTS.md:14`「新增数据集优先只改 datasets.yaml」的期望相反（严格说 `docs/interfaces/dataset_registry.md:122` 已说明类型化表需要改 `sinks.rs`，但入口文档的措辞容易误导）。供应商侧没有 trait 抽象，planner/calendar/executor 都依赖具体 `HithinkClient`；配置又有 env 和数据库两套来源（`backend/src/sync/mod.rs:61-64`），启动时读取一次，部分项（API Key、同步时间）却运行时可改。

改进方向：sink 改成声明式表加字段映射；同步侧定义 `MarketDataClient` trait；收敛配置来源并明确哪些项支持热更新。

### 3.7 API 层承担了应用服务和领域映射职责

`backend/src/api/` 共 6,570 行，handler 直接拼 SQL、直接解析供应商 payload（如 `backend/src/api/market.rs:396-490` 把财报 JSON 映射成 UI 字段）、直接把物理表名和视图名交给 SQL（`backend/src/api/market.rs:539-570`）。响应模型常常就是数据库行（8 处以上 `Serialize + ToSchema + sqlx::FromRow` 同体，如 `backend/src/api/market.rs:43`），`raw.record.payload` 直接成为对外契约（`backend/src/api/sync.rs:207`），改列名即破坏 OpenAPI 与前端生成类型。鉴权是路径和方法白名单（`backend/src/api/auth.rs:36-73`、`:266-284`），与路由注册分离；错误体只有 `ApiError` 是统一 JSON，参数校验和超时等框架错误仍是另一种形态。

改进方向：增加应用服务层与独立 DTO；权限声明随各模块 `routes()` 注册；统一错误包装中间件。

## 4. 前端与部署

### 4.1 前端约定靠复制，缓存失效过粗

12 处无参 `invalidateQueries()`；保存账户设置（`frontend/src/pages/AccountPage.tsx:706`）会把已挂载的全市场账户回测（`:727`）判脏并立即重算几十秒。手写 query key 与 orval 生成的 key 混用（`frontend/src/components/SignalStats.tsx:211`、`frontend/src/components/overlays.tsx:96`）；「冻结参数 + changed + cancel」的长任务状态机复制三份（`frontend/src/components/SignalStats.tsx:52-62`、`frontend/src/pages/StrategyPage.tsx:245-257`、`frontend/src/pages/AccountPage.tsx:734-745`）；可编辑表单用 `useState(serverValue)` 且不回填（`frontend/src/pages/AccountPage.tsx:695-699`）。权限散在路由守卫、`useIsAdmin`、导航表、`fetcher` 四层：导航表里 `who: "all"` 是死字段（`frontend/src/components/Layout.tsx:22-25`），「数据」对所有用户显示，但 `/sync` 被 `AdminOnly` 包住（`frontend/src/router.tsx:104-111`），普通用户点进去只会看到管理员说明页。测试只有 `lib/` 的纯函数，没有 DOM 环境，上述几个高危区域零覆盖。

改进方向：按域提供 invalidate 函数；抽 `useManualQuery` 统一长任务状态；路由集中声明角色并由导航派生；补齐 jsdom + Testing Library。

### 4.2 部署与 CI 的边界和门禁

Redis 在文档里是「可去掉的加速层」（`docs/architecture/read_cache.md:21`），但 Compose 用 `depends_on` 加 `${REDIS_URL:-redis://redis:6379}` 把它变成硬依赖（`deploy/compose.yml:68-70`、`:65`），`.env` 里写空值也关不掉。CI 只跑 `typecheck`/`lint`/`test`（`.github/workflows/ci.yml:67`），`pnpm build` 只在打标签构建镜像时才执行（`deploy/Dockerfile:12`），`openapi.json` 与 generated 的漂移没有任何门禁。一键升级按「旧容器快照」重建容器（`backend/src/upgrader/mod.rs:678-712`），compose 里新增的 environment 和资源上限对运行中的容器永远不生效，声明式配置与命令式重建没有收敛点。

改进方向：Redis 改为可选 profile，空值表示关闭；CI 增加 `pnpm build` 和 OpenAPI 漂移校验；upgrader 在部署目录执行 `docker compose up -d`，让 compose 成为唯一所有者。

## 5. 文档与代码不一致（只读报告）

- `docs/architecture/sync_pipeline.md:55`「任务和运行状态 SQL 只在 queue.rs/run.rs」→ `audit.rs`、`planner.rs` 多处违反。
- `docs/architecture/frontend.md:70`「『说明』和『数据』对所有用户显示」→ `/sync` 实际只有管理员可用。
- `docs/architecture/read_cache.md:21`「可以去掉 REDIS_URL」→ compose 默认依赖 Redis 健康。
- `AGENTS.md:14`「新增数据集优先只改 datasets.yaml」→ 有类型化表时仍必须改 `sinks.rs` 与迁移；`docs/interfaces/dataset_registry.md:122` 的说明更准确，建议把 AGENTS 的措辞收紧。

## 6. 修复顺序建议

1. **收回状态机与事件所有权**：把 `sync_event_pending` 并入 `try_complete` 的同一事务；禁止 `audit`/`planner` 直写 `ops.run`；消费者绑定执行者独占锁或使用可领取的事件表。
2. **建立单一交易模型**：`ExitRule`、`Fees`/`LotRule`、`Execution` 各一份，让巡检、单股回测、组合回测、账户建议、规则回测共用；跨资产改成仓储或枚举分派，修掉 `review.rs` 的股票专用视图。
3. **收口数据契约**：universe 从裸 SQL 改为结构化字段或投影；复权公式只留一处；`recompute_adjustments` 改为事务加可重试；`datasets.yaml` 的 sink 从 ID 硬编码改成声明式列映射。
4. **补门禁**：前端 mutation 精确失效；CI 增加构建与 OpenAPI 漂移校验；Redis 改为可选依赖。

## 7. 审查范围说明

本次审查本身为只读静态审查：未运行构建或测试，未启动服务，未改动业务代码；工作区 HEAD 为 `4dd1618`。报告归档时只新增本文件并更新 `docs/research/README.md`。证据以当前代码为准，行号可能随后续提交变化。
