//! 风格回测：假设过去 3 年每天都按一个风格的规则操作（参考它的策略买入，按止损、趋势均线、买入依据的卖出条件卖出，
//! 最多持几只、单只上限、保留现金、弱市处理、追高上限），账户总值怎么变。风格卡片上展示它，
//! 让用户知道选这个风格大概意味着什么。和每日建议用同一组规则，但有几处近似：
//! - 价格用前复权价，现金分红和送转已算在价格里；不按一手取整（允许零股），佣金和印花税照扣；
//! - 「只采纳过去表现好于随便买的策略」不生效：那需要当时之前的统计，回测里用全部参考策略；
//! - ST 按现在的简称判断，持有中被 ST 不卖出；不做单只超过上限时的减仓。
//!
//! 规则见 docs/domains/accounts.md#style_backtest。

use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::account::lots::{Fees, STAMP_TAX_RATE};
use crate::account::style::{Style, StyleParams, WeakMarket};
use crate::analysis::indicators::{Frame, IndicatorId, IndicatorRef};
use crate::analysis::market::{self, calendar_days_for};
use crate::analysis::prices;
use crate::analysis::strategy::Definition;

/// 回测几年
pub const YEARS: i64 = 3;
/// 本金（只影响金额，不影响收益率）
const CAPITAL: f64 = 1_000_000.0;
const FEES: Fees = Fees { commission_rate: 0.00025, min_commission: 5.0 };

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct StyleBacktestSummary {
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
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct StyleEquityPoint {
    pub date: NaiveDate,
    /// 账户总值相对本金的倍数（1.1 表示赚了 10%）
    pub value: f64,
    /// 基准指数相对第一天的倍数
    pub benchmark: Option<f64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct StyleBacktest {
    pub summary: StyleBacktestSummary,
    /// 每周一个点
    pub equity: Vec<StyleEquityPoint>,
}

/// 一只股票在回测区间内每天的数据（前复权价）
struct Stock {
    /// 第 k 天对应全局日历的下标
    day: Vec<u32>,
    q_open: Vec<f32>,
    q_close: Vec<f32>,
    one_price: Vec<bool>,
    ma_trend: Vec<f32>,
    /// 第 k 天哪些策略出现买点（含股票池），按位
    buy: Vec<u16>,
    /// 第 k 天哪些策略的卖出条件满足，按位
    exit: Vec<u16>,
    amount: Vec<f32>,
}

impl Stock {
    /// 全局第 d 天在这只股票里的下标；停牌时为空
    fn at(&self, d: u32) -> Option<usize> {
        self.day.binary_search(&d).ok()
    }
}

// @project-doc docs/domains/accounts.md#style_backtest
pub async fn run(pool: &PgPool, params: &StyleParams, defs: &[Definition]) -> anyhow::Result<Option<StyleBacktest>> {
    if defs.is_empty() || defs.len() > 16 {
        return Ok(None);
    }
    let Some(end) = market::resolve_date(pool, None).await? else { return Ok(None) };
    let start = end - chrono::Duration::days(365 * YEARS);
    let sec = market::securities(pool).await?;
    let warmup = defs.iter().map(|d| d.warmup()).max().unwrap_or(30).max(params.trend_ma) + 5;
    let mut raw: Vec<(Vec<NaiveDate>, Stock)> = vec![];
    // 每天：收盘站上 20 日均线的股票数、当天有日线的股票数
    let mut breadth: HashMap<NaiveDate, (u32, u32)> = HashMap::new();
    prices::for_each_stock(pool, start - chrono::Duration::days(calendar_days_for(warmup)), end, |code, bars| {
        let Some(meta) = sec.meta.get(code) else { return };
        let Some(first) = bars.iter().position(|b| b.date >= start) else { return };
        let f = Frame::new(bars, Some(meta));
        let ma20 = f.get(IndicatorRef::new(IndicatorId::Ma, Some(20)));
        let ma = f.get(IndicatorRef::new(IndicatorId::Ma, Some(params.trend_ma)));
        let buys: Vec<Vec<bool>> = defs.iter().map(|d| d.buy_series(&f)).collect();
        let exits: Vec<Vec<bool>> = defs.iter().map(|d| d.exit_series(&f)).collect();
        let n = bars.len() - first;
        let mut s = Stock {
            day: vec![],
            q_open: Vec::with_capacity(n),
            q_close: Vec::with_capacity(n),
            one_price: Vec::with_capacity(n),
            ma_trend: Vec::with_capacity(n),
            buy: Vec::with_capacity(n),
            exit: Vec::with_capacity(n),
            amount: Vec::with_capacity(n),
        };
        let mut dates = Vec::with_capacity(n);
        for i in first..bars.len() {
            let b = &bars[i];
            let e = breadth.entry(b.date).or_default();
            e.1 += 1;
            e.0 += (!ma20[i].is_nan() && b.q_close > ma20[i]) as u32;
            let mut bm = 0u16;
            let mut em = 0u16;
            for k in 0..defs.len() {
                if buys[k][i] && !meta.is_st && defs[k].in_universe(&f, meta, i) {
                    bm |= 1 << k;
                }
                if exits[k][i] {
                    em |= 1 << k;
                }
            }
            dates.push(b.date);
            s.q_open.push(b.q_open as f32);
            s.q_close.push(b.q_close as f32);
            s.one_price.push(b.high == b.low);
            s.ma_trend.push(ma[i] as f32);
            s.buy.push(bm);
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
    // 每天有买点的股票
    let mut signals: Vec<Vec<(usize, usize)>> = vec![vec![]; calendar.len()];
    for (si, s) in stocks.iter().enumerate() {
        for (k, &m) in s.buy.iter().enumerate() {
            if m != 0 {
                signals[s.day[k] as usize].push((si, k));
            }
        }
    }
    let bench = crate::analysis::portfolio::benchmark(pool, calendar[0], *calendar.last().expect("日历不为空")).await?;
    Ok(Some(simulate(params, &stocks, &signals, &calendar, &breadth, bench)))
}

struct Held {
    stock: usize,
    units: f64,
    cost: f64,
    entry_q: f64,
    /// 买入依据的策略（位）
    origin: u16,
    last_close: f64,
}

fn simulate(
    p: &StyleParams,
    stocks: &[Stock],
    signals: &[Vec<(usize, usize)>],
    calendar: &[NaiveDate],
    breadth: &HashMap<NaiveDate, (u32, u32)>,
    bench: Option<(String, HashMap<NaiveDate, f64>)>,
) -> StyleBacktest {
    let mut cash = CAPITAL;
    let mut held: Vec<Held> = vec![];
    let mut pending_sell: Vec<usize> = vec![];
    // (股票, 预算, 依据的策略位, 信号日收盘价)
    let mut pending_buy: Vec<(usize, f64, u16, f64)> = vec![];
    let (mut peak, mut max_dd) = (CAPITAL, 0.0f64);
    let (mut closed, mut wins) = (0usize, 0usize);
    let (mut pos_sum, mut cash_ratio_sum) = (0.0, 0.0);
    let mut equity: Vec<StyleEquityPoint> = vec![];
    let mut last_week = None;
    for (d, date) in calendar.iter().enumerate() {
        let d32 = d as u32;
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
            equity.push(StyleEquityPoint { date: *date, value: value / CAPITAL, benchmark: bench.as_ref().and_then(|(_, b)| b.get(date).copied()) });
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
        let mut list: Vec<&(usize, usize)> = signals[d].iter().filter(|(si, _)| !held.iter().any(|h| h.stock == *si)).collect();
        list.sort_by(|a, b| {
            let (sa, sb) = (&stocks[a.0], &stocks[b.0]);
            sb.buy[b.1].count_ones().cmp(&sa.buy[a.1].count_ones()).then_with(|| sb.amount[b.1].total_cmp(&sa.amount[a.1]))
        });
        for &&(si, k) in &list {
            if slots <= 0 || available <= 0.0 {
                break;
            }
            let mut target = (available / slots as f64).min(value * p.max_weight_pct / 100.0);
            if weak && p.weak_market == WeakMarket::Half {
                target *= 0.5;
            }
            let m = stocks[si].buy[k];
            pending_buy.push((si, target, m.isolate_lowest_one(), stocks[si].q_close[k] as f64));
            available -= target;
            slots -= 1;
        }
    }
    let n = calendar.len();
    let last = equity.last().map_or(1.0, |e| e.value);
    let days = (calendar[n - 1] - calendar[0]).num_days();
    StyleBacktest {
        summary: StyleBacktestSummary {
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
        },
        equity,
    }
}

/// 三种风格的回测：每周和策略表现一起重算，存在 `app.style_stat`
pub async fn refresh_presets(pool: &PgPool) -> anyhow::Result<()> {
    let ids = crate::analysis::screener::builtin_ids(pool).await?;
    let defs: HashMap<i64, Definition> = super::stats::builtin_definitions(pool).await?.into_iter().collect();
    for style in Style::ALL {
        let params = style.params();
        let started = std::time::Instant::now();
        let list: Vec<Definition> = params.strategies.iter().filter_map(|k| ids.get(k).and_then(|id| defs.get(id)).cloned()).collect();
        let Some(r) = run(pool, &params, &list).await? else { continue };
        let hash = super::stats::params_hash(&params);
        sqlx::query(
            "INSERT INTO app.style_stat (style, params_hash, end_date, result) VALUES ($1, $2, $3, $4)
             ON CONFLICT (style) DO UPDATE SET params_hash = EXCLUDED.params_hash, end_date = EXCLUDED.end_date, result = EXCLUDED.result, computed_at = now()",
        )
        .bind(style.key())
        .bind(&hash)
        .bind(r.summary.end)
        .bind(serde_json::to_value(&r)?)
        .execute(pool)
        .await?;
        tracing::info!(style = style.key(), total_return = r.summary.total_return, max_drawdown = r.summary.max_drawdown, ms = started.elapsed().as_millis() as u64, "风格回测完成");
    }
    Ok(())
}

pub async fn load_presets(pool: &PgPool) -> anyhow::Result<HashMap<String, StyleBacktest>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as("SELECT style, result FROM app.style_stat").fetch_all(pool).await?;
    rows.into_iter().map(|(k, v)| Ok((k, serde_json::from_value(v)?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两只股票、10 天：A 每天有买点、一路上涨；B 第 2 天有买点、之后跌破止损
    fn stocks() -> (Vec<Stock>, Vec<NaiveDate>) {
        let cal: Vec<NaiveDate> = (0..10).map(|i| NaiveDate::from_ymd_opt(2026, 9, 1).unwrap() + chrono::Duration::days(i)).collect();
        // 开盘价等于前一天收盘价
        let mk = |closes: Vec<f32>, buy_days: &[usize]| Stock {
            day: (0..10).collect(),
            q_open: (0..10).map(|i| closes[i.max(1) - 1]).collect(),
            q_close: closes.clone(),
            one_price: vec![false; 10],
            ma_trend: vec![f32::NAN; 10],
            buy: (0..10).map(|i| if buy_days.contains(&i) { 1 } else { 0 }).collect(),
            exit: vec![0; 10],
            amount: vec![1e9; 10],
        };
        let a = mk((0..10).map(|i| 10.0 + i as f32).collect(), &[0, 1, 2, 3]);
        let b = mk(vec![10.0, 10.0, 10.0, 9.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0], &[1]);
        (vec![a, b], cal)
    }

    #[test]
    fn buys_next_open_and_stops_out() {
        let (stocks, cal) = stocks();
        let mut signals: Vec<Vec<(usize, usize)>> = vec![vec![]; 10];
        for (si, s) in stocks.iter().enumerate() {
            for (k, m) in s.buy.iter().enumerate() {
                if *m != 0 {
                    signals[k].push((si, k));
                }
            }
        }
        let breadth: HashMap<NaiveDate, (u32, u32)> = cal.iter().map(|d| (*d, (60, 100))).collect();
        let p = Style::Aggressive.params();
        let r = simulate(&p, &stocks, &signals, &cal, &breadth, None);
        // A 第 1 天开盘买入一路涨；B 第 2 天买入，第 4 天收盘亏 20% 触发止损（10%），第 5 天卖出
        assert_eq!(r.summary.trades, 1, "{:?}", r.summary);
        assert_eq!(r.summary.win_rate, Some(0.0));
        assert!(r.summary.total_return > 0.0, "{:?}", r.summary);
        assert!(r.equity.len() >= 2 && r.equity[0].date == cal[0]);
    }
}
