# ADR：前端技术选型

- 状态：已采纳（accepted），2026-10-16。细化 [ADR：技术栈](technology_stack.md) 中留待首批页面明确后再定的「前端路由、组件库和图表」。
- 相关文档：[前端](../architecture/frontend.md)、[REST API](../interfaces/rest_api.md#openapi)、[ADR：单进程服务](single_process_service.md)。

## 背景

技术栈 ADR 已确定前端基线：React 19 + TypeScript 6.x + Vite + TanStack Query，类型和请求 hooks 用 orval 从后端的 OpenAPI 生成。路由、样式、图表、代码检查和包管理留到首批页面明确时再选。首批页面是：股票列表与详情（K 线、5 种复权口径、均线与交叉信号）、全市场均线扫描、指数与成分股、数据同步的运行与数据集状态。

## 决策

| 方面 | 选择 | 理由 |
| --- | --- | --- |
| 路由 | React Router 8（`createBrowserRouter`） | 页面少、参数简单，不需要 TanStack Router 的类型化搜索参数和文件路由插件；React Router 生态最广，2026-10-14 有发布 |
| 样式 | Tailwind CSS 4，不引入组件库 | 页面以表格、筛选和图表为主，需要的组件（卡片、标签、分段按钮、进度条）很少，自己写约 200 行。组件库会带来大量依赖和主题定制成本 |
| K 线图 | TradingView Lightweight Charts 5 | 专为金融行情设计：K 线、成交量分窗、均线、买卖点标记都是内置能力，体积小，Apache-2.0。ECharts 更通用，但做 K 线交互需要更多配置，最近一次发布是 2026-05 |
| 接口客户端 | orval 8，`fetch` 客户端加自定义请求函数 | 生成 TanStack Query hooks；自定义函数统一把非 2xx 响应转为 `ApiError`，不引入 axios |
| 代码检查与格式化 | Biome 2 | 一个工具同时做检查和格式化，配置少，比 ESLint 加 Prettier 快 |
| 测试 | Vitest 5 | 与 Vite 共用配置；目前只覆盖格式化等纯函数 |
| 包管理 | pnpm 12，`minimumReleaseAge` 3 天 | 依赖发布满 3 天才会被安装，降低恶意新版本直接进入构建的风险；只允许 esbuild 执行安装脚本 |
| 部署 | 由 `niulai serve` 提供构建产物 | 前端与 API 同源，不需要 CORS 和另外的 Web 服务器，延续单进程服务的决策 |

TypeScript 7.0（Go 重写的版本）已经发布，但 orval 等工具尚未声明支持，按技术栈 ADR 继续锁定 6.0。

## 维护状况核查（2026-10-15，npm registry）

| 依赖 | 采用版本 | 说明 |
| --- | --- | --- |
| React / React DOM | 19.3.0 | 19.3.1 发布不满 3 天 |
| React Router | 8.4.0 | 2026-10-14 发布 |
| TanStack Query | 5.104.2 | 5.104.3 发布不满 3 天 |
| Vite | 8.3.4 | 2026-10-08 发布 |
| Tailwind CSS | 4.3.3 | 活跃 |
| Lightweight Charts | 5.2.1 | 2026-08 发布，TradingView 官方维护 |
| orval | 8.41.0 | 8.43.0 发布不满 3 天 |
| Biome | 2.5.15 | 2026-09-30 发布 |
| Vitest | 5.0.4 | 5.0.5 发布不满 3 天 |

版本精确锁定在 `frontend/package.json` 和 `frontend/pnpm-lock.yaml` 中。

## 后果

- 前端的类型完全来自后端的 OpenAPI。后端接口变更后，需要重新导出规范、重新生成客户端，并通过前端类型检查。
- 生成代码要求规范合法：OpenAPI 3.1 要求每个响应都有 `description`，后端注解里必须写。
- Lightweight Charts 默认在图上显示 TradingView 标志，页脚也注明了图表库的来源，保留这两处署名。
- 升级依赖时，新版本要等发布满 3 天才能安装；需要紧急升级时，在 `pnpm-workspace.yaml` 中为单个包临时加例外。
