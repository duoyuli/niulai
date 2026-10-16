# A 股免费数据来源调查

核查日期：2026-10-15。范围：可供 niulai 日线研究使用的数据来源，重点关注能否程序化获取并保存到本地。本文区分无接口费、免费额度、已有付费产品附带权益、试用和公开网页。免费获取范围与团队使用授权分别核对。

本轮进行了公开网络检索，并核对官方文档、项目维护者文档和少量实现代码。BaoStock 的动态文档通过浏览器读取。没有注册账号、申请试用、调用股票数据 API 或执行批量下载，因此以下为候选清单，不是可用性实测报告。

## 1. 优先关注的来源

| 平台或工具 | 免费的具体含义 | 主要数据 | 对本项目的判断 |
| --- | --- | --- | --- |
| [BaoStock][bao-home] | 官网明确免费、无需注册；公共访问有限额 | 股票和指数 K 线、复权因子、部分财务与证券基础数据 | 优先验证历史日线；沪深覆盖、退市数据和北交所范围分别检查 |
| [AKShare][ak-stock] | 开源取数工具；常用股票接口封装公开网页数据，具体接口可能有不同条件 | 行情、财务、行业概念、资金与事件等 | 优先作为补充候选；应记录实际来源网站，不能只记“AKShare” |
| [TickFlow 免注册服务][tick-price] | `TickFlow.free()` 提供历史日 K 和标的查询 | 历史日 K，盘中不实时更新 | 可作为日线补充或小样本对照候选；不能据此认为复权因子和财务也免费 |
| [TickFlow 注册 Free 套餐][tick-price] | 注册后有免费 API Key 和有限调用额度 | 实时行情、实时更新的日 K，覆盖其说明中的 A 股／港股／美股 | 适合少量标的验证；公开限制为每分钟 10 次、单次最多 5 个标的，不支持标的池查询 |
| [Tushare Pro][tushare] | 有零费用积分档，更丰富接口需要相应积分或独立权限 | 免费档明确列出未复权日线 | 可以作为日线候选；不能将 Pro 全部目录视为免费数据 |
| [efinance][efinance] | 免费 Python 库；项目声明仅供学习交流、不得商用 | 股票 K 线、行情、财务和资金等 | 使用简单，但实际数据来源与 AKShare 部分重合；团队用途先核对声明 |
| [mootdx][mootdx] | 社区通达信数据读取工具；发布页同时列有学习交流、不得商用声明 | 通达信本地文件与行情接口 | 作为额外技术路线；它不是同花顺官方 API，也没有本轮验证的稳定性承诺 |
| [巨潮资讯][cninfo] | 公告及部分公开信息可浏览查阅；另有独立数据服务平台 | 公司公告、年报、披露日期、部分股权事件 | 适合保留公告原文证据；公开网页与批量结构化 API 的权限要分开确认 |

东方财富、新浪财经和腾讯财经也是常见公开数据来源，但应与封装它们的库分开理解。AKShare 的日线文档明确列出东方财富与新浪等上游；efinance 的部分实现也访问东方财富。因此，同时接入两个库不一定能形成独立数据源对照。[AKShare 股票接口][ak-stock]、[efinance 实现][efinance-source]

AKShare README 将数据用途描述为学术研究，并提示部分接口可能移除。代码免费本身不证明各上游数据允许任意团队共享或对外提供。[维护者说明][ak-readme]

## 2. 与同花顺使用习惯相关的结论

### 同花顺 iFinD：存在免费接口额度，但有前提

官方 FAQ 将“免费账号”解释为购买 iFinD 终端后可使用少量接口产品；权限页则说明使用 iFinD 账号登录。由此不能从“使用过普通同花顺看盘”推断已经具备 iFinD 数据接口权限，需要核对实际账号。[账号类型说明][ifind-faq]

免费版权限页列出：历史行情每月 100 万数据单元，历史行情和基础数据通常限最近 5 年；这里一个数据单元指一个字段值，不是一条包含全部字段的 K 线。具体账号能否使用，本轮没有登录验证。[免费额度与历史年限][ifind-permission]

如果已有符合条件的 iFinD 权益，它值得优先核对；如果没有，则不把它归为无购买前提的免费来源。

### 同花顺 SuperMind／问财：平台查询与本地入库要分开看

当前 SuperMind 本地 SDK 公开申请页面向符合条件的投资机构，并列出团队与研发经验要求。本轮没有查到面向所有个人的无条件免费本地 SDK 权益。[本地 SDK 申请说明][supermind-sdk]

问财 API 文档区分实时选股和历史回测选股，并明确说明问财接口不支持本地接口调用。它适合作为选股交互参考，不能直接当作我们的本地全量数据服务。[SuperMind API 文档][supermind-api]

## 3. 对三个主要候选的核查结果

### BaoStock

官网当前说明股票日／周／月线历史范围自 1990-12-19 起；这是平台范围，单只股票仍受上市时间与实际覆盖约束。日线计划在交易日 17:30 入库，复权因子在 18:00 入库。它还提供收盘后更新的分钟数据，不能因此认定有盘中实时行情。[平台说明][bao-home]

日线文档列出交易状态与 `isST` 字段，并说明停牌日也可能返回记录：价格沿用前收、量额为零。这与一些供应商“停牌期间没有日线记录”的处理不同，入库时需要保留状态。[日线说明][bao-kline]

访问规则明确：每日 API 请求不超过 5 万次，不能并发连接。本轮没有测量实际吞吐，也没有证据支持“无限量并发下载”。[访问规则][bao-limit]

### AKShare

东方财富日线接口文档覆盖沪深京，并支持不复权、前复权和后复权选择。新浪接口还列有复权因子选项；同一文档提醒频繁访问可能触发 IP 限制。应按接口核对历史覆盖与返回单位，不能把某个接口的能力推广到整个库。[股票接口文档][ak-stock]

我们的适用性判断：它的优势是数据类别广，适合补充字段和对照样本；网页来源变化与接口维护需要纳入长期使用成本。尚未实际调用，不能断言当前所有接口都可用。

### TickFlow

官方 FAQ 明确区分两种免费方式：免注册服务用于历史日 K 与标的查询；注册 Free 套餐用于有限额度的实时行情和日 K。两个通道不能混用权限结论。首页将除权因子列在 Starter 套餐，财务列在 Expert 套餐，本轮没有确认免费档能覆盖这些数据。[免费方式与限制][tick-price]、[功能分层][tick-home]

## 4. 其他平台为什么没有列为首选免费源

| 平台 | 本轮证据 | 当前处理 |
| --- | --- | --- |
| 聚宽 JQData | 官方 SDK 仓库提供申请试用和购买入口；官网详情本轮有地域限制 | 试用候选；不引用旧文章中的试用天数或免费额度作为当前承诺 |
| 米筐 RQData | 检索到官方早期试用资料；当前详情页本轮未完整获取 | 免费期限和额度待确认，不列入已核实的长期免费表 |
| BigQuant | 官方 SDK 文档的搜索结果说明需要 SDK 权限；部分会员资料中的“免费”是会员附带权益 | 权限候选；全文页面本轮抓取超时，尚未确认账号的实际免费范围 |

资料：[JQData 官方 SDK][jq]、[RQData 官方入口][rqdata]、[BigQuant SDK 文档][bigquant-sdk]、[BigQuant 会员 SDK 说明][bigquant-member]。此表表达本轮未确认的范围，不是对平台所有活动或套餐的否定。

## 5. 对照我们需要的七类数据

以下是下一轮核查分工，不代表已验证覆盖。

| 数据类别 | 优先核查候选 | 仍需确认的缺口 |
| --- | --- | --- |
| 股票档案 | BaoStock；AKShare 补充 | 退市股票、历史名称、北交所覆盖、代码变更 |
| 交易日历 | BaoStock 等具有交易日查询的来源 | 历史与未来日期范围、交易所区分 |
| 股票日线 | BaoStock；AKShare 或 TickFlow 对照 | 复权口径、停牌记录、实际历史完整度 |
| 复权与公司行为 | BaoStock、AKShare | 完整分红、送转、配股事件；事件日期不能只用因子代替 |
| 历史交易状态 | BaoStock 日线的状态与 ST 字段；其他源补充 | 每日准确涨跌停价、退市整理与上市初期等状态 |
| 每日股本与估值 | BaoStock 的部分估值字段；AKShare 补充 | 历史总股本、流通股本、市值的连续覆盖 |
| 指数日线 | BaoStock、AKShare | 所选指数的历史范围；指数股票池还需历史成分关系 |

本轮尚未确认哪个免费方案能够完整、稳定地覆盖全部七类数据。尤其是历史股本／市值、完整公司行为、历史涨跌停价格以及退市和北交所样本，需要专项核验。

## 6. 建议的选择顺序

1. 先验证 BaoStock 的历史日线、复权、状态与基础数据，检查它能覆盖多少首批需求。
2. 用 AKShare 的具体上游补充缺口和对照样本；对照时保留真实来源名称。
3. 将 TickFlow 免注册服务列为日线补充候选；额外数据不默认有免费权限。
4. 如果已有 iFinD 账号权益，再核对其剩余额度、年限及团队用途；普通同花顺使用经历不作为权限证明。

下一步可以抽取少量沪深、ST、停牌、分红、退市和北交所样本，逐项验证数据；在样本结果出来前，保留供应商选择，不进行全市场采集。免费接口费用、数据使用范围、完整度和维护成本是四个不同评价维度。

[bao-home]: https://www.baostock.com/mainContent?file=home.md
[bao-kline]: https://www.baostock.com/mainContent?file=stockKData.md
[bao-limit]: https://www.baostock.com/blacklist
[ak-stock]: https://akshare.akfamily.xyz/data/stock/stock.html
[ak-readme]: https://github.com/akfamily/akshare
[tick-price]: https://tickflow.org/pricing/
[tick-home]: https://tickflow.org/
[tushare]: https://tushare.pro/document/2?doc_id=290
[efinance]: https://efinance.readthedocs.io/en/latest/index.html
[efinance-source]: https://efinance.readthedocs.io/en/latest/_modules/efinance/stock/getter.html
[mootdx]: https://pypi.org/project/mootdx/
[cninfo]: https://www.cninfo.com.cn/
[ifind-faq]: https://quantapi.10jqka.com.cn/gwstatic/static/ds_web/quantapi-web/help-center/faq.html
[ifind-permission]: https://quantapi.10jqka.com.cn/gwstatic/static/ds_web/quantapi-web/help-center/permission.html
[supermind-sdk]: https://quant.10jqka.com.cn/view/help/3
[supermind-api]: https://quant.10jqka.com.cn/view/help/4
[jq]: https://github.com/JoinQuant/jqdatasdk
[rqdata]: https://www.ricequant.com/welcome/rqdata
[bigquant-sdk]: https://bigquant.com/wiki/doc/vac4qwmQr4
[bigquant-member]: https://bigquant.com/wiki/doc/COs1vUo1dU
