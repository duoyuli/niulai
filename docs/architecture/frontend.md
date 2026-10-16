# 前端

本文说明 `frontend/` 的结构、页面与接口的对应关系、接口客户端的生成方式、展示约定，以及前端如何由 `niulai serve` 提供。技术选型的理由见 [ADR：前端技术选型](../decisions/frontend_stack.md)，接口本身见 [REST API](../interfaces/rest_api.md)。

## 章节导航

- [目录结构](#structure)：新增页面或组件时。
- [页面与接口](#pages)：想知道某个页面用了哪些接口时。
- [接口客户端](#api_client)：后端接口变更后，或修改请求与错误处理时。
- [展示约定](#conventions)：涉及价格、涨跌颜色、复权口径、筛选条件时。
- [由 niulai serve 提供](#serving)：修改部署、缓存或路由回退时。

<a id="structure"></a>
## 目录结构

| 路径 | 内容 |
| --- | --- |
| `src/main.tsx`、`src/router.tsx` | 入口、TanStack Query 客户端、路由表 |
| `src/pages/` | 每个页面一个文件 |
| `src/components/ui.tsx` | 通用组件：卡片、表格、标签、分段按钮、加载与错误状态 |
| `src/components/PriceChart.tsx`、`EquityChart.tsx` | K 线、成交量和均线图；回测的账户总值曲线（策略与买入持有，可切换对数坐标） |
| `src/components/SecuritySearch.tsx` | 按代码或名称搜索股票的下拉框：页头的全局搜索和自选页添加股票共用 |
| `src/components/DiagnosisCard.tsx`、`SignalChip.tsx` | 个股诊断卡片；信号标签（偏多红色、偏空绿色） |
| `src/components/backtest.tsx` | 回测的共用部分：交易成本输入、结果展示（单只股票和全市场组合）、规则说明；个股页和策略页共用 |
| `src/lib/strategy.ts` | 信号和指标名称模板的填充、卖出原因的中文名 |
| `src/components/WatchButton.tsx`、`src/hooks/useWatchlist.ts` | 收藏星标；各页面共用一份自选股列表，收藏变化后刷新所有自选股查询 |
| `src/hooks/` | URL 筛选状态、防抖、深色模式、收藏状态 |
| `src/lib/format.ts`、`labels.ts` | 数字和日期格式化、涨跌颜色（`upDownClass`）、时间区间选项；后端枚举值的中文名 |
| `src/api/fetcher.ts` | 所有请求经过的函数，把错误响应转为 `ApiError` |
| `src/api/generated/` | orval 生成的类型和 hooks，不手工修改 |
| `openapi.json` | 后端导出的接口规范，生成客户端的输入，提交到版本库 |

<a id="pages"></a>
## 页面与接口

界面按用户要做的事组织，而不是按后端有哪些功能组织：主导航只有「今日」「策略」「自选」三项，个股从页头的搜索框进入，回测是策略和个股页里的一个标签页，数据同步和指数板块放在页头右侧的「数据」里，指标与信号的说明在页头右侧的「说明」里。为什么这样组织，见 [ADR：界面按任务组织](../decisions/ui_by_task.md)。

| 路径 | 页面 | 接口 |
| --- | --- | --- |
| `/` | 今日：大盘涨跌概况、近 5 日最强的行业、每个开启巡检的策略今天选出的股票（新入选的在前）、自选股的最近信号 | `GET /market/overview`、`GET /market/sectors`、`GET /strategies`、`GET /strategies/{id}/results`、`GET /watchlist` |
| `/strategies` | 策略列表：每个策略最近一次选出的数量，可立即巡检、新建 | `GET /strategies`、`POST /screening/run` |
| `/strategies/:id` | 策略详情：条件写成一段话；「选出的股票」标签页按交易日看结果（新入选、不再符合）；「过去表现如何」标签页做全市场组合回测，点「开始回测」才计算 | `GET /strategies/{id}`、`/results`、`PUT` / `DELETE /strategies/{id}`、`POST /screening/run`、`GET /backtest/portfolio` |
| `/strategies/new`、`/strategies/:id/edit` | 策略编辑器：从模板或空白开始，设置股票池、条件、排序和卖出规则，可预览结果 | `GET /strategies/catalog`、`POST /strategies/preview`、`POST /strategies`、`PUT /strategies/{id}` |
| `/watchlist` | 自选：收藏的股票、涨跌、趋势和最近信号，可搜索添加 | `GET /watchlist`、`PUT` / `DELETE /watchlist/{thscode}`、`GET /securities` |
| `/securities/:thscode` | 个股：页头是最新价和收藏；「走势与诊断」标签页是诊断卡片和 K 线（复权口径与均线交叉记录折叠在下面）；「回测」标签页用均线交叉或任意已保存的策略回测这只股票 | `GET /securities/{thscode}`、`/bars`、`/ma`、`/diagnosis`、`GET /backtest/ma-cross`、`/backtest/strategy`、`GET /strategies` |
| `/guide` | 指标与信号：三个概念的说明，每个信号和指标是什么、怎么看、要注意什么；可以查看今天哪些股票出现了某个信号，或用它新建策略。各页面的信号标签和策略编辑器的「这是什么？」链接到对应条目（`#sig-<信号>`、`#ind-<指标>`） | `GET /strategies/catalog`、`POST /strategies/preview` |
| `/indices` | 指数与板块列表，按分类筛选；从「数据」页和今日页的板块进入 | `GET /indices` |
| `/indices/:thscode` | 指数详情：K 线、当前成分股 | `GET /indices`、`/indices/{thscode}/bars`、`/members` |
| `/sync` | 数据同步：待执行和失败任务数、最近的运行、数据集状态，发起运行 | `GET /datasets`、`GET /sync/runs`、`POST /sync/runs` |
| `/sync/runs/:runId` | 运行详情：各数据集进度、失败任务 | `GET /sync/runs/{run_id}` |
| `/login` | 登录：输入管理员账号密码，成功后回到原来要打开的页面（`next` 参数）；不在 `Layout` 中 | `POST /auth/login` |
| `/setup` | 首次配置引导和数据源设置：说明去哪里生成 API Key，验证并保存；已配置时显示来源和末尾 4 位，可以更换 | `GET /setup`、`PUT /setup/api-key` |

旧地址 `/securities`、`/signals`、`/backtest`、`/screener/...` 跳转到对应的新页面。全市场均线扫描接口 `GET /signals/ma-cross` 仍然保留，但没有页面使用；同样的需求用「均线金叉」策略。

页面里常用的展示原则：每页先给一句白话结论，再给最多四个关键数字，其余数字、明细表和规则说明折叠起来。

任何请求收到 401 时，`apiFetch` 把页面整页跳转到 `/login?next=<当前地址>`；`Layout` 请求 `GET /auth/me` 确认已登录，页头右侧有「退出」，仍在使用默认账号密码时页面顶部有提示。登录之后，还没有配置 API Key 时（`GET /setup` 的 `api_key_configured` 为 false），`Layout` 把所有页面跳转到 `/setup`；页头右侧的「设置」也进入这个页面。页头右侧的服务状态每 30 秒请求一次 `GET /ready`。数据同步页每 5 秒刷新运行列表，运行详情在运行进行中时每 5 秒刷新。

<a id="api_client"></a>
## 接口客户端

类型和请求 hooks 由 orval 从 `frontend/openapi.json` 生成，配置在 `frontend/orval.config.ts`。后端接口变更后，在 `frontend/` 下执行：

1. `pnpm api:spec`：运行 `niulai openapi`，导出最新规范到 `openapi.json`。
2. `pnpm api:gen`：重新生成 `src/api/generated/`。
3. `pnpm typecheck`：确认页面代码与新接口一致。

生成的请求都经过 `src/api/fetcher.ts` 的 `apiFetch`：2xx 返回解析后的 JSON；其他状态抛出 `ApiError`，带 HTTP 状态和后端的 `code`、`message`（见 [错误格式](../interfaces/rest_api.md#errors)）。框架直接返回的 400 是纯文本，原文作为 `message`。TanStack Query 对 4xx 不重试，对 5xx 和网络错误最多重试 2 次。

<a id="conventions"></a>
## 展示约定

- **红涨绿跌**：按 A 股习惯，上涨、金叉用红色，下跌、死叉用绿色。颜色令牌是 `src/index.css` 中的 `up`、`down`，数值的涨跌颜色统一用 `upDownClass`。
- **价格是字符串**：后端的价格和成交额是十进制字符串，只在展示和画图时转成数字，不在前端做价格运算后再提交。
- **复权口径**：K 线默认等比前复权（`qfq`）。按钮上写明「等比」和「同花顺」，避免把同花顺的减法复权当成计算口径。均线和交叉信号固定按等比前复权计算（见 [均线与交叉信号](../domains/ma_signals.md)），所以只有选择前复权时才把均线画在 K 线上，选择其他口径时只显示说明。
- **K 线区间**：图表展示所选区间（1 年到全部）内的全部 K 线；区间选项和起始日期由 `RANGES`、`rangeStart` 统一给出，股票详情、指数详情和回测共用。
- **大数**：成交额、成交量用「万」「亿」表示；成交量单位是股。
- **时间**：交易日按原样显示；运行开始、结束等时间是 UTC，显示时换成上海时间。
- **筛选条件放在 URL**：搜索词、板块、复权口径、区间、均线周期等写在查询参数里，刷新或分享链接后保持不变；等于默认值的参数不写入 URL。
- **面向非专业用户**：页面上的术语（复权、金叉、成分股等）都附一句说明。

<a id="serving"></a>
## 由 niulai serve 提供

设置 `NIULAI_WEB_DIR`（指向 `frontend/dist`）后，`niulai serve` 在 API 之外同时提供前端页面，实现在 `backend/src/api/web.rs`：

- `/api/v1/*` 是接口；其他未匹配的 `/api/` 路径返回 JSON 404，不返回页面。
- 其他路径先找静态文件；找不到时返回 `index.html`，由前端路由处理，所以刷新 `/securities/600519.SH` 这样的地址也能打开。
- `/assets/*` 是带内容哈希的文件，响应头 `Cache-Control: public, max-age=31536000, immutable`；其他文件（包括 `index.html`）是 `no-cache`，发布新版本后立即生效。

Docker 镜像在构建阶段执行 `pnpm build`，产物放在 `/app/web`，镜像中默认 `NIULAI_WEB_DIR=/app/web`，见 [部署](../operations/runbook.md#deployment)。本地开发时用 Vite 开发服务器，`/api` 转发到 `http://127.0.0.1:8080`（可用 `NIULAI_API_URL` 改），见 [本地运行](../operations/runbook.md#local_setup)。
