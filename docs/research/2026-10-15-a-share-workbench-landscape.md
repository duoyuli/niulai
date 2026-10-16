# A 股研究工作台：市场与技术路线调研

调研日期：2026-10-15（Asia/Shanghai）。适用范围：小团队共用，优先覆盖日线技术面与中短期回测；机构数据进入本地 PostgreSQL；计划接入 Codex、Claude Code。本文用于比较产品做法和讨论建设路线，尚未形成最终选型。

本轮核对了国内量化平台、研究与回测框架、AI 投研项目及诊断交互工具，共 17 组代表产品，并补充数据服务与 Agent 集成方式。判断依据是官方文档、维护者仓库、部分实现代码和论文摘要；没有登录付费平台，没有运行回测、性能测试或完整部署。项目目录在调研开始时为空。

## 1. 最值得带入讨论的发现

1. **TSP（Tick Stock Panel）保留为底层机制参照，交互不作为本项目模板。** 从公开功能看，它整合了策略、回测、个股分析和 AI 工具调用；但用户在本次讨论中明确反馈其操作复杂、理解成本过高。本项目将低理解成本列为产品约束，先验证核心任务流程，再评价代码复用。团队身份、实验归属、数据版本与 PostgreSQL 适配也尚未确认。[TSP README][tsp]；用户体验反馈见第 9.4 节。
2. **可配置策略通常建立在可组合的计算能力之上。** 米筐拆分数据、因子、回测、组合优化和归因工具；BigQuant 让可视化模块和 Python／SQL 共同表达研究过程。这两种做法都值得参考。[RQSDK 文档][rqsdk]、[BigQuant 使用文档][bigquant]
3. **实验记录是独立能力。** Qlib 用 Experiment 和 Recorder 管理实验与单次运行，QuantConnect 则提供组织、项目协作和本地／云端研究环境。团队工作台需要比较、复查和延续实验，单纯共享一个页面无法覆盖这些需求。[Qlib Recorder][qlib-recorder]、[QuantConnect 协作][qc-collaboration]
4. **AI 分析与可成交的组合回测需要分别评估。** TradingAgents 当前源码明确把其 `backtest` 定义为决策质量评估；它没有将评级变为成交订单所需的完整现金账本。RD-Agent 的另一条路线是生成研究假设和代码，再通过实验反馈迭代。[TradingAgents 回测源码][ta-backtest]、[RD-Agent 论文][rd-paper]
5. **“代码可见”不能直接等同于“适合团队复用”。** RQAlpha 当前许可证包含商业及组织使用限制；TradingAgents-CN 的应用层使用专有许可；vectorbt 附有 Commons Clause。OpenBB 又处于产品开放与治理移交阶段。选型必须精确到组件与版本。[RQAlpha 许可][rq-license]、[TradingAgents-CN 许可][cn-license]、[vectorbt 许可][vbt-license]、[OpenBB 公告][openbb-transition]

以上是对本轮样本的判断，不代表市场占有率排名，也不证明这些项目在实际数据上的收益、速度或可靠性。

## 2. 如何阅读证据

| 标记 | 本文能确认什么 | 本文不能据此确认什么 |
| --- | --- | --- |
| 官方文档 | 产品公开描述的功能、接口和工作流 | 账号实际权限、当前套餐交付、生产稳定性 |
| 仓库文档 | 维护者在 README、指南中的说明 | 宣称的性能数字、完整功能质量 |
| 源码核查 | 所读文件中可见的实现和边界 | 所有调用路径都正确，或代码已在本环境跑通 |
| 论文说明 | 作者提出的方法与摘要中的实验主张 | 我们的数据、市场和成本下能复现同样结果 |
| 用户体验反馈 | 本次讨论中用户对产品操作与理解成本的评价 | 未观察的具体交互原因，或所有目标用户的普遍体验 |
| 调研判断 | 根据上述证据提出的适用性分析 | 已批准的项目方案或已经验证的能力 |

重要访问边界：聚宽官网返回地域限制，改用其官方 JQData SDK 仓库核对产品与接口定位；掘金首页返回 403，使用其官方策略示例库补充。两者的当前付费界面、团队权限与套餐没有得到完整验证。没有将第三方榜单、Star 数或营销收益曲线作为选型依据。

## 3. 产品全景对照

### 3.1 国内量化平台与整合式工作台

| 产品 | 公开材料呈现的主要做法 | 对本项目的借鉴 | 本轮边界 |
| --- | --- | --- | --- |
| [TSP / Tick Stock Panel][tsp] | 自托管；Parquet／DuckDB；统一指标数据；策略、回测、AI、开放 API 与 MCP | 共享计算口径、能力接口和任务隔离等底层机制 | 用户反馈操作复杂；交互不作为模板；团队身份和存储适配待验证 |
| [聚宽 / JQData][jq] | 量化研究平台与本地数据 SDK 并存；通过认证后按统一接口取数 | 数据服务可以独立使用，研究界面不必绑定数据供应商 | 官网访问受限；SDK 的开放不等于数据免费或可任意共享 |
| [米筐 / RQSDK / RQAlpha][rqsdk] | 数据、回测、因子、组合优化、绩效归因分别成产品；支持本地研究 | 模块职责清楚；可逐项采购或集成能力 | RQAlpha 与 RQAlpha Plus、RQData 需要分别评估能力和授权 |
| [BigQuant][bigquant] | 可视化与代码模式；DAI 做数据与因子计算，BigTrader 接收仓位目标并执行回测 | 表单／图形配置与代码共用底层表达；区分打分、仓位、成交 | 不将旧教程中的部署状态、免费额度当作当前合同承诺 |
| [掘金 / MyQuant][myquant] | 官方示例按策略与实现语言组织，配有策略说明、代码和配置 | 策略模板作为可阅读、可运行、可修改的研究单元 | 示例较早；本轮没有确认当前终端、SDK 与团队产品的完整能力 |

### 3.2 研究与回测框架

| 产品 | 公开材料呈现的主要做法 | 对本项目的借鉴 | 本轮边界 |
| --- | --- | --- | --- |
| [QuantConnect / LEAN][lean] | 模块化事件驱动引擎；平台提供本地／云端工作流及团队协作 | 项目、研究环境、回测结果、组织资源分开管理 | A 股数据与交易制度适配未验证；商业平台功能不能直接等同于引擎能力 |
| [Qlib + RD-Agent][qlib] | 数据、模型、策略、评估流水线；实验记录；自动因子／模型研究 | 把每次实验变成可比较的记录；AI 研究接受程序化验证 | 机器学习研究取向较强；不是拿来即用的团队个股诊断产品 |
| [VeighNa / vn.py][vnpy] | 策略与交易模块体系；当前仓库已有 `vnpy.alpha` 因子、模型、策略、Lab | 策略生命周期、研究与交易模块边界、可扩展框架 | 不能继续简单归类为“只有期货 CTA”；团队 Web 工作流仍需单独评估 |
| [vectorbt][vectorbt] | 数组化计算、信号／订单接口、参数组合分析与图表 | 快速比较 MA 窗口和参数稳定性 | A 股成交约束需要验证或补充；许可证带 Commons Clause |
| [Backtrader][backtrader] | Cerebro 组织数据源、策略、经纪商模拟、分析器 | 易于理解的事件驱动策略接口和分析器扩展方式 | 框架之外的数据治理、实验协作和工作台需要建设；GPL v3 需按复用方式评估 |
| [TradingView / Pine Script][tradingview] | 指标叠图、脚本策略、策略测试器及成交模拟设置 | 个股诊断体验、信号与成交位置联动、参数调整后的反馈 | 与本地 PostgreSQL、内部数据和团队研究资产的结合程度需另行验证 |

### 3.3 AI 投研与可扩展分析工作台

| 产品 | 公开材料呈现的主要做法 | 对本项目的借鉴 | 本轮边界 |
| --- | --- | --- | --- |
| [TradingAgents][ta] | 多角色分析、研究辩论、决策汇总、历史决策评估 | 分开保存证据、观点、决策；分析过程可以回看 | 其回测模块不是持仓连续演化的组合模拟器；历史信息可见性仍需用例核验 |
| [TradingAgents-CN][cn] | 中文及 A 股研究工作流、Web 应用、模型与数据源配置 | 中文研究任务组织、分析进度、配置管理 | 部分功能按版本分布；应用层有专有许可，不能按纯 Apache 项目整体复用 |
| [daily_stock_analysis][daily] | 自选股报告、Agent 问股、历史报告、持仓、回测入口和消息推送 | 每日研究入口、结构化报告、连续追问和研究留痕 | README 将日常报告、选股参考与策略验证区分到相关项目；各类“回测”需查具体指标 |
| [OpenBB / ODP / Workspace][openbb-odp] | 标准化数据接口、可配置组件、外接 Agent、研究看板 | 同一数据服务供人、API 和 Agent 使用；界面与分析服务解耦 | 产品正在开放和移交；不能把历史商业页面当作当前可直接采购或全部已交付的依据 |

### 3.4 个股诊断与自然语言选股的交互参照

| 产品 | 官方材料中的做法 | 对本项目的借鉴 | 本轮边界 |
| --- | --- | --- | --- |
| [通达信公式系统][tdx] | 分开定义技术指标、条件选股、专家系统和 K 线着色公式；参数与图表联动 | 将同一个研究条件用于图表解释、筛选和信号展示，减少重复定义 | 依据公开手册，未运行终端，也未核查内部数据接入和团队协作 |
| [同花顺问财 / SuperMind][iwencai] | 自然语言选股接入研究流程；文档区分实时查询和历史回测选股入口 | 自然语言可以先生成可检查的筛选条件，再进入研究任务 | 文档区分实时与历史接口，并说明问财接口不支持本地调用；不能直接等同于本地研究工具 |

这两类工具对“诊断工作台”的交互更直接。通达信手册将指标输出、条件筛选与买卖信号分开；SuperMind 文档则明确区分 `query_iwencai` 和 `get_iwencai`。**调研判断：** 让 AI 理解“十五日均线上穿三十日均线”之后，应展示解析后的周期、价格口径、信号条件和日期范围，用户能复查后再保存为研究配置。[通达信手册][tdx]、[SuperMind 问财接口说明][iwencai]

## 4. 重点机制拆解

### 4.1 TSP：底层机制的参照

**仓库文档。** TSP 将数据源、策略与信号、扩展数据和扩展页面作为扩展通道；研究功能共享指标数据。其 MCP 文档展示了「AI 客户端 → MCP → HTTP API → 工作台」的调用关系，权限最终由后端网关判断。[TSP README][tsp]、[MCP 文档][tsp-mcp]

**源码核查。** 在固定提交 `2a1c0768314df275b567990557ccfa6e781519b3` 中，回测 Worker 使用 `multiprocessing` 的 `spawn` 子进程，并通过队列和取消事件协调执行。这证实它把计算从主服务进程中分离，但不证明高并发吞吐或任意故障后的恢复能力。[Worker 实现][tsp-worker]

同一提交的面板登录输入只有密码字段，登录接口围绕访问密码和会话 Cookie 工作。它能证明所核查的面板路径使用密码会话，不能据此把整个产品所有扩展都判定为“不支持多用户”。**对我们的判断是：团队账号、项目归属、成员操作追踪必须专项核查，Token scope 也不能直接当作成员权限模型。**[面板认证实现][tsp-auth]

值得借鉴的地方是共享计算口径、核心能力开放和计算任务隔离。用户已反馈其操作复杂，因此若考虑改造，成本评估必须包含交互流程重设计，再验证数据存储接口、实验版本模型和身份模型。README 中的毫秒级扫描、测试数量和“零运维”等主张，本轮未复测。

### 4.2 米筐与 BigQuant：让策略可配置的两种方式

**官方文档。** RQSDK 将 RQData、RQAlpha Plus、RQFactor、RQOptimizer、RQPAttr 作为独立工具提供。本地研究可以调用数据、执行回测，再进行因子分析、组合优化和归因。它说明“完整工作台”可以由边界明确的能力组成。[RQSDK 文档][rqsdk]

BigQuant 的文档把数据与特征计算、选股与打分、仓位目标、BigTrader 执行分开；可视化与 Python／SQL 是研究表达的两种入口。文档还区分向量化的极速模式与更细的标准撮合模式。这里确认的是文档中的工作方式，并未验证模式间的误差或性能。[BigQuant 研究与回测流程][bigquant]

**调研判断。** MA15 和 MA30 应属于带参数的指标；“上穿”属于信号；买卖条件、股票池、仓位与执行假设组合后才构成完整策略。产品配置可以先用表单表达这些对象，代码扩展共享相同输入输出。图形流程编辑器是否进入首版，应取决于用户确实需要编排多少步骤。

### 4.3 QuantConnect 与 Qlib：把研究沉淀为可复查的实验

**官方文档。** QuantConnect 以组织和项目承载协作，成员能共同编辑项目；协作额度取决于组织层级。其 Local Platform 描述本地与云端工作流，Private Cloud 是另行提供的扩展能力。[项目协作][qc-collaboration]、[Local Platform 功能][qc-local]

Qlib Recorder 将 Experiment 与单次运行 Recorder 分开，并提供基于 MLflow 的实现。[Recorder 文档][qlib-recorder]

**源码核查。** Qlib 的 `SignalRecord`、`SigAnaRecord`、`PortAnaRecord` 分别组织预测信号、信号分析和组合分析；组合记录会保存收益报告、持仓及风险分析产物。这是把研究结果结构化保存的具体例子。[记录模板实现][qlib-record-source]

**调研判断。** 团队成员应该能从一张结果图追溯到数据版本、策略版本、参数、交易假设和执行环境。实验管理工具能够承载这些元数据，但接入 Recorder 或 MLflow 本身不等于已经实现行情快照、严格历史可见性或跨机器结果一致。

还需注意，Qlib 当前 README 提示官方示例数据集暂时停用，并指向社区数据；`main` 也包含尚未发布的迁移说明。示例数据、开发分支和正式发行版需要分别对待。[Qlib README][qlib]

### 4.4 OpenBB：数据、界面组件与 Agent 分别扩展

**官方文档。** ODP 将数据能力通过 Python、REST、MCP 等入口供不同消费者使用。Workspace 的自定义后端用 `widgets.json` 描述界面组件；自定义 Agent 用 `agents.json` 描述能力，并通过查询接口流式返回结果。[ODP 概览][openbb-odp]、[数据接入][openbb-data]、[Agent 接入][openbb-agent]

**当前状态。** 2026-08-25 的官方公告宣布开放整个产品套件；10 月 1 日更新提到 OpenBQ 和 FINOS 的承接安排。本轮读取 GitHub API 时，FINOS 的 Open Workspace 提案标题已标为 `APPROVED`，议题仍为 open。提案通过、仓库公开、依赖整理、发行版本与维护支持是不同事项，本文没有把它们合并为“已经全部可用”。[官方公告][openbb-transition]、[FINOS 提案][finos]

**调研判断。** 很值得参考它的组件契约和外接 Agent 方式。若直接采用其界面，应先核实具体仓库、许可证、可构建版本及后续维护路径。本轮不把仍可访问的历史报价用于项目预算。

### 4.5 TradingAgents：分析流程与组合回测的边界

**论文与仓库文档。** TradingAgents 通过技术、基本面、新闻、情绪等角色分析，再组织研究与风险讨论。论文描述了这种协作方法；本文没有复现论文收益结果，也不把角色数量视为准确性指标。[TradingAgents 论文][ta-paper]

**源码核查。** 在提交 `1394a3f72aa4393e1a98f51b382434c4b4c2d972` 中，`tradingagents/backtest.py` 明确以标的和日期形成独立评估单元，衡量评级对应的后续结果；它没有数量、成交价和现金账本组成的连续组合模型。因此这里的“回测”应称为历史决策评估，与组合收益模拟分开。[回测模块][ta-backtest]

**需要实测解决的文档差异。** 同一版本 README 的更新记录声明改进了按分析日期限制数据的能力，并说明部分无法证明历史可见性的财务数据会被屏蔽；但可重复性段落仍保留实时文本源会变化的描述。本文据此将严格历史回放标记为待验证，不断言所有数据路径仍有泄漏，也不直接接受“已完全解决”的结论。[固定版本 README][ta]

**调研判断。** AI 报告应绑定实际获取的数据与工具结果。对历史 AI 分析，需要额外验证新闻、财务修订和记忆的时间边界；同时以向前运行的新样本检查实际表现，不能只靠模型在提示词中扮演过去日期。

### 4.6 TradingAgents-CN 与 daily_stock_analysis：日常研究的产品入口

**仓库文档。** TradingAgents-CN 提供中文研究与 Web 配置能力，README 将不同版本的发布状态和功能分开描述。其核心与应用层采用不同许可证；`app/LICENSE` 明列禁止修改和分发，商业使用需书面许可，内部业务评估需事先书面同意。团队评估应用层之前，应核对可适用的授权条件。[README][cn]、[根许可证][cn-license]、[应用层许可证][cn-app-license]

daily_stock_analysis 把自选股、每日报告、历史记录、Agent 追问和通知放在日常工作流中；README 同时把 AlphaSift 列为选股实现参考，把 AlphaEvo 列为策略验证与进化项目。它提供了从“日常研究入口”组织产品的例子；本文没有核查其所有回测入口与相关项目。[DSA README][daily]

**调研判断。** 这些项目适合研究“人怎样持续使用诊断报告”：今天哪里变化、哪些证据支持结论、如何追问、如何回看旧报告。若用于我们的团队场景，还需要检查成员归属、报告引用的数据版本和数值生成路径。

### 4.7 RD-Agent：AI 参与实验，而不只生成解释

**论文说明。** RD-Agent-Quant 将过程分为研究假设、开发实现和实验反馈，并围绕因子与模型进行迭代。它体现了另一类 AI 工作流：让 AI 提交可运行的研究产物，再用实验结果评价。论文中的收益比较依赖作者的实验设置，本轮没有复现。[RD-Agent-Quant 论文][rd-paper]、[项目仓库][rd]

**调研判断。** 这比直接要求 AI 给出“买或卖”更适合工程化研究。例如，让 AI 将“均线上穿后成交量放大是否有帮助”转成候选规则、生成实验配置、比较样本外表现，并说明失败样本。是否引入自动迭代，需要先建立实验预算、独立测试区间和候选策略发布规则。

## 5. 数据服务与本地存储

### 5.1 数据服务应按数据集比较

| 候选 | 已核对的接口或产品特征 | 进入本项目之前要验证的事项 |
| --- | --- | --- |
| [JQData][jq] | 独立认证的本地 SDK，提供行情及研究数据接口 | 历史覆盖、退市标的、复权与修订口径、团队使用和批量存储权限 |
| [RQData][rqsdk] | RQSDK 的数据组件，文档提供 Python、HTTP、CLI 入口 | 所需数据集的实际权限、历史可见性、导出配额及服务报价 |
| [Tushare Pro][tushare-access] | 积分门槛接口与独立授权接口并存；文档区分个人与机构价格 | 日线之外的复权因子、股票状态等是否均有权限；不能用个人套餐估算机构成本 |
| [AKShare][akshare] | 多来源数据访问工具；README 提醒部分接口可能移除，并限定其数据用途说明 | 上游来源、字段稳定性、使用范围；适合作为补充候选，不能据此假设有生产服务承诺 |

本轮没有对这些供应商做同日抽样对账，不能评价谁的数据“最准”。Wind、iFinD 等机构数据服务的具体合同和接口未展开；如果团队已经有机构授权，应把现有授权放到下一轮优先核查清单。

这里可借鉴的是 TSP 的数据集能力路由、OpenBB 的统一数据接口，以及 JQData／RQData 的独立数据服务形态。**设计判断：应分别记录某个账号能否取得日线、复权因子、交易日历和历史状态；供应商名字或一个“连接成功”状态不足以代表全部能力。**

### 5.2 “本地存储”有多种实现

| 参照 | 文档可见的做法 | 对 PostgreSQL 方案的启发 |
| --- | --- | --- |
| TSP | Parquet 保存数据，DuckDB 查询，Polars 计算 | 在线业务库与批量分析存储可以有不同职责；迁移前先明确数据访问契约 |
| LEAN Local Platform | 数据位于组织工作目录，文档采用 CSV／JSON 等文件格式 | 回测引擎可能需要自己的数据布局；通常需要适配或导出步骤 |
| Qlib | 数据层与实验管理分开；独立的数据提供和处理流程 | 行情、特征缓存、实验元数据不必采用同一物理组织方式 |

资料：[TSP README][tsp]、[LEAN 本地数据][qc-data]、[Qlib 数据层][qlib-data]。

**对本项目的初步判断：** PostgreSQL 可以继续作为共享结构化数据与研究元数据的候选中心。是否增加 Parquet 等格式，应由实际查询、快照导出和回测数据访问测试决定。本轮没有性能证据支持“所有数据必须进入一种存储”或“必须立即增加某种分析数据库”。

尤其要明确复权基准。Tushare 的文档说明其前复权结果与请求的 `end_date` 有关；相同股票在不同截止日下得到的历史价格可能不同。因此保存一份“前复权日线”而不保存因子、基准和版本，难以解释后续实验差异。[Tushare 复权说明][tushare-adjust]

## 6. 回测能力需要用同一套问题比较

不同产品的“回测”不能直接放在同一列打勾。建议先按以下三类评估对象区分，再单独比较参数搜索能力；一个工具可以覆盖多类对象。

| 类型 | 核心问题 | 代表性参照 | 不能替代的能力 |
| --- | --- | --- | --- |
| 指标与信号检验 | 信号之后的收益分布、分组表现如何？ | Qlib 信号分析、因子研究工具 | 资金和订单约束下的组合结果 |
| 组合回测 | 持仓、现金、交易费用和成交约束共同作用后如何？ | RQAlpha／Plus、LEAN、Backtrader、BigTrader、vectorbt Portfolio | 真实市场里的执行验证 |
| AI 历史决策评估 | 当时的分析结论之后是否成立？ | TradingAgents 当前评估模块 | 连续仓位与成交模拟；模型和文本源的历史隔离 |

**参数搜索是横向研究能力。** 它可以用于信号检验、组合回测和 AI 评估。vectorbt 既提供组合模拟，也便于比较参数组合，不能将它仅归为筛选器。参数搜索应另行检查预算、选择依据与独立样本外验证；搜索得越多不代表结果越可信。

资料：[Qlib 记录实现][qlib-record-source]、[RQAlpha 模拟设置][rq-simulation]、[vectorbt 信号接口][vectorbt-portfolio]、[TradingAgents 回测源码][ta-backtest]。

下一轮适合共同验证的用例是：**相同历史股票池、相同原始行情与复权方式，比较 MA15／MA30 和另一组窗口，收盘后生成信号，按声明的下一时点执行规则回测，并保存全部实验条件。** 这是测试样例，不是投资建议或已经选定的策略。

验收问题应覆盖：

- 历史股票池是否保留当时存在、后来退市的股票，以及当时的 ST 等状态？
- 指标何时可知，订单何时发出，成交价如何确定？停牌、涨跌停、交易单位、卖出约束如何处理？
- 使用原始价格还是复权价格计算指标与成交？分红等公司行为如何影响现金和持仓？
- 税费与交易规则是否能按生效日期变化，而不是把当前设置套用到全部历史？
- 参数选择是否使用了最终测试区间？是否保存失败和较差的实验，而不只保存最好结果？
- 同一数据快照与策略版本重复运行时，数值结果是否一致？切换引擎后差异能否解释？

这些是待执行的验证问题。本轮没有跑通它们。TradingView 官方文档对成交模拟和未来信息偏差的说明，可作为设计用例的参照；上交所交易规则有明确版本和生效安排，说明历史规则配置需要保留时间维度。[TradingView 策略文档][tradingview]、[上交所规则发布页][sse-rules]

## 7. Codex 与 Claude Code：两种值得比较的接法

| 路线 | 已有参照与公开接口 | 适合验证什么 | 平台仍需承担什么 |
| --- | --- | --- | --- |
| 让现有 Agent 调用工作台 | TSP 用 MCP 包装自己的 HTTP API；OpenBB ODP 将数据暴露给 MCP 消费者 | 用户在 Agent 中查询指标、提交实验、获取报告 | 稳定工具契约、数据权限、任务编号、产物留存 |
| 在工作台内运行 Agent | OpenAI 提供 Codex SDK／非交互模式；Anthropic 提供 Claude Agent SDK | 工作台内的分析任务、代码生成、研究步骤执行 | 运行环境、会话、资源限制、取消／恢复、日志、模型用量和报告校验 |

资料：[TSP MCP][tsp-mcp]、[ODP][openbb-odp]、[OpenAI Codex SDK][codex-sdk]、[Codex 非交互模式][codex-exec]、[Claude Agent SDK][claude-sdk]。

OpenAI 官方文档给出了程序化控制本地 Codex 的方式，非交互模式提供 JSONL 事件与结构化输出；Claude Agent SDK 提供驱动 Claude Code 能力的编程接口。这里核对的是官方集成入口，没有实际启动两种运行时，也没有验证账号授权或费用。

**调研判断。** 两种入口可以复用同一套研究工具。工作台负责数据与数值计算，Agent 负责提出研究步骤、选择工具和解释结果；策略代码生成可作为独立能力评估。应分别保存“原始证据、程序计算结果、模型解释、人工意见”，并记录模型、提示词和工具调用版本。是否同时启用两个 Agent，以及是否需要相互复核，应通过准确性、耗时和成本实验决定。

## 8. 复用与成本边界

以下是本轮直接读到的仓库许可标记或限制摘要，不替代具体版本的完整条款。数据供应商授权与软件许可证是两件事。

| 项目或组件 | 当前核对结果 | 对路线选择的影响 |
| --- | --- | --- |
| TSP | [MIT][tsp-license] | 可进入代码复用评估；依赖和数据权限另查 |
| Qlib | [MIT][qlib-license] | 可进入研究组件评估；数据集可用性另查 |
| VeighNa 核心 | [MIT][vnpy-license] | 核心框架可进入评估；商业产品与独立插件分别核查 |
| daily_stock_analysis | [MIT][daily-license] | 可评估报告和工作流实现；外部服务能力另查 |
| RQAlpha | [当前 LICENSE][rq-license] 对商业用途和组织使用列出单独授权要求 | 不能根据 Apache 字样当作无限制的团队依赖；应明确目标版本和授权 |
| vectorbt | [Apache 2.0 + Commons Clause][vbt-license] | 后续对外提供收费产品或服务时要核查条款适用范围 |
| Backtrader | [GPL v3][backtrader-license] | 分发与集成路线需纳入许可证评估 |
| TradingAgents-CN | 核心与应用层分别许可；[`app/LICENSE`][cn-app-license] 明列禁止修改，内部业务评估需事先书面同意 | 不直接将整个项目作为无额外条件的团队二次开发基础 |
| OpenBB 产品套件 | [开放承诺与治理移交][openbb-transition] 持续进行 | 逐仓库核实实际公开代码、许可证、版本与维护责任 |

商业平台的真实成本需要拆成数据授权、平台或席位、计算资源、存储运维、模型调用五部分。米筐、聚宽、BigQuant 等本轮没有形成可比的团队报价，不能给出可信的总价排名。Tushare 的公开页面区分个人与机构价格，也区分积分接口和独立权限，说明“购买一个账号即可拉取全部数据”不是可靠的预算前提。[Tushare 权限说明][tushare-access]

## 9. 对 niulai 的讨论输入

### 9.1 可借鉴的共同机制

| 机制 | 参照 | 对我们有价值的原因 |
| --- | --- | --- |
| 按数据集描述供应商能力 | TSP、JQData／RQData、OpenBB ODP | 可以更换或补充数据源，并保留差异与来源 |
| 指标、信号、仓位、执行分离 | BigQuant、米筐、回测框架 | MA15／MA30 之后能扩展到组合规则，而不用为每个策略重新写页面 |
| 每次运行产生独立实验记录 | Qlib、QuantConnect | 团队能比较、复查并延续研究 |
| 界面与 Agent 共用研究工具 | OpenBB、TSP | Codex、Claude 或网页都能访问相同的计算与数据口径 |
| 计算在独立任务中运行 | TSP Worker、QuantConnect 计算环境 | 长回测不会与短查询争用同一请求生命周期 |
| AI 提交可验证产物 | RD-Agent | 可以评价代码和实验，而不只评价报告语言是否可信 |

这张表是综合判断。上述项目并不都完整实现了我们所需的数据版本、团队权限和运行恢复语义。

### 9.2 三条可讨论的建设路线

| 路线 | 能较快获得什么 | 主要代价 | 下一步需要回答的问题 |
| --- | --- | --- | --- |
| 采购数据与成熟研究平台，补充内部入口 | 数据服务、研究工具和已有工作流 | 本地控制、平台扩展范围、持续订阅与导出限制 | 是否已有机构授权？研究能否留在要求的环境？ |
| 复用现有项目的底层能力，例如 TSP | 已有计算和数据接口 | 交互流程重设计、存储迁移、团队身份、版本追溯与持续合并上游变化 | 能否在不继承复杂操作流程的前提下复用所需能力？ |
| 自建团队工作台，接入经过验证的计算组件 | 产品与研究资产模型可按团队需求定义 | 初期集成、数据适配与回测验证工作较多 | 哪些能力能复用，哪些构成我们长期需要掌握的核心？ |

结合用户对理解成本的反馈，下一步优先用低保真流程验证「看个股 → 查依据 → 历史验证 → 持续跟踪」是否容易理解。流程明确后，再比较现有组件的复用成本，并用统一数据和用例核查回测能力。当前没有选定整项目复用、回测引擎或语言栈。

### 9.3 下一轮讨论的三个问题

1. **工作台的主要入口是什么？** 是从一只股票进入技术诊断，再沉淀策略；还是先管理策略与实验，再查看个股解释？两种入口可以共存，但首版主流程需要清楚。
2. **团队是否希望积累自己的研究资产？** 包括数据版本、策略版本、实验记录、研究结论和人工意见。若这是核心诉求，应以这些对象评估现有项目，而不只比页面数量。
3. **AI 首先承担哪种工作？** 解释已有数值、将自然语言转成配置、生成策略代码，还是执行研究实验？每增加一级，工具权限、评估与成本要求都会变化。

建议用前述 MA15／MA30 样例串起这三个问题，再讨论产品范围和实现方案。

### 9.4 用户反馈：降低操作与理解成本

**用户体验反馈（2026-10-15）：**「TSP 的操作太复杂了，用户理解成本过高。」这是用户在本次讨论中提出的评价。本轮没有直接观察其具体操作过程，因此不将复杂性的原因当作已验证事实。

**设计判断：** 普通研究用户的主流程应围绕任务展开；数据源配置、指标注册、任务队列和模型运行设置由管理或开发入口承载。后台保留通用扩展能力，前台按任务逐步展示所需信息。

| 用户任务 | 建议的最短流程 | 首屏应回答的问题 |
| --- | --- | --- |
| 看懂一只股票 | 搜索股票 → 打开诊断 | 目前是什么状态、依据是什么、哪些信息不足？ |
| 验证一个想法 | 从诊断结果选择条件 → 查看规则与默认假设 → 运行历史验证 | 研究的是什么条件、历史表现如何、在哪些情况下失效？ |
| 跟踪后续变化 | 加入关注 → 查看变化记录 | 与上次相比发生了什么、原来的判断是否仍有依据？ |

先使用可解释的模板和默认值，再允许展开指标参数、仓位规则和代码。数据截至时间、验证区间、成本与成交假设等影响判断的条件应简明可见，不能因简化界面而隐藏。结论、数值证据、AI 解释和人工意见应有清楚区分。

AI 可以帮助解释术语、整理证据、将研究意图转换为可检查的配置；可重复的筛选、比较和参数调整仍应有明确的界面操作。避免每次研究都依赖重新描述一段聊天指令。

后续可用性验证应观察：首次使用者能否独立完成一次诊断，能否找到结论依据，能否理解并调整一个研究条件，以及遇到数据不足时能否知道如何继续。用户类型和具体页面布局仍待讨论。

## 10. 证据版本与未完成验证

固定源码参照：

- TSP：`2a1c0768314df275b567990557ccfa6e781519b3`。核查面板认证、回测 Worker、MCP 文档和许可证。[提交记录][tsp-commit]
- TradingAgents：`1394a3f72aa4393e1a98f51b382434c4b4c2d972`。核查 README 与回测模块。[提交记录][ta-commit]
- 其他仓库与官方文档按本次访问内容记录；链接中的 `main`、`master`、`latest` 会继续变化，实施选型时需重新固定版本。
- FINOS 提案通过 GitHub API 核对，记录到的更新时间为 2026-10-14；搜索索引仍可能显示更早的未批准标题。以本次直接读取结果为准。

未完成的验证包括：供应商账号与数据实测、严格历史时点对账、手续费和撮合边界用例、同一策略跨引擎对照、并发任务与中断恢复、团队权限检查、AI 数值引用准确性、模型费用测量、完整部署，以及商业平台询价。

本轮论文阅读限于摘要与仓库方法说明，没有完成全文实验审计或复现。报告中的适用性结论可用于缩小候选范围，不能用作收益或生产可靠性证明。

[tsp]: https://github.com/shy3130/tick-stock-panel/blob/2a1c0768314df275b567990557ccfa6e781519b3/README.md
[tsp-mcp]: https://github.com/shy3130/tick-stock-panel/blob/2a1c0768314df275b567990557ccfa6e781519b3/mcp-server/README.md
[tsp-auth]: https://github.com/shy3130/tick-stock-panel/blob/2a1c0768314df275b567990557ccfa6e781519b3/backend/app/api/auth.py#L104
[tsp-worker]: https://github.com/shy3130/tick-stock-panel/blob/2a1c0768314df275b567990557ccfa6e781519b3/backend/app/backtest/worker.py#L290
[tsp-license]: https://github.com/shy3130/tick-stock-panel/blob/2a1c0768314df275b567990557ccfa6e781519b3/LICENSE
[tsp-commit]: https://github.com/shy3130/tick-stock-panel/commit/2a1c0768314df275b567990557ccfa6e781519b3
[jq]: https://github.com/JoinQuant/jqdatasdk
[rqsdk]: https://www.ricequant.com/doc/rqsdk/index-rqsdk
[rq-license]: https://github.com/ricequant/rqalpha/blob/master/LICENSE
[rq-simulation]: https://rqalpha.readthedocs.io/zh-cn/latest/intro/run_algorithm.html
[bigquant]: https://bigquant.com/wiki/doc/oJyEEQGz0f
[myquant]: https://github.com/myquant/strategy
[lean]: https://github.com/QuantConnect/Lean
[qc-collaboration]: https://www.quantconnect.com/docs/v2/cloud-platform/projects/collaboration
[qc-local]: https://www.quantconnect.com/docs/v2/local-platform/key-concepts/features
[qc-data]: https://www.quantconnect.com/docs/v2/local-platform/datasets/getting-started
[qlib]: https://github.com/microsoft/qlib
[qlib-recorder]: https://qlib.readthedocs.io/en/latest/component/recorder.html
[qlib-record-source]: https://github.com/microsoft/qlib/blob/main/qlib/workflow/record_temp.py
[qlib-data]: https://qlib.readthedocs.io/en/latest/component/data.html
[qlib-license]: https://github.com/microsoft/qlib/blob/main/LICENSE
[rd]: https://github.com/microsoft/RD-Agent
[rd-paper]: https://arxiv.org/abs/2505.15155v2
[vnpy]: https://github.com/vnpy/vnpy
[vnpy-license]: https://github.com/vnpy/vnpy/blob/master/LICENSE
[vectorbt]: https://vectorbt.dev/
[vectorbt-portfolio]: https://vectorbt.dev/api/portfolio/base/
[vbt-license]: https://github.com/polakowo/vectorbt/blob/master/LICENSE.md
[backtrader]: https://www.backtrader.com/docu/cerebro/
[backtrader-license]: https://github.com/mementum/backtrader/blob/master/LICENSE
[tradingview]: https://www.tradingview.com/pine-script-docs/concepts/strategies/
[ta]: https://github.com/TauricResearch/TradingAgents/blob/1394a3f72aa4393e1a98f51b382434c4b4c2d972/README.md
[ta-backtest]: https://github.com/TauricResearch/TradingAgents/blob/1394a3f72aa4393e1a98f51b382434c4b4c2d972/tradingagents/backtest.py
[ta-commit]: https://github.com/TauricResearch/TradingAgents/commit/1394a3f72aa4393e1a98f51b382434c4b4c2d972
[ta-paper]: https://arxiv.org/abs/2412.20138v7
[cn]: https://github.com/hsliuping/TradingAgents-CN
[cn-license]: https://github.com/hsliuping/TradingAgents-CN/blob/main/LICENSE
[cn-app-license]: https://github.com/hsliuping/TradingAgents-CN/blob/main/app/LICENSE
[daily]: https://github.com/ZhuLinsen/daily_stock_analysis
[daily-license]: https://github.com/ZhuLinsen/daily_stock_analysis/blob/main/LICENSE
[openbb-odp]: https://docs.openbb.co/odp
[openbb-data]: https://docs.openbb.co/workspace/developers/data-integration
[openbb-agent]: https://docs.openbb.co/workspace/developers/agents-integration
[openbb-transition]: https://openbb.co/blog/openbb-belongs-to-everyone/
[finos]: https://github.com/finos/community/issues/445
[tushare-access]: https://tushare.pro/document/2?doc_id=290
[tushare-adjust]: https://tushare.pro/document/2?doc_id=146
[akshare]: https://github.com/akfamily/akshare
[sse-rules]: https://www.sse.com.cn/lawandrules/sselawsrules2025/trade/universal/c/c_20260424_10816492.shtml
[codex-sdk]: https://learn.chatgpt.com/docs/codex-sdk
[codex-exec]: https://learn.chatgpt.com/docs/non-interactive-mode
[claude-sdk]: https://code.claude.com/docs/en/agent-sdk/overview
[tdx]: https://help.tdx.com.cn/gspt/%E9%80%9A%E8%BE%BE%E4%BF%A1%E5%85%AC%E5%BC%8F%E7%B3%BB%E7%BB%9F.pdf
[iwencai]: https://quant.10jqka.com.cn/view/help/4
