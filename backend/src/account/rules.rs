//! 账户的调仓规则：按哪些因子给全市场排名、持有几只、缓冲区多大，以及留多少现金、追高上限和 ST 怎么办。
//! 默认值是研究记录里的组合 E。规则见 docs/domains/accounts.md#rules。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::analysis::factors::FactorId;

/// 一个因子和它的权重（按因子的预期方向合成，负数表示反过来用）
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq)]
pub struct FactorWeight {
    pub factor: FactorId,
    pub weight: f64,
}

/// 一个账户的全部规则
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct Rules {
    pub factors: Vec<FactorWeight>,
    /// 持有几只
    pub top_n: u32,
    /// 已持有的股票排名在 top_n × (1 + buffer) 以内时继续拿，减少换手
    pub buffer: f64,
    /// 至少留多少现金（占总资产的百分比）
    pub min_cash_pct: f64,
    /// 第二天开盘比参考价高多少（%）就不买
    pub chase_limit_pct: f64,
    /// 持有的股票被 ST 时卖出
    pub sell_st: bool,
}

impl Default for Rules {
    // @project-doc docs/domains/accounts.md#rules
    /// 组合 E：波动率、特质波动率、单季利润增速、应计利润、成交额等权，持有 30 只，缓冲 1.5
    fn default() -> Self {
        let w = |factor| FactorWeight {
            factor,
            weight: 1.0,
        };
        Rules {
            factors: vec![
                w(FactorId::Volatility),
                w(FactorId::IdioVol),
                w(FactorId::ProfitGrowth),
                w(FactorId::Accruals),
                w(FactorId::Liquidity),
            ],
            top_n: 30,
            buffer: 1.5,
            min_cash_pct: 0.0,
            chase_limit_pct: 5.0,
            sell_st: true,
        }
    }
}

impl Rules {
    /// 排名在这个名次以内（从 1 算）的持仓继续拿
    pub fn keep_limit(&self) -> usize {
        ((self.top_n as f64) * (1.0 + self.buffer)).ceil() as usize
    }

    /// 参数是否在合理范围内
    pub fn validate(&self) -> Result<(), String> {
        if self.factors.is_empty() || self.factors.len() > 12 {
            return Err("至少 1 个因子，最多 12 个".into());
        }
        let mut seen = std::collections::HashSet::new();
        if self.factors.iter().any(|f| !seen.insert(f.factor)) {
            return Err("因子不能重复".into());
        }
        if self
            .factors
            .iter()
            .any(|f| !f.weight.is_finite() || f.weight == 0.0 || f.weight.abs() > 10.0)
        {
            return Err("因子权重应是 -10 到 10 之间、不为 0 的数".into());
        }
        if !(5..=50).contains(&self.top_n) {
            return Err("持有只数应在 5 到 50 之间".into());
        }
        let range = |v: f64, lo: f64, hi: f64, what: &str| {
            if v.is_finite() && (lo..=hi).contains(&v) {
                Ok(())
            } else {
                Err(format!("{what}应在 {lo} 到 {hi} 之间"))
            }
        };
        range(self.buffer, 0.0, 2.0, "缓冲区")?;
        range(self.min_cash_pct, 0.0, 90.0, "保留现金（%）")?;
        range(self.chase_limit_pct, 0.0, 20.0, "追高上限（%）")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_combo_e_and_ranges_are_checked() {
        let r = Rules::default();
        r.validate().unwrap();
        assert_eq!((r.top_n, r.keep_limit()), (30, 75));
        assert!(
            Rules {
                top_n: 60,
                ..r.clone()
            }
            .validate()
            .is_err()
        );
        let mut dup = r.clone();
        dup.factors.push(dup.factors[0]);
        assert!(dup.validate().is_err());
    }
}
