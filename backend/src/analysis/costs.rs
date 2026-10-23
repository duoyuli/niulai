//! 交易成本和一字板成交约束：因子组合回测按这里扣费、判断能不能成交。

use serde::Serialize;
use utoipa::ToSchema;

/// 佣金、印花税、过户费和滑点
#[derive(Serialize, ToSchema, Clone, Copy, Debug)]
#[schema(as = TradingCosts)]
pub struct Costs {
    /// 佣金费率，买卖都收
    pub commission_rate: f64,
    /// 单笔最低佣金（元）
    pub min_commission: f64,
    /// 印花税率，只在卖出时收
    pub stamp_tax_rate: f64,
    /// 买卖双边过户费，按成交金额计
    pub transfer_fee_rate: f64,
    /// 固定滑点（基点），买卖各收一次
    pub slippage_bps: f64,
}

impl Default for Costs {
    fn default() -> Self {
        // 常见的万 2.5 佣金、最低 5 元；印花税 2023-08-28 起为卖出 0.05%
        Costs {
            commission_rate: 0.000_25,
            min_commission: 5.0,
            stamp_tax_rate: 0.000_5,
            transfer_fee_rate: 0.0,
            slippage_bps: 0.0,
        }
    }
}

impl Costs {
    pub fn commission(&self, amount: f64) -> f64 {
        (amount * self.commission_rate).max(self.min_commission)
    }

    /// 在包含佣金和过户费的预算内，最多能成交多少金额（不含费用本身）。
    /// 佣金有最低额，先判断按比例收的佣金是否已经超过最低佣金，避免低估费用
    pub fn affordable_amount(&self, budget: f64) -> f64 {
        if !budget.is_finite() || budget <= 0.0 {
            return 0.0;
        }
        let transfer = self.transfer_fee_rate.max(0.0);
        let commission_rate = self.commission_rate.max(0.0);
        let min_commission = self.min_commission.max(0.0);
        let full = budget / (1.0 + transfer + commission_rate);
        if full * commission_rate >= min_commission {
            full
        } else {
            ((budget - min_commission) / (1.0 + transfer)).max(0.0)
        }
    }
}

/// 一字板的成交约束：买单遇到向上封板、卖单遇到向下封板时顺延
pub fn order_blocked(is_buy: bool, one_price: bool, open: f64, previous_close: f64) -> bool {
    one_price
        && if is_buy {
            open > previous_close
        } else {
            open < previous_close
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affordable_amount_leaves_room_for_fees() {
        let c = Costs::default();
        let a = c.affordable_amount(10_000.0);
        assert!((a + c.commission(a) - 10_000.0).abs() < 1e-6);
        let a = c.affordable_amount(100_000.0);
        assert!((a + c.commission(a) - 100_000.0).abs() < 1e-6);
        assert_eq!(c.affordable_amount(3.0), 0.0);
    }

    #[test]
    fn one_price_bars_block_the_sealed_side_only() {
        assert!(order_blocked(true, true, 11.0, 10.0));
        assert!(!order_blocked(false, true, 11.0, 10.0));
        assert!(order_blocked(false, true, 9.0, 10.0));
        assert!(!order_blocked(true, false, 11.0, 10.0));
    }
}
