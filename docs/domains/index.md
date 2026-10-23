# 领域文档索引

> 上级索引：[工程文档](../index.md)

## 范围

本分类记录 A 股行情数据的业务规则：除权除息与复权、时间戳与交易日、证券上市退市、快照精度、涨跌停判断与市场环境、权威涨跌停池与热点页面，横截面因子研究，以及按因子排名的账户与调仓建议。同步框架本身见 [架构](../architecture/index.md)，供应商接口的限制见 [接口](../interfaces/index.md)。

## 文档

- [复权](price_adjustment.md)：除权除息和复权的概念、等比复权因子的计算、同花顺减法复权、公司行动的来源与去重规则、复权视图与 API 参数。阅读时机：涉及价格、复权因子、`mkt.corp_action`、`*_qfq` / `*_hfq` 视图，或需要向非金融背景的读者解释复权。
- [行情数据规则](market_data_rules.md)：时间戳换算、交易日、上市与退市推断、ST、停牌、快照与历史接口的精度差异、涨跌停判断、市场概况与板块强弱、数据覆盖范围。阅读时机：处理供应商时间戳、请求窗口、交易日、`ref.security`、写入当天日线，或修改 `analysis/market.rs`。
- [今日热点](hotspots.md)：权威涨停、跌停、炸板池的数据来源和口径，`GET /market/hot` 接口，首页「今日热点」卡片和 `/hot` 页面的结构，以及数据滞后时的空态。阅读时机：修改 `analysis/hotspots.rs`、`api/market.rs` 的热点接口、`TodayPage.tsx` 或 `HotPage.tsx`，或解释页面上的涨停、跌停、炸板、连板数字。
- [因子研究](factor_research.md)：12 个横截面因子的口径和预期方向、股票池、财务数据的可用日、因子检验（RankIC、分组收益、多空、换手、衰减）、挑选偏差（PBO、Deflated Sharpe）和多因子组合回测的交易规则。阅读时机：修改 `factors.rs`、`factor_lab.rs`、`universe.rs`、`costs.rs`、`research.rs` 的统计函数、`research_run.rs`、`api/research.rs` 或 `/research` 页面，或解释因子检验和多因子组合的数字。
- [账户与调仓建议](accounts.md)：账户、持仓、流水和分红送转自动记账、撤销流水与已实现盈亏、配股提醒与认购、每日净值曲线，交易规则与费用，调仓规则，账户的因子排名，每日调仓建议的生成规则和理由，建议回看。阅读时机：修改 `backend/src/account/`、`backend/src/advice/`、`api/accounts.rs` 或账户页面，或解释某条建议为什么出现、为什么没出现。
