# niulai 服务现状文档核对

审查日期：2026 年 10 月 27 日（America/Chicago）。目标服务：当前仓库的 `niulai serve`，重点是因子研究的启动、调用、结果解释和排错。基准提交：`6e05023d513ea19a28f89d0f57babe5e9abf25d4`。开始时 `git status --short --branch` 为 `## HEAD (no branch)`，无已修改或未跟踪文件；`git rev-parse HEAD` 返回上述提交。

本报告是过程材料，不替代 [工程文档](../index.md)。本次只修改 `docs/` 中的服务现状说明和必要的直接索引，不修改业务代码、迁移、部署文件或生成客户端，不提交、发布或操作生产环境。历史 ADR 用于确认意图，研究笔记不作为当前收益、供应商能力或性能的证明。

## 范围与方法

已使用当前会话加载的仓库内 `project-doc` skill，完整阅读 `docs/index.md`，再读取分类索引和相关现状章节；中文改写使用 `tech-doc-style-chinese`。主要目标文档是 [因子研究](../domains/factor_research.md)，支撑文档是 [运维手册](../operations/runbook.md)、[REST API](../interfaces/rest_api.md)、[配置](../interfaces/configuration.md)、[前端](../architecture/frontend.md)、[共享读取缓存](../architecture/read_cache.md) 和 [项目概览](../project_overview.md)。其他领域、数据目录和同步机制仅核对直接依赖，不宣称完成所有业务主张的逐项审计。

证据分为源事实、测试证据、运行证据、历史决策、推断和未验证。已有测试的断言属于源事实；只有本轮实际执行成功后才记为测试证据。现状文档不是事实来源。

## 文档核对表

位置以文件和稳定章节 ID 表示，避免正文重排后行号失效。表中结论是修改前的状态；动作列记录本轮处理方式。

| 文档位置与重要主张 | 依据 | 证据类型 | 原结论 | 处理动作 |
| --- | --- | --- | --- | --- |
| `factor_research.md` 开篇未明确 ETF、不自动交易和服务边界 | `factor_lab::load` 按 `AssetType::Stock` 过滤；`api/research.rs::routes`；`research_run.rs` | 源事实 | 缺失 | 补充股票研究、同步 HTTP 请求和持久化边界 |
| `factor_research.md#catalog`：12 个因子及方向 | `analysis/factors.rs::FactorId`、`CATALOG`、`PriceFactors::value` | 源事实 | 准确，但方向易被理解为收益保证 | 保留口径，明确它是实现约定 |
| `factor_research.md#catalog`：`GET /strategies/catalog` | `api/mod.rs::routes`、`api/research.rs::routes`；`decisions/factor_only.md` | 源事实、历史决策 | 过时 | 删除无效调用说明，保留实际目录接口 |
| `factor_research.md#data`：完整运行最多 2 个槽位 | `analysis/compute.rs::run`；`research_run.rs` | 源事实 | 含糊 | 明确每进程前台 2 个、后台另 1 个，排队也是请求的一部分 |
| `factor_research.md#data`：270 个交易日预热、12 字节日线、90 MB | `factor_lab::load`、`CBar`；`factors::WARMUP_BARS` | 源事实、历史测量未验证 | 混合当前机制与历史容量 | 保留预热机制，将容量与耗时分离为历史记录 |
| `factor_research.md#data`：交易日历、`thin_days` | `factor_lab::load`、`finish`、`simulate` | 源事实 | 基本准确 | 补充缺口检测不等于数据完整性认证 |
| `factor_research.md#data`：股票池排除条件 | `analysis/universe.rs::contains`、`api/research.rs::PoolQuery` | 源事实 | 含糊 | 明确未复权价格、本次加载的日线根数、退市日之前仍可入池 |
| `factor_research.md#data`：基准使用前一天在池股票的收益 | `factor_lab::finish` 使用相邻两根有效日线 | 源事实 | 不准确 | 改为前一根日线在池；停牌间隔收益记在复牌日 |
| `factor_research.md#data`：财报可用日、行业归属 | `factor_lab::load_fundamentals`、`load_industries`；`factors::available_on`、`fundamental_at` | 源事实；供应商原样本未验证 | 含糊 | 区分当前近似规则与对真实首次披露的推断 |
| `factor_research.md#test`：取样不重叠，因此 t 值不会虚高 | `factor_lab::evaluate`、`forward_return` | 源事实、推断 | 断言过强 | 保留固定间隔，说明延期卖出和统计依赖的边界 |
| `factor_research.md#test`：预处理、50 只门槛、合成 | `factor_lab::process`、`standardize`、`evaluate` | 源事实 | 基本准确 | 保留，明确有效因子和前向收益配对数量 |
| `factor_research.md#test`：统计字段、PBO、Deflated Sharpe | `factor_lab::evaluate`、`overfit`；`analysis/research.rs` | 源事实 | 阈值易被当成充分证明 | 说明实现采用的阈值，不承诺样本外表现 |
| `factor_research.md#run_record`：每次运行保存完整配置和摘要 | `research_run::record`、`trials`；`20261023000003_review_fixes.sql`、`20261024000002_drop_transitional.sql` | 源事实 | 缺少写入时点和范围 | 补充成功请求写库、全库试验计数、无行情快照和完整成交回读 |
| `factor_research.md#portfolio`：按 20 个交易日调仓 | `factor_lab::load`、`simulate`；`decisions/factor_only.md` | 源事实、历史决策 | 准确但容易与账户自然月混淆 | 明确研究与账户不是同一调仓日历 |
| `factor_research.md#portfolio`：买入预算用开盘总值 | `simulate` 中 `open_value` 使用保留持仓的 `h.last` | 源事实 | 不准确 | 明确卖出后现金加最近收盘估值，不是全部按当日开盘重估 |
| `factor_research.md#portfolio`：缓冲区、延期、费用、零股 | `factor_lab::simulate`、`costs.rs` | 源事实 | 基本准确 | 保留，补充模拟了结无滑点、无费用的例外 |
| `factor_research.md#portfolio`：信息比率等于年化超额除跟踪误差 | `simulate` 中 `information_ratio = mean(active) / std(active) * sqrt(252)` | 源事实 | 不准确 | 改为日超额收益均值和标准差的公式 |
| `factor_research.md#portfolio`：最新得分来自区间最后一天 | `simulate` 在缺口或有效排名不足时跳过，保留上次 `picks_date` | 源事实 | 不准确 | 改为最近一次有效排名；说明 `held` 的取样时点 |
| `factor_research.md#portfolio`：`trades` 与费用、盈亏、持有天数 | `FactorTrade`、`simulate`；两个 `simulate_*` 单元测试 | 源事实、待执行测试 | 基本准确但过于集中 | 拆成字段与顺序说明，区分计划调仓和实际模拟成交 |
| `factor_research.md#portfolio`：历史收益和执行耗时 | 原文只有月份、部分参数和机器描述，无完整运行标识 | 未验证 | 无法作为当前验收依据 | 保留为历史测量，标记待确认及影响范围 |
| `factor_research.md`：启动、登录调用、结果保存和排错 | CLI、路由、`RunError`、`compute`、`research_run::record` | 源事实 | 缺失 | 增加可执行流程，复用运维登录步骤 |
| `rest_api.md#conventions`：所有响应都是 JSON | `api/metrics.rs`、204 接口、框架拒绝、`TimeoutLayer` | 源事实 | 不准确 | 限定 JSON 契约，列出文本和空响应例外 |
| `rest_api.md#conventions`：价格和金额一律字符串 | `FactorTrade`、`FactorPortfolioSummary` 是 `f64` | 源事实 | 含糊 | 明确研究结果是 JSON 数字 |
| `rest_api.md#errors`：JSON 错误格式覆盖全部失败 | `error.rs`、`api/metrics.rs`、`api/mod.rs` | 源事实 | 缺失 | 补充 408、指标文本、框架拒绝的边界 |
| `rest_api.md#endpoints`：研究 GET 是只读查询 | `research_run::record` | 源事实 | 缺失 | 标明运行记录副作用，禁止把接口用于探活或无条件重试 |
| `rest_api.md#auth`：研究 GET 对普通用户开放 | `api/auth.rs::allowed` | 源事实 | 准确 | 保留 |
| 预设文档与页面暗示所有用户可保存自己的预设 | handler 使用 `CurrentUser`，但 `MEMBER_WRITES` 未列预设 PUT / DELETE | 源事实；预期权限待确认 | 相互矛盾 | 记录当前管理员写入边界，保留设计意图待确认，不改权限代码 |
| `configuration.md#sources`：全部变量都读取应用 `.env` | `Settings::load`、`telemetry::init`、`metrics::expected_token`、`server::run_workers` | 源事实 | 不准确 | 区分 `Settings` 配置与只读取进程环境的变量 |
| `configuration.md#settings`：指标令牌启动时读取 | `metrics::expected_token` 使用首次请求初始化的 `OnceLock` | 源事实 | 不准确 | 改为首次指标请求读取后缓存；更新仍需重启 |
| `configuration.md#settings`：Redis 不可用总能降级 | `Cache::new` 解析 URL 会失败；连接失败才降级 | 源事实 | 缺失例外 | 补充 URL 无效导致 `serve` 启动失败 |
| `configuration.md#deploy_env`：删除 `REDIS_URL` 即关闭缓存 | `compose.yml` 的 `${REDIS_URL-redis://redis:6379}` | 源事实 | 与缓存文档矛盾 | 明确 Compose 必须显式写 `REDIS_URL=` |
| `architecture/read_cache.md`：删除配置可关闭缓存 | `compose.yml`、`config.rs`、`cache.rs` | 源事实 | 不准确 | 同步关闭方式与故障边界 |
| `runbook.md#local_setup`：依赖某台机器的 dev-infra | CLI、工具链、前端 package scripts | 源事实 | 不可移植 | 明确自备空开发库、工作目录、锁定依赖和前后端启动步骤 |
| `runbook.md#local_setup`：启动与停止 | `niulai.rs::main`、`server::serve`、`SHUTDOWN_DEADLINE` | 源事实 | 缺少恢复条件 | 补充启动阶段、100 秒收尾、租约恢复和同库重启边界 |
| `runbook.md#health_monitoring`：ready 代表服务就绪 | `api/mod.rs::ready` 只执行 `SELECT 1` | 源事实 | 含糊 | 区分存活、数据库、数据覆盖与业务功能 |
| `runbook.md#metrics`：队列长期非零即卡住 | `sync/queue.rs::backlog`；增量和回补任务生命周期 | 源事实、推断 | 断言过强 | 改为结合趋势、完成数、重试和日志判断；补充查询失败的 -1 |
| `runbook.md#monitoring`：SQL 列出未解决失败 | 原 SQL 查全部 `dead`；迁移定义 `ops.v_unresolved_dead` | 源事实 | 不准确 | 使用已有视图避免把已被后续成功覆盖的历史失败计入 |
| `runbook.md#server_install`：Docker 启动、持久化、默认密码 | `compose.yml`、`.env.example`、`Dockerfile` | 源事实；部署未验证 | 步骤缺少配置预检与首次凭据条件 | 增加静态预检，明确首次设置与既有数据库不变 |
| `frontend.md#pages`：成交按买卖筛选、显示盈亏 | `ResearchPage.tsx::TradeHistory` | 源事实 | 缺少显示语义 | 补充倒序、「卖出」包括了结、前端分页与汇总不受筛选影响 |
| `frontend.md#conventions`：取消等于计算立即停止 | `useManualRun`、fetch signal、`compute::run_cpu` | 源事实 | 含糊 | 仅承诺取消客户端请求，不承诺同步 CPU 计算立即停止或写入回滚 |
| `project_overview.md`：原始记录保存所有数据、不依赖其他服务 | `store_record` 配置；`Engine`、`cache.rs`、升级查询 | 源事实 | 过度概括 | 限定记录库和必要、可选外部依赖 |
| 各目标文档的索引、链接与代码锚点 | `docs/index.md`、分类索引、生产代码的 `@project-doc` | 待验证 | 待确认 | 保留既有 ID，修改完成后独立检查 |

## 验证记录

- 源码与配置核对：读取 `backend/src/api/`、`analysis/factor_lab.rs`、`analysis/research_run.rs`、`analysis/compute.rs`、`config.rs`、`db.rs`、`metrics.rs`、`telemetry.rs`、`deploy/compose.yml`、`deploy/Dockerfile`、前端研究页和现有测试；确认当前 `HEAD` 为 `6e05023d513ea19a28f89d0f57babe5e9abf25d4`。
- Rust 单元与集成测试：在临时 PostgreSQL 数据库上执行 `cargo test --locked --offline`，95 个单元测试通过、24 个集成测试通过、2 个测试忽略；另单独执行 `cargo test --locked --offline --test integration factor_research_end_to_end -- --nocapture`，因子研究端到端测试通过。
- Rust 静态检查：在 `backend/` 执行 `cargo clippy --locked --offline --all-targets -- -D warnings`，通过。
- 前端和契约：在 `frontend/` 执行 `pnpm install --frozen-lockfile --offline`、`pnpm typecheck`、`pnpm test`（7 个文件、27 个测试通过）、`pnpm lint`、`pnpm api:spec`、`pnpm api:gen`，生成的 `frontend/openapi.json` 和 `frontend/src/api/generated/` 无 diff；lint 只有既有的 Biome 配置弃用提示。
- 文档结构：检查 60 篇 Markdown 的相对链接和锚点，结果为 0 个损坏；检查 69 个生产代码 `@project-doc` 锚点，全部指向存在的文件和稳定 ID；`git diff --check` 通过。
- 运行证据：使用临时数据库和临时端口启动 `niulai serve --no-worker --no-scheduler`，实际请求 `/api/v1/health`、`/api/v1/ready`、`/api/v1/openapi.json`、`/api/v1/auth/login` 和登录后的 `/api/v1/research/factors`；结果分别为 200、200、包含 `FactorTrade`、登录 200、返回 12 个因子。随后发送 `SIGTERM`，服务退出并删除临时数据库。

这些结果证明当前源码、测试、生成契约和本机临时运行路径一致。没有验证供应商 API Key、历史回补、Redis 连接、Docker Compose 生产部署、GHCR 镜像、外部 HTTPS 代理、真实数据上的收益或生产服务状态；因子研究的历史性能和耗时仍属于待确认测量。

初次临时链接检查器存在相对路径与绝对路径混用，并误扫 skill 中的示例锚点；最终检查已统一路径，并只统计生产代码中的锚点。

## 设计分歧与范围外事项

- 预设写入：handler 和界面按用户组织，但普通用户写入不在授权名单。当前源码足以确认现状，无法确认这是产品限制还是遗漏；只说明当前限制，权限修复留待确认。
- 同步 CPU 段的立即取消、完整运行结果持久化、原始行情快照和历史点时数据改进均需要改业务行为，不在本次范围。
- 其他现状文档中的供应商历史覆盖、财务规则和旧测量数字，以及 ADR 中的旧产品描述，不因为本次检查而获得当前验证。
