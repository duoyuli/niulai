# 账户的「买入前回看」设计

> 2026-10-18 设计方案，交接材料，不属于 Project Doc 文档库。实现后把定稿写进 `docs/domains/accounts.md` 和 `docs/decisions/account_advice.md`。

## 一、背景和结论

工作区里有一版没提交的改动，在策略定义里加了「历史表现过滤」（`Definition.performance_filter`），并新增内置策略「均线交叉（15 日 / 30 日，五周期为正）」：一只股票按基础策略回测，过去 3 个月到 5 年都赚钱，才算买点。

用户决定：策略本身就只是策略，只管「什么时候买、什么时候卖」；历史表现的检查改为账户里的一个维度，由账户决定「这次买不买这只股票」。策略层的这套改动全部拆掉。

## 二、和「不设有效门槛」的区别

之前去掉的有效门槛，是拿某个策略在全市场的平均表现，决定这个策略能不能参与选股（见交接第十节）。

这次是按「一只股票 + 一个策略」检查：同一个策略，在这只股票上过去一直赚钱才买，一直亏就不买。这是用户在账户里主动打开的、规则明确的买入条件，不是对策略的评分，也不在设置页罗列各策略的历史数字让人挑。

## 三、设计

### 1. 账户规则里的开关

设置页新增「买入前回看」，默认说明：「只买勾选的策略在这只股票上，过去所选周期按它的买卖点操作都赚钱的股票」。

周期可多选：3 个月、6 个月、1 年、3 年、5 年，至少选一个；收益门槛默认「大于 0」，可以调高。

`Rules` 新增可选字段 `history_check`（`#[serde(default)]`），旧数据读出来就是不开：

```rust
/// 买入前回看：勾选的策略在这只股票上过去各周期都赚钱才买；None 表示不检查
#[serde(default)]
pub history_check: Option<HistoryCheck>,

pub struct HistoryCheck {
    /// 全部满足才算通过
    pub windows: Vec<HistoryWindow>,
    /// 总收益要大于这个值（百分比），0 表示赚钱就行
    #[serde(default)]
    pub min_return_pct: f64,
}
```

### 2. 只管买入，按「股票 + 策略」判断

某只股票被策略 A、B 同时看好时，分别回看 A 在这只股票上、B 在这只股票上的表现；通过回看的策略才计入「至少几个勾选的策略同时看好」（`min_picks`），两者自然配合。

已经持有的股票照常按策略的卖出条件和止损卖出，不受回看影响。

上市时间或日线不够最长周期的股票不买；设置页写明「上市不满 N 年的股票不会买入」，免得用户以为策略坏了。

### 3. 计算位置和开销

在 `advice/inputs.rs` 的 `shared()` 里，对当天所有被勾选策略命中的（股票，策略），一次算出 5 个周期的收益，所有账户共用；每个账户再按自己的周期和门槛筛选。

只加载候选股票的长历史，不扫全市场 5 年数据，巡检不受影响。

回测复用单股回测的引擎（`analysis::backtest::run`），只用当天及以前的数据，不会用到未来数据。费用用默认费率，所有账户共用同一份结果；如果要按账户费率算，就按费率分组缓存。

先按回看筛选，再截取前 `MAX_CANDIDATES`（40）个候选，避免通过回看的股票被截掉。

`analysis/performance.rs` 里现有的按周期回测逻辑可以保留下来，改成输入（股票日线，策略），输出各周期收益。

### 4. 建议理由

理由第一行只写结论：「通过买入前回看：『均线交叉』在这只股票上过去 1 年、3 年都赚钱」；各周期的具体收益放在展开的明细里，不放在最显眼的位置。

没有通过回看的股票不进建议；可以在建议底部提示「有 N 只股票因买入前回看没有通过而不买」。

### 5. 三组参考值

推荐：稳健型默认打开（1 年 + 3 年，大于 0），均衡型、积极型默认关闭。三组的区别是风控参数、`min_picks` 和这个开关。

### 6. 账户规则回测

「用自己的规则回测」也要应用回看：每个模拟交易日只用那一天以前的数据回看，否则回测和实际建议口径不一致。这一步计算量最大，先实现再实测耗时；太慢时按（股票，策略，月份）缓存回看结果。

## 四、要拆掉的策略层改动

这些都在工作区，没有提交，线上 `v2026.10.18.13` 没有，直接删除，不需要迁移：

- `backend/src/analysis/strategy.rs`：`Definition.performance_filter`、`PerformanceFilter`、`StrategyRef`、`PerformanceWindow`，以及内置策略 `ma_cross_history_filter` 和它的测试。
- `backend/src/analysis/screener.rs`：`resolve_performance_filters`、两处 `performance::passes`、为过滤加长的 `history_days`、`corp_events_for_window` 的调用。
- `backend/src/api/strategy.rs`：`Explained.performance`。
- `frontend/src/pages/StrategyEditorPage.tsx` 的过滤设置，和 `frontend/src/api/generated/model/` 下 `performanceFilter.ts`、`performanceWindow.ts`、`strategyRef.ts`（重新 `pnpm api:spec && pnpm api:gen`）。
- 相关文档里对「历史表现过滤」的描述。

拆掉后，之前复核里的「被引用策略删除后巡检失败」「过滤导致全市场读 5 年日线」「各页面口径不一致」三条问题随之消失。

## 五、和本方案无关、仍要修的

- `backend/src/api/strategy.rs:176`：保存策略顺序时不要更新 `updated_at`。
- `backend/src/sync/run.rs:160`：规划中断只看开始了 10 分钟，慢的首次大回补规划会被误判；改为规划过程写心跳，按心跳超时判断。
- `deploy/.env.example:25`：推送已移除，删掉 `NIULAI_PUBLIC_URL`。
- 迁移 `20261019000016` 可以合并进还没发布的 `20261019000015`（可选）。

## 六、待用户确认

1. 稳健型参考值是否默认打开回看（推荐打开）。
2. 建议理由的明细里是否显示各周期的具体收益（推荐显示在展开的明细里，第一行只写结论）。
