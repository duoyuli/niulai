//! 单只股票回测：只做多，买入信号出现后满仓买入，卖出信号或风控触发后全部卖出。
//! 买卖信号可以来自均线交叉，也可以来自选股策略（见 strategy::Definition）。
//!
//! 规则（详见 docs/domains/backtest.md）：
//! - 信号用等比前复权价计算；信号在收盘后才确定，所以在下一个交易日的开盘价成交，不使用当天还不知道的信息。
//! - 成交、持仓市值都用未复权价，每手 100 股；分红和送转按公司行动事件处理（ADR：采用等比复权，决策第 3 条）。
//! - 开盘即一字涨停买不进、一字跌停卖不出，订单顺延到下一个交易日，直到成交或被反向信号取消。

use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use super::prices::{CorpEvent, DailyBar};

/// 回测区间短于这么多自然日时不计算年化收益：时间太短，年化后的数字没有意义
const MIN_DAYS_FOR_ANNUAL: i64 = 90;

/// 单证券回测的申报单位。由资产扩展提供，默认值保持 A 股主板的 100 股一手。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LotRule {
    pub min: i64,
    pub step: i64,
}

impl Default for LotRule {
    fn default() -> Self {
        Self {
            min: 100,
            step: 100,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Costs {
    /// 佣金费率，买卖都收
    pub commission_rate: f64,
    /// 单笔最低佣金（元）
    pub min_commission: f64,
    /// 印花税率，只在卖出时收
    pub stamp_tax_rate: f64,
    /// 买卖双边过户费或交易所附加费，按成交金额计。
    pub transfer_fee_rate: f64,
    /// 持有现金的年化利息；回测按交易日折算。
    pub cash_interest_rate: f64,
    /// 分红税率；0 表示按税前金额入账。
    pub dividend_tax_rate: f64,
    /// 成交价口径：open / close / vwap。
    pub deal_price: crate::analysis::execution::DealPrice,
    /// 固定双边滑点，单位为基点。
    pub slippage_bps: f64,
    /// 每日最多成交量占比。
    pub participation_rate: Option<f64>,
}

impl Default for Costs {
    fn default() -> Self {
        // 常见的万 2.5 佣金、最低 5 元；印花税 2023-08-28 起为卖出 0.05%
        Costs {
            commission_rate: 0.000_25,
            min_commission: 5.0,
            stamp_tax_rate: 0.000_5,
            transfer_fee_rate: 0.0,
            cash_interest_rate: 0.0,
            dividend_tax_rate: 0.0,
            deal_price: crate::analysis::execution::DealPrice::Open,
            slippage_bps: 0.0,
            participation_rate: None,
        }
    }
}

impl Costs {
    pub(crate) fn commission(&self, amount: f64) -> f64 {
        (amount * self.commission_rate).max(self.min_commission)
    }

    pub fn execution(&self) -> crate::analysis::execution::ExecutionConfig {
        crate::analysis::execution::ExecutionConfig {
            deal_price: self.deal_price,
            slippage_bps: self.slippage_bps,
            participation_rate: self.participation_rate,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// 从这一天起允许交易；指标和信号仍然用之前的历史计算
    pub start: Option<NaiveDate>,
    pub capital: f64,
    pub costs: Costs,
    pub risk: Risk,
    pub lot: LotRule,
}

/// 风控：持仓期间按收盘价判断，触发后在下一个交易日开盘卖出。涨跌按前复权价计算，含分红送转
#[derive(Clone, Copy, Debug, Default)]
pub struct Risk {
    /// 例如 8 表示亏损 8% 时卖出
    pub stop_loss_pct: Option<f64>,
    pub take_profit_pct: Option<f64>,
    pub max_hold_days: Option<u32>,
}

/// 每天收盘后的买入信号和卖出信号（与日线等长）
pub struct Signals<'a> {
    pub entry: &'a [bool],
    pub exit: &'a [bool],
}

/// 卖出的原因
#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    /// 满足卖出条件
    Signal,
    StopLoss,
    TakeProfit,
    /// 持有满最长天数
    MaxHold,
}

/// 统一的收盘后卖出优先级：策略卖出条件、止损、止盈、最长持有期。
/// 交易执行（下一个交易日开盘、涨跌停顺延）由各回测状态机负责，原因判断只保留这一处。
pub fn exit_reason(
    signal: bool,
    return_ratio: f64,
    stop_loss_pct: Option<f64>,
    take_profit_pct: Option<f64>,
    max_hold_days: Option<u32>,
    held_days: usize,
) -> Option<ExitReason> {
    if signal {
        Some(ExitReason::Signal)
    } else if stop_loss_pct.is_some_and(|v| return_ratio <= -v / 100.0) {
        Some(ExitReason::StopLoss)
    } else if take_profit_pct.is_some_and(|v| return_ratio >= v / 100.0) {
        Some(ExitReason::TakeProfit)
    } else if max_hold_days.is_some_and(|v| held_days >= v as usize) {
        Some(ExitReason::MaxHold)
    } else {
        None
    }
}

/// A 股一字板的成交约束：买单遇到向上封板、卖单遇到向下封板时顺延。
/// 所有交易状态机共用这条执行规则。
pub fn order_blocked(is_buy: bool, one_price: bool, open: f64, previous_close: f64) -> bool {
    one_price
        && if is_buy {
            open > previous_close
        } else {
            open < previous_close
        }
}

#[derive(Serialize, ToSchema, Debug)]
pub struct Trade {
    pub entry_date: NaiveDate,
    /// 买入成交价（未复权开盘价）
    pub entry_price: f64,
    /// 买入股数
    pub shares: i64,
    /// 卖出日期；期末仍持有时为空
    pub exit_date: Option<NaiveDate>,
    /// 卖出成交价（未复权开盘价）；期末仍持有时为空
    pub exit_price: Option<f64>,
    /// 卖出时的股数，持有期间送转股会使它大于买入股数
    pub exit_shares: i64,
    /// 卖出的原因；期末仍持有时为空
    pub exit_reason: Option<ExitReason>,
    /// 买入金额加佣金
    pub cost: f64,
    /// 卖出所得（已扣佣金和印花税）；期末仍持有时是按最后收盘价计算的市值，不扣费
    pub proceeds: f64,
    /// 持有期间收到的现金分红（税前）
    pub dividends: f64,
    /// 盈亏（元）= 卖出所得 + 分红 − 成本
    pub pnl: f64,
    /// 盈亏 / 成本
    pub return_pct: f64,
    /// 持有的交易日数
    pub bars_held: i64,
}

#[derive(Serialize, ToSchema, Debug)]
pub struct EquityPoint {
    pub date: NaiveDate,
    /// 策略的账户总值（现金加持仓市值）
    pub equity: f64,
    /// 同样的本金在第一天开盘全部买入并一直持有，不计费用
    pub benchmark: f64,
    /// 当天收盘时是否持仓
    pub holding: bool,
}

#[derive(Serialize, ToSchema, Debug)]
pub struct Summary {
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// 区间内的交易日数
    pub days: usize,
    pub capital: f64,
    pub final_equity: f64,
    pub total_return: f64,
    /// 年化收益；区间不足 90 个自然日时为空
    pub annual_return: Option<f64>,
    /// 最大回撤：账户总值从最高点下跌的最大幅度（正数，0.2 表示 20%）
    pub max_drawdown: f64,
    pub benchmark_return: f64,
    pub benchmark_annual_return: Option<f64>,
    pub benchmark_max_drawdown: f64,
    pub risk: crate::analysis::research::RiskMetrics,
    /// 已经卖出的交易笔数（不含期末仍持有的那笔）
    pub closed_trades: usize,
    pub wins: usize,
    /// 赚钱的笔数 / 已卖出的笔数；没有已卖出的交易时为空
    pub win_rate: Option<f64>,
    /// 盈亏比：赚钱的交易平均赚多少 ÷ 亏钱的交易平均亏多少；大于 1 表示赚的时候比亏的时候多
    pub payoff_ratio: Option<f64>,
    pub avg_bars_held: Option<f64>,
    /// 持仓的交易日占比
    pub exposure: f64,
    /// 佣金和印花税合计（元）
    pub fees: f64,
    /// 收到的现金分红合计（元，税前）
    pub dividends: f64,
    /// 因一字涨跌停顺延的次数
    pub deferred_orders: usize,
    /// 资金不够买一手而放弃的买入信号次数
    pub skipped_buys: usize,
}

#[derive(Debug)]
pub struct Outcome {
    pub summary: Summary,
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Side {
    Buy,
    Sell,
}

struct Position {
    idx: usize,
    entry_shares: i64,
    shares: i64,
    price: f64,
    /// 买入时的前复权开盘价，判断止损止盈用
    q_price: f64,
    cost: f64,
    dividends: f64,
}

/// 买入持有基准：不计费用、可以买零股，分红留作现金、送转增加股数
struct Benchmark {
    shares: f64,
    cash: f64,
}

fn annualize(r: f64, start: NaiveDate, end: NaiveDate) -> Option<f64> {
    let days = (end - start).num_days();
    (days >= MIN_DAYS_FOR_ANNUAL).then(|| (1.0 + r).powf(365.25 / days as f64) - 1.0)
}

/// 区间内没有日线时返回 None。bars 按日期升序，包含 start 之前的历史（用于计算均线）；
/// events 是这只股票的公司行动，按除权日升序
// @project-doc docs/domains/backtest.md#rules
pub fn run(bars: &[DailyBar], events: &[CorpEvent], sig: &Signals, p: &Params) -> Option<Outcome> {
    let first = bars
        .iter()
        .position(|b| p.start.is_none_or(|s| b.date >= s))?;

    let mut cash = p.capital;
    let mut pos: Option<Position> = None;
    let mut pending: Option<Side> = None;
    let mut sell_reason = ExitReason::Signal;
    let mut trades = vec![];
    let mut equity = Vec::with_capacity(bars.len() - first);
    let (mut fees, mut dividends, mut deferred, mut skipped, mut held_days) =
        (0.0, 0.0, 0usize, 0usize, 0usize);
    let mut bench = Benchmark {
        shares: p.capital / bars[first].open,
        cash: 0.0,
    };
    let (mut peak, mut max_dd, mut bench_peak, mut bench_dd) = (f64::MIN, 0.0f64, f64::MIN, 0.0f64);
    // 第一天及之前除权的事件与回测无关：第一天开盘时还没有持仓
    let mut ev = events.partition_point(|e| e.ex_date <= bars[first].date);
    let execution = p.costs.execution();

    for i in first..bars.len() {
        let b = &bars[i];
        // 1. 开盘前：除权除息。前一天收盘时持有的股票享有分红送转；停牌期间的事件在复牌第一天处理
        while ev < events.len() && events[ev].ex_date <= b.date {
            let e = &events[ev];
            if let Some(o) = pos.as_mut() {
                let cash_div = o.shares as f64 * e.cash_div * (1.0 - p.costs.dividend_tax_rate);
                o.dividends += cash_div;
                dividends += cash_div;
                cash += cash_div;
                o.shares += (o.shares as f64 * e.bonus_ratio).floor() as i64;
            }
            bench.cash += bench.shares * e.cash_div;
            bench.shares *= 1.0 + e.bonus_ratio;
            ev += 1;
        }

        // 2. 开盘：执行昨天收盘后下的单
        if let Some(side) = pending {
            // 一字板：全天只有一个价格。按前复权价和前一天比较，除权日也不会误判
            let prev = if i > 0 { bars[i - 1].q_close } else { b.q_open };
            let blocked = order_blocked(matches!(side, Side::Buy), b.high == b.low, b.q_open, prev);
            if blocked {
                deferred += 1;
            } else {
                match side {
                    Side::Buy => {
                        let mut shares =
                            (cash / (b.open * (1.0 + p.costs.commission_rate)) / p.lot.step as f64)
                                .floor() as i64
                                * p.lot.step;
                        if shares < p.lot.min {
                            shares = 0;
                        }
                        while shares >= p.lot.min
                            && shares as f64 * b.open + p.costs.commission(shares as f64 * b.open)
                                > cash
                        {
                            shares -= p.lot.step;
                        }
                        if shares < p.lot.min {
                            shares = 0;
                        }
                        if shares == 0 {
                            skipped += 1;
                        } else if let Some(fill) =
                            execution.fill(b, side, shares as f64, &p.costs, 0.0)
                        {
                            let filled_shares =
                                (fill.filled / p.lot.step as f64).floor() * p.lot.step as f64;
                            if filled_shares < p.lot.min as f64 {
                                skipped += 1;
                                pending = None;
                                continue;
                            }
                            let Some(fill) = execution.fill(b, side, filled_shares, &p.costs, 0.0)
                            else {
                                skipped += 1;
                                pending = None;
                                continue;
                            };
                            let amount = fill.filled * fill.price;
                            let fee = fill.commission + fill.transfer_fee;
                            if amount + fee > cash + 1e-8 {
                                skipped += 1;
                                pending = None;
                                continue;
                            }
                            pending = None;
                            cash -= amount + fee;
                            fees += fee;
                            pos = Some(Position {
                                idx: i,
                                entry_shares: fill.filled as i64,
                                shares: fill.filled as i64,
                                price: fill.price,
                                q_price: b.q_open,
                                cost: amount + fee,
                                dividends: 0.0,
                            });
                        }
                    }
                    Side::Sell => {
                        if let Some(o) = pos.take() {
                            if let Some(fill) = execution.fill(
                                b,
                                side,
                                o.shares as f64,
                                &p.costs,
                                p.costs.stamp_tax_rate,
                            ) {
                                let sold = fill.filled.floor() as i64;
                                if sold <= 0 {
                                    pos = Some(o);
                                    pending = Some(Side::Sell);
                                    continue;
                                }
                                let gross = sold as f64 * fill.price;
                                let fee = fill.commission + fill.transfer_fee + fill.stamp_tax;
                                cash += gross - fee;
                                fees += fee;
                                if sold < o.shares {
                                    let mut rest = o;
                                    rest.shares -= sold;
                                    pos = Some(rest);
                                    pending = Some(Side::Sell);
                                } else {
                                    pending = None;
                                    trades.push(close_trade(
                                        bars,
                                        &o,
                                        i,
                                        Some((fill.price, sell_reason)),
                                        gross - fee,
                                    ));
                                }
                            } else {
                                pos = Some(o);
                            }
                        }
                    }
                }
            }
        }

        // 3. 收盘：按未复权收盘价计算账户总值
        if p.costs.cash_interest_rate > 0.0 {
            cash *= 1.0 + p.costs.cash_interest_rate / 252.0;
        }
        let value = cash + pos.as_ref().map_or(0.0, |o| o.shares as f64 * b.close);
        let bench_value = bench.cash + bench.shares * b.close;
        peak = peak.max(value);
        max_dd = max_dd.max(1.0 - value / peak);
        bench_peak = bench_peak.max(bench_value);
        bench_dd = bench_dd.max(1.0 - bench_value / bench_peak);
        held_days += pos.is_some() as usize;
        equity.push(EquityPoint {
            date: b.date,
            equity: value,
            benchmark: bench_value,
            holding: pos.is_some(),
        });

        // 4. 收盘后：根据当天的信号和风控决定明天的订单；反向信号取消还没成交的订单
        match &pos {
            Some(o) => {
                let r = b.q_close / o.q_price - 1.0;
                let reason = exit_reason(
                    sig.exit[i],
                    r,
                    p.risk.stop_loss_pct,
                    p.risk.take_profit_pct,
                    p.risk.max_hold_days,
                    i + 1 - o.idx,
                );
                if let Some(reason) = reason {
                    pending = Some(Side::Sell);
                    sell_reason = reason;
                } else if sig.entry[i] {
                    pending = None;
                }
            }
            None if sig.entry[i] && !sig.exit[i] => pending = Some(Side::Buy),
            None if sig.exit[i] => pending = None,
            None => {}
        }
    }

    let last = bars.last()?;
    if let Some(o) = pos.take() {
        trades.push(close_trade(
            bars,
            &o,
            bars.len() - 1,
            None,
            o.shares as f64 * last.close,
        ));
    }
    let start = bars[first].date;
    let final_equity = equity.last().map_or(p.capital, |e| e.equity);
    let total_return = final_equity / p.capital - 1.0;
    let benchmark_return = equity.last().map_or(0.0, |e| e.benchmark / p.capital - 1.0);
    let annual_return = annualize(total_return, start, last.date);
    let risk = crate::analysis::research::risk_metrics(
        &equity.iter().map(|e| e.equity).collect::<Vec<_>>(),
        annual_return,
        max_dd,
    );
    let closed: Vec<&Trade> = trades.iter().filter(|t| t.exit_date.is_some()).collect();
    let wins = closed.iter().filter(|t| t.pnl > 0.0).count();
    let payoff_ratio = payoff(&closed.iter().map(|t| t.pnl).collect::<Vec<_>>());
    let summary = Summary {
        start,
        end: last.date,
        days: equity.len(),
        capital: p.capital,
        final_equity,
        total_return,
        annual_return,
        max_drawdown: max_dd,
        benchmark_return,
        benchmark_annual_return: annualize(benchmark_return, start, last.date),
        benchmark_max_drawdown: bench_dd,
        risk,
        closed_trades: closed.len(),
        wins,
        win_rate: (!closed.is_empty()).then(|| wins as f64 / closed.len() as f64),
        payoff_ratio,
        avg_bars_held: (!closed.is_empty())
            .then(|| closed.iter().map(|t| t.bars_held as f64).sum::<f64>() / closed.len() as f64),
        exposure: held_days as f64 / equity.len() as f64,
        fees,
        dividends,
        deferred_orders: deferred,
        skipped_buys: skipped,
    };
    Some(Outcome {
        summary,
        trades,
        equity,
    })
}

/// 盈亏比：平均盈利 ÷ 平均亏损的绝对值。没有赚钱或没有亏钱的交易时为空
pub fn payoff(pnls: &[f64]) -> Option<f64> {
    let wins: Vec<f64> = pnls.iter().copied().filter(|v| *v > 0.0).collect();
    let losses: Vec<f64> = pnls.iter().copied().filter(|v| *v < 0.0).collect();
    if wins.is_empty() || losses.is_empty() {
        return None;
    }
    let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    Some(avg(&wins) / avg(&losses).abs())
}

fn close_trade(
    bars: &[DailyBar],
    o: &Position,
    exit_idx: usize,
    exit: Option<(f64, ExitReason)>,
    proceeds: f64,
) -> Trade {
    let pnl = proceeds + o.dividends - o.cost;
    Trade {
        entry_date: bars[o.idx].date,
        entry_price: o.price,
        shares: o.entry_shares,
        exit_date: exit.map(|_| bars[exit_idx].date),
        exit_price: exit.map(|(p, _)| p),
        exit_shares: o.shares,
        exit_reason: exit.map(|(_, r)| r),
        cost: o.cost,
        proceeds,
        dividends: o.dividends,
        pnl,
        return_pct: pnl / o.cost,
        bars_held: (exit_idx - o.idx) as i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::ma::{CrossSignal, ma_cross};

    /// 按收盘价序列生成 K 线：开盘价等于收盘价，最高、最低各差 0.1（不是一字板），前复权价等于未复权价
    fn bars(closes: &[f64]) -> Vec<DailyBar> {
        let d0 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| DailyBar {
                date: d0 + chrono::Duration::days(i as i64),
                open: c,
                high: c + 0.1,
                low: c - 0.1,
                close: c,
                q_open: c,
                q_high: c + 0.1,
                q_low: c - 0.1,
                q_close: c,
                volume: 1e6,
                amount: c * 1e6,
            })
            .collect()
    }

    fn params() -> Params {
        Params {
            start: None,
            capital: 10_000.0,
            costs: Costs {
                commission_rate: 0.0,
                min_commission: 0.0,
                stamp_tax_rate: 0.0,
                ..Costs::default()
            },
            risk: Risk::default(),
            lot: LotRule::default(),
        }
    }

    fn cross_signal_flags(bars: &[DailyBar], fast: usize, slow: usize) -> (Vec<bool>, Vec<bool>) {
        let closes: Vec<f64> = bars.iter().map(|b| b.q_close).collect();
        let rows = ma_cross(&closes, fast, slow);
        let entry = rows
            .iter()
            .map(|r| r.is_some_and(|r| r.signal == Some(CrossSignal::Golden)))
            .collect();
        let exit = rows
            .iter()
            .map(|r| r.is_some_and(|r| r.signal == Some(CrossSignal::Death)))
            .collect();
        (entry, exit)
    }

    #[test]
    fn lot_rule_is_used_for_non_main_board_assets() {
        let b = bars(&[1.0, 1.0, 1.0]);
        let entry = [true, false, false];
        let exit = [false; 3];
        let mut p = params();
        p.capital = 250.0;
        p.lot = LotRule { min: 200, step: 1 };
        let out = run(
            &b,
            &[],
            &Signals {
                entry: &entry,
                exit: &exit,
            },
            &p,
        )
        .expect("有足够历史和资金时应生成回测");
        assert_eq!(out.trades[0].shares, 250);
    }

    /// 2 日均线与 3 日均线交叉的回测
    fn run_ma(b: &[DailyBar], events: &[CorpEvent], p: &Params) -> Outcome {
        let (entry, exit) = cross_signal_flags(b, 2, 3);
        run(
            b,
            events,
            &Signals {
                entry: &entry,
                exit: &exit,
            },
            p,
        )
        .unwrap()
    }

    #[test]
    fn trades_next_open_after_signal() {
        // 下标 5 收盘金叉，下标 6 开盘 8 元买入
        let b = bars(&[5.0, 4.0, 3.0, 2.0, 3.0, 5.0, 8.0, 10.0, 6.0, 3.0, 2.0, 2.0]);
        let out = run_ma(&b, &[], &params());
        let t = &out.trades[0];
        assert_eq!(
            (t.entry_date, t.entry_price, t.shares),
            (b[6].date, 8.0, 1200)
        );
        // 快线 9 8 4.5，慢线 7.67 8 6.33：下标 9 收盘死叉，下标 10 开盘 2 元卖出
        assert_eq!((t.exit_date, t.exit_price), (Some(b[10].date), Some(2.0)));
        assert!((t.pnl - 1200.0 * (2.0 - 8.0)).abs() < 1e-9);
        assert_eq!(out.summary.closed_trades, 1);
        assert!((out.summary.final_equity - (10_000.0 - 7200.0)).abs() < 1e-9);
        assert_eq!(out.equity.len(), b.len());
        // 基准：第一天 5 元买入 2,000 股，最后 2 元
        assert!((out.summary.benchmark_return - (2.0 / 5.0 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn limit_up_open_defers_buy_and_costs_apply() {
        let mut b = bars(&[5.0, 4.0, 3.0, 2.0, 3.0, 5.0, 8.0, 9.0, 9.5, 9.6]);
        // 下标 6 一字涨停：买不进，顺延到下标 7
        b[6].high = 8.0;
        b[6].low = 8.0;
        let mut p = params();
        p.costs = Costs::default();
        let out = run_ma(&b, &[], &p);
        assert_eq!(out.summary.deferred_orders, 1);
        let t = &out.trades[0];
        assert_eq!((t.entry_date, t.entry_price), (b[7].date, 9.0));
        assert!(t.exit_date.is_none(), "期末仍持有");
        assert_eq!(t.shares, 1100);
        assert!(
            (out.summary.fees - 5.0).abs() < 1e-9,
            "9,900 元的万 2.5 不到 5 元，按最低 5 元收"
        );
    }

    #[test]
    fn dividends_and_bonus_shares_follow_corp_actions() {
        // 下标 6 买入 1,200 股（8 元）；下标 7 除权：每股分红 0.5 元、每股送 0.5 股，价格除权到 6 元
        let mut b = bars(&[5.0, 4.0, 3.0, 2.0, 3.0, 5.0, 8.0, 6.0, 6.0]);
        let factor = 6.0 / 8.5;
        for x in &mut b[..7] {
            x.q_open *= factor;
            x.q_close *= factor;
        }
        let events = [
            CorpEvent {
                ex_date: b[0].date,
                cash_div: 9.0,
                bonus_ratio: 9.0,
            },
            CorpEvent {
                ex_date: b[7].date,
                cash_div: 0.5,
                bonus_ratio: 0.5,
            },
        ];
        let out = run_ma(&b, &events, &params());
        let t = &out.trades[0];
        assert_eq!((t.shares, t.exit_shares), (1200, 1800));
        assert!(
            (t.dividends - 600.0).abs() < 1e-9,
            "第一天当天的事件不处理，只算下标 7 的分红"
        );
        // 期末：剩余现金 400 + 分红 600 + 1,800 股 × 6 元
        assert!((out.summary.final_equity - (400.0 + 600.0 + 10_800.0)).abs() < 1e-9);
        assert!((t.pnl - (10_800.0 + 600.0 - 9_600.0)).abs() < 1e-9);
        // 基准：5 元买 2,000 股，分红 1,000 元，送股后 3,000 股 × 6 元
        assert!((out.summary.benchmark_return - (19_000.0 / 10_000.0 - 1.0)).abs() < 1e-12);
    }

    #[test]
    fn stop_loss_and_max_hold_close_positions() {
        // 第一天收盘就买入信号，之后一路下跌：跌 8% 后下一天开盘止损
        let b = bars(&[10.0, 10.0, 9.5, 9.0, 8.5, 8.0, 7.5]);
        let entry = vec![true, false, false, false, false, false, false];
        let exit = vec![false; 7];
        let mut p = params();
        p.risk.stop_loss_pct = Some(8.0);
        let out = run(
            &b,
            &[],
            &Signals {
                entry: &entry,
                exit: &exit,
            },
            &p,
        )
        .unwrap();
        let t = &out.trades[0];
        // 下标 1 开盘 10 元买入；下标 3 收盘 9.0（−10%）触发，下标 4 开盘卖出
        assert_eq!(
            (t.entry_date, t.exit_date, t.exit_reason),
            (b[1].date, Some(b[4].date), Some(ExitReason::StopLoss))
        );

        let flat = bars(&[10.0; 8]);
        let entry = vec![true, false, false, false, false, false, false, false];
        let exit = vec![false; 8];
        let mut p = params();
        p.risk.max_hold_days = Some(3);
        let out = run(
            &flat,
            &[],
            &Signals {
                entry: &entry,
                exit: &exit,
            },
            &p,
        )
        .unwrap();
        let t = &out.trades[0];
        assert_eq!((t.bars_held, t.exit_reason), (3, Some(ExitReason::MaxHold)));
    }

    #[test]
    fn payoff_ratio() {
        assert_eq!(payoff(&[30.0, 10.0, -10.0]), Some(2.0));
        assert_eq!(payoff(&[5.0]), None);
    }
}
