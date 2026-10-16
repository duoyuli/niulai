//! 全市场组合回测：用一个选股策略在一段时间里每天选股，最多同时持有 max_positions 只，每只等额买入。
//! 规则与近似见 docs/domains/backtest.md#portfolio。
//!
//! 做法：先逐只股票找出所有「信号日」，并按卖出规则算出每个信号对应的一笔候选交易（何时买、何时卖）；
//! 再按日期模拟账户：每天开盘先卖出到期的持仓，再用空出来的仓位按排序买入当天的候选。

use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::backtest::{Costs, ExitReason, payoff};
use super::indicators::Frame;
use super::market::{self, calendar_days_for};
use super::prices::{self, DailyBar};
use super::strategy::Definition;

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
    signal_date: NaiveDate,
    entry_date: NaiveDate,
    entry_q: f64,
    entry_raw: f64,
    /// (卖出日, 前复权开盘价, 未复权开盘价, 原因)；期末仍持有时为空
    exit: Option<(NaiveDate, f64, f64, ExitReason)>,
    bars_held: i64,
    sort_value: f64,
}

struct Stock {
    code: String,
    /// 区间内每天的前复权收盘价，用于每天计算持仓市值
    closes: Vec<(NaiveDate, f64)>,
}

/// 从信号日 i 起模拟一笔交易：i + 1 开盘买入，按卖出规则在之后某天开盘卖出。开盘一字涨停买不进时返回 None
fn simulate(bars: &[DailyBar], f: &Frame, d: &Definition, i: usize, last: usize) -> Option<(usize, Option<(usize, ExitReason)>)> {
    let j = i + 1;
    if j > last {
        return None;
    }
    let one_price = |k: usize| bars[k].high == bars[k].low;
    if one_price(j) && bars[j].q_open > bars[i].q_close {
        return None;
    }
    let entry_q = bars[j].q_open;
    let r = &d.exit;
    let mut pending: Option<ExitReason> = None;
    for k in j..=last {
        if let Some(reason) = pending {
            // 开盘一字跌停卖不出，顺延
            if !(one_price(k) && bars[k].q_open < bars[k - 1].q_close) {
                return Some((j, Some((k, reason))));
            }
        }
        if pending.is_none() {
            let ret = bars[k].q_close / entry_q - 1.0;
            pending = if d.exit_signal(f, k) {
                Some(ExitReason::Signal)
            } else if r.stop_loss_pct.is_some_and(|s| ret <= -s / 100.0) {
                Some(ExitReason::StopLoss)
            } else if r.take_profit_pct.is_some_and(|t| ret >= t / 100.0) {
                Some(ExitReason::TakeProfit)
            } else if r.max_hold_days.is_some_and(|h| k + 1 - j >= h as usize) {
                Some(ExitReason::MaxHold)
            } else {
                None
            };
        }
    }
    Some((j, None))
}

// @project-doc docs/domains/backtest.md#portfolio
pub async fn run(pool: &PgPool, d: &Definition, p: &Params) -> anyhow::Result<Option<PortfolioOutcome>> {
    let sec = market::securities(pool).await?;
    let warmup = d.warmup() + 5;
    let mut stocks: Vec<Stock> = vec![];
    let mut cands: Vec<Candidate> = vec![];
    let mut calendar: BTreeSet<NaiveDate> = BTreeSet::new();
    prices::for_each_stock(pool, p.start - chrono::Duration::days(calendar_days_for(warmup)), p.end, |code, bars| {
        let Some(meta) = sec.meta.get(code) else { return };
        let Some(first) = bars.iter().position(|b| b.date >= p.start) else { return };
        for b in &bars[first..] {
            calendar.insert(b.date);
        }
        let f = Frame::new(bars, Some(meta));
        let last = bars.len() - 1;
        let stock = stocks.len();
        let mut any = false;
        for i in first..last {
            if !d.in_universe(&f, meta, i) || !d.matches(&f, i) {
                continue;
            }
            let Some((j, exit)) = simulate(bars, &f, d, i, last) else { continue };
            any = true;
            cands.push(Candidate {
                stock,
                signal_date: bars[i].date,
                entry_date: bars[j].date,
                entry_q: bars[j].q_open,
                entry_raw: bars[j].open,
                exit: exit.map(|(k, r)| (bars[k].date, bars[k].q_open, bars[k].open, r)),
                bars_held: exit.map_or(last, |(k, _)| k) as i64 - j as i64,
                sort_value: f.at(d.sort.by, i),
            });
        }
        if any {
            stocks.push(Stock { code: code.to_string(), closes: bars[first..].iter().map(|b| (b.date, b.q_close)).collect() });
        }
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
            let o = if d.sort.desc { y.total_cmp(&x) } else { x.total_cmp(&y) };
            if x.is_nan() || y.is_nan() { x.is_nan().cmp(&y.is_nan()) } else { o }
        });
    }

    let bench = benchmark(pool, p.start, p.end).await?;
    struct Held {
        cand: usize,
        units: f64,
        cost: f64,
        cursor: usize,
        last_close: f64,
    }
    let mut cash = p.capital;
    let mut held: Vec<Held> = vec![];
    let mut trades: Vec<PortfolioTrade> = vec![];
    let mut equity: Vec<PortfolioPoint> = vec![];
    let (mut fees, mut skipped, mut peak, mut max_dd, mut pos_days) = (0.0, 0usize, f64::MIN, 0.0f64, 0usize);
    let name = |c: &Candidate| sec.names.get(&stocks[c.stock].code).cloned().unwrap_or_default();
    let trade = |c: &Candidate, ret: f64, closed: bool| PortfolioTrade {
        thscode: stocks[c.stock].code.clone(),
        name: name(c),
        signal_date: c.signal_date,
        entry_date: c.entry_date,
        entry_price: c.entry_raw,
        exit_date: if closed { c.exit.map(|e| e.0) } else { None },
        exit_price: if closed { c.exit.map(|e| e.2) } else { None },
        exit_reason: if closed { c.exit.map(|e| e.3) } else { None },
        return_pct: ret,
        bars_held: c.bars_held,
    };
    for &day in &calendar {
        // 开盘：先卖出到期的持仓
        let mut k = 0;
        while k < held.len() {
            let c = &cands[held[k].cand];
            if let Some((date, q, _, _)) = c.exit
                && date == day
            {
                let gross = held[k].units * q;
                let fee = p.costs.commission(gross) + gross * p.costs.stamp_tax_rate;
                fees += fee;
                cash += gross - fee;
                trades.push(trade(c, (gross - fee) / held[k].cost - 1.0, true));
                held.swap_remove(k);
            } else {
                k += 1;
            }
        }
        // 再买入当天的候选：空余仓位平分现金
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
                let invest = cash / free as f64;
                let fee = p.costs.commission(invest);
                if invest <= fee {
                    skipped += 1;
                    continue;
                }
                fees += fee;
                cash -= invest;
                held.push(Held { cand: ci, units: (invest - fee) / c.entry_q, cost: invest, cursor: 0, last_close: c.entry_q });
            }
        }
        // 收盘：按前复权收盘价计算市值；停牌的股票用最近一个收盘价
        let mut value = cash;
        for h in held.iter_mut() {
            let closes = &stocks[cands[h.cand].stock].closes;
            while h.cursor < closes.len() && closes[h.cursor].0 <= day {
                h.last_close = closes[h.cursor].1;
                h.cursor += 1;
            }
            value += h.units * h.last_close;
        }
        peak = peak.max(value);
        max_dd = max_dd.max(1.0 - value / peak);
        pos_days += held.len();
        let b = bench.as_ref().and_then(|(_, series)| series.get(&day).map(|r| r * p.capital));
        equity.push(PortfolioPoint { date: day, equity: value, positions: held.len(), benchmark: b });
    }
    // 期末仍持有的按最后收盘价计算，不扣卖出费用
    for h in &held {
        let c = &cands[h.cand];
        trades.push(trade(c, h.units * h.last_close / h.cost - 1.0, false));
    }

    let (start, end) = (*calendar.first().unwrap(), *calendar.last().unwrap());
    let final_equity = equity.last().map_or(p.capital, |e| e.equity);
    let total_return = final_equity / p.capital - 1.0;
    let closed: Vec<&PortfolioTrade> = trades.iter().filter(|t| t.exit_date.is_some()).collect();
    let wins = closed.iter().filter(|t| t.return_pct > 0.0).count();
    let days = (end - start).num_days();
    let annual = (days >= 90).then(|| (1.0 + total_return).powf(365.25 / days as f64) - 1.0);
    let benchmark_return = equity.iter().rev().find_map(|e| e.benchmark).map(|v| v / p.capital - 1.0);
    let summary = PortfolioSummary {
        start,
        end,
        days: equity.len(),
        capital: p.capital,
        final_equity,
        total_return,
        annual_return: annual,
        max_drawdown: max_dd,
        benchmark_name: bench.as_ref().map(|(n, _)| n.clone()),
        benchmark_return,
        closed_trades: closed.len(),
        wins,
        win_rate: (!closed.is_empty()).then(|| wins as f64 / closed.len() as f64),
        payoff_ratio: payoff(&closed.iter().map(|t| t.return_pct).collect::<Vec<_>>()),
        avg_bars_held: (!closed.is_empty()).then(|| closed.iter().map(|t| t.bars_held as f64).sum::<f64>() / closed.len() as f64),
        avg_positions: pos_days as f64 / equity.len() as f64,
        fees,
        signals: cands.len(),
        skipped_full: skipped,
    };
    trades.sort_by_key(|t| std::cmp::Reverse(t.entry_date));
    trades.truncate(MAX_TRADES_RETURNED);
    Ok(Some(PortfolioOutcome { summary, equity, trades }))
}

/// 基准：沪深 300，没有时用上证指数。返回名称和「每天收盘 ÷ 区间第一天收盘」
async fn benchmark(pool: &PgPool, start: NaiveDate, end: NaiveDate) -> sqlx::Result<Option<(String, HashMap<NaiveDate, f64>)>> {
    for (code, name) in [("000300.SH", "沪深 300"), ("000001.SH", "上证指数")] {
        let rows: Vec<(NaiveDate, f64)> = sqlx::query_as(
            "SELECT trade_date, close::float8 FROM mkt.index_bar_1d WHERE thscode = $1 AND trade_date >= $2 AND trade_date <= $3 ORDER BY trade_date",
        )
        .bind(code)
        .bind(start)
        .bind(end)
        .fetch_all(pool)
        .await?;
        // 指数日线从 2022 年起才有：开头缺太多时不用
        if let Some(first) = rows.first()
            && (first.0 - start).num_days() <= 10
        {
            let base = first.1;
            return Ok(Some((name.to_string(), rows.into_iter().map(|(d, c)| (d, c / base)).collect())));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::tests::bars_from;
    use crate::analysis::strategy::{Condition, ExitRule};

    #[test]
    fn simulate_exits_on_max_hold() {
        let b = bars_from(&[10.0; 10]);
        let f = Frame::new(&b, None);
        let mut d: Definition = serde_json::from_value(serde_json::json!({"all": [{"kind": "compare", "left": {"id": "close"}, "op": ">", "right": {"value": 0}}]})).unwrap();
        d.exit = ExitRule { any: Vec::<Condition>::new(), stop_loss_pct: None, take_profit_pct: None, max_hold_days: Some(3) };
        let (j, exit) = simulate(&b, &f, &d, 2, 9).unwrap();
        assert_eq!(j, 3);
        // 下标 3 买入，持有 3 天（3、4、5），下标 5 收盘触发，下标 6 开盘卖出
        assert_eq!(exit.map(|e| e.0), Some(6));
    }
}
