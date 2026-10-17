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
| `src/components/PriceChart.tsx`、`EquityChart.tsx` | K 线、成交量、均线、叠加的指标线和标记；回测的账户总值曲线（策略与买入持有，可切换对数坐标） |
| `src/components/SecuritySearch.tsx` | 按代码或名称搜索股票的下拉框：页头的全局搜索和自选页添加股票共用 |
| `src/components/DiagnosisCard.tsx`、`SignalChip.tsx` | 个股诊断卡片；信号标签（偏多红色、偏空绿色、中性橙色，样式在 `src/lib/tone.ts`，K 线上的信号标记也用它） |
| `src/components/backtest.tsx` | 回测的共用部分：交易成本输入、结果展示（单只股票和全市场组合）、规则说明；个股页和策略页共用 |
| `src/lib/strategy.ts` | 信号和指标名称模板的填充、条件的白话（和后端写法一致）、时间范围、卖出原因的中文名 |
| `src/components/conditions.tsx` | 条件编辑器：策略的买入卖出和自定义信号共用，每条条件下实时显示白话，信号和指标链接到说明页 |
| `src/components/overlays.tsx`、`SampleChart.tsx` | K 线叠加：指标线（价格类画在 K 线上，其他画在副图）、信号出现的日子、策略的买卖点，选择写在 URL 的 `overlay` 参数；信号页和指标页的「画在 K 线上」 |
| `src/components/SignalStats.tsx`、`StrategyNav.tsx` | 信号的过去表现和今天出现的股票；「策略」栏目下我的策略、信号、指标三页的切换 |
| `src/components/WatchButton.tsx`、`src/hooks/useWatchlist.ts` | 收藏星标；各页面共用一份自选股列表，收藏变化后刷新所有自选股查询 |
| `src/hooks/` | URL 筛选状态、防抖、深色模式、收藏状态 |
| `src/lib/format.ts`、`labels.ts` | 数字和日期格式化、涨跌颜色（`upDownClass`）、时间区间选项；后端枚举值的中文名 |
| `src/api/fetcher.ts` | 所有请求经过的函数，把错误响应转为 `ApiError` |
| `src/api/generated/` | orval 生成的类型和 hooks，不手工修改 |
| `openapi.json` | 后端导出的接口规范，生成客户端的输入，提交到版本库 |

<a id="pages"></a>
## 页面与接口

界面按用户要做的事组织，而不是按后端有哪些功能组织：主导航只有「今日」「策略」「自选」三项，个股从页头的搜索框进入，回测是策略和个股页里的一个标签页，信号和指标是「策略」下面的两页（搭策略用的积木，放在用的地方），数据同步和指数板块放在页头右侧的「数据」里，指标、条件、信号、策略四个概念的介绍在页头右侧的「说明」里。为什么这样组织，见 [ADR：界面按任务组织](../decisions/ui_by_task.md)。

| 路径 | 页面 | 接口 |
| --- | --- | --- |
| `/` | 今日：大盘涨跌概况、近 5 日最强的行业、每个开启巡检并且「在今日展示」的策略今天的买点（新入选的在前）和卖点，点股票进入个股页并标出这个策略的买卖点、自选股的最近信号 | `GET /market/overview`、`GET /market/sectors`、`GET /strategies`、`GET /strategies/{id}/results`、`GET /watchlist` |
| `/strategies` | 我的策略：每个策略最近一次的买点、卖点和持有中数量，可立即巡检、新建 | `GET /strategies`、`POST /screening/run` |
| `/strategies/:id` | 策略详情：买入和卖出写成一段话；「今日买卖点」标签页按交易日看今日买点（新入选）、今日卖点（原因）和持有中（按规则推算）；「过去表现如何」标签页做全市场组合回测，点「开始回测」才计算 | `GET /strategies/{id}`、`/results`、`PUT` / `DELETE /strategies/{id}`、`POST /screening/run`、`GET /backtest/portfolio` |
| `/strategies/new`、`/strategies/:id/edit` | 策略编辑器：从模板或空白开始，设置股票池、买入条件、排序和卖出规则，条件可以用自定义信号，可预览结果；`?signal=<key>` 从信号页进来时先放好这个信号 | `GET /strategies/catalog`、`GET /signals`、`POST /strategies/preview`、`POST /strategies`、`PUT /strategies/{id}` |
| `/signals` | 信号：我的信号和按分组的内置信号，每个有一句话定义、事件或状态、被几个策略用 | `GET /signals` |
| `/signals/:key` | 信号详情：怎么判断（条件的白话、用到的指标）、是什么怎么看、画在 K 线上（信号的日子和用到的指标线，可换股票）、过去表现、今天出现的股票、被哪些策略用；复制一份改、编辑、删除、用它新建策略 | `GET /signals/{key}`、`POST /signals/today`、`POST /signals/stats`、`POST /securities/{thscode}/signal-days`、`GET /securities/{thscode}/indicator` |
| `/signals/new`、`/signals/:key/edit` | 信号编辑器：名称、偏多偏空、全部或任一满足、条件，合起来的白话；「试一试」不保存就看今天出现的股票和过去表现。`?from=<key>` 复制一个信号，`?cond=<条件 JSON>` 从指标页的常用写法开始 | `GET /strategies/catalog`、`GET /signals/{key}`、`POST /signals`、`PUT /signals/my/{id}`、`POST /signals/today`、`POST /signals/stats` |
| `/indicators`、`/indicators/:id` | 指标：按分组列出；详情是是什么、怎么算、画在 K 线上、常用写法（点一下新建信号）、被哪些信号和策略用 | `GET /indicators`、`GET /securities/{thscode}/indicator` |
| `/watchlist` | 自选：收藏的股票、涨跌、趋势和最近信号，可搜索添加 | `GET /watchlist`、`PUT` / `DELETE /watchlist/{thscode}`、`GET /securities` |
| `/securities/:thscode` | 个股：页头是最新价和收藏；「走势与诊断」标签页是诊断卡片和 K 线，K 线上可以叠加任意指标、信号和策略买卖点（`overlay` 参数，复权口径与均线交叉记录折叠在下面）；「回测」标签页用均线交叉或任意已保存的策略回测这只股票 | `GET /securities/{thscode}`、`/bars`、`/ma`、`/diagnosis`、`/indicator`、`/signal-days`、`/strategy-points`、`GET /signals`、`GET /backtest/ma-cross`、`/backtest/strategy`、`GET /strategies` |
| `/guide` | 说明：指标、条件、信号、策略四个概念和买卖点的规则。旧链接 `#sig-<信号>`、`#ind-<指标>` 跳到信号页和指标页 | — |
| `/indices` | 指数与板块列表，按分类筛选；从「数据」页和今日页的板块进入 | `GET /indices` |
| `/indices/:thscode` | 指数详情：K 线、当前成分股 | `GET /indices`、`/indices/{thscode}/bars`、`/members` |
| `/sync` | 数据同步：页头有「立即同步全部」（增量同步全部已开启的数据）和「手动同步」（按同步计划勾选或全选，每组能展开到单个数据集，没有开启的数据单独一组；选增量或全部重取，可以只同步指定股票；表单展开在页头下面）；然后是一句话状态（最近一个交易日的数据是否就绪、补历史的进度）；「同步计划」列出每日收盘后、每周、每月、财报季各在什么时候同步、包括哪些数据、下次和上次同步，可以「现在同步一次」，「修改时间」改每日和每周的时间（上海时间）；同步记录按来源显示（每日、每周、手动、补齐缺口等，同一次同步的几个波次合并成一行）；「需要注意」列出缺口和失败，分「可以补」和「补不回来」，可以一键补齐；「数据核检」显示最近一次核检的结论、每个数据集查了多少和缺口的去向（新发现、已补回、确认停牌、还缺、重取的同步），列出还缺数据的股票，可以「立即核检」；最下面是按分类分组的数据集：中文名、一句话说明、所属同步计划、最近更新和状态，没有开启的收在最后 | `GET /sync/health`、`POST /sync/repair`、`GET`/`POST /sync/check`、`GET /datasets`、`GET /sync/schedule`、`PUT /sync/schedule`、`GET /sync/runs`、`POST /sync/runs` |
| `/sync/runs/:runId` | 一次同步的详情：几批合在一起看（传任何一批的编号都行），状态和进度与同步记录里那一行用同一个函数合计（`lib/runs.ts`），数字一致；列出每一批的数据和状态、还没开始的批次；各数据集进度（各批合计，用中文名）；这次没有新建任务的数据写明原因（同样的内容已经在哪次同步的队列里，或已是最新）；失败的任务 | `GET /sync/runs/{run_id}`、`GET /datasets` |
| `/login` | 登录：输入管理员账号密码，成功后回到原来要打开的页面（`next` 参数）；不在 `Layout` 中 | `POST /auth/login` |
| `/setup` | 首次配置引导和数据源设置：说明去哪里生成 API Key，验证并保存；已配置时显示来源和末尾 4 位，可以更换 | `GET /setup`、`PUT /setup/api-key` |

旧地址 `/securities`、`/backtest`、`/screener/...` 跳转到对应的新页面。各页面的信号标签链接到 `/signals/<信号>`。全市场均线扫描接口 `GET /signals/ma-cross` 仍然保留，但没有页面使用；同样的需求用「均线金叉」策略。

页面里常用的展示原则：每页先给一句白话结论，再给最多四个关键数字，其余数字、明细表和规则说明折叠起来。

任何请求收到 401 时，`apiFetch` 把页面整页跳转到 `/login?next=<当前地址>`；`Layout` 请求 `GET /auth/me` 确认已登录，页头右侧有「退出」；logo 后面的版本号（`VersionBadge`）每 30 分钟请求一次 `GET /system/version`，有新版本时亮红点，点开可以一键升级，升级期间每 2 秒刷新，运行的版本变成目标版本后整页刷新；仍在使用默认账号密码时页面顶部有提示。登录之后，还没有配置 API Key 时（`GET /setup` 的 `api_key_configured` 为 false），`Layout` 把所有页面跳转到 `/setup`；页头右侧的「设置」也进入这个页面。页头右侧的服务状态每 30 秒请求一次 `GET /ready`。数据同步页每 5 秒刷新运行列表，运行详情在运行进行中时每 5 秒刷新。

<a id="api_client"></a>
## 接口客户端

类型和请求 hooks 由 orval 从 `frontend/openapi.json` 生成，配置在 `frontend/orval.config.ts`。后端接口变更后，在 `frontend/` 下执行：

1. `pnpm api:spec`：运行 `niulai openapi`，导出最新规范到 `openapi.json`。
2. `pnpm api:gen`：重新生成 `src/api/generated/`。
3. `pnpm typecheck`：确认页面代码与新接口一致。

生成的请求都经过 `src/api/fetcher.ts` 的 `apiFetch`：2xx 返回解析后的 JSON；其他状态抛出 `ApiError`，带 HTTP 状态和后端的 `code`、`message`（见 [错误格式](../interfaces/rest_api.md#errors)）。框架直接返回的 400 是纯文本，原文作为 `message`。TanStack Query 对 4xx 不重试，对 5xx 和网络错误最多重试 2 次。

<a id="conventions"></a>
## 展示约定

- **红涨绿跌**：按 A 股习惯，上涨、金叉用红色，下跌、死叉用绿色；不说明方向的中性信号（十字星、缩量等）用橙色圆点。颜色令牌是 `src/index.css` 中的 `up`、`down`，数值的涨跌颜色统一用 `upDownClass`。
- **价格是字符串**：后端的价格和成交额是十进制字符串，只在展示和画图时转成数字，不在前端做价格运算后再提交。
- **复权口径**：K 线默认等比前复权（`qfq`）。按钮上写明「等比」和「同花顺」，避免把同花顺的减法复权当成计算口径。均线和交叉信号固定按等比前复权计算（见 [均线与交叉信号](../domains/ma_signals.md)），所以只有选择前复权时才把均线画在 K 线上，选择其他口径时只显示说明。
- **K 线区间**：股票详情和指数详情用 `RangePicker`（`src/components/RangePicker.tsx`）选择要加载的区间：一个起止日期框（开始和结束日期都包含在内），旁边是快捷区间 1 月、3 月、6 月、1 年、3 年、5 年、10 年、全部。点快捷区间会把起止日期填进框里（结束日期是今天，「全部」从最早的日线开始）；在框里改日期就是自定义区间（URL 参数 `range=custom&from=&to=`），快捷区间不再高亮。区间和均线周期、叠加的指标信号和策略买卖点都放在图的上方，改完马上能看到。起点已经早于最早日线的区间看到的和「全部」一样，置灰并提示数据从哪天开始：个股按本地最早的日线，指数和板块从 2022-01-04 开始。加载新区间时旁边显示加载中。回测的区间只按年（`RANGES`）。
- **在图上框选**：在 K 线图上按住鼠标左右拖动，框选一段时间，松开后放大到这段，图下显示「已放大到……」和「还原」。框选只是在已加载的 K 线里放大，不重新请求；所以按住拖动不再平移图表，平移用滚轮缩放后拖动底部时间轴，触屏仍然可以拖动平移。
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
