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

/// 单个训练或测试窗口的最大自然日数（约 10 年）。
pub const MAX_WINDOW_DAYS: i64 = 3650;
/// embargo 的最大天数。
pub const MAX_EMBARGO_DAYS: i64 = 365;
/// 一次请求最多生成多少个窗口。每个窗口要跑两次回测，必须有上限。
pub const MAX_WINDOWS: usize = 200;

/// 生成滚动样本外窗口。测试区间永远晚于训练区间，并留出 embargo 天数。
/// 参数超出合理范围时返回错误，避免日期运算溢出或窗口数失控。
pub fn walk_forward_windows(
    start: NaiveDate,
    end: NaiveDate,
    train_days: i64,
    test_days: i64,
    embargo_days: i64,
) -> Result<Vec<ValidationWindow>, String> {
    if start >= end {
        return Err("开始日期要早于结束日期".into());
    }
    if train_days <= 0 || test_days <= 0 {
        return Err("训练天数和测试天数必须大于 0".into());
    }
    if train_days > MAX_WINDOW_DAYS || test_days > MAX_WINDOW_DAYS {
        return Err(format!("训练天数和测试天数都不能超过 {MAX_WINDOW_DAYS} 天"));
    }
    if !(0..=MAX_EMBARGO_DAYS).contains(&embargo_days) {
        return Err(format!("embargo_days 应在 0 到 {MAX_EMBARGO_DAYS} 之间"));
    }
    let offset = train_days
        .checked_add(embargo_days)
        .ok_or_else(|| "日期范围溢出".to_string())?;
    let mut test_start = start
        .checked_add_signed(chrono::Duration::days(offset))
        .ok_or_else(|| "日期范围溢出".to_string())?;
    let mut out = vec![];
    while test_start <= end {
        if out.len() >= MAX_WINDOWS {
            return Err(format!(
                "验证窗口超过 {MAX_WINDOWS} 个，请缩短区间或加大测试天数"
            ));
        }
        let test_end = test_start
            .checked_add_signed(chrono::Duration::days(test_days - 1))
            .ok_or_else(|| "日期范围溢出".to_string())?
            .min(end);
        let train_end = test_start
            .checked_sub_signed(chrono::Duration::days(embargo_days + 1))
            .ok_or_else(|| "日期范围溢出".to_string())?;
        let train_start = train_end
            .checked_sub_signed(chrono::Duration::days(train_days - 1))
            .ok_or_else(|| "日期范围溢出".to_string())?;
        if train_start >= start && train_end < test_start {
            out.push(ValidationWindow {
                train_start,
                train_end,
                test_start,
                test_end,
            });
        }
        test_start = test_end
            .checked_add_signed(chrono::Duration::days(1))
            .ok_or_else(|| "日期范围溢出".to_string())?;
    }
    Ok(out)
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
        let windows = walk_forward_windows(s, e, 60, 20, 5).unwrap();
        assert!(!windows.is_empty());
        assert!(windows.iter().all(|w| w.train_end < w.test_start));
    }

    #[test]
    fn walk_forward_windows_reject_unbounded_parameters() {
        let s = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let e = NaiveDate::from_ymd_opt(2021, 1, 1).unwrap();
        assert!(walk_forward_windows(s, e, 0, 20, 5).is_err());
        assert!(walk_forward_windows(s, e, 60, 0, 5).is_err());
        assert!(walk_forward_windows(s, e, MAX_WINDOW_DAYS + 1, 20, 5).is_err());
        assert!(walk_forward_windows(s, e, 60, 20, -1).is_err());
        assert!(walk_forward_windows(s, e, 60, 20, MAX_EMBARGO_DAYS + 1).is_err());
        // 每天一个测试窗口，区间太长时直接拒绝
        assert!(walk_forward_windows(s, e, 60, 1, 0).is_err());
    }
}
