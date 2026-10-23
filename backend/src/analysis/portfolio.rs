//! 全市场组合回测：用一个选股策略在一段时间里每天选股，最多同时持有 max_positions 只，每只等额买入。
//! 规则与近似见 docs/domains/backtest.md#portfolio。
//!
//! 做法：先按每日巡检和 K 线买卖点同一套规则（`points::run`）逐只股票找出全部候选交易（何时买、何时卖）；
//! 再按日期模拟账户：每天开盘先卖出到期的持仓，再用空出来的仓位按排序买入当天的候选。
//! 资金、仓位、费用和「前复权价近似分红再投资」是这一路径和单股回测不同的地方，口径见
//! docs/domains/backtest.md#calibers。

use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::backtest::{Costs, ExitReason, payoff};
use super::indicators::{Frame, StockMeta};
use super::market::{self, calendar_days_for};
use super::points;
use super::prices::{self, DailyBar};
use super::strategy::Definition;
use crate::instrument::{AssetType, extension};

/// 回测区间上限：几年的全市场日线逐只计算，区间再长耗时和内存都会明显增加
pub const MAX_YEARS: i64 = 5;
/// 响应中最多返回多少笔交易（最近的）
const MAX_TRADES_RETURNED: usize = 500;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub capital: f64,
    pub costs: Costs,
    pub max_positions: usize,
    pub risk: PortfolioRisk,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, Default)]
pub struct PortfolioRisk {
    /// 单票买入金额最多占期初本金的百分比；为空表示不额外限制。
    pub max_weight_pct: Option<f64>,
    /// 目标年化波动率；为空表示等额分配。
    pub target_vol_pct: Option<f64>,
    /// 单日成交金额占期初本金的上限；为空表示不限制。
    pub max_turnover_pct: Option<f64>,
}

struct ExitInfo {
    date: NaiveDate,
    reason: ExitReason,
}

#[derive(Serialize, ToSchema, Debug)]
pub struct PortfolioTrade {
    pub thscode: String,
    pub name: String,
    pub signal_date: NaiveDate,
    pub entry_date: NaiveDate,
    /// 买入价（未复权开盘价）
    pub entry_price: f64,
    /// 期末仍持有时为空
    pub exit_date: Option<NaiveDate>,
    pub exit_price: Option<f64>,
    pub exit_reason: Option<ExitReason>,
    /// 扣费后的收益率（按前复权价计算，含分红送转）
    pub return_pct: f64,
    pub bars_held: i64,
}

#[derive(Serialize, ToSchema, Debug)]
pub struct PortfolioPoint {
    pub date: NaiveDate,
    pub equity: f64,
    /// 收盘时持有的股票数
    pub positions: usize,
    /// 基准指数按同样本金折算的值；没有指数日线时为空
    pub benchmark: Option<f64>,
}

#[derive(Serialize, ToSchema, Debug)]
pub struct PortfolioSummary {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub days: usize,
    pub capital: f64,
    pub final_equity: f64,
    pub total_return: f64,
    pub annual_return: Option<f64>,
    pub max_drawdown: f64,
    pub risk: crate::analysis::research::RiskMetrics,
    /// 基准指数的名称和区间涨跌；没有指数日线时为空
    pub benchmark_name: Option<String>,
    pub benchmark_return: Option<f64>,
    pub closed_trades: usize,
    pub wins: usize,
    pub win_rate: Option<f64>,
    pub payoff_ratio: Option<f64>,
    pub avg_bars_held: Option<f64>,
    /// 平均每天持有几只
    pub avg_positions: f64,
    pub fees: f64,
    /// 买卖成交金额合计 / 初始资金。
    pub turnover: f64,
    /// 区间内出现的买入信号总数
    pub signals: usize,
    /// 因为仓位已满而没有买入的信号数
    pub skipped_full: usize,
}

#[derive(Serialize, ToSchema, Debug)]
pub struct PortfolioOutcome {
    pub summary: PortfolioSummary,
    pub equity: Vec<PortfolioPoint>,
    /// 最近的 500 笔，新的在前
    pub trades: Vec<PortfolioTrade>,
}

/// 一个信号对应的候选交易
struct Candidate {
    stock: usize,
    asset_type: AssetType,
    signal_date: NaiveDate,
    entry_date: NaiveDate,
    /// 前复权开盘价，实际成交价
    entry_q: f64,
    /// 未复权开盘价，只用于交易记录展示
    entry_raw: f64,
    entry_q_high: f64,
    entry_q_low: f64,
    entry_q_close: f64,
    entry_volume: f64,
    exit: Option<ExitInfo>,
    bars_held: i64,
    sort_value: f64,
    volatility: f64,
}

struct Stock {
    code: String,
    /// 区间内的日线；收盘价用于每天计算持仓市值，完整日线用于分批成交在后续交易日继续执行
    bars: Vec<DailyBar>,
}

/// 把一根未复权日线换成只用前复权价参与成交的执行用日线。
fn q_bar(b: &DailyBar) -> DailyBar {
    DailyBar {
        date: b.date,
        open: b.q_open,
        high: b.q_high,
        low: b.q_low,
        close: b.q_close,
        q_open: b.q_open,
        q_high: b.q_high,
        q_low: b.q_low,
        q_close: b.q_close,
        volume: b.volume,
        amount: b.amount,
    }
}

/// 按策略的买卖点规则（`points::run`，与每日巡检、K 线买卖点、单股回测同一套）从第 from 根日线
/// （空仓起步）起找出全部候选交易。资金、仓位、费用和分红处理不在这里，留给账户循环。
fn candidates(
    bars: &[DailyBar],
    f: &Frame,
    meta: &StockMeta,
    d: &Definition,
    from: usize,
    stock: usize,
) -> Vec<Candidate> {
    points::run(bars, f, meta, d, from)
        .trades
        .into_iter()
        .filter_map(|t| {
            let si = bars.binary_search_by_key(&t.signal_date, |b| b.date).ok()?;
            let entry_bar = &bars[bars.binary_search_by_key(&t.entry_date, |b| b.date).ok()?];
            let entry_q = entry_bar.q_open;
            let exit = match (t.exit_date, t.exit_reason) {
                (Some(date), Some(reason)) => Some(ExitInfo { date, reason }),
                _ => None,
            };
            // 用信号日以前的 20 个日收益率计算年化波动率；直接用价格水平会把上涨趋势误当成高波动
            let mut closes: Vec<f64> = bars
                .iter()
                .take(si)
                .rev()
                .take(21)
                .map(|b| b.q_close)
                .collect();
            closes.reverse();
            let volatility = if closes.len() >= 3 {
                let returns: Vec<f64> = closes
                    .windows(2)
                    .filter(|w| w[0] > 0.0)
                    .map(|w| w[1] / w[0] - 1.0)
                    .filter(|r| r.is_finite())
                    .collect();
                if returns.len() >= 2 {
                    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
                    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>()
                        / (returns.len() - 1) as f64;
                    variance.sqrt() * 252.0_f64.sqrt()
                } else {
                    0.0
                }
            } else {
                0.0
            };
            Some(Candidate {
                stock,
                asset_type: meta.asset_type,
                signal_date: t.signal_date,
                entry_date: t.entry_date,
                entry_q,
                entry_raw: t.entry_price,
                entry_q_high: entry_bar.q_high,
                entry_q_low: entry_bar.q_low,
                entry_q_close: entry_bar.q_close,
                entry_volume: entry_bar.volume,
                exit,
                bars_held: t.bars_held,
                sort_value: f.at(d.sort.by, si),
                volatility,
            })
        })
        .collect()
}

// @project-doc docs/domains/backtest.md#portfolio
pub async fn run(
    pool: &PgPool,
    d: &Definition,
    p: &Params,
) -> anyhow::Result<Option<PortfolioOutcome>> {
    let sec = market::securities(pool).await?;
    let warmup = d.warmup() + 5;
    let lo = p.start - chrono::Duration::days(calendar_days_for(warmup));
    // 权威涨跌停池：候选买卖点里的涨跌停、炸板、连板信号同样以供应商的名单为准
    let limits = super::limit_pool::LimitPool::load(pool, lo, p.end).await?;
    let mut stocks: Vec<Stock> = vec![];
    let mut cands: Vec<Candidate> = vec![];
    let mut calendar: BTreeSet<NaiveDate> = BTreeSet::new();
    let mut ew = EqualWeight::default();
    let include_stock = d.universe.asset_types.iter().any(|a| a == "stock");
    let include_etf = d.universe.asset_types.iter().any(|a| a == "etf");
    let costs = p.costs;
    let any_asset = Definition::any_assets(d.universe.asset_types.clone());
    prices::for_each_assets(pool, lo, p.end, include_stock, include_etf, |code, bars| {
        let Some(meta) = sec.meta.get(code) else {
            return;
        };
        let Some(first) = bars.iter().position(|b| b.date >= p.start) else {
            return;
        };
        for b in &bars[first..] {
            calendar.insert(b.date);
        }
        let history = sec.st_history.get(code).map(Vec::as_slice).unwrap_or(&[]);
        let f = Frame::with_authority_and_st_history(bars, Some(meta), &limits, code, history);
        ew.add(&any_asset, bars, &f, meta, first);
        let found = candidates(bars, &f, meta, d, first, stocks.len());
        if found.is_empty() {
            return;
        }
        cands.extend(found);
        stocks.push(Stock {
            code: code.to_string(),
            bars: bars[first..].to_vec(),
        });
    })
    .await?;
    if calendar.is_empty() {
        return Ok(None);
    }

    // 同一天的候选按策略的排序规则排好
    let mut by_entry: HashMap<NaiveDate, Vec<usize>> = HashMap::new();
    for (k, c) in cands.iter().enumerate() {
        by_entry.entry(c.entry_date).or_default().push(k);
    }
    for list in by_entry.values_mut() {
        list.sort_by(|a, b| {
            let (x, y) = (cands[*a].sort_value, cands[*b].sort_value);
            let o = if d.sort.desc {
                y.total_cmp(&x)
            } else {
                x.total_cmp(&y)
            };
            if x.is_nan() || y.is_nan() {
                x.is_nan().cmp(&y.is_nan())
            } else {
                o
            }
        });
    }

    let bench = ew.series(p.start, p.end);
    struct Held {
        cand: usize,
        units: f64,
        cost: f64,
        /// 分批卖出时已经收到的净回款
        proceeds: f64,
        cursor: usize,
        last_close: f64,
    }
    let mut cash = p.capital;
    let mut held: Vec<Held> = vec![];
    let mut trades: Vec<PortfolioTrade> = vec![];
    let mut equity: Vec<PortfolioPoint> = vec![];
    let (mut fees, mut skipped, mut peak, mut max_dd, mut pos_days, mut turnover) =
        (0.0, 0usize, f64::MIN, 0.0f64, 0usize, 0.0f64);
    let name = |c: &Candidate| {
        sec.names
            .get(&stocks[c.stock].code)
            .cloned()
            .unwrap_or_default()
    };
    let trade = |c: &Candidate, ret: f64, exit: Option<(NaiveDate, f64)>| PortfolioTrade {
        thscode: stocks[c.stock].code.clone(),
        name: name(c),
        signal_date: c.signal_date,
        entry_date: c.entry_date,
        entry_price: c.entry_raw,
        exit_date: exit.map(|e| e.0),
        exit_price: exit.map(|e| e.1),
        exit_reason: exit.and_then(|_| c.exit.as_ref().map(|e| e.reason)),
        return_pct: ret,
        bars_held: c.bars_held,
    };
    for &day in &calendar {
        // 当天买卖成交金额合计，用来执行单日换手上限；不按整个区间累计
        let mut day_turnover = 0.0;
        // 开盘：先卖出到期的持仓。成交量参与率导致只成交一部分时，后续交易日继续卖
        let mut k = 0;
        while k < held.len() {
            let c = &cands[held[k].cand];
            if !c.exit.as_ref().is_some_and(|e| e.date <= day) {
                k += 1;
                continue;
            }
            let Some(bar) = stocks[c.stock]
                .bars
                .binary_search_by_key(&day, |b| b.date)
                .ok()
                .map(|pos| &stocks[c.stock].bars[pos])
            else {
                // 停牌：没有日线，下一个交易日再试
                k += 1;
                continue;
            };
            let raw_open = bar.open;
            let bar = q_bar(bar);
            let exec = costs.execution();
            let Some(fill) = exec.fill(
                &bar,
                super::backtest::Side::Sell,
                held[k].units,
                &costs,
                extension(c.asset_type).stamp_tax_rate(),
            ) else {
                k += 1;
                continue;
            };
            if fill.filled <= 0.0 {
                k += 1;
                continue;
            }
            let gross = fill.filled * fill.price;
            day_turnover += gross;
            turnover += gross;
            let stamp_tax_rate = if extension(c.asset_type).stamp_tax_rate() == 0.0 {
                0.0
            } else {
                costs.stamp_tax_rate
            };
            let fee = fill.commission + fill.transfer_fee + gross * stamp_tax_rate;
            fees += fee;
            cash += gross - fee;
            held[k].proceeds += gross - fee;
            if fill.filled >= held[k].units - 1e-9 {
                let ret = held[k].proceeds / held[k].cost - 1.0;
                trades.push(trade(c, ret, Some((day, raw_open))));
                held.swap_remove(k);
            } else {
                held[k].units -= fill.filled;
                k += 1;
            }
        }
        // 再买入当天的候选：空余仓位平分现金，并先留出佣金和过户费
        if let Some(list) = by_entry.get(&day) {
            for &ci in list {
                let free = p.max_positions - held.len();
                let c = &cands[ci];
                if held.iter().any(|h| cands[h.cand].stock == c.stock) {
                    continue; // 已经持有这只股票
                }
                if free == 0 {
                    skipped += 1;
                    continue;
                }
                let mut budget = cash / free as f64;
                if let Some(max_weight) = p.risk.max_weight_pct {
                    budget = budget.min(p.capital * max_weight / 100.0);
                }
                if let Some(target_vol) = p.risk.target_vol_pct
                    && c.volatility > 0.0
                {
                    budget = budget.min(cash / free as f64 * target_vol / 100.0 / c.volatility);
                }
                budget = budget.min(cash);
                let bar = DailyBar {
                    date: day,
                    open: c.entry_q,
                    high: c.entry_q_high,
                    low: c.entry_q_low,
                    close: c.entry_q_close,
                    q_open: c.entry_q,
                    q_high: c.entry_q_high,
                    q_low: c.entry_q_low,
                    q_close: c.entry_q_close,
                    volume: c.entry_volume,
                    amount: 0.0,
                };
                let exec = costs.execution();
                let Some(price) = exec.estimate_price(&bar, super::backtest::Side::Buy) else {
                    skipped += 1;
                    continue;
                };
                let affordable = costs.affordable_amount(budget);
                if affordable < price {
                    skipped += 1;
                    continue;
                }
                if let Some(max_turnover) = p.risk.max_turnover_pct
                    && day_turnover + affordable > p.capital * max_turnover / 100.0
                {
                    skipped += 1;
                    continue;
                }
                let Some(fill) = exec.fill(
                    &bar,
                    super::backtest::Side::Buy,
                    affordable / price,
                    &costs,
                    0.0,
                ) else {
                    skipped += 1;
                    continue;
                };
                let fee = fill.commission + fill.transfer_fee;
                let total_cost = fill.filled * fill.price + fee;
                if fill.filled <= 0.0 || total_cost <= fee || total_cost > cash + 1e-8 {
                    skipped += 1;
                    continue;
                }
                fees += fee;
                day_turnover += fill.filled * fill.price;
                turnover += fill.filled * fill.price;
                cash -= total_cost;
                held.push(Held {
                    cand: ci,
                    units: fill.filled,
                    cost: total_cost,
                    proceeds: 0.0,
                    cursor: 0,
                    last_close: c.entry_q,
                });
            }
        }
        // 收盘：按前复权收盘价计算市值；停牌的股票用最近一个收盘价
        let mut value = cash;
        for h in held.iter_mut() {
            let bars = &stocks[cands[h.cand].stock].bars;
            while h.cursor < bars.len() && bars[h.cursor].date <= day {
                h.last_close = bars[h.cursor].q_close;
                h.cursor += 1;
            }
            value += h.units * h.last_close;
        }
        peak = peak.max(value);
        max_dd = max_dd.max(1.0 - value / peak);
        pos_days += held.len();
        let b = bench
            .as_ref()
            .and_then(|(_, series)| series.get(&day).map(|r| r * p.capital));
        equity.push(PortfolioPoint {
            date: day,
            equity: value,
            positions: held.len(),
            benchmark: b,
        });
    }
    // 期末仍持有的按最后收盘价计算，不扣卖出费用；分批卖出过的部分计入已收到的回款
    for h in &held {
        let c = &cands[h.cand];
        trades.push(trade(
            c,
            (h.proceeds + h.units * h.last_close) / h.cost - 1.0,
            None,
        ));
    }

    let (start, end) = (*calendar.first().unwrap(), *calendar.last().unwrap());
    let final_equity = equity.last().map_or(p.capital, |e| e.equity);
    let total_return = final_equity / p.capital - 1.0;
    let closed: Vec<&PortfolioTrade> = trades.iter().filter(|t| t.exit_date.is_some()).collect();
    let wins = closed.iter().filter(|t| t.return_pct > 0.0).count();
    let days = (end - start).num_days();
    let annual = (days >= 90).then(|| (1.0 + total_return).powf(365.25 / days as f64) - 1.0);
    let risk = crate::analysis::research::risk_metrics(
        &equity.iter().map(|e| e.equity).collect::<Vec<_>>(),
        annual,
        max_dd,
    );
    let benchmark_return = equity
        .iter()
        .rev()
        .find_map(|e| e.benchmark)
        .map(|v| v / p.capital - 1.0);
    let summary = PortfolioSummary {
        start,
        end,
        days: equity.len(),
        capital: p.capital,
        final_equity,
        total_return,
        annual_return: annual,
        max_drawdown: max_dd,
        risk,
        benchmark_name: bench.as_ref().map(|(n, _)| n.clone()),
        benchmark_return,
        closed_trades: closed.len(),
        wins,
        win_rate: (!closed.is_empty()).then(|| wins as f64 / closed.len() as f64),
        payoff_ratio: payoff(&closed.iter().map(|t| t.return_pct).collect::<Vec<_>>()),
        avg_bars_held: (!closed.is_empty())
            .then(|| closed.iter().map(|t| t.bars_held as f64).sum::<f64>() / closed.len() as f64),
        avg_positions: pos_days as f64 / equity.len() as f64,
        fees,
        turnover: turnover / p.capital,
        signals: cands.len(),
        skipped_full: skipped,
    };
    trades.sort_by_key(|t| std::cmp::Reverse(t.entry_date));
    trades.truncate(MAX_TRADES_RETURNED);
    Ok(Some(PortfolioOutcome {
        summary,
        equity,
        trades,
    }))
}

/// 基准的名字
pub(crate) const EQUAL_WEIGHT_NAME: &str = "全市场等权（含分红）";

/// 基准：全市场等权指数，含分红。每天把前一天收盘时默认股票池里的全部股票等额买入（和策略统计里的「随便买」同一个股票池），
/// 按前复权收盘价算当天的平均涨跌，再连乘。前复权价含分红送转，和回测的净值同一个口径；
/// 沪深 300 这类价格指数不含分红，3 年会少算好几个百分点，回测就显得跑赢了。不依赖指数日线，任何区间都有
#[derive(Default)]
pub(crate) struct EqualWeight(HashMap<NaiveDate, (f64, u32)>);

impl EqualWeight {
    /// 一只股票从第 from 根日线起每天的涨跌（前一天收盘时在默认股票池里才算）
    pub fn add(
        &mut self,
        pool: &Definition,
        bars: &[DailyBar],
        f: &Frame,
        meta: &super::indicators::StockMeta,
        from: usize,
    ) {
        for i in from.max(1)..bars.len() {
            if !pool.in_universe(f, meta, i - 1) {
                continue;
            }
            let r = bars[i].q_close / bars[i - 1].q_close - 1.0;
            if r.is_finite() {
                let e = self.0.entry(bars[i].date).or_default();
                e.0 += r;
                e.1 += 1;
            }
        }
    }

    /// [start, end] 内的净值，第一天为 1
    pub fn series(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Option<(String, HashMap<NaiveDate, f64>)> {
        let mut days: Vec<(&NaiveDate, &(f64, u32))> = self
            .0
            .iter()
            .filter(|(d, _)| **d >= start && **d <= end)
            .collect();
        days.sort_by_key(|(d, _)| **d);
        let (first, _) = days.first()?;
        let mut level = 1.0;
        let mut out = HashMap::with_capacity(days.len());
        out.insert(**first, level);
        for (d, (sum, n)) in &days[1..] {
            level *= 1.0 + sum / f64::from(*n);
            out.insert(**d, level);
        }
        Some((EQUAL_WEIGHT_NAME.to_string(), out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::tests::bars_from;

    /// 组合回测的候选交易必须就是 `points::run` 算出的买卖点：日期、成交价、卖出原因都一致
    #[test]
    fn candidates_follow_points() {
        let closes = [
            10.0, 9.0, 8.0, 7.0, 8.0, 9.0, 10.0, 11.0, 10.0, 9.0, 8.0, 7.0,
        ];
        let b = bars_from(&closes);
        let f = Frame::new(&b, None);
        let meta = StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
            delisted_on: None,
            asset_type: crate::analysis::indicators::AssetType::Stock,
        };
        let d: Definition = serde_json::from_value(serde_json::json!({
            "universe": {"exclude_st": false},
            "all": [{"kind": "compare", "left": {"id": "close"}, "op": "crosses_above", "right": {"id": "ma", "n": 3}}],
            "exit": {"any": [{"kind": "compare", "left": {"id": "close"}, "op": "crosses_below", "right": {"id": "ma", "n": 3}}]}
        }))
        .unwrap();
        let cs = candidates(&b, &f, &meta, &d, 0, 7);
        let pts = points::run(&b, &f, &meta, &d, 0);
        assert!(!pts.trades.is_empty());
        assert_eq!(cs.len(), pts.trades.len());
        for (c, t) in cs.iter().zip(&pts.trades) {
            assert_eq!(c.stock, 7);
            assert_eq!((c.signal_date, c.entry_date), (t.signal_date, t.entry_date));
            assert_eq!(c.entry_raw, t.entry_price);
            assert_eq!(c.bars_held, t.bars_held);
            assert_eq!(
                c.exit.as_ref().map(|e| (e.date, e.reason)),
                t.exit_date.zip(t.exit_reason)
            );
        }
    }
}
