//! 交易规则：每个板块最少买多少股、按多少股递增，以及佣金和印花税的估算。规则见 docs/domains/accounts.md#lots。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::instrument::AssetType;

/// 一个板块的申报数量规则
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LotRule {
    /// 买入至少多少股
    pub min: i64,
    /// 超过最少数量后按多少股递增
    pub step: i64,
}

/// 申报单位：沪深主板、创业板一手 100 股；科创板至少 200 股、之后按 1 股递增；北交所至少 100 股、之后按 1 股递增；
/// ETF 按场内基金常见的 100 份一手
pub fn lot_rule_for(asset_type: &str, board: &str) -> LotRule {
    let (min, step) = AssetType::from_db(Some(asset_type)).lot_rule(board);
    LotRule { min, step }
}

/// 账户的费率
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq)]
pub struct Fees {
    /// 佣金率，例如 0.00025 表示万分之 2.5
    pub commission_rate: f64,
    /// 每笔最低佣金（元）
    pub min_commission: f64,
}

impl Default for Fees {
    fn default() -> Self {
        Self {
            commission_rate: 0.00025,
            min_commission: 5.0,
        }
    }
}

impl Fees {
    pub fn commission(&self, amount: f64) -> f64 {
        round2((amount * self.commission_rate).max(self.min_commission))
    }

    pub fn buy_fee(&self, amount: f64) -> f64 {
        self.commission(amount)
    }

    /// 卖出的佣金加印花税（股票千分之 0.5，ETF 不收）
    pub fn sell_fee(&self, asset_type: &str, amount: f64) -> f64 {
        round2(
            self.commission(amount)
                + amount * AssetType::from_db(Some(asset_type)).stamp_tax_rate(),
        )
    }
}

/// 预算内按参考价最多能买多少股（含佣金不超过预算）；连最少数量都买不起时为 0
pub fn max_buy_shares(rule: LotRule, budget: f64, price: f64, fees: &Fees) -> i64 {
    if price <= 0.0 || budget <= 0.0 {
        return 0;
    }
    let mut n = (budget / price).floor() as i64;
    if n < rule.min {
        return 0;
    }
    n = rule.min + (n - rule.min) / rule.step * rule.step;
    while n >= rule.min && n as f64 * price + fees.buy_fee(n as f64 * price) > budget {
        n -= rule.step;
    }
    if n < rule.min { 0 } else { n }
}

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEES: Fees = Fees {
        commission_rate: 0.00025,
        min_commission: 5.0,
    };

    #[test]
    fn rounds_to_board_lots_within_budget() {
        let main = lot_rule_for("stock", "main");
        assert_eq!(
            max_buy_shares(main, 15_800.0, 79.0, &FEES),
            100,
            "200 股要 15,800 元加 5 元佣金，超过预算"
        );
        assert_eq!(max_buy_shares(main, 15_810.0, 79.0, &FEES), 200);
        assert_eq!(max_buy_shares(main, 7_000.0, 79.0, &FEES), 0, "不够一手");
        let star = lot_rule_for("stock", "star");
        assert_eq!(
            max_buy_shares(star, 10_000.0, 60.0, &FEES),
            0,
            "科创板至少 200 股：166 股不够"
        );
        assert_eq!(
            max_buy_shares(star, 10_000.0, 40.0, &FEES),
            249,
            "超过 200 股后按 1 股递增"
        );
        assert_eq!(max_buy_shares(star, 10_000.0, 30.0, &FEES), 333);
        assert_eq!(
            max_buy_shares(lot_rule_for("stock", "bse"), 2_000.0, 10.0, &FEES),
            199
        );
        assert_eq!(
            lot_rule_for("etf", "star"),
            LotRule {
                min: 100,
                step: 100
            }
        );
    }

    #[test]
    fn fees_include_minimum_commission_and_stamp_tax() {
        assert_eq!(FEES.buy_fee(10_000.0), 5.0, "不足 5 元按 5 元");
        assert_eq!(
            FEES.sell_fee("stock", 100_000.0),
            75.0,
            "佣金 25 元加印花税 50 元"
        );
        assert_eq!(FEES.sell_fee("etf", 100_000.0), 25.0, "ETF 不收印花税");
    }
}
