# 均线与交叉信号

本文说明均线的计算口径、上穿和下穿的判定规则，以及自选股诊断的范围。价格口径的背景见 [复权](price_adjustment.md)，接口参数见 [REST API](../interfaces/rest_api.md#endpoints)。计算在 `backend/src/analysis/ma.rs`。[交叉判定](#cross_rule) 是全项目通用的：条件里所有「上穿」「下穿」都用它。自选股诊断直接用 `ma.rs` 计算；内置的「均线交叉」策略和 [回测](backtest.md) 里的均线交叉由条件「`ma(fast)` 上穿 `ma(slow)`」表达，判定规则相同；K 线上的均线是叠加的指标，见 [选股策略与诊断](strategies.md)。为什么不在 SQL 中计算，见 [ADR：指标在 Rust 中计算](../decisions/indicator_compute.md)。

<a id="ma_definition"></a>
## 均线口径

- 价格：等比前复权收盘价（`mkt.v_stock_bar_qfq.close`）。等比前复权和等比后复权只差一个常数倍，信号结果相同，见 [等比复权因子](price_adjustment.md#equal_ratio_factor)。
- 均线：简单移动平均。N 日均线是当天及之前共 N 根 K 线收盘价的平均值；停牌日没有 K 线，不计入。按滑动窗口加补偿求和计算，与逐个相加求平均的相对差小于 1e-13。
- 周期：快线 `fast` 默认 15，慢线 `slow` 默认 30。两者都必须在 2 到 250 之间，且 `fast < slow`。
- 只输出 K 线数量达到 `slow` 根之后的点；上市初期不足 `slow` 根时，没有均线和信号。
- 指标接口（`GET /securities/{thscode}/indicator`）指定 `start` 时，均线仍然用 `start` 之前的历史计算，只是不返回 `start` 之前的点；指定 `end` 时，只用 `end` 及之前的数据。

<a id="cross_rule"></a>
## 交叉判定

每天按快线和慢线的相对差确定一个状态：

```
相对差 = (快线 − 慢线) / |慢线|
相对差 >  1e-9   → 状态 +1（快线在上）
相对差 < −1e-9   → 状态 −1（快线在下）
其余              → 状态 0（视为相等）
```

信号规则：
- **上穿**（`golden`）：状态从上一个非零状态 −1 变为 +1。
- **下穿**（`death`）：状态从上一个非零状态 +1 变为 −1。
- 状态为 0 的日子不改变「上一个非零状态」，也不产生信号。例如 −1、0、+1 的序列，在 +1 那天产生上穿。

需要容差的原因：快线和慢线完全相等时（例如连续一字涨停、长时间价格不变），浮点误差会让相对差在 ±1e-16 附近来回跳，没有容差就会产生假交叉。全市场实测中，不加容差时等比前复权和等比后复权有 1,823 次交叉日期不同，全部来自这种情况；加容差后差异为 0。

<a id="watchlist"></a>
## 自选股诊断

`GET /api/v1/watchlist` 对每只收藏的证券给出快速诊断，实现在 `backend/src/api/watchlist.rs`，均线用 `analysis::ma`。自选按 `ref.instrument` 支持股票和 ETF：股票读 `mkt.stock_bar_1d`（等比前复权），ETF 读 `mkt.etf_bar_1d`（供应商前复权）。
- 每只证券取最近 500 根前复权日线（约两年），在这个范围内计算均线和状态，判定规则与单只证券相同。最近一次交叉只在这个范围内找，更早的交叉不返回。
- `change_1d`、`change_20d` 用前复权收盘价计算，除权除息日不会出现假涨跌。
- 还没有日线时 `diagnosis` 为空，不足 `slow` 根时 `diagnosis.ma` 为空。
- `recent_signals` 是最近 5 个交易日出现过的事件类信号，规则与 [个股诊断](strategies.md#diagnosis) 的最近信号相同。
