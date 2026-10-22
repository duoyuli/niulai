//! 策略的买卖点：按单只股票的状态机计算，不涉及资金和仓位。
//! 空仓时满足买入条件（含股票池）记一次买点，下一个交易日开盘买入；持有时满足任一卖出规则记一次卖点，
//! 下一个交易日开盘卖出。持有中再出现买点忽略。开盘一字涨停买不进、一字跌停卖不出时顺延，直到成交或被反向信号取消。
//! 和单只股票回测（backtest::run）的买卖规则相同，只是不算资金和手数。
//! 每日巡检和个股 K 线上的买卖点都用这里，规则见 docs/domains/strategies.md#points。

use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use super::backtest::ExitReason;
use super::indicators::{Frame, StockMeta};
use super::prices::DailyBar;
use super::strategy::Definition;

/// 一次买卖
#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct PointTrade {
    /// 满足买入条件的那天（收盘后知道）
    pub signal_date: NaiveDate,
    /// 下一个交易日开盘买入
    pub entry_date: NaiveDate,
    /// 买入价（未复权开盘价）
    pub entry_price: f64,
    /// 触发卖出规则的那天；还在持有时为空
    pub exit_signal_date: Option<NaiveDate>,
    /// 下一个交易日开盘卖出；还没卖出时为空
    pub exit_date: Option<NaiveDate>,
    pub exit_price: Option<f64>,
    pub exit_reason: Option<ExitReason>,
    /// 卖出原因的白话，例如「收盘价下穿 20 日均线」「亏损达到 8% 止损」
    pub exit_text: Option<String>,
    /// 收益率（%，按前复权价，含分红送转，不扣费用）：卖出时按卖出价，还在持有时按最新收盘价
    pub return_pct: f64,
    /// 持有的交易日数
    pub bars_held: i64,
}

/// 最后一个交易日的状态
#[derive(Debug, Clone, PartialEq)]
pub enum Today {
    /// 今天出现买点，明天开盘买入
    Buy,
    /// 今天触发卖出规则，明天开盘卖出
    Sell { reason: ExitReason, text: String },
    /// 持有中
    Holding,
    /// 空仓
    Flat,
}

pub struct Points {
    pub trades: Vec<PointTrade>,
    pub today: Today,
}

fn exit_text(d: &Definition, f: &Frame, k: usize, reason: ExitReason) -> String {
    let r = &d.exit;
    match reason {
        ExitReason::Signal => d
            .exit_reason_text(f, k)
            .unwrap_or_else(|| "卖出条件".into()),
        ExitReason::StopLoss => format!("亏损达到 {}% 止损", r.stop_loss_pct.unwrap_or_default()),
        ExitReason::TakeProfit => {
            format!("盈利达到 {}% 止盈", r.take_profit_pct.unwrap_or_default())
        }
        ExitReason::MaxHold => format!("持有满 {} 个交易日", r.max_hold_days.unwrap_or_default()),
    }
}

// @project-doc docs/domains/strategies.md#points
/// 从第 from 天（含）开始、空仓起步，算出这只股票的全部买卖点和最后一天的状态
pub fn run(bars: &[DailyBar], f: &Frame, meta: &StockMeta, d: &Definition, from: usize) -> Points {
    let buy = d.buy_series(f);
    let sell = d.exit_series(f);

    let mut trades: Vec<PointTrade> = vec![];
    // 持有中：(在 trades 中的下标, 买入日下标, 买入的前复权价)
    let mut holding: Option<(usize, usize, f64)> = None;
    // 还没成交的单：买入时是信号日，卖出时是信号日和原因
    let mut pending_buy: Option<usize> = None;
    let mut pending_sell: Option<(usize, ExitReason)> = None;
    for k in from..bars.len() {
        // 开盘：执行昨天收盘后下的单。一字板全天一个价格，按前复权价和前一天比较，买不进、卖不出时顺延
        let prev = if k > 0 {
            bars[k - 1].q_close
        } else {
            bars[k].q_open
        };
        let one_price = bars[k].high == bars[k].low;
        if let Some(i) = pending_buy
            && !super::backtest::order_blocked(true, one_price, bars[k].q_open, prev)
        {
            pending_buy = None;
            trades.push(PointTrade {
                signal_date: bars[i].date,
                entry_date: bars[k].date,
                entry_price: bars[k].open,
                exit_signal_date: None,
                exit_date: None,
                exit_price: None,
                exit_reason: None,
                exit_text: None,
                return_pct: 0.0,
                bars_held: 0,
            });
            holding = Some((trades.len() - 1, k, bars[k].q_open));
        }
        if let (Some((s, reason)), Some((t, j, q))) = (pending_sell, holding)
            && !super::backtest::order_blocked(false, one_price, bars[k].q_open, prev)
        {
            let tr = &mut trades[t];
            tr.exit_signal_date = Some(bars[s].date);
            tr.exit_date = Some(bars[k].date);
            tr.exit_price = Some(bars[k].open);
            tr.exit_reason = Some(reason);
            tr.exit_text = Some(exit_text(d, f, s, reason));
            tr.return_pct = (bars[k].q_open / q - 1.0) * 100.0;
            tr.bars_held = (k - j) as i64;
            holding = None;
            pending_sell = None;
        }
        // 收盘：决定明天的单；反向信号取消还没成交的单
        match holding {
            Some((t, j, q)) => {
                trades[t].return_pct = (bars[k].q_close / q - 1.0) * 100.0;
                trades[t].bars_held = (k - j) as i64;
                let ret = bars[k].q_close / q - 1.0;
                let reason = super::backtest::exit_reason(
                    sell[k],
                    ret,
                    d.exit.stop_loss_pct,
                    d.exit.take_profit_pct,
                    d.exit.max_hold_days,
                    k + 1 - j,
                );
                match reason {
                    Some(reason) => pending_sell = Some((k, reason)),
                    None if buy[k] && d.in_universe(f, meta, k) => pending_sell = None,
                    None => {}
                }
            }
            None => {
                let entry = buy[k] && d.in_universe(f, meta, k);
                if entry && !sell[k] {
                    pending_buy = Some(k);
                } else if sell[k] {
                    pending_buy = None;
                }
            }
        }
    }
    let today = match (pending_buy, pending_sell, holding) {
        (Some(_), _, None) => Today::Buy,
        (_, Some((s, reason)), Some(_)) => Today::Sell {
            reason,
            text: exit_text(d, f, s, reason),
        },
        (_, _, Some(_)) => Today::Holding,
        _ => Today::Flat,
    };
    Points { trades, today }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::tests::bars_from;

    fn def(json: serde_json::Value) -> Definition {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn buys_on_signal_and_sells_on_exit_rule_next_open() {
        // 先跌后涨再跌：收盘价上穿 3 日均线买入，下穿时卖出
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
        let d = def(serde_json::json!({
            "universe": {"exclude_st": false},
            "all": [{"kind": "compare", "left": {"id": "close"}, "op": "crosses_above", "right": {"id": "ma", "n": 3}}],
            "exit": {"any": [{"kind": "compare", "left": {"id": "close"}, "op": "crosses_below", "right": {"id": "ma", "n": 3}}]}
        }));
        let p = run(&b, &f, &meta, &d, 0);
        assert_eq!(p.trades.len(), 1);
        let t = &p.trades[0];
        assert_eq!(
            (t.signal_date, t.entry_date),
            (b[4].date, b[5].date),
            "信号当天收盘后知道，第二天开盘买入"
        );
        assert_eq!(t.exit_reason, Some(ExitReason::Signal));
        assert_eq!(t.exit_text.as_deref(), Some("收盘价下穿 3 日均线"));
        assert_eq!(p.today, Today::Flat);

        // 只看到第 5 天（下标 4）为止：今天是买点
        let p = run(&b[..5], &Frame::new(&b[..5], None), &meta, &d, 0);
        assert_eq!(p.today, Today::Buy);
        // 看到第 9 天：持有中
        let p = run(&b[..8], &Frame::new(&b[..8], None), &meta, &d, 0);
        assert_eq!(p.today, Today::Holding);
        assert!(p.trades[0].return_pct > 0.0);
    }

    #[test]
    fn stop_loss_and_ignore_buys_while_holding() {
        let closes = [10.0, 10.0, 10.0, 11.0, 9.0, 8.0, 8.0, 8.0];
        let b = bars_from(&closes);
        let meta = StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
            delisted_on: None,
            asset_type: crate::analysis::indicators::AssetType::Stock,
        };
        let d = def(serde_json::json!({
            "universe": {"exclude_st": false},
            "all": [{"kind": "compare", "left": {"id": "close"}, "op": ">", "right": {"value": 0}}],
            "exit": {"stop_loss_pct": 15}
        }));
        let p = run(&b, &Frame::new(&b, None), &meta, &d, 0);
        assert_eq!(
            p.trades[0].exit_reason,
            Some(ExitReason::StopLoss),
            "{:?}",
            p.trades
        );
        assert_eq!(p.trades[0].exit_text.as_deref(), Some("亏损达到 15% 止损"));
        // 每天都满足买入条件，但持有期间不重复买
        assert!(
            p.trades
                .windows(2)
                .all(|w| w[0].exit_date.is_some_and(|x| x <= w[1].entry_date))
        );
    }

    /// 随机日线，夹着一字涨停和一字跌停
    fn random_bars(n: usize, seed: u64) -> Vec<DailyBar> {
        let mut x = seed;
        let mut rnd = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as f64 / (1u64 << 31) as f64
        };
        let mut c = 10.0;
        let closes: Vec<f64> = (0..n)
            .map(|_| {
                c *= 1.0 + (rnd() - 0.5) * 0.08;
                (c * 100.0).round() / 100.0
            })
            .collect();
        let mut b = bars_from(&closes);
        for i in 1..b.len() {
            let r = rnd();
            let prev = b[i - 1].q_close;
            let price = if r < 0.05 {
                Some(prev * 1.1)
            } else if r < 0.1 {
                Some(prev * 0.9)
            } else {
                None
            };
            let bar = &mut b[i];
            match price {
                Some(p) => (bar.open, bar.high, bar.low, bar.close) = (p, p, p, p),
                None => {
                    bar.open = bar.close * (1.0 + (rnd() - 0.5) * 0.04);
                    bar.high = bar.close.max(bar.open) * 1.01;
                    bar.low = bar.close.min(bar.open) * 0.99;
                }
            }
            (bar.q_open, bar.q_high, bar.q_low, bar.q_close) =
                (bar.open, bar.high, bar.low, bar.close);
        }
        b
    }

    #[test]
    fn same_trades_as_single_stock_backtest() {
        use crate::analysis::backtest::{self, Costs, Params, Risk, Signals};
        let meta = StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
            delisted_on: None,
            asset_type: crate::analysis::indicators::AssetType::Stock,
        };
        let d = def(serde_json::json!({
            "universe": {"exclude_st": false},
            "all": [{"kind": "compare", "left": {"id": "close"}, "op": "crosses_above", "right": {"id": "ma", "n": 5}}],
            "exit": {"any": [{"kind": "compare", "left": {"id": "close"}, "op": "crosses_below", "right": {"id": "ma", "n": 10}}],
                     "stop_loss_pct": 6, "take_profit_pct": 15, "max_hold_days": 12}
        }));
        for seed in 1..=8 {
            let b = random_bars(500, seed);
            let f = Frame::new(&b, None);
            let p = run(&b, &f, &meta, &d, 0);
            let entry: Vec<bool> = (0..b.len())
                .map(|i| d.in_universe(&f, &meta, i) && d.matches(&f, i))
                .collect();
            let exit: Vec<bool> = (0..b.len()).map(|i| d.exit_signal(&f, i)).collect();
            let risk = Risk {
                stop_loss_pct: d.exit.stop_loss_pct,
                take_profit_pct: d.exit.take_profit_pct,
                max_hold_days: d.exit.max_hold_days,
            };
            let params = Params {
                start: None,
                capital: 1e12,
                costs: Costs::default(),
                risk,
                lot: Default::default(),
            };
            let bt = backtest::run(
                &b,
                &[],
                &Signals {
                    entry: &entry,
                    exit: &exit,
                },
                &params,
            )
            .unwrap();
            let ours: Vec<_> = p
                .trades
                .iter()
                .map(|t| (t.entry_date, t.exit_date, t.exit_reason))
                .collect();
            let theirs: Vec<_> = bt
                .trades
                .iter()
                .map(|t| (t.entry_date, t.exit_date, t.exit_reason))
                .collect();
            assert!(ours.len() > 10, "seed {seed} 交易太少");
            assert_eq!(ours, theirs, "seed {seed}");
        }
    }
}
