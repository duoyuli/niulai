# 架构评审：niulai 全仓（`backend/src`、`frontend/src`）

## 结论

整体结构健康。前后端的顶层模块都没有依赖环，也没有逆着稳定方向的依赖。同步层通过领域事件和业务层解耦，而且事件会写进数据库、可以重放，这部分设计得好。

最需要处理的三个问题：

1. **股票和 ETF 的区别处理得太分散。** 分表规则和「分析价格」口径散落在 6 个以上模块。`AssetType` 本来就是为集中这件事而建的，却被各处绕过。
2. **账户因子排名缓存失效时机不对。** 它靠进程内调用 `clear()` 失效，而系统已经有一套按 `ops.derived_version` 版本键失效的机制，用的不是这套，有一个能复现的过期场景。
3. **涨跌停热点的业务计算和后台预热放在了 `api` 层。** 同类的市场概况、板块强弱都在 `analysis`，两者位置不一致。

最值得保持的是：`sync` → 事件 → `bootstrap` 编排这条依赖方向，以及建议生成和因子计算都写成纯函数。

## 范围与依据

- **评审对象**：整个仓库。先看宏观依赖，再抽查规模大、改动多、扇入高的模块：`api/market.rs`、`api/accounts.rs`、`account`、`advice`、`analysis/prices.rs`、`sync/mod.rs`、`server.rs`、`bootstrap.rs`、`frontend/src/lib`。
- **参考的项目决策**：`docs/index.md`、`docs/project_overview.md`、架构索引、决策索引，重点读了 ADR「单进程服务」「限速在进程内」「Redis 共享读取缓存」「只保留多因子选股」。还有各模块 `//!` 注释里声明的依赖方向，例如「analysis 不知道账户的存在」「同步层只发布事实」。
- **使用的工具**：
  - `dep_graph.py` 分别跑了后端和前端，结果是 0 个环、0 条违反稳定方向的边。
  - 统计了 SQL 在各文件的分布、全局 `static`、`tokio::spawn`、`env::var` 的位置。
  - 用 `git log` 看了文件改动频率，并查了 ETF 接入相关的提交。

## 结构概览

后端分层基本清楚：

```
server / bin ──► api ──► advice ──► account ──► analysis ──► instrument / time
                  │         └──────────────────────┘
                  ├──► sync ──► vendor / cache / readiness / scheduling
bootstrap ──► sync(events) + analysis + advice（同步完成后的编排）
```

- **`api`**：扇出 12，不稳定度 I = 0.92，处在最外层，这是正常的。
- **`analysis`**：I = 0.33，不引用 `account`、`advice`、`api`、`sync`，和注释里声明的方向一致。
- **`sync`**：不依赖任何业务模块。依赖 `cache` 只是因为 `Engine` 持有缓存对象，见发现 4。

前端的依赖方向是 `pages → components → hooks/lib → api(generated)`，单向，没有手写的 `fetch`（只有 `api/fetcher.ts` 一处）。

## 发现

### [中] 股票和 ETF 的分表与「分析价格」口径散落在多个模块，`AssetType` 被绕过

- **原则**：信息隐藏，DRY（算法耦合跨越了模块）
- **位置**：
  - `backend/src/instrument.rs:26`：`bar_table()` 是本该集中的地方。
  - `backend/src/analysis/prices.rs:113`、`:153`、`:251`：三次写 `AssetType::from_db(..).bar_table() == "mkt.etf_bar_1d"`，拿表名字符串来判断类型。
  - `backend/src/advice/review.rs:112-120`：自己用 `UNION ALL` 拼了 `v_stock_bar_qfq` 和 `etf_bar_1d`。
  - `backend/src/api/market.rs:538`（`bar_view` 自带一张映射表）、`:229`/`:234`、`:339`/`:345`、`:887`。
  - `backend/src/api/watchlist.rs:67-69`。
  - `backend/src/analysis/market.rs:15-25` 和 `backend/src/sync/postprocess.rs:78-84`：各写了一遍「股票加 ETF 的交易日并集」。
- **现象**：
  - 「ETF 用供应商前复权价、股票用等比前复权视图」这条口径，在 `prices.rs` 有一份权威实现，`review.rs` 又复制了一份 SQL。
  - `hot_trade_date`（`api/market.rs:880`）的注释写着「和 /market/overview 一样」，实际只查 `stock_bar_1d`，而 overview 用的是股票和 ETF 的并集，两者已经不一致。
  - 资产类型在 26 个 DTO 和领域结构体里是 `String`（例如 `advice/generate.rs:38`、`api/market.rs:49`）。`AssetType::from_db` 遇到不认识的值会静默当成股票。
- **代价**：已经发生过。ETF 接入那次提交 `6fbb89f` 改了 28 个 Rust 文件，紧接着又需要 `041d3c1 Harden ... ETF backtests` 补修。以后如果改 ETF 的分析价格口径（比如换成自己算的等比复权），或者新增一类资产（指数、可转债），要逐个找这些 SQL；漏掉任何一处，复盘收益、自选、行情页就会用不同的价格。
- **建议**：
  1. 把「一只证券的分析价格」收敛成一个出口，可以是一个 SQL 视图（比如 `mkt.v_analysis_bar`），也可以是 `analysis::prices` 里唯一的读取函数。`review.rs`、`watchlist.rs` 改成调用它。
  2. 让 `AssetType` 实现 `sqlx::Type` 和 `ToSchema`，替换掉 DTO 里的 `String`，同时去掉 `from_db` 的静默默认值。
  3. 交易日并集收敛到 `analysis::market::resolve_date` 一处。
- **成本与时机**：中等，大约一到两天。如果近期不打算新增资产类型，可以先只做第 1 步。

### [中] 账户因子排名缓存靠进程内 `clear()` 失效，和已有的版本键机制并存

- **原则**：单一事实来源（缓存失效），显式优于隐式
- **位置**：
  - `backend/src/advice/ranking.rs:22-32`、`:38`：缓存键是 `日期|因子`。
  - `backend/src/advice/mod.rs:252`：失效只在同步完成后调用 `ranking::clear()`。
  - 对比的正确做法：`backend/src/analysis/market.rs:222-246` 的 `OverviewCache` 和 Redis 缓存，都把 `ops.derived_version` 放进键里。
- **现象**：`sync/postprocess.rs:94` 重算复权后会更新 `derived_version.adj_factor`，市场概况和 Redis 缓存都会因此失效。但排名缓存不看这个版本，只认本进程内的 `clear()` 调用。
- **代价**（能复现的场景）：
  - `niulai serve` 运行时执行 `niulai recompute`（`bin/niulai.rs:230-231`）。这个命令只重算复权、更新版本号，不发事件。之后 serve 进程里用户改持仓触发的 `regenerate_soon`，仍然用旧复权价算出的排名，而首页概况已经换成新价格。
  - 文档允许额外启动 `serve --no-worker` 实例（`in_process_rate_limit.md`）。待处理事件只会被其中一个进程消费，另一个进程的排名缓存不会被清空。
- **建议**：把 `CacheVersions`（adj_factor、limit_pool）加进排名缓存的键，然后删掉 `clear()`。成本约半天，改动局限在 `ranking.rs`。

### [中] 涨跌停热点的业务计算和首页预热写在 `api` 层

- **原则**：关注点分离与分层，共同闭包原则（CCP）
- **位置**：
  - `backend/src/api/market.rs:851-1160`：`hot_trade_date`、`compute_hot`、`limit_up_stock`、`hot_stats` 等，大约 300 行。
  - `backend/src/api/precompute.rs:59-67`：后台组件反过来调用 `super::market::compute_hot`。
- **现象**：
  - `api/market.rs` 有 1163 行，在一个文件里放了证券列表、基本面解析、日线、指数、热点五个主题。
  - 热点部分直接解析 `raw.record` 里供应商原始的 JSON 字段（`thscode`、`ticker`、连板数），还写死了领域规则「涨停池 2020-07-22 起才有」。
  - 而同类的市场概况、板块强弱都在 `analysis::market`，并且已经由 `analysis` 负责缓存版本。
- **代价**：
  - 供应商改了涨跌停池字段名，要改的是 API 处理文件。
  - 想让调仓建议或 CLI 用到热点数据（比如在建议理由里标注「连板」），只能从 `advice` 引用 `api`，依赖方向就反了。
  - 预热是一个由 `server` 启动的常驻组件，却放在 `api` 下面。
- **建议**：
  - 把 `compute_hot` 和 payload 解析挪到 `analysis::hotspots`（文档 `domains/hotspots.md` 已经有这个领域），`api` 只保留路由和缓存读写。
  - `precompute` 也可以挪到 `analysis` 或 `server` 旁边。
- **成本与时机**：约半天，纯搬移。只要没人从别处复用热点数据，可以等到下次改热点功能时顺手做。

### [低] `sync::Engine` 兼任整个应用的依赖容器

- **原则**：接口隔离原则（ISP），共同闭包原则（CCP）
- **位置**：`backend/src/sync/mod.rs:39-95`，`backend/src/api/mod.rs:37-55`
- **现象**：`Engine::new` 除了做同步相关的初始化，还负责：
  - 创建 Redis 缓存（同步层本身不用它）；
  - 从 `app.setting` 读取 API Key；
  - 写全局的 `readiness::CORE_DATASETS`。

  `AppState` 从 `Engine` 派生，同时又重复保存了 `pool` 和 `cache`。`api`、`setup` 的处理函数拿到的是整个 `Engine`。
- **代价**：以后新增一个应用级资源（比如通知渠道、第二个缓存），自然会被塞进 `sync`。要测试只读接口，也必须构造带供应商客户端和注册表文件的完整 `Engine`。
- **建议**：把缓存的创建和 `CORE_DATASETS` 的初始化挪到 `bootstrap` 或 `server`，由 `AppState` 分别持有。`Engine` 只保留注册表、供应商客户端和事件总线。目前代价不大，可以暂缓。

### [低] 后台任务和进程级标志不受 `server` 的组件监督

- **原则**：异步任务生命周期由入口集中管理（见 `rust.md`），显式优于隐式
- **位置**：
  - `backend/src/api/sync.rs:85`（数据核检）、`:344`（规划运行）；
  - `backend/src/advice/mod.rs:183-230`（`PENDING` 加 `tokio::spawn`）；
  - `backend/src/sync/check.rs:28`（`static RUNNING`）。
- **现象**：`server.rs` 用 `JoinSet` 和停机期限认真地监督各个常驻组件。但这三类任务由处理函数自己启动后就脱离了监督，「同时只运行一个」的判断靠进程内的全局变量。
- **代价**：
  - 停机或进程重启时，这些任务会被中途取消，没有收尾，也不会重试。
  - 多实例部署时，`RUNNING` 只能防住本进程。
  - 测试也受了影响：提交 `3dd7170 test: wait for the in-process data check guard` 就是为了等这个全局标志。
- **建议**：去重标志改用数据库咨询锁，或者在状态表里加一个 running 标记。规划任务的状态本来就在 `ops.run` 里，确认它中断后能被回收即可（见「未覆盖与待确认」）。

### [低] 「改了账户就重新生成建议」由 API 处理函数逐个调用

- **原则**：迪米特法则，时序耦合跨越了模块边界
- **位置**：`backend/src/api/accounts.rs:259-264`，在 `:290`、`:321`、`:404`、`:446`、`:462`、`:497` 共 6 处调用 `regenerate`。
- **现象**：「账户数据变了，建议就要重新生成」这条规则只靠 API 层的调用习惯维持。`record_advice` 和 `ignore_rights` 没有调用，从业务上看大概是有意的，但代码里没有写明原因。
- **代价**：以后新增修改账户的入口（比如 CLI 导入、批量粘贴），很容易忘记触发，用户看到的建议就是旧的。
- **建议**：在 `advice` 里提供一层「修改账户并安排重新生成」的包装，或者让 `account` 的写操作返回一个是否需要重新生成的标记，并在不需要重新生成的入口写注释说明原因。

### [低] `analysis::prices` 同时承担运行时并发工具

- **原则**：高内聚
- **位置**：`backend/src/analysis/prices.rs:279`（`in_background`）、`:359`（`run_cpu`）。`bootstrap.rs:33`、`:54` 和 `advice/ranking.rs` 都通过 `prices::` 引用它们。
- **现象**：调度计算资源的工具放在「价格读取」模块里。
- **建议**：挪到 `analysis::compute` 之类的模块，或者放进 `server` 附近。只是移动文件，可以等到下次改这些函数时一起做。

### [低] 已声明的依赖方向没有自动检查

- **原则**：架构适应度函数
- **现象**：模块注释写明了「analysis 不知道账户的存在」「同步层只发布事实」，但 CI 里只有迁移不可修改、API 生成无漂移、clippy 这几项检查，没有检查依赖方向。
- **建议**：加一个单元测试，扫描 `src/analysis/**`、`src/sync/**` 里的 `crate::(account|advice|api)` 引用并断言为空。大约 20 行，成本很低。

## 值得保持的结构

- **同步层和业务层的解耦**：`sync` 只通过 `DomainEvent` 发布事实，由 `bootstrap::run_events` 编排复权后处理和建议刷新。待处理事件写在 `ops.run.params` 里，可以重放。这道边界让同步框架可以独立演进，也让事件处理失败后能补做。
- **纯函数核心**：`advice::generate` 是纯函数，`advice::inputs` 负责从数据库组装输入；`factor_lab` 把读数据和计算分开。账户排名复用了因子研究的同一套 `factor_lab::ranking`，研究结论和实际调仓的口径一致。不要在 `generate.rs` 里加查询。
- **组件监督**：`server.rs` 用 `JoinSet` 管理常驻组件，任一组件意外退出就让整个进程失败退出，并且在 Docker 的停机期限内收尾，避免执行循环悄悄停掉而 API 还显示正常。
- **派生结果的版本键**：`ops.derived_version` 配合 `CacheVersions`，让概况缓存和 Redis 缓存在多进程下也能正确失效。这是排名缓存应该采用的同一套做法。
- **前后端契约**：`api/generated` 是唯一来源，CI 检查生成代码没有漂移，前端没有手写请求，依赖单向。

## 对现有决策的意见

ADR「单进程服务」和「限速在进程内」允许额外启动 `serve --no-worker` 实例。但目前有几处状态只在单个进程内有效：排名缓存、`check::RUNNING`、`advice::PENDING`、`calendar::LATEST`。建议二选一，并写进 ADR：

- 要么在 ADR 里写明「部署只运行一个 serve 进程」；
- 要么要求所有进程内状态都按数据库版本键失效，或改用数据库锁。

现在文档承诺了多实例，但代码默认只有一个进程，两边对不上。

## 未覆盖与待确认

- **没有细看的部分**：`upgrader`、`auth`、`vendor` 的内部实现，`sync/planner.rs` 和 `scheduler.rs` 的状态机细节，前端大页面（`AccountPage.tsx` 805 行、`SyncPage.tsx` 770 行）内部的职责划分。
- **待确认**：
  - `api/sync.rs:344` 规划中途被取消时，`ops.run` 能否被回收或重新规划。我没有追到这部分代码。
  - `hot_trade_date` 只看股票日线，是不是有意的。如果是，应该把「和 overview 一样」那句注释改掉。
- **观察**：
  - `account`、`advice`、`sync` 的领域结构体直接带 `ToSchema`，同时充当 API 契约。前端由生成代码兜底、CI 会发现漂移，所以目前可以接受。但改内部字段就等于改 API，要留意。
  - `backend/tests/integration.rs` 是一个 137 KB 的单文件，按领域拆开后定位失败会更容易。
