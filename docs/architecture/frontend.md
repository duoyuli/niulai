# 前端

本文说明 `frontend/` 的结构、页面与接口的对应、接口客户端的生成、展示约定，以及页面如何由 `niulai serve` 提供。选型理由见 [ADR：前端技术选型](../decisions/frontend_stack.md)，接口本身见 [REST API](../interfaces/rest_api.md)。本地开发见 [本地运行](../operations/runbook.md#local_setup)。

## 章节导航

- [目录结构](#structure)：新增页面或组件时。
- [页面与接口](#pages)：想知道某个页面用了哪些接口时。
- [接口客户端](#api_client)：后端接口变更后，或修改请求与错误处理时。
- [展示约定](#conventions)：涉及价格、涨跌颜色、复权口径、K 线、URL 参数时。
- [账户修改后的刷新](#account_invalidation)：新增账户写操作或账户派生的查询时。
- [由 niulai serve 提供](#serving)：修改部署、缓存或路由回退时。

<a id="structure"></a>
## 目录结构

| 路径 | 内容 |
| --- | --- |
| `src/main.tsx`、`src/router.tsx` | 入口、TanStack Query 客户端、路由表 |
| `src/pages/` | 每个页面一个文件 |
| `src/components/ui.tsx` | 通用组件：卡片、表格、标签、分段按钮、加载与错误状态 |
| `src/components/Layout.tsx`、`VersionBadge.tsx` | 页头、登录与配置检查；版本号、一键升级和升级进度 |
| `src/components/PriceChart.tsx`、`EquityChart.tsx`、`src/lib/movingAverage.ts` | K 线、快慢均线和成交量；因子组合回测的总值曲线（组合与基准） |
| `src/components/RangePicker.tsx` | K 线和因子研究的起止日期框与快捷区间 |
| `src/components/FactorCharts.tsx`、`FactorPicker.tsx`、`researchFields.tsx`、`src/lib/factors.ts` | 因子研究的多曲线图（各组净值）；预设卡片和按类别选因子、填权重（因子研究页和账户调仓规则共用，见 [因子预设](../domains/accounts.md#presets)）；交易成本、数字输入、规则说明和坐标切换；选中因子和权重在 URL 里的写法、结论的颜色、账户规则到组合回测的链接 |
| `src/components/longTask.tsx` | 长任务的等待秒数和取消 |
| `src/components/AdviceView.tsx`、`RulesForm.tsx`、`PositionsEditor.tsx`、`AccountEquityChart.tsx`、`TodayAccounts.tsx` | 账户：调仓建议和理由（后台重新生成时轮询）、调仓规则（可增删因子、填权重，持有只数、缓冲区、留现金、追高上限、ST）、持仓录入与粘贴识别、净值曲线、首页的账户卡片 |
| `src/components/UserMenu.tsx`、`AdminOnly.tsx`、`RouteError.tsx` | 用户名的悬浮菜单；管理员页面的拦截说明；页面出错时的兜底 |
| `src/components/DataCheck.tsx`、`ChainProgress.tsx` | 同步页的数据核检卡片；一次同步的进度条，同步记录和同步详情共用（页面之间不互相引用组件） |
| `src/components/SecuritySearch.tsx`、`WatchButton.tsx` | 股票搜索框；收藏星标 |
| `src/lib/runs.ts` | 一次同步各批的合计 |
| `src/lib/accounts.ts`、`queryInvalidation.ts` | 账户的金额格式，建议动作、理由种类和流水类型的中文名与颜色；账户修改之后要刷新的查询，见 [账户修改后的刷新](#account_invalidation) |
| `src/lib/format.ts`、`labels.ts` | 数字和日期格式化、涨跌颜色；后端枚举值的中文名 |
| `src/hooks/` | URL 状态（`useSearchState`）、防抖、深色模式、自选股列表 |
| `src/api/fetcher.ts` | 所有请求经过的函数，把错误响应转为 `ApiError` |
| `src/api/generated/` | orval 生成的类型和 hooks，不手工修改 |
| `openapi.json` | 后端导出的接口规范，生成客户端的输入，提交到版本库 |

<a id="pages"></a>
## 页面与接口

界面按用户要做的事组织：主导航是「首页」「账户」「研究」「自选」，个股从页头搜索进入，数据同步、指数板块和「说明」在页头右侧。理由见 [ADR：界面按任务组织](../decisions/ui_by_task.md)。每页先给一句白话结论，再给少量关键数字，明细和规则说明折叠。

| 路径 | 页面 | 接口 |
| --- | --- | --- |
| `/` | 首页：核心数据还在首次下载时顶部提示「正在准备核心数据」和大约还要多久；顶部依次显示我的账户（最多 3 个，每个账户一行：总资产、当日盈亏、「10-19（周一）运营建议：……」）和我的自选（最多 6 只，按收藏时间倒序，显示当日和近 20 日涨跌）；超出时点击「查看更多」进入对应独立页；其后标题写明数据截至哪天收盘（「10-16（周五）收盘」）、大盘概况、今日热点（涨停、跌停、炸板家数，最高连板和每个连板高度前几只股票，点进个股 K 线）、近 5 日最强行业 | `GET /accounts`、`/market/overview`、`/market/hot`、`/market/sectors`、`/watchlist` |
| `/accounts` | 我的账户：每个账户一张卡片（总资产、当日盈亏、现金、持仓只数、下个交易日的运营建议、核对提醒） | `GET /accounts` |
| `/accounts/new` | 新建账户，两步：名字（页面说明一句选股方法）；现金和持仓（`PositionsEditor`，可粘贴，可跳过）。规则用默认值，以后在设置里调 | `POST /accounts`、`POST /accounts/{id}/positions/parse`、`PUT /accounts/{id}/positions` |
| `/accounts/:id` | 一个账户，四个标签：运营（下个交易日的买卖建议、执行、不做、理由和提醒）；持仓；记录；设置（账户名称和备注、调仓规则、费率；规则里有到因子研究组合回测的链接）。保存后建议在后台重新生成，运营标签显示「正在更新」并自动刷新 | `GET /accounts/{id}` 及其下的 `advice`、`trades`、`review` 等 |
| `/research` | 因子研究，两个标签是先后两步：「① 因子检验」逐个体检因子（不显示预设、不填权重，结果下面可以把有效因子带到第二步），「② 多因子组合」把因子合成一个选股方案（可套用预设、填权重、保存为预设）。「因子检验」看 RankIC、分组净值、IC 衰减、挑选偏差和因子相关性，「多因子组合」看净值、超额、信息比率、Deflated Sharpe、成交记录（可按买卖方向和股票筛选，卖出带盈亏和持有天数）和最新得分前列；参数写在 URL，点「开始计算」才算，可以取消 | `GET /research/factors`、`/research/factor-test`、`/research/factor-portfolio` |
| `/watchlist` | 自选：最新收盘、当日和近 20 日涨跌，可搜索添加 | `GET /watchlist`、`PUT` / `DELETE /watchlist/{thscode}`、`GET /securities` |
| `/securities/:thscode` | 证券（股票或 ETF）：页头是最新价、收藏、上市日期和本地日线范围。先是 K 线，再是基本面；股票的基本面显示估值历史、最近一期三张报表和财务指标，并标出披露日期；ETF 显示「ETF 基本资料」（最新收盘、单位净值、基金规模、管理费、管理人、历史日线根数、最新日期和价格口径），K 线只提供不复权、前复权、后复权，价格按供应商前复权口径保存 | `GET /securities/{thscode}`、`/fundamentals`、`/bars` |
| `/hot` | 今日热点：顶部是涨停、跌停、炸板家数、最高连板和封板率；下面按连板高度分组列出涨停股（涨停原因、封单额、首次涨停时间）、跌停和炸板；日期框可往回看，没有同步时显示白话空态。不加主导航项，从首页卡片进入 | `GET /market/hot` |
| `/guide` | 说明：因子、排名、组合、账户调仓四步，以及建议怎么执行 | — |
| `/indices`、`/indices/:thscode` | 指数与板块列表；指数 K 线和当前成分股 | `GET /indices`、`/indices/{thscode}/bars`、`/members` |
| `/sync`（管理员） | 数据同步，从上到下：「立即同步全部」「手动同步」（按计划全选或勾选到单个数据集，选增量或全部重取）；一句话状态；「股票与 ETF 行情同步」卡片（后端标记 `etf_market` 的数据集，即写 ETF 日线或 ETF 资料的数据集的状态和最近完成时间）；同步计划（时间可改，可「现在同步一次」）；数据核检；同步记录（标明来源，几批合成一行）；需要注意（缺口和失败，一键补齐）；按分类的数据集（分类的先后用后端给的 `category_order`，即分类在 `datasets.yaml` 中第一次出现的顺序；前端不抄数据集 ID 和分类名） | `GET /sync/health`、`/sync/schedule`、`/sync/runs`、`/sync/check`、`/datasets`，`PUT /sync/schedule`，`POST /sync/runs`、`/sync/repair`、`/sync/check` |
| `/sync/runs/:runId` | 一次同步的详情（任意一批的编号都行）：各批状态、还没开始的批次、各数据集进度、没有新建任务的数据及原因、失败的任务；合计与同步记录用同一个函数（`lib/runs.ts`） | `GET /sync/runs/{run_id}`、`/datasets` |
| `/login` | 登录，成功后回到 `next` 参数指向的页面；不在 `Layout` 中 | `POST /auth/login` |
| `/setup` | API Key 引导和更换：验证并保存，显示来源和末尾 4 位（管理员） | `GET /setup`、`PUT /setup/api-key` |
| `/users` | 用户管理（管理员）：添加用户（默认普通用户，不填密码时生成一个、只显示一次），设为管理员或普通用户，重置密码，停用和启用；不能修改自己 | `GET /users`、`POST /users`、`PUT /users/{user_id}`、`POST /users/{user_id}/password` |
| `/operation-logs` | 操作日志（管理员）：查看最近成功登录的用户、来源 IP、设备和时间 | `GET /operation-logs` |
| `/me` | 本人：角色，改密码（要输入现在的密码） | `GET /auth/me`、`PUT /auth/password` |

全局行为（`Layout`、`VersionBadge`）：
- 请求收到 401 时整页跳到 `/login?next=<当前地址>`；管理员登录后还没配置 API Key（`GET /setup` 的 `api_key_configured` 为 false）时，管理员页面跳到 `/setup`，普通用户不执行这项前端检查；`GET /auth/me` 的 `must_change_password` 为 true 时页面顶部提醒去「我的」改密码。
- 按角色显示：页头右侧的「说明」对所有用户显示，「数据」只对管理员显示；个人功能「密码设置」和管理员功能「用户管理」「操作日志」「数据源设置」分成两个区域收在用户名的悬浮菜单里（`UserMenu`，鼠标移上去或点击展开）；所有用户都能看到个人区域，只有管理员看到管理员区域；普通用户看不到版本号和升级、用户管理、操作日志和数据源设置；直接打开管理员页面时 `AdminOnly` 给一句说明，不发请求。后端同样拦截，前端只是不显示用不了的按钮。
- 版本号每 30 分钟请求一次 `GET /system/version`，有新版本时亮红点，点开可以一键升级。升级进行中每 2 秒刷新（服务重启时请求失败也继续），进度条和每一步的状态只显示在版本号弹窗里（`UpgradeProgress`，步骤来自升级助手写的状态），点别处收起，进度保留，版本号上的圆点一直闪，再点开接着看；所有步骤结束、运行的版本变成目标版本后整页刷新。
- 刷新间隔：同步页的运行列表 10 秒、同步状态 15 秒、数据集和同步计划 30 秒；运行详情在进行中时 5 秒。

研究页的成交明细来自本次响应的 `trades`，前端倒序显示；同日的显示顺序也随之反转，不应据此解释后端成交先后。「卖出」筛选包括 `sell` 和 `liquidate`；顶部成交数量、胜率和利润合计针对全部记录，不随筛选改变。首屏最多显示 50 笔，点击后每次最多再显示 200 笔，只是浏览器中的显示截断，不会请求服务端分页。价格是模拟前复权价，盈亏和持有天数的口径见 [成交记录](../domains/factor_research.md#portfolio)。

当前「保存为预设」入口不代表普通用户拥有写权限，普通用户保存、删除会返回 403；这是待确认的界面与权限分歧，见 [因子预设](../domains/accounts.md#presets)。

<a id="api_client"></a>
## 接口客户端

类型和 hooks 由 orval 从 `openapi.json` 生成（配置在 `orval.config.ts`）。后端接口变更后，在 `frontend/` 下执行 `pnpm api:spec`（运行 `niulai openapi` 导出规范）、`pnpm api:gen`（重新生成 `src/api/generated/`）、`pnpm typecheck`。

请求都经过 `src/api/fetcher.ts` 的 `apiFetch`：2xx 返回解析后的 JSON；其他状态抛出 `ApiError`，带 HTTP 状态和后端的 `code`、`message`（见 [错误格式](../interfaces/rest_api.md#errors)），框架直接返回的纯文本 400 原文作为 `message`。TanStack Query 对 4xx 不重试，对 5xx 和网络错误最多重试 2 次。

<a id="conventions"></a>
## 展示约定

- **红涨绿跌：** 上涨、买入用红色，下跌、卖出用绿色。颜色令牌是 `src/index.css` 中的 `up`、`down`，数值的涨跌颜色统一用 `upDownClass`。
- **数值类型：** 行情接口的价格和成交额是十进制字符串，只在展示和画图时转成数字；研究结果的模拟价格、金额、费用和统计量本身就是 JSON 数字。按各接口 schema 处理，不统一强转。
- **复权口径：** K 线默认等比前复权（`qfq`），按钮上写明「等比」和「同花顺」。因子和收益都按等比前复权计算。
- **K 线区间：** `RangePicker` 是一个起止日期框（两头都包含）加快捷区间 1 月到全部。点快捷区间把日期填进框里（结束日期是今天，「全部」从最早的日线开始），改日期就是自定义区间（`range=custom&from=&to=`）。起点早于最早日线的快捷区间置灰：个股按本地最早的日线，指数和板块从 2022-01-04 开始。因子研究的「因子检验」和「多因子组合」用同一个组件选计算区间，快捷区间是 3 个月、6 个月、1 年、3 年，默认 3 年；后端要求区间至少 60 天，最长分别是 8 年和 5 年。
- **在图上框选：** 在 K 线上按住鼠标左右拖动，松开后放大到这段（只在已加载的 K 线里放大），图下有「还原」；平移用滚轮缩放后拖动底部时间轴，触屏仍可拖动平移。
- **K 线均线：** 个股和指数的 K 线默认画快线 15 日、慢线 30 日两条简单移动平均（收盘价，口径跟随所选复权），周期在日期区间那一行的右侧改（K 线图组件通过 `maSlot` 渲染到页面留出的位置），范围 2 到 250 个交易日，写在 URL 的 `ma1`、`ma2`。周期短的是快线、长的是慢线，按周期判断，不按第几条；颜色按行情软件的习惯：快线黄色、慢线紫色，深色模式调亮。快线上穿慢线（金叉）在 K 线下方标红色「买」，下穿（死叉）在上方标绿色「卖」，可以关掉；前一天快线不高于慢线、当天高于才算上穿，下穿同理（`crosses`）。为了让均线从区间第一天就有值，请求日线时把开始日期再往前推（按最长那条均线换算成自然日，留出长假余量，`src/lib/movingAverage.ts`），多取的日线只用来算均线，不显示；「全部」区间不往前推，开头几天没有均线。
- **名词说明：** 页面上的英文术语有通用中文名时显示中文（例如「信息系数」「夏普比率」「净资产收益率」），英文缩写放进说明里；没有合适中文的保留原文。需要解释的名词后面放小问号（`ui.tsx` 的 `HelpTip`，带文字时用 `Term`），鼠标悬停、键盘聚焦或点击都能展开，点别处、按 Esc 或滚动时收起；说明框用 fixed 定位，放在横向滚动的表格里也不会被裁掉。表单项的 `hint` 也显示成小问号，不再只靠浏览器的 `title` 提示。
- **URL 参数：** 搜索词、复权口径、区间、因子研究的参数等写在查询参数里，刷新或分享后保持；等于默认值的参数不写入 URL。
- **大数和时间：** 成交额、成交量用「万」「亿」，成交量单位是股；时间按上海时间显示，见 [时区](../interfaces/configuration.md#time_zone)。
- **长任务要点按钮才算：** 因子检验和多因子组合改参数不会自动重算，点按钮才开始；改参数时保留上一次的结果并提示参数已改。`ResearchPage.tsx::useManualRun` 使用 TanStack Query 取消及生成客户端的请求 signal 中断客户端请求，`longTask.tsx` 提供等待秒数和按钮。研究查询关闭自动重试和窗口聚焦刷新，缓存新鲜期为 10 分钟；它不是服务端持久化结果。取消不能保证同步 CPU 段立即停止或已写入的运行记录回滚，见 [研究排错](../domains/factor_research.md#troubleshooting)。
- **面向非专业用户：** 术语（复权、因子、成分股等）都附一句说明。

<a id="account_invalidation"></a>
**账户修改后的刷新**：所有账户写操作成功后都调 `invalidateAccountQueries(qc, 账户, 修改类型, 接口返回的新结果)`（`src/lib/queryInvalidation.ts`），不在组件里各自列刷新清单。修改类型决定刷新哪些查询：

| 修改类型 | 什么时候 | 刷新 |
| --- | --- | --- |
| `ledger` | 记一笔、撤销、校准、配股认购或暂不处理 | 账户列表、账户详情、建议、历史建议、建议回看、影子订单、净值、流水、待处理的配股 |
| `advice` | 已执行、不做、撤销不做（「已执行」会记流水） | 同上，除了待处理的配股 |
| `settings` | 改名字、规则、费率 | 账户列表、账户详情、建议、历史建议 |

接口已经返回的账户详情或建议直接放进缓存，不再请求一次。新增账户派生的查询时，只在这个文件里登记它受哪类修改影响；对应的测试在 `queryInvalidation.test.ts`。

<a id="serving"></a>
## 由 niulai serve 提供

设置 `NIULAI_WEB_DIR`（指向 `frontend/dist`）后，`niulai serve` 同时提供页面，实现在 `backend/src/api/web.rs`：

- `/api/v1/*` 是接口；其他未匹配的 `/api/` 路径返回 JSON 404。
- 其他路径先找静态文件，找不到时返回 `index.html` 由前端路由处理，所以刷新 `/securities/600519.SH` 也能打开。`/assets/` 下找不到的文件返回 404，不回退到页面。
- 页面和接口的响应按浏览器的 `Accept-Encoding` 用 gzip 压缩。
- `/assets/*` 带内容哈希，`Cache-Control: public, max-age=31536000, immutable`；其他文件（含 `index.html`）是 `no-cache`，发布后立即生效。

Docker 镜像在构建时执行 `pnpm build`，产物在 `/app/web`，镜像中默认 `NIULAI_WEB_DIR=/app/web`。本地开发用 Vite 开发服务器（`pnpm dev`），`/api` 转发到 `http://127.0.0.1:8080`（`NIULAI_API_URL` 可改）。
