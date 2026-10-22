//! 回测研究与可复现性元数据。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, Default)]
pub struct RiskMetrics {
    /// 年化波动率。
    pub annual_volatility: Option<f64>,
    /// 无风险利率为 0 的年化 Sharpe。
    pub sharpe: Option<f64>,
    /// 只使用负收益的年化 Sortino。
    pub sortino: Option<f64>,
    /// 年化收益 / 最大回撤。
    pub calmar: Option<f64>,
    /// 每日平均收益。
    pub average_daily_return: Option<f64>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct BacktestManifest {
    pub strategy_definition_hash: String,
    pub data_start: NaiveDate,
    pub data_end: NaiveDate,
    pub price_basis: String,
    pub execution_model: String,
    pub point_in_time_universe: bool,
    pub derived_versions: std::collections::BTreeMap<String, String>,
    pub trial_count: u64,
    pub validation_kind: String,
    pub coverage: Coverage,
    pub delisted_policy: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, Default)]
pub struct Coverage {
    pub instruments: u64,
    pub observed_bars: u64,
    pub missing_instruments: u64,
    pub unknown_state_instruments: u64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug)]
pub struct ValidationWindow {
    pub train_start: NaiveDate,
    pub train_end: NaiveDate,
    pub test_start: NaiveDate,
    pub test_end: NaiveDate,
}

/// 生成滚动样本外窗口。测试区间永远晚于训练区间，并留出 embargo 天数。
pub fn walk_forward_windows(
    start: NaiveDate,
    end: NaiveDate,
    train_days: i64,
    test_days: i64,
    embargo_days: i64,
) -> Vec<ValidationWindow> {
    if train_days <= 0 || test_days <= 0 || start >= end {
        return vec![];
    }
    let mut out = vec![];
    let mut test_start = start + chrono::Duration::days(train_days + embargo_days);
    while test_start <= end {
        let test_end = (test_start + chrono::Duration::days(test_days - 1)).min(end);
        let train_end = test_start - chrono::Duration::days(embargo_days + 1);
        let train_start = train_end - chrono::Duration::days(train_days - 1);
        if train_start >= start && train_end < test_start {
            out.push(ValidationWindow {
                train_start,
                train_end,
                test_start,
                test_end,
            });
        }
        test_start = test_end + chrono::Duration::days(1);
    }
    out
}

pub fn definition_hash<T: Serialize>(value: &T) -> String {
    let json = serde_json::to_vec(value).unwrap_or_default();
    Sha256::digest(json)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn risk_metrics(values: &[f64], annual_return: Option<f64>, max_drawdown: f64) -> RiskMetrics {
    if values.len() < 2 {
        return RiskMetrics::default();
    }
    let returns: Vec<f64> = values
        .windows(2)
        .filter(|w| w[0] > 0.0)
        .map(|w| w[1] / w[0] - 1.0)
        .filter(|v| v.is_finite())
        .collect();
    if returns.len() < 2 {
        return RiskMetrics::default();
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance =
        returns.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
    let vol = variance.sqrt();
    let downside = returns
        .iter()
        .filter(|v| **v < 0.0)
        .map(|v| v.powi(2))
        .sum::<f64>()
        / returns.len() as f64;
    let downside_dev = downside.sqrt();
    let annual_volatility = (vol.is_finite() && vol > 0.0).then_some(vol * 252.0_f64.sqrt());
    let sharpe = annual_volatility
        .map(|_| mean / vol * 252.0_f64.sqrt())
        .filter(|v| v.is_finite());
    let sortino = (downside_dev > 0.0)
        .then_some(mean / downside_dev * 252.0_f64.sqrt())
        .filter(|v| v.is_finite());
    let calmar = annual_return
        .filter(|_| max_drawdown > 0.0)
        .map(|v| v / max_drawdown)
        .filter(|v| v.is_finite());
    RiskMetrics {
        annual_volatility,
        sharpe,
        sortino,
        calmar,
        average_daily_return: Some(mean),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_metrics_use_daily_returns() {
        let r = risk_metrics(&[100.0, 110.0, 99.0, 108.9], Some(0.1), 0.1);
        assert!(r.sharpe.is_some());
        assert!(r.sortino.is_some());
        assert_eq!(r.calmar, Some(1.0));
    }

    #[test]
    fn walk_forward_windows_have_embargo() {
        let s = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let e = NaiveDate::from_ymd_opt(2021, 1, 1).unwrap();
        let windows = walk_forward_windows(s, e, 60, 20, 5);
        assert!(!windows.is_empty());
        assert!(windows.iter().all(|w| w.train_end < w.test_start));
    }
}
