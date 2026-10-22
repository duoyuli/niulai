//! 统一的日线执行模型。
//!
//! 信号使用前复权价，成交使用未复权行情。单股回测、组合回测和账户规则回测
//! 通过这里共享成交价、滑点、成交量参与率和费用的口径。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::backtest::{Costs, Side};
use super::prices::DailyBar;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum DealPrice {
    #[default]
    Open,
    Close,
    Vwap,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Debug, Default)]
pub struct ExecutionConfig {
    pub deal_price: DealPrice,
    /// 固定双边滑点，单位为基点。
    pub slippage_bps: f64,
    /// 每日最多成交量占比；例如 0.1 表示最多使用当日成交量的 10%。
    pub participation_rate: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub price: f64,
    pub requested: f64,
    pub filled: f64,
    pub commission: f64,
    pub transfer_fee: f64,
    pub stamp_tax: f64,
    pub slippage: f64,
}

impl ExecutionConfig {
    pub fn validate(self) -> Result<Self, String> {
        if !self.slippage_bps.is_finite() || !(0.0..=500.0).contains(&self.slippage_bps) {
            return Err("slippage_bps 应在 0 到 500 之间".into());
        }
        if let Some(v) = self.participation_rate
            && (!v.is_finite() || !(0.0..=1.0).contains(&v) || v == 0.0)
        {
            return Err("participation_rate 应在 0 到 1 之间且不能为 0".into());
        }
        Ok(self)
    }

    fn base_price(self, bar: &DailyBar) -> f64 {
        match self.deal_price {
            DealPrice::Open => bar.open,
            DealPrice::Close => bar.close,
            DealPrice::Vwap => (bar.high + bar.low + bar.close) / 3.0,
        }
    }

    /// 按成交价口径和滑点估算每单位成交价。下单前用它计算能买多少，避免到成交时才发现资金不够。
    pub fn estimate_price(self, bar: &DailyBar, side: Side) -> Option<f64> {
        let direction = if side == Side::Buy { 1.0 } else { -1.0 };
        let price = self.base_price(bar) * (1.0 + direction * self.slippage_bps / 10_000.0);
        (price.is_finite() && price > 0.0).then_some(price)
    }

    /// 计算一笔日线成交。涨跌停排队仍由上层状态机决定；这里负责成交价、参与率和费用。
    pub fn fill(
        self,
        bar: &DailyBar,
        side: Side,
        requested: f64,
        costs: &Costs,
        stamp_tax_rate: f64,
    ) -> Option<Fill> {
        if requested <= 0.0 || !requested.is_finite() {
            return None;
        }
        let max_by_volume = self
            .participation_rate
            .map(|p| bar.volume.max(0.0) * p)
            .unwrap_or(f64::INFINITY);
        let filled = requested.min(max_by_volume);
        if filled <= 0.0 {
            return None;
        }
        let base = self.base_price(bar);
        let price = self.estimate_price(bar, side)?;
        let amount = filled * price;
        let commission = costs.commission(amount);
        let transfer_fee = amount * costs.transfer_fee_rate;
        let stamp_tax = if side == Side::Sell {
            amount * stamp_tax_rate
        } else {
            0.0
        };
        Some(Fill {
            price,
            requested,
            filled,
            commission,
            transfer_fee,
            stamp_tax,
            slippage: (price - base) * filled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::backtest::Costs;
    use crate::analysis::prices::DailyBar;

    fn bar() -> DailyBar {
        DailyBar {
            date: chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
            open: 10.0,
            high: 12.0,
            low: 8.0,
            close: 11.0,
            q_open: 10.0,
            q_high: 12.0,
            q_low: 8.0,
            q_close: 11.0,
            volume: 1_000.0,
            amount: 10_000.0,
        }
    }

    #[test]
    fn vwap_and_participation_are_applied() {
        let f = ExecutionConfig {
            deal_price: DealPrice::Vwap,
            slippage_bps: 10.0,
            participation_rate: Some(0.1),
        }
        .fill(&bar(), Side::Buy, 200.0, &Costs::default(), 0.0005)
        .unwrap();
        assert_eq!(f.filled, 100.0);
        assert!((f.price - 10.3436666667).abs() < 1e-8);
        assert!(f.commission >= 5.0);
    }
}
