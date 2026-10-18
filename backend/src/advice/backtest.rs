//! 规则回测：假设过去 3 年每天都按一组规则操作，账户总值怎么变。参考值（保守型、稳健型、进取型）的回测展示在高级设置里，
//! 账户也可以用自己的规则现算一次。和每日建议用同一套选股优先级和风控规则：
//! - 候选：参考的策略当天的买点（和巡检相同的买卖点规则）；
//! - 有效策略逐年滚动判断：回测中的每一年，只用这一年之前 3 年的数据（买点之后 20 个交易日、且这 20 天在这一年之前结束）
//!   比较策略和随便买，不用未来的数据挑策略；
//! - 排序：同时选中的有效策略数 → 它们的超额之和 → 成交额，至少 min_picks 个；
//! - 卖出：止损、趋势均线、买入依据策略的卖出条件；最多持几只、单只上限、保留现金、弱市、追高上限。
//!
//! 近似：价格用前复权价（分红送转已在价格里），允许零股，佣金和印花税照扣；ST 按现在的简称判断，持有中被 ST 不卖；
//! 不做单只超过上限时的减仓。规则见 docs/domains/accounts.md#rules_backtest。

use std::collections::{BTreeSet, HashMap};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::account::lots::{Fees, STAMP_TAX_RATE};
use crate::account::rules::{Preset, Rules, WeakMarket};
use crate::analysis::indicators::{Frame, IndicatorId, IndicatorRef};
use crate::analysis::market::{self, calendar_days_for};
use crate::analysis::points;
use crate::analysis::prices;
use crate::analysis::strategy::{Definition, SignalLib, Universe};

/// 判断有效策略往前看几年；参考值的回测区间也是这么长
pub const YEARS: i64 = 3;

/// 回测区间最长几年
pub const MAX_YEARS: i64 = 5;

/// 回测区间：开始日期为空时是结束日期那年的 1 月 1 日（今年以来），结束日期为空或晚于最新交易日时取最新交易日
pub fn resolve_range(latest: NaiveDate, start: Option<NaiveDate>, end: Option<NaiveDate>) -> Result<(NaiveDate, NaiveDate), String> {
    let end = end.map_or(latest, |e| e.min(latest));
    let start = start.unwrap_or_else(|| NaiveDate::from_ymd_opt(end.year(), 1, 1).unwrap_or(end));
    if start >= end {
        return Err("开始日期要早于结束日期（结束日期最晚是最新交易日）".into());
    }
    if end - start > chrono::Duration::days(365 * MAX_YEARS + 2) {
        return Err(format!("回测区间最长 {MAX_YEARS} 年"));
    }
    Ok((start, end))
}
/// 买点之后看多少个交易日
const HORIZON: usize = 20;
/// 一个窗口里至少有多少次买点才判断这个策略有没有效
const MIN_SAMPLES: u64 = 30;
/// 一次最多参考多少个策略（按位记录）
pub const MAX_STRATEGIES: usize = 64;
/// 本金（只影响金额，不影响收益率）
const CAPITAL: f64 = 1_000_000.0;
const FEES: Fees = Fees { commission_rate: 0.00025, min_commission: 5.0 };

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RulesBacktestSummary {
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// 区间涨跌（0.1 表示 10%）
    pub total_return: f64,
    pub annual_return: Option<f64>,
    /// 最大回撤（0.2 表示从最高点跌了 20%）
    pub max_drawdown: f64,
    pub benchmark_name: Option<String>,
    pub benchmark_return: Option<f64>,
    /// 已卖出的交易笔数
    pub trades: usize,
    pub win_rate: Option<f64>,
    /// 平均每天持有几只
    pub avg_positions: f64,
    /// 平均现金占比
    pub avg_cash_ratio: f64,
    /// 参考了几个策略，以及回测期间平均有几个算作有效
    pub strategies: usize,
    pub avg_effective: f64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RulesEquityPoint {
    pub date: NaiveDate,
    /// 账户总值相对本金的倍数（1.1 表示赚了 10%）
    pub value: f64,
    /// 基准指数相对第一天的倍数
    pub benchmark: Option<f64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RulesBacktest {
    pub summary: RulesBacktestSummary,
    /// 每周一个点
    pub equity: Vec<RulesEquityPoint>,
}

/// 一只股票在回测区间内每天的数据（前复权价）
struct Stock {
    /// 第 k 天对应全局日历的下标
    day: Vec<u32>,
    q_open: Vec<f32>,
    q_close: Vec<f32>,
    one_price: Vec<bool>,
    ma_trend: Vec<f32>,
    /// 第 k 天哪些策略出现买点，按位
    buy: Vec<u64>,
    /// 第 k 天哪些策略的卖出条件满足，按位
    exit: Vec<u64>,
    amount: Vec<f32>,
}

impl Stock {
    /// 全局第 d 天在这只股票里的下标；停牌时为空
    fn at(&self, d: u32) -> Option<usize> {
        self.day.binary_search(&d).ok()
    }
}

/// 某一年哪些策略有效（按位），以及各自的超额（%）
#[derive(Clone, Default, Debug)]
struct YearEdge {
    mask: u64,
    edge: Vec<f64>,
}

/// 按「20 天在哪一年结束」累计的买点之后涨跌，以及随便买的
#[derive(Default)]
struct Sums {
    by_year: HashMap<(usize, i32), (f64, u64)>,
    base: HashMap<i32, (f64, u64)>,
}

impl Sums {
    /// 第 year 年的有效策略（超额大于 min_edge）：只看 year 之前 3 年里结束的
    fn edges(&self, n: usize, year: i32, min_edge: f64) -> YearEdge {
        let window = (year - YEARS as i32)..year;
        let (bs, bn) = window.clone().filter_map(|y| self.base.get(&y)).fold((0.0, 0u64), |a, b| (a.0 + b.0, a.1 + b.1));
        let mut out = YearEdge { mask: 0, edge: vec![f64::NAN; n] };
        if bn == 0 {
            return out;
        }
        let base = bs / bn as f64;
        for k in 0..n {
            let (s, c) = window.clone().filter_map(|y| self.by_year.get(&(k, y))).fold((0.0, 0u64), |a, b| (a.0 + b.0, a.1 + b.1));
            if c >= MIN_SAMPLES {
                let e = s / c as f64 - base;
                out.edge[k] = e;
                if e > min_edge {
                    out.mask |= 1 << k;
                }
            }
        }
        out
    }
}

fn add(m: &mut (f64, u64), r: f64) {
    *m = (m.0 + r, m.1 + 1);
}

// @project-doc docs/domains/accounts.md#rules_backtest
/// 回测 [start, end]（用 resolve_range 定好的区间）
pub async fn run(pool: &PgPool, rules: &Rules, all: &[(i64, Definition)], start: NaiveDate, end: NaiveDate) -> anyhow::Result<Option<RulesBacktest>> {
    let defs: Vec<&Definition> = all.iter().filter(|(id, _)| rules.uses(*id)).map(|(_, d)| d).take(MAX_STRATEGIES).collect();
    if defs.is_empty() {
        return Ok(None);
    }
    let edge_from = start - chrono::Duration::days(365 * YEARS);
    let sec = market::securities(pool).await?;
    let base_def = Definition { universe: Universe::default(), all: vec![], any: vec![], exit: Default::default(), sort: Default::default(), limit: 50, lib: SignalLib::default() };
    let warmup = defs.iter().map(|d| d.warmup()).max().unwrap_or(30).max(rules.trend_ma) + 5;
    let mut raw: Vec<(Vec<NaiveDate>, Stock)> = vec![];
    let mut sums = Sums::default();
    // 每天：收盘站上 20 日均线的股票数、当天有日线的股票数
    let mut breadth: HashMap<NaiveDate, (u32, u32)> = HashMap::new();
    prices::for_each_stock(pool, edge_from - chrono::Duration::days(calendar_days_for(warmup)), end, |code, bars| {
        let Some(meta) = sec.meta.get(code) else { return };
        let Some(first_edge) = bars.iter().position(|b| b.date >= edge_from) else { return };
        let f = Frame::new(bars, Some(meta));
        let after = |s: usize| bars.get(s + HORIZON).map(|l| (l.date.year(), (l.q_close / bars[s].q_close - 1.0) * 100.0)).filter(|(_, r)| r.is_finite());
        for i in first_edge..bars.len() {
            if base_def.in_universe(&f, meta, i)
                && let Some((y, r)) = after(i)
            {
                add(sums.base.entry(y).or_default(), r);
            }
        }
        let first = bars.iter().position(|b| b.date >= start);
        let n = first.map_or(0, |f0| bars.len() - f0);
        let mut buy = vec![0u64; n];
        for (k, d) in defs.iter().enumerate() {
            for t in points::run(bars, &f, meta, d, first_edge).trades {
                let Ok(s) = bars.binary_search_by_key(&t.signal_date, |b| b.date) else { continue };
                if let Some((y, r)) = after(s) {
                    add(sums.by_year.entry((k, y)).or_default(), r);
                }
                if let Some(f0) = first
                    && s >= f0
                    && !meta.is_st
                {
                    buy[s - f0] |= 1 << k;
                }
            }
        }
        let Some(f0) = first else { return };
        let ma20 = f.get(IndicatorRef::new(IndicatorId::Ma, Some(20)));
        let ma = f.get(IndicatorRef::new(IndicatorId::Ma, Some(rules.trend_ma)));
        let exits: Vec<Vec<bool>> = defs.iter().map(|d| d.exit_series(&f)).collect();
        let mut s = Stock {
            day: vec![],
            q_open: Vec::with_capacity(n),
            q_close: Vec::with_capacity(n),
            one_price: Vec::with_capacity(n),
            ma_trend: Vec::with_capacity(n),
            buy,
            exit: Vec::with_capacity(n),
            amount: Vec::with_capacity(n),
        };
        let mut dates = Vec::with_capacity(n);
        for i in f0..bars.len() {
            let b = &bars[i];
            let e = breadth.entry(b.date).or_default();
            e.1 += 1;
            e.0 += (!ma20[i].is_nan() && b.q_close > ma20[i]) as u32;
            let mut em = 0u64;
            for (k, x) in exits.iter().enumerate() {
                if x[i] {
                    em |= 1 << k;
                }
            }
            dates.push(b.date);
            s.q_open.push(b.q_open as f32);
            s.q_close.push(b.q_close as f32);
            s.one_price.push(b.high == b.low);
            s.ma_trend.push(ma[i] as f32);
            s.exit.push(em);
            s.amount.push(b.amount as f32);
        }
        raw.push((dates, s));
    })
    .await?;
    let calendar: Vec<NaiveDate> = raw.iter().flat_map(|(d, _)| d.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect();
    if calendar.len() < 20 {
        return Ok(None);
    }
    let index: HashMap<NaiveDate, u32> = calendar.iter().enumerate().map(|(i, d)| (*d, i as u32)).collect();
    let stocks: Vec<Stock> = raw
        .into_iter()
        .map(|(dates, mut s)| {
            s.day = dates.iter().map(|d| index[d]).collect();
            s
        })
        .collect();
    let mut signals: Vec<Vec<(usize, usize)>> = vec![vec![]; calendar.len()];
    for (si, s) in stocks.iter().enumerate() {
        for (k, &m) in s.buy.iter().enumerate() {
            if m != 0 {
                signals[s.day[k] as usize].push((si, k));
            }
        }
    }
    let edges: HashMap<i32, YearEdge> = (calendar[0].year()..=calendar[calendar.len() - 1].year()).map(|y| (y, sums.edges(defs.len(), y, rules.min_edge_pct))).collect();
    let bench = crate::analysis::portfolio::benchmark(pool, calendar[0], *calendar.last().expect("日历不为空")).await?;
    let mut r = simulate(rules, &stocks, &signals, &calendar, &breadth, &edges, bench);
    r.summary.strategies = defs.len();
    Ok(Some(r))
}

struct Held {
    stock: usize,
    units: f64,
    cost: f64,
    entry_q: f64,
    /// 买入依据的策略（位）
    origin: u64,
    last_close: f64,
}

fn simulate(
    p: &Rules,
    stocks: &[Stock],
    signals: &[Vec<(usize, usize)>],
    calendar: &[NaiveDate],
    breadth: &HashMap<NaiveDate, (u32, u32)>,
    edges: &HashMap<i32, YearEdge>,
    bench: Option<(String, HashMap<NaiveDate, f64>)>,
) -> RulesBacktest {
    let mut cash = CAPITAL;
    let mut held: Vec<Held> = vec![];
    let mut pending_sell: Vec<usize> = vec![];
    // (股票, 预算, 依据的策略位, 信号日收盘价)
    let mut pending_buy: Vec<(usize, f64, u64, f64)> = vec![];
    let (mut peak, mut max_dd) = (CAPITAL, 0.0f64);
    let (mut closed, mut wins) = (0usize, 0usize);
    let (mut pos_sum, mut cash_ratio_sum, mut eff_sum) = (0.0, 0.0, 0.0);
    let mut equity: Vec<RulesEquityPoint> = vec![];
    let mut last_week = None;
    let none = YearEdge::default();
    for (d, date) in calendar.iter().enumerate() {
        let d32 = d as u32;
        let year = edges.get(&date.year()).unwrap_or(&none);
        eff_sum += year.mask.count_ones() as f64;
        // 开盘：卖出（一字跌停或停牌顺延），再买入（停牌、一字涨停、高开超过追高上限时放弃）
        let mut still = vec![];
        for &si in &pending_sell {
            let s = &stocks[si];
            let Some(k) = s.at(d32) else {
                still.push(si);
                continue;
            };
            if s.one_price[k] && k > 0 && s.q_open[k] < s.q_close[k - 1] {
                still.push(si);
                continue;
            }
            if let Some(pos) = held.iter().position(|h| h.stock == si) {
                let h = held.swap_remove(pos);
                let gross = h.units * s.q_open[k] as f64;
                let net = gross - FEES.commission(gross) - gross * STAMP_TAX_RATE;
                cash += net;
                closed += 1;
                wins += (net > h.cost) as usize;
            }
        }
        pending_sell = still;
        for (si, budget, origin, ref_close) in pending_buy.drain(..) {
            let s = &stocks[si];
            let Some(k) = s.at(d32) else { continue };
            let open = s.q_open[k] as f64;
            if (s.one_price[k] && open > ref_close) || open / ref_close - 1.0 > p.chase_limit_pct / 100.0 {
                continue;
            }
            let invest = budget.min(cash);
            let fee = FEES.commission(invest);
            if invest <= fee * 2.0 {
                continue;
            }
            cash -= invest;
            held.push(Held { stock: si, units: (invest - fee) / open, cost: invest, entry_q: open, origin, last_close: open });
        }

        // 收盘：市值
        for h in held.iter_mut() {
            if let Some(k) = stocks[h.stock].at(d32) {
                h.last_close = stocks[h.stock].q_close[k] as f64;
            }
        }
        let value = cash + held.iter().map(|h| h.units * h.last_close).sum::<f64>();
        peak = peak.max(value);
        max_dd = max_dd.max(1.0 - value / peak);
        pos_sum += held.len() as f64;
        cash_ratio_sum += cash / value.max(1.0);
        let week = date.format("%G-%V").to_string();
        if last_week.as_ref() != Some(&week) || d + 1 == calendar.len() {
            last_week = Some(week);
            equity.push(RulesEquityPoint { date: *date, value: value / CAPITAL, benchmark: bench.as_ref().and_then(|(_, b)| b.get(date).copied()) });
        }

        // 收盘后：决定明天卖什么
        for h in &held {
            if pending_sell.contains(&h.stock) {
                continue;
            }
            let s = &stocks[h.stock];
            let Some(k) = s.at(d32) else { continue };
            let close = s.q_close[k] as f64;
            let ma = s.ma_trend[k] as f64;
            let stop = close / h.entry_q - 1.0 <= -p.stop_loss_pct / 100.0;
            let trend = !ma.is_nan() && close < ma;
            if stop || trend || s.exit[k] & h.origin != 0 {
                pending_sell.push(h.stock);
            }
        }
        // 再决定明天买什么
        let (above, total) = breadth.get(date).copied().unwrap_or((0, 0));
        let weak = total > 0 && (above as f64 / total as f64 * 100.0) < p.weak_threshold_pct;
        if weak && p.weak_market == WeakMarket::Pause {
            continue;
        }
        let mut slots = p.max_positions as i64 - held.len().saturating_sub(pending_sell.len()) as i64;
        let proceeds: f64 = held.iter().filter(|h| pending_sell.contains(&h.stock)).map(|h| h.units * h.last_close).sum();
        let mut available = cash + proceeds - value * p.min_cash_pct / 100.0;
        if weak && p.weak_market == WeakMarket::Half {
            available *= 0.5;
        }
        let edge_sum = |m: u64| (0..64usize).filter(|k| m & (1 << k) != 0).map(|k| year.edge.get(k).copied().filter(|e| e.is_finite()).unwrap_or(0.0)).sum::<f64>();
        let mut list: Vec<(usize, usize, u64)> = signals[d]
            .iter()
            .map(|&(si, k)| (si, k, stocks[si].buy[k] & year.mask))
            .filter(|(si, _, m)| m.count_ones() >= p.min_picks && !held.iter().any(|h| h.stock == *si))
            .collect();
        list.sort_by(|a, b| {
            b.2.count_ones()
                .cmp(&a.2.count_ones())
                .then_with(|| edge_sum(b.2).total_cmp(&edge_sum(a.2)))
                .then_with(|| stocks[b.0].amount[b.1].total_cmp(&stocks[a.0].amount[a.1]))
        });
        for (si, k, m) in list {
            if slots <= 0 || available <= 0.0 {
                break;
            }
            let target = (available / slots as f64).min(value * p.max_weight_pct / 100.0);
            pending_buy.push((si, target, m.isolate_lowest_one(), stocks[si].q_close[k] as f64));
            available -= target;
            slots -= 1;
        }
    }
    let n = calendar.len();
    let last = equity.last().map_or(1.0, |e| e.value);
    let days = (calendar[n - 1] - calendar[0]).num_days();
    RulesBacktest {
        summary: RulesBacktestSummary {
            start: calendar[0],
            end: calendar[n - 1],
            total_return: last - 1.0,
            annual_return: (days >= 90).then(|| last.powf(365.25 / days as f64) - 1.0),
            max_drawdown: max_dd,
            benchmark_name: bench.as_ref().map(|(name, _)| name.clone()),
            benchmark_return: equity.iter().rev().find_map(|e| e.benchmark).map(|b| b - 1.0),
            trades: closed,
            win_rate: (closed > 0).then(|| wins as f64 / closed as f64),
            avg_positions: pos_sum / n as f64,
            avg_cash_ratio: cash_ratio_sum / n as f64,
            strategies: 0,
            avg_effective: eff_sum / n as f64,
        },
        equity,
    }
}

/// 三组参考值的回测：每周和策略表现一起重算，存在 `app.preset_stat`
pub async fn refresh_presets(pool: &PgPool) -> anyhow::Result<()> {
    let defs = crate::analysis::screener::screening_strategies(pool).await?;
    for preset in Preset::ALL {
        let rules = preset.rules();
        let started = std::time::Instant::now();
        let Some(latest) = market::resolve_date(pool, None).await? else { return Ok(()) };
        let start = latest.checked_sub_months(chrono::Months::new(12 * YEARS as u32)).unwrap_or(latest);
        let Some(r) = run(pool, &rules, &defs, start, latest).await? else { continue };
        sqlx::query(
            "INSERT INTO app.preset_stat (preset, rules_hash, end_date, result) VALUES ($1, $2, $3, $4)
             ON CONFLICT (preset) DO UPDATE SET rules_hash = EXCLUDED.rules_hash, end_date = EXCLUDED.end_date, result = EXCLUDED.result, computed_at = now()",
        )
        .bind(preset.key())
        .bind(super::stats::rules_hash(&rules))
        .bind(r.summary.end)
        .bind(serde_json::to_value(&r)?)
        .execute(pool)
        .await?;
        tracing::info!(preset = preset.key(), total_return = r.summary.total_return, max_drawdown = r.summary.max_drawdown, ms = started.elapsed().as_millis() as u64, "参考值回测完成");
    }
    Ok(())
}

pub async fn load_presets(pool: &PgPool) -> anyhow::Result<HashMap<String, RulesBacktest>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as("SELECT preset, result FROM app.preset_stat").fetch_all(pool).await?;
    rows.into_iter().map(|(k, v)| Ok((k, serde_json::from_value(v)?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两只股票、10 天：A 只被策略 1 选中、一路上涨；B 被两个策略选中、之后跌破止损。开盘价等于前一天收盘价
    fn stocks() -> (Vec<Stock>, Vec<NaiveDate>) {
        let cal: Vec<NaiveDate> = (0..10).map(|i| NaiveDate::from_ymd_opt(2026, 9, 1).unwrap() + chrono::Duration::days(i)).collect();
        let mk = |closes: Vec<f32>, buy_days: &[usize], bits: u64| Stock {
            day: (0..10).collect(),
            q_open: (0..10).map(|i| closes[i.max(1) - 1]).collect(),
            q_close: closes.clone(),
            one_price: vec![false; 10],
            ma_trend: vec![f32::NAN; 10],
            buy: (0..10).map(|i| if buy_days.contains(&i) { bits } else { 0 }).collect(),
            exit: vec![0; 10],
            amount: vec![1e9; 10],
        };
        let a = mk((0..10).map(|i| 10.0 + i as f32).collect(), &[0, 1, 2, 3], 0b01);
        let b = mk(vec![10.0, 10.0, 10.0, 9.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0], &[1], 0b11);
        (vec![a, b], cal)
    }

    fn signals(stocks: &[Stock]) -> Vec<Vec<(usize, usize)>> {
        let mut out: Vec<Vec<(usize, usize)>> = vec![vec![]; 10];
        for (si, s) in stocks.iter().enumerate() {
            for (k, m) in s.buy.iter().enumerate() {
                if *m != 0 {
                    out[k].push((si, k));
                }
            }
        }
        out
    }

    #[test]
    fn only_effective_strategies_count_and_stops_out() {
        let (stocks, cal) = stocks();
        let breadth: HashMap<NaiveDate, (u32, u32)> = cal.iter().map(|d| (*d, (60, 100))).collect();
        let p = Preset::Aggressive.rules();
        // 两个策略都有效：A 第 1 天买入一路涨；B 第 2 天买入，第 3 天收盘亏 10% 触发止损，第 4 天卖出
        let both: HashMap<i32, YearEdge> = HashMap::from([(2026, YearEdge { mask: 0b11, edge: vec![1.0, 2.0] })]);
        let r = simulate(&p, &stocks, &signals(&stocks), &cal, &breadth, &both, None);
        assert_eq!(r.summary.trades, 1, "{:?}", r.summary);
        assert_eq!(r.summary.win_rate, Some(0.0));
        assert!(r.summary.total_return > 0.0, "{:?}", r.summary);
        assert!(r.equity.len() >= 2 && r.equity[0].date == cal[0]);
        // 只有策略 2 有效：A 只被策略 1 选中，不买；只买 B
        let second: HashMap<i32, YearEdge> = HashMap::from([(2026, YearEdge { mask: 0b10, edge: vec![-1.0, 2.0] })]);
        let r = simulate(&p, &stocks, &signals(&stocks), &cal, &breadth, &second, None);
        assert!(r.summary.total_return < 0.0, "{:?}", r.summary);
        // 要求至少 2 个有效策略：只剩 B（两个策略都选中）
        let strict = Rules { min_picks: 2, ..p };
        let r = simulate(&strict, &stocks, &signals(&stocks), &cal, &breadth, &both, None);
        assert_eq!(r.summary.trades, 1);
        assert!(r.summary.total_return < 0.0);
    }

    #[test]
    fn ranges_default_to_this_year_and_stop_at_latest_day() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        let latest = d(2026, 10, 16);
        assert_eq!(resolve_range(latest, None, None), Ok((d(2026, 1, 1), latest)), "默认今年以来");
        assert_eq!(resolve_range(latest, Some(d(2026, 7, 16)), Some(d(2026, 12, 31))), Ok((d(2026, 7, 16), latest)), "结束日期不晚于最新交易日");
        assert!(resolve_range(latest, Some(d(2026, 10, 16)), None).is_err());
        assert!(resolve_range(latest, Some(d(2020, 1, 1)), None).is_err(), "最长 5 年");
    }

    #[test]
    fn edges_only_use_earlier_years() {
        let mut s = Sums::default();
        for y in [2021, 2022, 2023] {
            s.base.insert(y, (100.0, 100));
            s.by_year.insert((0, y), (300.0, 100));
            s.by_year.insert((1, y), (50.0, 100));
        }
        // 2024 年里的表现不影响 2024 年的判断
        s.by_year.insert((1, 2024), (1e9, 100));
        let e = s.edges(2, 2024, 0.0);
        assert_eq!(e.mask, 0b01);
        assert_eq!(s.edges(2, 2024, 2.5).mask, 0, "门槛 2.5% 时策略 1 的 2% 不够");
        assert!((e.edge[0] - 2.0).abs() < 1e-9 && (e.edge[1] + 0.5).abs() < 1e-9);
        assert_eq!(s.edges(2, 2021, 0.0).mask, 0, "之前没有数据时没有有效策略");
    }
}
