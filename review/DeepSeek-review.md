# 架构评审：niulai 全仓

> 只读评审报告，记录代码事实与改进建议，不修改任何代码。评审日期 2026-10-23，评审提交 `a63ebec`（fix(sync): skip dead tasks of delisted securities during repair）。依据的架构原则见 `.agents/skills/architecture-review/references/principles.md`。

## 结论

整体结构比同规模项目健康：依赖图无环、无稳定依赖方向的违例，`sync` 的状态机、建议生成的纯函数核心、常驻服务的组件监督都是值得保持的设计，CI 也已经把「迁移只能新增」「接口与生成客户端零漂移」变成了构建失败。

最需要处理的三个问题都在「边界被日常便利穿过」这一类：账户估值有两份实现且 API 层逐只查价；今日热点的业务规则和数据集常量跨落在 `api` 与 `analysis`；建议结果的 JSON 形状靠字符串约定在三个模块里各自理解。它们都不是设计错误，而是近期账户相关改动的热路径上最容易继续扩散的位置。

## 范围与依据

- 评审对象：整个仓库（`backend/src` 24,187 行、`frontend/src` 8,218 行手写加 10,580 行生成、`deploy/`、`.github/workflows/ci.yml`）
- 参考的项目决策：`docs/index.md` 路由到的 `docs/architecture/{sync_pipeline,data_model,frontend,read_cache}.md`；ADR：单进程服务、通用记录库加类型化投影、计算在 Rust 中做、只保留多因子选股、限速在进程内；`AGENTS.md` 的约定
- 使用的工具：`scripts/dep_graph.py --lang rust backend/src`、`--lang ts frontend/src`，两者再按 `--depth 2` 跑一次；`git log --format= --name-only` 统计改动频率；脚本扫描 `ToSchema` 重名与 `AssertSqlSafe` 审计注释
- 关键输出：后端 21 个顶层模块、无依赖环、无「从更稳定指向更不稳定」的边；`api` 扇出 12、扇入 1（I = 0.92），`time`、`config`、`analysis`、`sync` 扇入 4 到 6；前端 `pages → components/lib/api/hooks` 单向，无环。`sync` 的依赖只有 `time`、`vendor`、`readiness`、`cache`、`config`、`scheduling`，确实不依赖 `analysis`
- 改动频率：`backend/src/api/accounts.rs`（26 次）与 `frontend/src/pages/AccountPage.tsx`（28 次）是现存文件中最热的两个，远超其他文件
- 合规抽查：`ToSchema` 类型 129 个，无重名；`#[utoipa::path]` 的响应全部带 `description`；`AssertSqlSafe` 38 处，审计注释大多在，`analysis/prices.rs` 的部分写在了 `format!` 上方而非 `AssertSqlSafe` 上方

## 结构概览

```
server ─► api ─► {sync, analysis, account, advice, auth, cache, setup, operation_log, version, telemetry}
bootstrap ─► {sync, analysis, advice, config, auth}
advice ─► {account, analysis, instrument, readiness, time}
account ─► {analysis, instrument, time}
sync ─► {vendor, time, cache, config, readiness, scheduling}     ← 不依赖 analysis/advice，靠 events.rs 反转
api ─► precompute ─► api::market                                 ← 后台预热依赖 HTTP 处理函数模块
error ─► auth                                                    ← API 错误类型依赖用户模块
```

`sync` 内部按「入口 / 规划 / 执行步骤 / 状态 / 配置」五层组织，`--depth 2` 复核无环；`account`、`advice`、`analysis` 三层方向与文档声明一致。没有需要报告的宏观依赖问题，下面的发现都在模块内部的职责落点。

## 发现

### [中] 账户估值有两份实现，且 API 层按持仓逐只查价

- 原则：单一事实来源（4.6）、DRY（4.1）、副作用隔离（4.5）
- 位置：`backend/src/api/accounts.rs:119`（`quote`）、`:183`（`summary`）、`:222`（`detail` 又查一遍）；对比 `backend/src/account/equity.rs:145`（一次 UNION 读全部持仓日线）、`backend/src/analysis/prices.rs:169`（`recent_instrument_bars_batch`，注释写明「避免列表页面为每只证券发起 N+1 查询」，但只有 `backend/src/api/watchlist.rs:79` 在用）
- 现象：「持仓市值 / 总资产 / 当日盈亏」在 `account/equity.rs` 有一套按交易日批量重算的实现，口径写在模块文档里；`api/accounts.rs::summary` 又有一套只算当天的实现，逐只持仓调 `quote`（每只 1 次 `ref.instrument` 查询加 1 次日线查询）。`detail` 先取一次持仓、再调 `summary` 取第二次持仓并再查一遍价格，账户详情页的查询数约为 `6 + 4 × 持仓数`。`backend/src/advice/inputs.rs:129` 和 `:165` 同样逐只读，每个账户几十次单只查询，而 `bootstrap.rs:55` 在每次同步完成时对所有账户跑一遍 `refresh_all`。
- 代价：两套口径已经开始分头演进。`equity.rs` 对停牌股「沿用之前最近的收盘价」且当天盈亏记 0，`summary` 用最近两根日线的涨跌还原当日盈亏，停牌股的当日盈亏因此会被算成几天前那根 K 线的涨跌。改其中一套时没有任何测试会提示另一套不一致。同时持仓数或账户数一涨，首页、账户详情页和每次同步后的建议生成都会线性变慢。
- 建议：把「按持仓算市值与当日盈亏」收进 `account`（复用 `equity.rs` 的口径）一个函数，内部用 `recent_instrument_bars_batch` 一次读全；API 只做字段映射。半天量级，下一次改账户展示时顺带做即可。

### [中] 今日热点的规则、数据集常量与预热路径跨了 `api` 和 `analysis` 两层

- 原则：关注点分离（1.1）、信息隐藏（2.1）、DRY（4.1）
- 位置：`backend/src/api/market.rs:851`（`market_hot`）、`:914`（`compute_hot`，约 190 行）、`:1056` 到 `:1163`（8 个解析供应商 payload 的辅助函数）、`:743` 到 `:749`（`HOT_LIMIT_UP` 等常量）；重复定义在 `backend/src/analysis/limit_pool.rs:13` 到 `:19`；反向依赖在 `backend/src/api/precompute.rs:47`、`:70`
- 现象：`analysis/mod.rs` 声明「这里只做计算和读取价格，HTTP 参数校验和响应格式在 `api` 中」，`api/insight.rs` 的市场概况也确实只做委托。今日热点是唯一例外：连板分组、组内排序、截断到 50 只、四种空态文案、按 `ticker` 补代码全写在 `api/market.rs`，`analysis` 侧只有 `limit_pool` 认识同一批数据集。后台预热组件（不是 HTTP 层）反过来调用 `api::market::{hot_trade_date, compute_hot}` 和 `api::derived_cache_key`。
- 代价：改一个池的数据集 ID 或增加第五个数据集（例如「昨日炸板今日溢价」），要同时改 `api/market.rs`、`analysis/limit_pool.rs`、`api/precompute.rs` 三处，漏一处就会出现「市场概况用新池、热点页用旧池」这种无报错的不一致。`api/market.rs` 已经 1,163 行、混了证券、指数、热点三个业务主题；想复用预热逻辑或把计算挪出 API 进程时，得先把它从 `api` 里剥出来。
- 建议：把 `compute_hot` 与四个 dataset ID 常量搬到 `analysis`（`market` 或新的 `hotspot` 模块），`api/market.rs` 只留参数解析和缓存装配，顺带把它按「证券 / 指数 / 热点」拆文件。`docs/domains/hotspots.md` 需要同步改一句代码位置。成本约一天，可在下次调整热点口径时做。

### [中] 建议结果的 JSON 形状靠字符串约定，跨三个模块读写同一批表

- 原则：信息隐藏（2.1）、单一事实来源（4.6）、显式优于隐式（4.7）
- 位置：`backend/src/api/accounts.rs:171`（`jsonb_path_exists(a.result, '$.items[*] ? (@.action == "sell" || @.action == "buy" || @.action == "reduce")')`）、`api/accounts.rs:739`、`:771`（直接写和删 `app.advice_item_state`）、`account/mod.rs:578`（`record_advice`）、`sync/retention.rs:89`（删除 `app.advice`）、`advice/mod.rs:381`、`advice/inputs.rs:102`、`advice/review.rs:144`
- 现象：`action` 是 `advice/generate.rs:95` 的枚举（`sell`、`buy`、`hold`、`wait`），但有一处判定不是在 Rust 里做，而是把枚举值拼进 JSON Path 字符串；那里残留的 `"reduce"` 是已删除的旧动作，现在永远匹配不到。`executed` 与 `skipped` 两个状态字面量出现在 `advice`、`account`、`api` 三个模块的 SQL 里。删建议的保留策略写在 `sync` 模块。
- 代价：改 `Action` 变体名、给建议项加字段或换存储结构时，编译器完全看不到这七处；结果是「检查提示不再触发」「跳过状态写不进去」这类静默失效，`"reduce"` 残留说明这条路径已经漂移过一次。`sync::retention` 里删 `app.advice` 会让「建议保留多久」这个业务决策住在同步模块里，改保留策略的人得先知道去那里找。
- 建议：优先让 `api/accounts.rs:171` 的条件字符串由 `Action::key()` 生成，或改成在 `advice` 里提供 `has_unhandled_items(account_id, since)`；把 `retention.rs` 的 advice 清理挪进 `advice` 模块（表在谁那里，清理规则就在谁那里）。事务边界（`record_advice` 同时写流水和建议状态）是合理的，不用动。成本约半天。

### [中] 进程级隐式全局状态决定就绪判断与并发槽位

- 原则：显式优于隐式（4.7）、依赖规则（1.2）、可测试性（5.3）
- 位置：`backend/src/readiness.rs:6` 到 `:20`（`CORE_DATASETS` 是 `OnceLock`，未初始化时 `core_backfill_left` 返回 0，即「已就绪」），写入点在 `sync/mod.rs:52`（`Engine::new`）；`advice/ranking.rs:22`（全局排名缓存）、`advice/mod.rs:183`（全局「重新生成中」表）、`analysis/prices.rs:268` 到 `:280`（两个静态 `Semaphore` 加 `task_local` 的 `in_background`）
- 现象：四个跨模块共享的可变状态都是模块级静态量，其中 `readiness` 的缺省值是「已就绪」，`in_background` 是一种必须由调用方记得包一层的隐式模式切换（`bootstrap.rs:33`、`:56` 记得包，其他路径靠自觉）。
- 代价：今天所有入口（含 CLI）都在 `bin/niulai.rs:139` 构造 `Engine`，所以没出事；但只要新增一个不经过 `Engine::new` 的路径（新的 CLI 子命令、只读诊断工具、集成测试里自己拼 `PgPool`），`advice::inputs::shared` 会静默地认为核心数据已齐，用半份日线生成建议且不报错。ADR：单进程服务留下的「以后用 `--no-api` 实例或独立进程做重计算」这个选项，也会被这几个静态量卡住。
- 建议：把 `CORE_DATASETS` 变成 `core_backfill_left(pool, &[String])` 的显式参数（由调用方从 `Engine` 传下来），或至少在未初始化时返回错误而不是 0；`in_background` 改成一个显式的执行器参数（例如 `engine.background(|| ...)`）。成本约半天，建议在下次动建议或预热路径时做。

### [低] 前端硬编码后端的数据集 ID 和分类顺序

- 原则：单一事实来源（4.6）、DRY（4.1）
- 位置：`frontend/src/pages/SyncPage.tsx:47`（`ETF_DATASET_IDS`）、`:465`（`if (!d) return null`）、`frontend/src/lib/labels.ts:64`（`DATASET_CATEGORIES`）
- 现象：`backend/config/datasets.yaml` 是数据集注册表的唯一来源，前端还手抄了一份「ETF 相关的三个 ID」和一份分类顺序；注释里写着「与 datasets.yaml 的 category 一致」。
- 代价：改一个数据集 ID 或分类名，前端卡片会静默消失（`return null`），`typecheck`、`lint`、`test` 都不会报；分类改名后排序退化到末尾而不是报错。
- 建议：`/datasets` 已经返回 `category`，让后端一并返回分类顺序与「行情同步摘要」需要的 ID 列表，或在前端加一个测试断言这些常量都在 YAML 导出的集合里。成本很小。

### [低] 复权口径在 SQL 视图和 Rust 读取里各写一份，只有注释保证一致

- 原则：DRY（4.1）、架构适应度函数（5.2）
- 位置：`backend/src/analysis/prices.rs:1` 到 `:45`（`BAR_COLUMNS` 里的 `b.close * coalesce(s.qfq_factor, 1)`）对比迁移里的 `mkt.v_stock_bar_qfq`；测试只在两端分别断言（`backend/tests/integration.rs:195`、`:298`、`:447`），没有一处断言两条路径给出同一组价格
- 代价：改复权定义（例如分段的边界、`coalesce` 的缺省、四舍五入）时，K 线接口与因子、市场概况会算出不同价格，而 CI 全绿。这是本项目最贵的静默漂移类型，因子研究的结论直接依赖价格口径。
- 建议：加一个集成断言：拿模板日线分别走 `/securities/{code}/bars?adjust=qfq` 与 `prices::instrument_bars`，逐根比较 `close`（6 位小数内）。一次写，长期兜底。

### [低] `AssetType` 抽象被字符串比较和手写 UNION 绕过

- 原则：信息隐藏（2.1）、显式优于隐式（4.7）
- 位置：`backend/src/analysis/prices.rs:113`、`:153`、`:251`（`AssetType::from_db(x).bar_table() == "mkt.etf_bar_1d"`）、`backend/src/api/market.rs:227`、`:270`、`:539`（`asset_type == "etf"`）、`backend/src/account/equity.rs:145`、`backend/src/api/watchlist.rs:66`（手写 `stock_bar_1d UNION ALL etf_bar_1d`）
- 现象：`instrument::AssetType` 已经把「资产类型 → 日线表 / 手数 / 印花税」收在一处并有单元测试，但读取路径大量退回字符串判断和手写 UNION。
- 代价：新增一种资产类型（LOF、可转债）时，编译器只会提示 `AssetType` 的 `match` 不全，上面这些分支全部静默按股票处理；`bar_table() == "mkt.etf_bar_1d"` 这种「把抽象输出和字面量比较」的写法在改 `bar_table` 返回值时会静默走错分支。
- 建议：这几处改成 `matches!(AssetType::from_db(..), AssetType::Etf)`，批量 UNION 抽成 `prices` 里的一个「两类日线合并」常量或函数。成本很小，可以顺手做。

### [低] 缓存装配样板在四个地方重复

- 原则：DRY（4.1）、深模块（2.2）
- 位置：`backend/src/api/insight.rs:39`、`:95`，`backend/src/api/market.rs:862`，写回在 `backend/src/api/precompute.rs:56`、`:70`、`:89`，键格式在 `api/mod.rs:55`
- 现象：每个需要共享缓存的接口都手写一遍「读缓存、命中返回、计算、写回、失败只记 debug 日志」。
- 代价：换键格式、改 TTL、加「预热与按需计算共用同一入口」时，要改 6 处，漏一处会出现预热写入的键和接口读取的键不一致（表现为缓存永不命中，只在 Redis 里悄悄变成两倍键）。
- 建议：在 `api/mod.rs` 加一个 `cached_json(&state, key, || compute)` 之类的深模块，把降级与日志收进去；以后新增页面只需一行。

### [低] `MarketDataClient` 只有一个实现，也没有测试替身

- 原则：过度设计的信号（第 6 节「只有一个实现的抽象」）、YAGNI（4.2）
- 位置：`backend/src/vendor/mod.rs:11`（`pub trait MarketDataClient`），`sync/{fetch,planner,executor,calendar,mod}.rs` 里 8 处 `dyn MarketDataClient`
- 现象：只有 `HithinkClient` 一个实现；集成测试用的是真实 `HithinkClient` 加一个本地 mock HTTP 服务（`backend/tests/integration.rs:738`、`:755`），没有 mock 这个 trait。
- 代价：一次动态分发和一层 `async_trait` 间接是小事；真正的成本是它没有兑现承诺。换供应商时端点名、分页、错误码、导出文件格式都还散布在 `sync/fetch.rs` 与 `sync/sinks.rs` 里。
- 建议：如果近期的确有第二个数据源的计划，保留并在 ADR 里写明「trait 只隔离传输层」；如果没有，`Arc<HithinkClient>` 更简单，等真需要时再抽。观察即可，不建议现在动。

### [低] 页面之间直接引用组件

- 原则：前端分层（页面不互相引用）、共同闭包（1.6）
- 位置：`frontend/src/pages/SyncPage.tsx:44`（`import { ChainProgress } from "./RunDetailPage"`）
- 现象：全仓唯一一处跨页面引用，被引用的 `ChainProgress` 是「同步记录和详情共用」的展示组件。
- 代价：改 `RunDetailPage` 的导出或拆分它时会牵连同步页；`RunDetailPage` 一旦引入页面级依赖，就会形成页面之间的双向耦合。
- 建议：把 `ChainProgress` 移到 `components/`（或 `components/ui.tsx` 旁），成本几分钟。

## 值得保持的结构

- **`sync` 的分层与状态归属**：`queue.rs` 独占任务状态迁移、`run.rs` 独占运行终态判定、`events.rs` 只有事件枚举、绝不引用 `analysis` 和 `advice`，所以「同步完成 → 生成建议」是从低层向高层反转的，没有形成环。这是这份代码里最值钱的边界，新增同步能力时应继续只走事件。
- **建议生成的纯函数核心**：`advice/generate.rs` 是数据进、数据出的纯函数，`inputs.rs` 负责装配，`generate.rs` 有 6 个单元测试，`analysis/factor_lab.rs` 的数学部分同样纯函数化。这让 A 股最容易出错的口径（调仓、费率、手数）能脱离数据库测试。
- **`server.rs` 的组件监督**：`watch` 停止信号加 `JoinSet`，任一组件意外退出即整体失败，停机超时后取消，避免了「API 还活着、同步已经停了」的半死状态。
- **CI 里已有的适应度函数**：`clippy --all-targets -D warnings`、迁移文件只增不改、`openapi.json` 与 `src/api/generated` 零漂移、前端 `typecheck && lint && test`。这类检查越多，`AGENTS.md` 里的约定越不需要靠人记。
- **`datasets.yaml` 声明式注册表**：新增只写 `raw.record` 的数据集确实只改配置，这条决策在 `sync/spec.rs` 里守住了（sink 与类型化表的耦合被收在 `sinks.rs` 一处）。

## 对现有决策的意见

- **ADR：单进程服务**依然成立，但它隐含一个前提：业务计算不要住在 `api` 层。上面第 2、4 条发现各占一半：热点计算和预热依赖 `api::market`，以及几个「只有 `Engine::new` 才初始化」的静态量，会让「以后用 `--no-api` 实例或独立计算进程」这个保留选项变贵。如果这个选项还想留着，优先把热点计算和 `ops.backtest_run` 的写入（现在在 `api/research.rs:189`）搬进 `analysis`。
- **ADR：通用记录库加类型化投影**带来的下游成本值得记一笔：只写 `raw.record` 的数据集确实不用建表，但读取它们需要知道数据集 ID 和 payload 字段，这些知识目前分别落在 `api/market.rs`、`analysis/limit_pool.rs`、`analysis/factor_lab.rs`。建议在 `analysis` 里加一个很小的「只读 raw 数据集」入口（数据集常量加按日期取记录），让「新增数据集只改 `datasets.yaml`」这条决策在读取侧也成立。

## 未覆盖与待确认

- 未运行 `cargo clippy` 与 `cargo test`（本次是只读评审，集成测试需要 `NIULAI_TEST_DATABASE_URL`）。第一条发现里的查询数是按代码路径推算，建议实测一次账户详情页的 SQL 条数确认。
- `backend/migrations/` 只读了 `docs/architecture/data_model.md` 与迁移规则，未逐条评审 SQL 与索引设计；`deploy/` 只看结构与 `ci.yml` 的对应关系。
- `backend/src/upgrader/`（1,953 行）只做了结构阅读：它是短生命周期的旁路容器，未按常驻服务标准评审；`backup.rs`、`docker.rs`、`migrate.rs` 的错误处理与幂等性需要单独一轮。
- 前端抽查了 `AccountPage`、`SyncPage`、`ResearchPage`、`RunDetailPage`、`components/ui.tsx` 与 `lib/`，其余页面与组件未逐一阅读；`lib/labels.ts` 那份 158 行的后端枚举中文名映射未评估（属于项目已声明的约定，只在条目增多时有 DRY 风险）。
- 「每个 `AssertSqlSafe` 上方都要有审计注释」这条约定，在 `analysis/prices.rs` 里部分写在 `format!` 上方而不是 `AssertSqlSafe` 上方，语义上说得通，是否算违例建议团队确认后固化成 CI grep 检查。
