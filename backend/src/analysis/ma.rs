//! 简单移动平均与金叉死叉。单只股票、全市场扫描、自选股诊断和回测都用这里的同一套计算，结果一致。

use serde::Serialize;
use utoipa::ToSchema;

/// 均线交叉判断的容差：|MA快 − MA慢| / MA慢 小于这个值时视为相等，避免浮点误差制造假交叉
pub const CROSS_EPS: f64 = 1e-9;

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CrossSignal {
    /// 快线上穿慢线
    Golden,
    /// 快线下穿慢线
    Death,
}

/// 快慢线周期是否合法；不合法时返回给用户看的原因
pub fn validate_windows(fast: i32, slow: i32) -> Result<(usize, usize), String> {
    if !(2..=250).contains(&fast) || !(2..=250).contains(&slow) || fast >= slow {
        return Err("fast 和 slow 应在 2–250 之间，且 fast < slow".into());
    }
    Ok((fast as usize, slow as usize))
}

/// 快线相对慢线的状态：1 在上，−1 在下，0 视为相等
pub fn state_of(fast: f64, slow: f64) -> i8 {
    let rel = (fast - slow) / slow.abs().max(f64::MIN_POSITIVE);
    if rel > CROSS_EPS {
        1
    } else if rel < -CROSS_EPS {
        -1
    } else {
        0
    }
}

// @project-doc docs/domains/ma_signals.md#cross_rule
/// 根据每天的快慢线计算交叉信号。快慢线相等的日子不改变状态：
/// 从"上次非零状态"翻转到相反状态时才算交叉。
pub fn cross_signals(series: &[(f64, f64)]) -> Vec<Option<CrossSignal>> {
    let mut last = 0i8;
    series
        .iter()
        .map(|&(f, s)| {
            let st = state_of(f, s);
            let sig = match (last, st) {
                (-1, 1) => Some(CrossSignal::Golden),
                (1, -1) => Some(CrossSignal::Death),
                _ => None,
            };
            if st != 0 {
                last = st;
            }
            sig
        })
        .collect()
}

/// 补偿求和（Kahan）：滑动窗口不断加减，普通累加的舍入误差会越积越大
#[derive(Default)]
struct KahanSum {
    sum: f64,
    c: f64,
}

impl KahanSum {
    fn add(&mut self, x: f64) {
        let y = x - self.c;
        let t = self.sum + y;
        self.c = (t - self.sum) - y;
        self.sum = t;
    }
}

/// n 日简单移动平均，O(len)。前 n − 1 个位置没有完整窗口，为 None
pub fn sma(values: &[f64], n: usize) -> Vec<Option<f64>> {
    let mut acc = KahanSum::default();
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            acc.add(v);
            if i >= n {
                acc.add(-values[i - n]);
            }
            (i + 1 >= n).then(|| acc.sum / n as f64)
        })
        .collect()
}

/// 某一天的均线与信号
#[derive(Clone, Copy, Debug)]
pub struct MaRow {
    pub ma_fast: f64,
    pub ma_slow: f64,
    pub signal: Option<CrossSignal>,
}

// @project-doc docs/domains/ma_signals.md#ma_definition
/// 对按日期升序的收盘价计算快慢线和交叉信号。前 slow − 1 根没有完整的慢线窗口，为 None，
/// 也不参与交叉判定（与只看 K 线数量达到 slow 根之后的点等价）
pub fn ma_cross(closes: &[f64], fast: usize, slow: usize) -> Vec<Option<MaRow>> {
    let f = sma(closes, fast);
    let s = sma(closes, slow);
    let start = slow.saturating_sub(1).min(closes.len());
    let pairs: Vec<(f64, f64)> = (start..closes.len()).map(|i| (f[i].unwrap_or(f64::NAN), s[i].unwrap_or(f64::NAN))).collect();
    let signals = cross_signals(&pairs);
    let mut out = vec![None; start];
    out.extend(pairs.into_iter().zip(signals).map(|((ma_fast, ma_slow), signal)| Some(MaRow { ma_fast, ma_slow, signal })));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crosses_ignore_equal_days_and_float_noise() {
        let s = [(1.0, 2.0), (2.0, 2.0), (3.0, 2.0), (3.0, 3.0 + 1e-16), (1.0, 3.0)];
        assert_eq!(cross_signals(&s), vec![None, None, Some(CrossSignal::Golden), None, Some(CrossSignal::Death)]);
    }

    #[test]
    fn sma_matches_direct_average() {
        let v: Vec<f64> = (0..5000).map(|i| 1000.0 + ((i * 7919) % 613) as f64 * 0.37).collect();
        let m = sma(&v, 30);
        assert!(m[28].is_none());
        for i in 29..v.len() {
            let direct = v[i + 1 - 30..=i].iter().sum::<f64>() / 30.0;
            assert!((m[i].unwrap() - direct).abs() / direct < 1e-13, "第 {i} 个点误差过大");
        }
    }

    #[test]
    fn ma_cross_starts_at_full_slow_window() {
        let closes = [5.0, 4.0, 3.0, 2.0, 3.0, 5.0, 8.0];
        let rows = ma_cross(&closes, 2, 3);
        assert!(rows[0].is_none() && rows[1].is_none());
        let sigs: Vec<_> = rows.iter().flatten().map(|r| r.signal).collect();
        // 快线 3.5 2.5 2.5 4 6.5，慢线 4 3 2.67 3.33 5.33：第 6 根上穿
        assert_eq!(sigs, vec![None, None, None, Some(CrossSignal::Golden), None]);
    }
}
