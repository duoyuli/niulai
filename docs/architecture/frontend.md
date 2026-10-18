# 前端

本文说明 `frontend/` 的结构、页面与接口的对应、接口客户端的生成、展示约定，以及页面如何由 `niulai serve` 提供。选型理由见 [ADR：前端技术选型](../decisions/frontend_stack.md)，接口本身见 [REST API](../interfaces/rest_api.md)。本地开发见 [本地运行](../operations/runbook.md#local_setup)。

## 章节导航

- [目录结构](#structure)：新增页面或组件时。
- [页面与接口](#pages)：想知道某个页面用了哪些接口时。
- [接口客户端](#api_client)：后端接口变更后，或修改请求与错误处理时。
- [展示约定](#conventions)：涉及价格、涨跌颜色、复权口径、K 线、URL 参数时。
- [由 niulai serve 提供](#serving)：修改部署、缓存或路由回退时。

<a id="structure"></a>
## 目录结构

| 路径 | 内容 |
| --- | --- |
| `src/main.tsx`、`src/router.tsx` | 入口、TanStack Query 客户端、路由表 |
| `src/pages/` | 每个页面一个文件 |
| `src/components/ui.tsx` | 通用组件：卡片、表格、标签、分段按钮、加载与错误状态 |
| `src/components/Layout.tsx`、`VersionBadge.tsx` | 页头、登录与配置检查；版本号、一键升级和升级进度 |
| `src/components/PriceChart.tsx`、`EquityChart.tsx` | K 线、成交量、叠加的指标线和标记；回测的账户总值曲线 |
| `src/components/RangePicker.tsx` | K 线的起止日期框和快捷区间 |
| `src/components/overlays.tsx`、`SampleChart.tsx` | K 线叠加：指标线、信号出现的日子、策略的买卖点，选择写在 URL 的 `overlay` 参数；信号页和指标页的「画在 K 线上」 |
| `src/components/conditions.tsx` | 条件编辑器：策略和自定义信号共用，每条条件下实时显示白话 |
| `src/components/DiagnosisCard.tsx`、`SignalChip.tsx` | 个股诊断卡片；信号标签 |
| `src/components/SignalStats.tsx`、`StrategyNav.tsx` | 信号的过去表现和今天出现的股票；「策略」栏目下三页的切换 |
| `src/components/backtest.tsx` | 回测的共用部分：交易成本、结果展示、规则说明 |
| `src/components/DataCheck.tsx` | 同步页的数据核检卡片 |
| `src/components/SecuritySearch.tsx`、`WatchButton.tsx` | 股票搜索框；收藏星标 |
| `src/lib/strategy.ts`、`tone.ts`、`runs.ts` | 名称模板和条件的白话（与后端写法一致）；信号偏多、偏空、中性的样式；一次同步各批的合计 |
| `src/lib/format.ts`、`labels.ts` | 数字和日期格式化、涨跌颜色；后端枚举值的中文名 |
| `src/hooks/` | URL 状态（`useSearchState`）、防抖、深色模式、自选股列表 |
| `src/api/fetcher.ts` | 所有请求经过的函数，把错误响应转为 `ApiError` |
| `src/api/generated/` | orval 生成的类型和 hooks，不手工修改 |
| `openapi.json` | 后端导出的接口规范，生成客户端的输入，提交到版本库 |

<a id="pages"></a>
## 页面与接口

界面按用户要做的事组织：主导航是「首页」「账户」「策略」「自选」，个股从页头搜索进入，回测是策略页和个股页的标签页，信号和指标在「策略」下面，数据同步、指数板块和「说明」在页头右侧。理由见 [ADR：界面按任务组织](../decisions/ui_by_task.md)。每页先给一句白话结论，再给少量关键数字，明细和规则说明折叠。

| 路径 | 页面 | 接口 |
| --- | --- | --- |
| `/` | 首页：我的账户（每个账户一行：总资产、当日盈亏、「10-19（周一）运营建议：……」）、标题写明数据截至哪天收盘（「10-16（周五）收盘」）、大盘概况、近 5 日最强行业、「在首页展示」的策略最近一次巡检的买点（新入选在前）和卖点、自选股的最近信号 | `GET /accounts`、`/market/overview`、`/market/sectors`、`/strategies`、`/strategies/{id}/results`、`/watchlist` |
| `/accounts` | 我的账户：每个账户一张卡片（总资产、当日盈亏、现金、持仓只数、下个交易日的运营建议、核对提醒） | `GET /accounts` |
| `/accounts/new` | 新建账户，两步：名字；现金和持仓（`PositionsEditor`，可粘贴，可跳过）。规则用默认值，不在主流程里选 | `POST /accounts`、`POST /accounts/{id}/positions/parse`、`PUT /accounts/{id}/positions` |
| `/accounts/:id` | 一个账户，四个标签：运营（`AdviceView`：标题「10-19（周一）运营建议：结论」，副标题「根据 10-16 收盘的数据，生成下一个交易日的运营建议」；因规则选不出股票时黄色提示怎么调并链到设置；卖出表：卖多少、参考价、为什么卖，一字跌停卖不出下一份建议继续提示；买入表：买多少、参考价、最高买入价（开盘高于它就放弃）、止损价、约多少钱；每行「已执行」可改实际成交、「不做」可撤销、「为什么」展开理由、需要注意、怎么做；继续拿的持仓；照着做完之后；交易日收盘出现买点、但没选上的）；持仓（现价、市值占比、盈亏、止损价和趋势均线，记一笔、校准）；记录（建议准不准、每天的建议、流水）；设置（名字和备注；选股和风控规则 `RulesEditor`：左边是有效门槛、参考的策略（按比随便买多涨多少排序；够门槛的在上面，不够门槛的折叠在一条分界线下面、灰显，分界线上说明一次；勾上的才参考，「全选」包括以后新增的，「反选」；全部取消时提示并不能保存；自建的策略标「自建」）和风控参数；右边「建议参考」是三类账户怎么设计和回测，只是建议，每组下有一个不显眼的「照这个填」；费率；用自己的规则回测（起止日期框加快捷区间，结果下面是每一笔交易：买入、卖出、为什么卖、持有天数、收益，可筛选赚钱、亏钱、还持有）；删除要输入账户名） | `GET /accounts/{id}` 及其下的 `advice`、`trades`、`review` 等 |
| `/strategies` | 策略：每个策略最近一次的买点、卖点、持有中数量，内置策略标「内置」；立即巡检、新建（管理员） | `GET /strategies`、`POST /screening/run` |
| `/strategies/:id` | 策略详情：规则的白话；「最新买卖点」按交易日看买点、卖点（原因）、持有中；「过去表现如何」点「开始回测」做全市场组合回测 | `GET` / `PUT` / `DELETE /strategies/{id}`、`/results`、`POST /screening/run`、`GET /backtest/portfolio` |
| `/strategies/new`、`/:id/edit` | 策略编辑器：从模板或空白开始，股票池、买入条件、排序、卖出规则，可预览；`?signal=<key>` 先放好一个信号 | `GET /strategies/catalog`、`/signals`、`POST /strategies/preview`、`POST` / `PUT /strategies` |
| `/signals`、`/signals/:key` | 信号列表（我的信号和按分组的内置信号）；详情：怎么判断、怎么看、画在 K 线上、过去表现、今天出现的股票、被哪些策略用 | `GET /signals`、`/signals/{key}`、`POST /signals/today`、`/signals/stats`、`/securities/{thscode}/signal-days`、`GET /securities/{thscode}/indicator` |
| `/signals/new`、`/:key/edit` | 信号编辑器：名称、偏多偏空、全部或任一满足、条件；「试一试」不保存就看结果。`?from=<key>` 复制，`?cond=<条件 JSON>` 从常用写法开始 | `GET /strategies/catalog`、`POST /signals`、`PUT /signals/my/{id}`、`POST /signals/today`、`/signals/stats` |
| `/indicators`、`/indicators/:id` | 指标：是什么、怎么算、画在 K 线上、常用写法（点一下新建信号）、被哪些信号和策略用 | `GET /indicators`、`/securities/{thscode}/indicator` |
| `/watchlist` | 自选：涨跌、趋势和最近信号，可搜索添加 | `GET /watchlist`、`PUT` / `DELETE /watchlist/{thscode}`、`GET /securities` |
| `/securities/:thscode` | 个股：页头是最新价、收藏、上市日期和本地日线范围。「走势与诊断」先是诊断（四个维度、最近信号、关键价位、今天符合的策略），再是 K 线：起止日期框和快捷区间；已叠加的项（标签，× 去掉）和「＋ 叠加到图上」面板（策略买卖点、信号、指标三栏，可搜索）；图。诊断里点信号或策略就画到 K 线上（两张卡片共用 `overlay` 参数，`useOverlayParam`；默认 20 日和 60 日均线，全部去掉记为 `-`）。「回测」用均线交叉或任一策略回测这只股票 | `GET /securities/{thscode}`、`/bars`、`/diagnosis`、`/indicator`、`/signal-days`、`/strategy-points`、`GET /backtest/ma-cross`、`/backtest/strategy` |
| `/guide` | 说明：指标、条件、信号、策略四个概念和买卖点规则 | — |
| `/indices`、`/indices/:thscode` | 指数与板块列表；指数 K 线和当前成分股 | `GET /indices`、`/indices/{thscode}/bars`、`/members` |
| `/sync` | 数据同步，从上到下：「立即同步全部」「手动同步」（按计划全选或勾选到单个数据集，选增量或全部重取）；一句话状态；同步计划（时间可改，可「现在同步一次」）；同步记录（标明来源，几批合成一行）；需要注意（缺口和失败，一键补齐）；数据核检；按分类的数据集 | `GET /sync/health`、`/sync/schedule`、`/sync/runs`、`/sync/check`、`/datasets`，`PUT /sync/schedule`，`POST /sync/runs`、`/sync/repair`、`/sync/check` |
| `/sync/runs/:runId` | 一次同步的详情（任意一批的编号都行）：各批状态、还没开始的批次、各数据集进度、没有新建任务的数据及原因、失败的任务；合计与同步记录用同一个函数（`lib/runs.ts`） | `GET /sync/runs/{run_id}`、`/datasets` |
| `/login` | 登录，成功后回到 `next` 参数指向的页面；不在 `Layout` 中 | `POST /auth/login` |
| `/setup` | API Key 引导和更换：验证并保存，显示来源和末尾 4 位（管理员） | `GET /setup`、`PUT /setup/api-key` |
| `/users` | 用户管理（管理员）：添加用户（默认普通用户，不填密码时生成一个、只显示一次），设为管理员或普通用户，重置密码，停用和启用；不能修改自己 | `GET /users`、`POST /users`、`PUT /users/{user_id}`、`POST /users/{user_id}/password` |
| `/me` | 本人：角色，改密码（要输入现在的密码） | `GET /auth/me`、`PUT /auth/password` |

全局行为（`Layout`、`VersionBadge`）：
- 请求收到 401 时整页跳到 `/login?next=<当前地址>`；管理员登录后还没配置 API Key（`GET /setup` 的 `api_key_configured` 为 false）时，所有页面跳到 `/setup`；`GET /auth/me` 的 `must_change_password` 为 true 时页面顶部提醒去「我的」改密码。
- 按角色显示（`useIsAdmin`）：页头右侧只放「说明」和「数据」（普通用户是「指数」），修改密码、用户管理和数据源设置（管理员）、退出登录收在用户名的悬浮菜单里（`UserMenu`，鼠标移上去或点击展开）；普通用户看不到版本号和升级、「数据」、用户管理和数据源设置，策略和信号页上没有新建、修改、删除、巡检按钮；直接打开管理员页面时 `AdminOnly` 给一句说明，不发请求。后端同样拦截，前端只是不显示用不了的按钮。
- 版本号每 30 分钟请求一次 `GET /system/version`，有新版本时亮红点，点开可以一键升级。升级进行中每 2 秒刷新（服务重启时请求失败也继续），进度条和每一步的状态只显示在版本号弹窗里（`UpgradeProgress`，步骤来自升级助手写的状态），点别处收起，进度保留，版本号上的圆点一直闪，再点开接着看；所有步骤结束、运行的版本变成目标版本后整页刷新。
- 刷新间隔：同步页的运行列表 10 秒、同步状态 15 秒、数据集和同步计划 30 秒；运行详情在进行中时 5 秒。

<a id="api_client"></a>
## 接口客户端

类型和 hooks 由 orval 从 `openapi.json` 生成（配置在 `orval.config.ts`）。后端接口变更后，在 `frontend/` 下执行 `pnpm api:spec`（运行 `niulai openapi` 导出规范）、`pnpm api:gen`（重新生成 `src/api/generated/`）、`pnpm typecheck`。

请求都经过 `src/api/fetcher.ts` 的 `apiFetch`：2xx 返回解析后的 JSON；其他状态抛出 `ApiError`，带 HTTP 状态和后端的 `code`、`message`（见 [错误格式](../interfaces/rest_api.md#errors)），框架直接返回的纯文本 400 原文作为 `message`。TanStack Query 对 4xx 不重试，对 5xx 和网络错误最多重试 2 次。

<a id="conventions"></a>
## 展示约定

- **红涨绿跌：** 上涨、偏多的信号用红色，下跌、偏空的用绿色，不说明方向的中性信号（十字星、缩量等）用橙色圆点（`src/lib/tone.ts`）。颜色令牌是 `src/index.css` 中的 `up`、`down`，数值的涨跌颜色统一用 `upDownClass`。
- **价格是字符串：** 后端的价格和成交额是十进制字符串，只在展示和画图时转成数字。
- **复权口径：** K 线默认等比前复权（`qfq`），按钮上写明「等比」和「同花顺」。指标和信号都按等比前复权计算，所以只有选择前复权时才把价格类指标线画在 K 线上。
- **K 线区间：** `RangePicker` 是一个起止日期框（两头都包含）加快捷区间 1 月到全部。点快捷区间把日期填进框里（结束日期是今天，「全部」从最早的日线开始），改日期就是自定义区间（`range=custom&from=&to=`）。起点早于最早日线的快捷区间置灰：个股按本地最早的日线，指数和板块从 2022-01-04 开始。回测的区间只按年（`RANGES`）。
- **在图上框选：** 在 K 线上按住鼠标左右拖动，松开后放大到这段（只在已加载的 K 线里放大），图下有「还原」；平移用滚轮缩放后拖动底部时间轴，触屏仍可拖动平移。
- **URL 参数：** 搜索词、复权口径、区间、叠加项等写在查询参数里，刷新或分享后保持；等于默认值的参数不写入 URL。
- **大数和时间：** 成交额、成交量用「万」「亿」，成交量单位是股；时间按上海时间显示，见 [时区](../interfaces/configuration.md#time_zone)。
- **面向非专业用户：** 术语（复权、金叉、成分股等）都附一句说明。

<a id="serving"></a>
## 由 niulai serve 提供

设置 `NIULAI_WEB_DIR`（指向 `frontend/dist`）后，`niulai serve` 同时提供页面，实现在 `backend/src/api/web.rs`：

- `/api/v1/*` 是接口；其他未匹配的 `/api/` 路径返回 JSON 404。
- 其他路径先找静态文件，找不到时返回 `index.html` 由前端路由处理，所以刷新 `/securities/600519.SH` 也能打开。
- `/assets/*` 带内容哈希，`Cache-Control: public, max-age=31536000, immutable`；其他文件（含 `index.html`）是 `no-cache`，发布后立即生效。

Docker 镜像在构建时执行 `pnpm build`，产物在 `/app/web`，镜像中默认 `NIULAI_WEB_DIR=/app/web`。本地开发用 Vite 开发服务器（`pnpm dev`），`/api` 转发到 `http://127.0.0.1:8080`（`NIULAI_API_URL` 可改）。
