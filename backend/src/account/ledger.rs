//! 流水回放：从流水重建一只股票现在的股数、成本价，以及每一笔卖出的已实现盈亏。
//!
//! 账户的持仓和现金本来就是流水的结果。要撤销（红冲）一条流水时，把这条流水标成已撤销，再用这里的口径在
//! 同一个事务里把这只股票的持仓重算一遍，现金按这条流水的金额反向加减，就不会留下和流水对不上的持仓。
//!
//! 口径是「移动加权平均成本」，和记账时一致：
//!
//! - 买入：成本价 =（原成本 × 原股数 + 这次付出的钱）÷ 新股数，付出的钱含佣金；
//! - 卖出：不改变成本价；这笔卖出的已实现盈亏 = 卖出净回款（扣佣金和印花税）− 卖出股数 × 卖出时的成本价；
//! - 分红：现金加到现金上，成本价按「（原成本 × 原股数 − 分红）÷ 原股数」摊薄；
//! - 送转：股数增加，成本价按新股数摊薄；
//! - 校准（`adjust`）：直接把持仓换成校准时填的股数和成本价，成本价没填时为空。
//!
//! 回放只处理持仓相关的流水，现金另行加减；已撤销的流水在查询时就不取。规则见 docs/domains/accounts.md#undo。

use chrono::NaiveDate;

use super::TradeKind;

/// 回放用的一行流水；`shares`、`price` 的含义和 `app.account_trade` 一样
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub trade_id: i64,
    pub kind: TradeKind,
    pub trade_date: NaiveDate,
    /// 持仓股数的变化：买入、送转为正，卖出为负；校准是股数的变化量
    pub shares: Option<i64>,
    /// 校准后的成本价（校准流水）；买入、卖出的成交价不参与回放
    pub price: Option<f64>,
    /// 现金的变化（含费用）：买入为负，卖出、分红为正
    pub amount: f64,
}

/// 一笔卖出的已实现盈亏；成本价没填时为空（算不出来）
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SellPnl {
    pub trade_id: i64,
    pub pnl: Option<f64>,
}

/// 回放一条持仓（同一只股票）在全部有效流水之后的股数、成本价和已实现盈亏
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Replay {
    pub shares: i64,
    pub cost_price: Option<f64>,
    /// 每一笔卖出的盈亏，按流水顺序
    pub sell_pnls: Vec<SellPnl>,
    /// 全部卖出都能算出成本价时才有值；一笔都没有卖出时是 0
    pub realized_pnl: Option<f64>,
    /// 有卖出因为没填成本价算不出盈亏
    pub missing_cost: bool,
    /// 这一轮持仓最早一笔买入的日期；持仓被清空后重新买入时跟着变。撤销把持仓清空的卖出、要恢复持仓时用
    pub opened_on: Option<NaiveDate>,
}

/// 回放时发现前后对不上：说明被撤销的那条流水在当时支撑着后面的卖出或分红送转
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayError {
    /// 卖出股数超过当时持有的股数
    Oversold { trade_id: i64 },
    /// 已经没有持仓时还有分红送转
    HoldingGone { trade_id: i64 },
}

impl ReplayError {
    /// 给用户看的原因。撤销一笔买入会让后面的卖出、分红送转对不上，这时不能撤销
    pub fn trade_id(self) -> i64 {
        match self {
            ReplayError::Oversold { trade_id } | ReplayError::HoldingGone { trade_id } => trade_id,
        }
    }
}

// @project-doc docs/domains/accounts.md#undo
/// 回放同一只股票的有效流水（按流水编号从早到晚）
pub fn replay(entries: &[Entry]) -> Result<Replay, ReplayError> {
    let mut out = Replay { realized_pnl: Some(0.0), ..Replay::default() };
    for e in entries {
        match e.kind {
            TradeKind::Buy => {
                let n = e.shares.unwrap_or(0);
                if n <= 0 {
                    continue;
                }
                let pay = -e.amount;
                out.cost_price = if out.shares == 0 {
                    out.opened_on = Some(e.trade_date);
                    Some(round4(pay / n as f64))
                } else {
                    out.cost_price.map(|c| round4((c * out.shares as f64 + pay) / (out.shares + n) as f64))
                };
                out.shares += n;
            }
            TradeKind::Rights => {
                // 配股认购和买入一样参与平均成本，但前提是当时还持有这只股票（配股是按持仓配的）
                let n = e.shares.unwrap_or(0);
                if n <= 0 {
                    continue;
                }
                if out.shares <= 0 {
                    return Err(ReplayError::HoldingGone { trade_id: e.trade_id });
                }
                let pay = -e.amount;
                out.cost_price = out.cost_price.map(|c| round4((c * out.shares as f64 + pay) / (out.shares + n) as f64));
                out.shares += n;
            }
            TradeKind::Sell => {
                let n = e.shares.unwrap_or(0).abs();
                if n <= 0 {
                    continue;
                }
                if n > out.shares {
                    return Err(ReplayError::Oversold { trade_id: e.trade_id });
                }
                let pnl = out.cost_price.map(|c| round2(e.amount - c * n as f64));
                if pnl.is_none() {
                    out.missing_cost = true;
                    out.realized_pnl = None;
                } else if let Some(total) = &mut out.realized_pnl {
                    *total += pnl.unwrap_or(0.0);
                }
                out.sell_pnls.push(SellPnl { trade_id: e.trade_id, pnl });
                out.shares -= n;
                if out.shares == 0 {
                    // 卖完时持仓行删除，成本价和买入日也一起清掉
                    out.cost_price = None;
                    out.opened_on = None;
                }
            }
            TradeKind::Dividend => {
                if out.shares <= 0 {
                    return Err(ReplayError::HoldingGone { trade_id: e.trade_id });
                }
                if let Some(c) = out.cost_price {
                    out.cost_price = Some(round4(((c * out.shares as f64 - e.amount) / out.shares as f64).max(0.0001)));
                }
            }
            TradeKind::Bonus => {
                let bonus = e.shares.unwrap_or(0);
                if bonus <= 0 {
                    continue;
                }
                if out.shares <= 0 {
                    return Err(ReplayError::HoldingGone { trade_id: e.trade_id });
                }
                let new_shares = out.shares + bonus;
                out.cost_price = out.cost_price.map(|c| round4((c * out.shares as f64 / new_shares as f64).max(0.0001)));
                out.shares = new_shares;
            }
            TradeKind::Adjust => {
                let before = out.shares;
                let new_shares = before + e.shares.unwrap_or(0);
                if new_shares <= 0 {
                    // 校准把持仓整个移除，或者股数被校准成 0
                    out.shares = 0;
                    out.cost_price = None;
                    out.opened_on = None;
                } else {
                    if before == 0 {
                        // 校准建仓：校准时填的买入日只存在持仓行里，回放只能当没填
                        out.opened_on = None;
                    }
                    out.shares = new_shares;
                    out.cost_price = e.price.map(round4);
                }
            }
            TradeKind::CashIn | TradeKind::CashOut => {}
        }
    }
    out.realized_pnl = out.realized_pnl.map(round2);
    Ok(out)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, day).unwrap()
    }

    fn entry(trade_id: i64, kind: TradeKind, date: NaiveDate, shares: Option<i64>, price: Option<f64>, amount: f64) -> Entry {
        Entry { trade_id, kind, trade_date: date, shares, price, amount }
    }

    #[test]
    fn average_cost_and_each_sell_pnl() {
        // 买 1,000 股 × 23 元，佣金 5 元：成本 23.005
        let buy1 = entry(1, TradeKind::Buy, d(2), Some(1_000), Some(23.0), -23_005.0);
        // 再买 1,000 股 × 25 元，佣金 5 元：成本 (23,005 + 25,005) ÷ 2,000 = 24.005
        let buy2 = entry(2, TradeKind::Buy, d(3), Some(1_000), Some(25.0), -25_005.0);
        // 卖 500 股 × 30 元，佣金 3.75 元 + 印花税 7.5 元：净回款 14,988.75，盈亏 14,988.75 − 500 × 24.005
        let sell = entry(3, TradeKind::Sell, d(4), Some(-500), Some(30.0), 14_988.75);
        let r = replay(&[buy1, buy2, sell]).unwrap();
        assert_eq!(r.shares, 1_500);
        assert_eq!(r.cost_price, Some(24.005), "卖出不改变平均成本");
        assert_eq!(r.realized_pnl, Some(2_986.25));
        assert_eq!(r.sell_pnls, vec![SellPnl { trade_id: 3, pnl: Some(2_986.25) }]);
        assert_eq!(r.opened_on, Some(d(2)));
        assert!(!r.missing_cost);
    }

    #[test]
    fn dividend_and_bonus_thin_the_cost() {
        // 买 1,000 股成本 23.005 → 每 10 股派 2 元、送 3 股
        let buy = entry(1, TradeKind::Buy, d(2), Some(1_000), Some(23.0), -23_005.0);
        let dividend = entry(2, TradeKind::Dividend, d(5), None, Some(0.2), 200.0);
        let bonus = entry(3, TradeKind::Bonus, d(5), Some(300), None, 0.0);
        let r = replay(&[buy, dividend, bonus]).unwrap();
        assert_eq!(r.shares, 1_300);
        // 先摊分红：23.005 − 0.2 = 22.805；再摊送转：22.805 × 1,000 ÷ 1,300 = 17.5423
        assert_eq!(r.cost_price, Some(17.5423));
        assert_eq!(r.realized_pnl, Some(0.0), "没有卖出时已实现盈亏是 0");
        assert_eq!(r.sell_pnls.len(), 0);
    }

    #[test]
    fn rights_subscription_joins_the_average_cost() {
        // 买 1,000 股成本 23.005；每 10 股配 3 股、配股价 10 元，认购 300 股，费用 5 元：
        // 成本 (23,005 + 3,005) ÷ 1,300 = 20.0077
        let buy = entry(1, TradeKind::Buy, d(2), Some(1_000), Some(23.0), -23_005.0);
        let rights = entry(2, TradeKind::Rights, d(8), Some(300), Some(10.0), -3_005.0);
        let r = replay(&[buy, rights.clone()]).unwrap();
        assert_eq!(r.shares, 1_300);
        assert_eq!(r.cost_price, Some(20.0077));
        // 配股要有持仓才能认购：撤销支撑它的买入以后就对不上了
        assert_eq!(replay(&[rights]), Err(ReplayError::HoldingGone { trade_id: 2 }));
    }

    #[test]
    fn sells_after_removing_a_buy_are_oversold() {
        let buy = entry(1, TradeKind::Buy, d(2), Some(1_000), Some(23.0), -23_005.0);
        let sell = entry(2, TradeKind::Sell, d(3), Some(-1_000), Some(24.0), 23_983.0);
        assert_eq!(replay(&[sell]), Err(ReplayError::Oversold { trade_id: 2 }));
        assert_eq!(replay(&[buy]).unwrap().shares, 1_000);
    }

    #[test]
    fn dividend_needs_the_buy_that_was_removed() {
        let dividend = entry(2, TradeKind::Dividend, d(5), None, Some(0.2), 200.0);
        assert_eq!(replay(&[dividend]), Err(ReplayError::HoldingGone { trade_id: 2 }));
    }

    #[test]
    fn missing_cost_cannot_guess_realized_pnl() {
        // 校准进来 1,000 股没填成本价，后来卖 500 股：这笔卖出的盈亏算不出来
        let adjust = entry(1, TradeKind::Adjust, d(2), Some(1_000), None, 0.0);
        let sell = entry(2, TradeKind::Sell, d(3), Some(-500), Some(10.0), 4_998.75);
        let r = replay(&[adjust, sell]).unwrap();
        assert_eq!(r.shares, 500);
        assert_eq!(r.cost_price, None);
        assert_eq!(r.realized_pnl, None);
        assert!(r.missing_cost);
        assert_eq!(r.sell_pnls[0].pnl, None);
    }

    #[test]
    fn adjust_sets_shares_and_cost_and_removing_keeps_metadata_unknown() {
        let buy = entry(1, TradeKind::Buy, d(2), Some(1_000), Some(23.0), -23_005.0);
        // 校准成 1,200 股、成本 20 元
        let adjust = entry(2, TradeKind::Adjust, d(3), Some(200), Some(20.0), 0.0);
        let r = replay(&[buy.clone(), adjust.clone()]).unwrap();
        assert_eq!((r.shares, r.cost_price), (1_200, Some(20.0)));
        assert_eq!(r.opened_on, Some(d(2)), "校准改股数不改买入日");
        // 校准整个移除，再校准建仓：买入日恢复不了，成本价按校准填的
        let remove = entry(3, TradeKind::Adjust, d(4), Some(-1_200), None, 0.0);
        let rebuild = entry(4, TradeKind::Adjust, d(5), Some(300), Some(18.0), 0.0);
        let r = replay(&[buy, adjust, remove, rebuild]).unwrap();
        assert_eq!((r.shares, r.cost_price), (300, Some(18.0)));
        assert_eq!(r.opened_on, None);
    }

    #[test]
    fn closing_a_position_resets_cost_and_opened_on() {
        let buy1 = entry(1, TradeKind::Buy, d(2), Some(1_000), Some(23.0), -23_005.0);
        let sell_all = entry(2, TradeKind::Sell, d(3), Some(-1_000), Some(24.0), 23_983.0);
        // 重新买 500 股：成本按这次买入算（含佣金 5 元），买入日也是新的
        let buy2 = entry(3, TradeKind::Buy, d(6), Some(500), Some(26.0), -13_005.0);
        let r = replay(&[buy1, sell_all, buy2]).unwrap();
        assert_eq!(r.shares, 500);
        assert_eq!(r.cost_price, Some(26.01));
        assert_eq!(r.opened_on, Some(d(6)));
        assert_eq!(r.realized_pnl, Some(978.0), "卖出净回款 23,983 − 成本 23,005");
    }
}
