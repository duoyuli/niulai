//! 可复用的横截面因子基础。因子只描述数值特征，组合和账户决定仓位与执行。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::indicators::{Frame, IndicatorId, IndicatorRef};

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FactorId {
    Momentum,
    Reversal,
    Volatility,
    Liquidity,
}

#[derive(Serialize, ToSchema, Clone, Copy)]
pub struct FactorInfo {
    pub id: FactorId,
    pub name: &'static str,
    pub description: &'static str,
    pub higher_is_better: bool,
}

pub const CATALOG: &[FactorInfo] = &[
    FactorInfo {
        id: FactorId::Momentum,
        name: "动量",
        description: "过去 20 个交易日的涨跌幅",
        higher_is_better: true,
    },
    FactorInfo {
        id: FactorId::Reversal,
        name: "反转",
        description: "过去 5 个交易日跌幅的反向值",
        higher_is_better: true,
    },
    FactorInfo {
        id: FactorId::Volatility,
        name: "波动率",
        description: "ATR 占收盘价的比例的反向值",
        higher_is_better: true,
    },
    FactorInfo {
        id: FactorId::Liquidity,
        name: "流动性",
        description: "成交额的对数",
        higher_is_better: true,
    },
];

pub fn value(f: &Frame, id: FactorId, i: usize) -> f64 {
    match id {
        FactorId::Momentum => f.at(IndicatorRef::new(IndicatorId::Momentum, Some(20)), i),
        FactorId::Reversal => -f.at(IndicatorRef::new(IndicatorId::Momentum, Some(5)), i),
        FactorId::Volatility => -f.at(IndicatorRef::new(IndicatorId::AtrPct, Some(14)), i),
        FactorId::Liquidity => f.at(IndicatorRef::new(IndicatorId::Amount, None), i).ln(),
    }
}

/// 将横截面数值转换为 0 到 1 的百分位排名；NaN 不参与排名。
pub fn percentile(values: &[f64]) -> Vec<Option<f64>> {
    let mut finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    finite.sort_by(f64::total_cmp);
    values
        .iter()
        .map(|v| {
            if !v.is_finite() || finite.is_empty() {
                None
            } else {
                let rank = finite.partition_point(|x| *x <= *v);
                Some(
                    (rank.saturating_sub(1) as f64)
                        / (finite.len().saturating_sub(1).max(1) as f64),
                )
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_ignores_nan() {
        let p = percentile(&[1.0, 3.0, f64::NAN, 2.0]);
        assert_eq!(p[0], Some(0.0));
        assert_eq!(p[1], Some(1.0));
        assert_eq!(p[2], None);
    }
}
