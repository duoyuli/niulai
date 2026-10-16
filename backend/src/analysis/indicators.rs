//! 技术指标：在一只股票按日期升序的日线上计算，价格一律用等比前复权价，成交量和成交额用原值。
//! 每个指标返回与日线等长的序列，数据不够算出的位置是 NaN。口径见 docs/domains/strategies.md#indicators。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::ma::sma;
use super::prices::DailyBar;

/// 指标周期 n 的上限：巡检和回测按它确定要读多少根历史日线
pub const MAX_WINDOW: u32 = 250;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorId {
    Close,
    Open,
    High,
    Low,
    ChangePct,
    Amplitude,
    Amount,
    Volume,
    Ma,
    Ema,
    MaGap,
    MacdDif,
    MacdDea,
    MacdHist,
    Rsi,
    KdjK,
    KdjD,
    KdjJ,
    BollUpper,
    BollMid,
    BollLower,
    Atr,
    AtrPct,
    VolRatio,
    Momentum,
    HighN,
    LowN,
    PctFromHigh,
    PctFromLow,
}

/// 一个指标及其周期，例如「20 日均线」是 {id: ma, n: 20}
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IndicatorRef {
    pub id: IndicatorId,
    /// 周期；不带周期的指标忽略它，带周期的指标缺省时用目录中的默认值
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
}

/// 指标目录中的一项：给前端展示和白话描述用
#[derive(Serialize, ToSchema, Clone, Copy)]
pub struct IndicatorInfo {
    pub id: IndicatorId,
    /// 名称，带周期的用 {n} 占位，例如「{n} 日均线」
    pub name: &'static str,
    pub group: &'static str,
    /// 单位：price 价格、pct 百分数（5 表示 5%）、ratio 倍数、yuan 元、shares 股、index 指数值
    pub unit: &'static str,
    pub description: &'static str,
    /// 周期的默认值和范围；不带周期时为空
    pub n_default: Option<u32>,
    pub n_min: Option<u32>,
    pub n_max: Option<u32>,
}

const fn info(id: IndicatorId, name: &'static str, group: &'static str, unit: &'static str, description: &'static str, n: Option<(u32, u32, u32)>) -> IndicatorInfo {
    let (n_default, n_min, n_max) = match n {
        Some((d, lo, hi)) => (Some(d), Some(lo), Some(hi)),
        None => (None, None, None),
    };
    IndicatorInfo { id, name, group, unit, description, n_default, n_min, n_max }
}

pub const CATALOG: &[IndicatorInfo] = &[
    info(IndicatorId::Close, "收盘价", "价格", "price", "当天收盘价（前复权）", None),
    info(IndicatorId::Open, "开盘价", "价格", "price", "当天开盘价（前复权）", None),
    info(IndicatorId::High, "最高价", "价格", "price", "当天最高价（前复权）", None),
    info(IndicatorId::Low, "最低价", "价格", "price", "当天最低价（前复权）", None),
    info(IndicatorId::ChangePct, "涨跌幅", "价格", "pct", "相对前一个交易日收盘的涨跌，5 表示涨 5%", None),
    info(IndicatorId::Amplitude, "振幅", "价格", "pct", "（最高 − 最低）/ 前收盘，衡量当天波动有多大", None),
    info(IndicatorId::Amount, "成交额", "量能", "yuan", "当天成交金额（元），常用来排除成交太少的股票", None),
    info(IndicatorId::Volume, "成交量", "量能", "shares", "当天成交股数", None),
    info(IndicatorId::VolRatio, "量比", "量能", "ratio", "当天成交量是前 {n} 天平均成交量的几倍，大于 1 表示放量", Some((5, 2, 60))),
    info(IndicatorId::Ma, "{n} 日均线", "均线", "price", "最近 {n} 个交易日收盘价的平均值", Some((20, 2, MAX_WINDOW))),
    info(IndicatorId::Ema, "{n} 日指数均线", "均线", "price", "越近的价格权重越大的均线，比普通均线反应更快", Some((20, 2, MAX_WINDOW))),
    info(IndicatorId::MaGap, "相对 {n} 日均线偏离", "均线", "pct", "收盘价比 {n} 日均线高出（正）或低于（负）多少，2 表示高 2%", Some((20, 2, MAX_WINDOW))),
    info(IndicatorId::MacdDif, "MACD 快线（DIF）", "MACD", "price", "12 日与 26 日指数均线之差，大于 0 表示短期强于长期", None),
    info(IndicatorId::MacdDea, "MACD 慢线（DEA）", "MACD", "price", "DIF 的 9 日指数均线", None),
    info(IndicatorId::MacdHist, "MACD 柱", "MACD", "price", "2 ×（DIF − DEA），由负转正常被看作转强", None),
    info(IndicatorId::Rsi, "RSI（{n}）", "超买超卖", "index", "0 到 100，反映最近 {n} 天涨的力量占比；高于 70 常被看作超买，低于 30 看作超卖", Some((14, 2, 60))),
    info(IndicatorId::KdjK, "KDJ 的 K 值", "超买超卖", "index", "随机指标 K（9、3、3），0 到 100 附近，高位超买、低位超卖", None),
    info(IndicatorId::KdjD, "KDJ 的 D 值", "超买超卖", "index", "K 值的平滑", None),
    info(IndicatorId::KdjJ, "KDJ 的 J 值", "超买超卖", "index", "3K − 2D，最敏感，可以超出 0 到 100", None),
    info(IndicatorId::BollUpper, "布林带上轨", "波动", "price", "20 日均线加 2 倍标准差，收盘突破上轨表示走势很强", None),
    info(IndicatorId::BollMid, "布林带中轨", "波动", "price", "20 日均线", None),
    info(IndicatorId::BollLower, "布林带下轨", "波动", "price", "20 日均线减 2 倍标准差", None),
    info(IndicatorId::Atr, "ATR（{n}）", "波动", "price", "平均真实波幅：最近 {n} 天每天的平均波动金额", Some((14, 2, 60))),
    info(IndicatorId::AtrPct, "ATR 占股价（{n}）", "波动", "pct", "ATR 除以收盘价，用百分比比较不同价格的股票波动有多大", Some((14, 2, 60))),
    info(IndicatorId::Momentum, "{n} 日涨跌幅", "动量", "pct", "相对 {n} 个交易日前收盘价的涨跌，20 表示涨了 20%", Some((20, 1, MAX_WINDOW))),
    info(IndicatorId::HighN, "{n} 日最高价", "区间", "price", "最近 {n} 个交易日（含当天）的最高价", Some((60, 2, MAX_WINDOW))),
    info(IndicatorId::LowN, "{n} 日最低价", "区间", "price", "最近 {n} 个交易日（含当天）的最低价", Some((60, 2, MAX_WINDOW))),
    info(IndicatorId::PctFromHigh, "距 {n} 日最高收盘", "区间", "pct", "收盘价比最近 {n} 天最高收盘价低多少，0 表示就在最高点，−10 表示低 10%", Some((60, 2, MAX_WINDOW))),
    info(IndicatorId::PctFromLow, "距 {n} 日最低收盘", "区间", "pct", "收盘价比最近 {n} 天最低收盘价高多少，0 表示就在最低点", Some((60, 2, MAX_WINDOW))),
];

pub fn info_of(id: IndicatorId) -> &'static IndicatorInfo {
    CATALOG.iter().find(|i| i.id == id).expect("每个指标都在目录中")
}

impl IndicatorRef {
    pub const fn new(id: IndicatorId, n: Option<u32>) -> Self {
        IndicatorRef { id, n }
    }

    /// 实际使用的周期：不带周期的指标为 0
    pub fn period(&self) -> u32 {
        let i = info_of(self.id);
        match i.n_default {
            Some(d) => self.n.unwrap_or(d),
            None => 0,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let i = info_of(self.id);
        if let (Some(n), Some(lo), Some(hi)) = (self.n, i.n_min, i.n_max)
            && !(lo..=hi).contains(&n)
        {
            return Err(format!("{} 的周期应在 {lo} 到 {hi} 之间", i.name.replace("{n}", "N")));
        }
        Ok(())
    }

    /// 白话名称，例如「20 日均线」
    pub fn label(&self) -> String {
        info_of(self.id).name.replace("{n}", &self.period().to_string())
    }

    /// 算出这个指标需要的历史日线根数（含指数均线的收敛期）
    pub fn warmup(&self) -> u32 {
        match self.id {
            IndicatorId::Ema => self.period() * 3,
            IndicatorId::MacdDif | IndicatorId::MacdDea | IndicatorId::MacdHist => 120,
            IndicatorId::Rsi | IndicatorId::Atr | IndicatorId::AtrPct => self.period() * 4,
            IndicatorId::KdjK | IndicatorId::KdjD | IndicatorId::KdjJ => 40,
            IndicatorId::BollUpper | IndicatorId::BollMid | IndicatorId::BollLower => 20,
            _ => self.period() + 1,
        }
    }
}

/// 股票的静态信息，计算涨跌停价时用
#[derive(Clone, Debug)]
pub struct StockMeta {
    /// main / chinext / star / bse
    pub board: String,
    /// 当前简称含 ST（没有历史 ST 状态，整段历史都按当前状态计算）
    pub is_st: bool,
    pub list_date: Option<NaiveDate>,
}

/// 指标缓存的键：指标和实际周期
type IndicatorKey = (IndicatorId, u32);

/// 一只股票的日线和已经算过的指标
pub struct Frame<'a> {
    pub bars: &'a [DailyBar],
    pub meta: Option<&'a StockMeta>,
    cache: RefCell<HashMap<IndicatorKey, Rc<Vec<f64>>>>,
    /// 信号序列的缓存，键是信号和参数（由 signals 模块生成）
    bools: RefCell<HashMap<String, Rc<Vec<bool>>>>,
    limits: RefCell<Option<Rc<Limits>>>,
}

/// 涨跌停价与当天是否涨停、跌停、炸板
pub struct Limits {
    pub up_price: Vec<f64>,
    pub down_price: Vec<f64>,
    pub limit_up: Vec<bool>,
    pub limit_down: Vec<bool>,
    /// 盘中触及涨停但收盘没封住
    pub broken: Vec<bool>,
}

impl<'a> Frame<'a> {
    pub fn new(bars: &'a [DailyBar], meta: Option<&'a StockMeta>) -> Self {
        Frame { bars, meta, cache: RefCell::new(HashMap::new()), bools: RefCell::new(HashMap::new()), limits: RefCell::new(None) }
    }

    /// 布尔序列（信号）的缓存：同一只股票上同一个信号只算一次
    pub fn memo_bools(&self, key: String, compute: impl FnOnce() -> Vec<bool>) -> Rc<Vec<bool>> {
        if let Some(v) = self.bools.borrow().get(&key) {
            return v.clone();
        }
        let v = Rc::new(compute());
        self.bools.borrow_mut().insert(key, v.clone());
        v
    }

    pub fn len(&self) -> usize {
        self.bars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// 指标序列（与日线等长，不可用的位置是 NaN）
    pub fn get(&self, r: IndicatorRef) -> Rc<Vec<f64>> {
        let key = (r.id, r.period());
        if let Some(v) = self.cache.borrow().get(&key) {
            return v.clone();
        }
        let v = Rc::new(self.compute(r.id, r.period() as usize));
        self.cache.borrow_mut().insert(key, v.clone());
        v
    }

    /// 某一天的值
    pub fn at(&self, r: IndicatorRef, i: usize) -> f64 {
        self.get(r)[i]
    }

    fn col(&self, f: impl Fn(&DailyBar) -> f64) -> Vec<f64> {
        self.bars.iter().map(f).collect()
    }

    fn compute(&self, id: IndicatorId, n: usize) -> Vec<f64> {
        use IndicatorId as I;
        let close = || self.col(|b| b.q_close);
        match id {
            I::Close => close(),
            I::Open => self.col(|b| b.q_open),
            I::High => self.col(|b| b.q_high),
            I::Low => self.col(|b| b.q_low),
            I::Amount => self.col(|b| b.amount),
            I::Volume => self.col(|b| b.volume),
            I::ChangePct => {
                let c = close();
                (0..c.len()).map(|i| if i == 0 { f64::NAN } else { (c[i] / c[i - 1] - 1.0) * 100.0 }).collect()
            }
            I::Amplitude => {
                let b = self.bars;
                (0..b.len()).map(|i| if i == 0 { f64::NAN } else { (b[i].q_high - b[i].q_low) / b[i - 1].q_close * 100.0 }).collect()
            }
            I::Ma => sma(&close(), n).into_iter().map(|v| v.unwrap_or(f64::NAN)).collect(),
            I::Ema => ema(&close(), n),
            I::MaGap => {
                let c = close();
                let m = self.get(IndicatorRef::new(I::Ma, Some(n as u32)));
                c.iter().zip(m.iter()).map(|(c, m)| (c / m - 1.0) * 100.0).collect()
            }
            I::MacdDif => {
                let c = close();
                ema(&c, 12).iter().zip(ema(&c, 26)).map(|(a, b)| a - b).collect()
            }
            I::MacdDea => ema(&self.get(IndicatorRef::new(I::MacdDif, None)), 9),
            I::MacdHist => {
                let dif = self.get(IndicatorRef::new(I::MacdDif, None));
                let dea = self.get(IndicatorRef::new(I::MacdDea, None));
                dif.iter().zip(dea.iter()).map(|(a, b)| 2.0 * (a - b)).collect()
            }
            I::Rsi => rsi(&close(), n),
            I::KdjK | I::KdjD | I::KdjJ => {
                let (k, d, j) = kdj(self.bars, 9);
                match id {
                    I::KdjK => k,
                    I::KdjD => d,
                    _ => j,
                }
            }
            I::BollMid => self.get(IndicatorRef::new(I::Ma, Some(20))).to_vec(),
            I::BollUpper | I::BollLower => {
                let c = close();
                let mid = self.get(IndicatorRef::new(I::Ma, Some(20)));
                let sd = rolling_std(&c, 20);
                let k = if id == I::BollUpper { 2.0 } else { -2.0 };
                mid.iter().zip(sd).map(|(m, s)| m + k * s).collect()
            }
            I::Atr => atr(self.bars, n),
            I::AtrPct => {
                let a = self.get(IndicatorRef::new(I::Atr, Some(n as u32)));
                a.iter().zip(self.bars).map(|(a, b)| a / b.q_close * 100.0).collect()
            }
            I::VolRatio => {
                let v = self.col(|b| b.volume);
                (0..v.len())
                    .map(|i| {
                        if i < n {
                            return f64::NAN;
                        }
                        let avg = v[i - n..i].iter().sum::<f64>() / n as f64;
                        if avg > 0.0 { v[i] / avg } else { f64::NAN }
                    })
                    .collect()
            }
            I::Momentum => {
                let c = close();
                (0..c.len()).map(|i| if i < n { f64::NAN } else { (c[i] / c[i - n] - 1.0) * 100.0 }).collect()
            }
            I::HighN => rolling(&self.col(|b| b.q_high), n, f64::max),
            I::LowN => rolling(&self.col(|b| b.q_low), n, f64::min),
            I::PctFromHigh => {
                let c = close();
                let hi = rolling(&c, n, f64::max);
                c.iter().zip(hi).map(|(c, h)| (c / h - 1.0) * 100.0).collect()
            }
            I::PctFromLow => {
                let c = close();
                let lo = rolling(&c, n, f64::min);
                c.iter().zip(lo).map(|(c, l)| (c / l - 1.0) * 100.0).collect()
            }
        }
    }

    // @project-doc docs/domains/strategies.md#limits
    /// 涨跌停价按交易所规则推算：涨跌幅限制按板块和 ST 确定，基准是前一交易日的收盘价（除权日按除权参考价），
    /// 四舍五入到分。上市后前 5 个交易日没有涨跌幅限制，不判断
    pub fn limits(&self) -> Rc<Limits> {
        if let Some(l) = self.limits.borrow().as_ref() {
            return l.clone();
        }
        let b = self.bars;
        let n = b.len();
        let mut l = Limits { up_price: vec![f64::NAN; n], down_price: vec![f64::NAN; n], limit_up: vec![false; n], limit_down: vec![false; n], broken: vec![false; n] };
        for i in 1..n {
            if self.ipo_period(i) {
                continue;
            }
            let pct = self.limit_pct(b[i].date);
            // 前复权价之比消去了除权：今天的原价 × 昨天和今天的前复权价之比 = 昨天的收盘价按除权调整后的值
            let base = b[i].close * b[i - 1].q_close / b[i].q_close;
            let up = round2(base * (1.0 + pct));
            let down = round2(base * (1.0 - pct));
            l.up_price[i] = up;
            l.down_price[i] = down;
            l.limit_up[i] = b[i].close >= up - 0.005;
            l.limit_down[i] = b[i].close <= down + 0.005;
            l.broken[i] = b[i].high >= up - 0.005 && !l.limit_up[i];
        }
        let l = Rc::new(l);
        *self.limits.borrow_mut() = Some(l.clone());
        l
    }

    fn ipo_period(&self, i: usize) -> bool {
        match self.meta.and_then(|m| m.list_date) {
            // 本库第一根日线就是上市首日时，前 5 根没有涨跌幅限制
            Some(d) => self.bars[0].date <= d + chrono::Duration::days(3) && i < 5,
            None => false,
        }
    }

    fn limit_pct(&self, date: NaiveDate) -> f64 {
        let Some(m) = self.meta else { return 0.10 };
        let chinext_reform = NaiveDate::from_ymd_opt(2020, 8, 24).unwrap();
        match m.board.as_str() {
            "bse" => 0.30,
            "star" => 0.20,
            "chinext" if date >= chinext_reform => 0.20,
            _ if m.is_st => 0.05,
            _ => 0.10,
        }
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// 指数均线，第一天的值取当天收盘
pub fn ema(values: &[f64], n: usize) -> Vec<f64> {
    let alpha = 2.0 / (n as f64 + 1.0);
    let mut out = Vec::with_capacity(values.len());
    let mut prev = f64::NAN;
    for &v in values {
        prev = if prev.is_nan() { v } else { alpha * v + (1.0 - alpha) * prev };
        out.push(prev);
    }
    out
}

/// 平滑移动平均 SMA(X, n, 1)：Y = (X + (n − 1) × Y') / n，与通达信、同花顺的 RSI、KDJ 口径一致
fn smooth(values: &[f64], n: usize, seed: f64) -> Vec<f64> {
    let mut prev = seed;
    values
        .iter()
        .map(|&v| {
            if v.is_nan() {
                return f64::NAN;
            }
            prev = if prev.is_nan() { v } else { (v + (n as f64 - 1.0) * prev) / n as f64 };
            prev
        })
        .collect()
}

fn rsi(close: &[f64], n: usize) -> Vec<f64> {
    if close.is_empty() {
        return vec![];
    }
    let mut up = vec![f64::NAN];
    let mut abs = vec![f64::NAN];
    for i in 1..close.len() {
        let d = close[i] - close[i - 1];
        up.push(d.max(0.0));
        abs.push(d.abs());
    }
    let su = smooth(&up, n, f64::NAN);
    let sa = smooth(&abs, n, f64::NAN);
    su.iter()
        .zip(sa)
        .enumerate()
        .map(|(i, (u, a))| if i < n { f64::NAN } else if a > 0.0 { u / a * 100.0 } else { 50.0 })
        .collect()
}

fn kdj(bars: &[DailyBar], n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let hi = rolling(&bars.iter().map(|b| b.q_high).collect::<Vec<_>>(), n, f64::max);
    let lo = rolling(&bars.iter().map(|b| b.q_low).collect::<Vec<_>>(), n, f64::min);
    let rsv: Vec<f64> = bars
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let range = hi[i] - lo[i];
            if hi[i].is_nan() { f64::NAN } else if range > 0.0 { (b.q_close - lo[i]) / range * 100.0 } else { 50.0 }
        })
        .collect();
    let k = smooth(&rsv, 3, 50.0);
    let d = smooth(&k, 3, 50.0);
    let j = k.iter().zip(d.iter()).map(|(k, d)| 3.0 * k - 2.0 * d).collect();
    (k, d, j)
}

fn atr(bars: &[DailyBar], n: usize) -> Vec<f64> {
    let tr: Vec<f64> = (0..bars.len())
        .map(|i| {
            let b = &bars[i];
            if i == 0 {
                return b.q_high - b.q_low;
            }
            let pc = bars[i - 1].q_close;
            (b.q_high - b.q_low).max((b.q_high - pc).abs()).max((b.q_low - pc).abs())
        })
        .collect();
    sma(&tr, n).into_iter().map(|v| v.unwrap_or(f64::NAN)).collect()
}

/// 最近 n 个值（含当天）的样本标准差
fn rolling_std(values: &[f64], n: usize) -> Vec<f64> {
    (0..values.len())
        .map(|i| {
            if i + 1 < n {
                return f64::NAN;
            }
            let w = &values[i + 1 - n..=i];
            let mean = w.iter().sum::<f64>() / n as f64;
            (w.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt()
        })
        .collect()
}

/// 最近 n 个值（含当天）按 f 归约，例如最高值、最低值
fn rolling(values: &[f64], n: usize, f: fn(f64, f64) -> f64) -> Vec<f64> {
    (0..values.len())
        .map(|i| if i + 1 < n { f64::NAN } else { values[i + 1 - n..=i].iter().copied().reduce(f).unwrap_or(f64::NAN) })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn bars_from(closes: &[f64]) -> Vec<DailyBar> {
        let d0 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| DailyBar {
                date: d0 + chrono::Duration::days(i as i64),
                open: c,
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                q_open: c,
                q_high: c * 1.01,
                q_low: c * 0.99,
                q_close: c,
                volume: 1000.0 * (i as f64 + 1.0),
                amount: c * 1000.0 * (i as f64 + 1.0),
            })
            .collect()
    }

    #[test]
    fn moving_averages_and_momentum() {
        let b = bars_from(&[10.0, 11.0, 12.0, 13.0, 14.0]);
        let f = Frame::new(&b, None);
        let ma3 = f.get(IndicatorRef::new(IndicatorId::Ma, Some(3)));
        assert!(ma3[1].is_nan());
        assert!((ma3[4] - 13.0).abs() < 1e-12);
        assert!((f.at(IndicatorRef::new(IndicatorId::Momentum, Some(2)), 4) - (14.0 / 12.0 - 1.0) * 100.0).abs() < 1e-9);
        assert!((f.at(IndicatorRef::new(IndicatorId::ChangePct, None), 1) - 10.0).abs() < 1e-9);
        // 成交量 1000、2000、3000、4000、5000：第 5 天是前 3 天平均 3000 的 5/3 倍
        assert!((f.at(IndicatorRef::new(IndicatorId::VolRatio, Some(3)), 4) - 5.0 / 3.0).abs() < 1e-9);
        assert!((f.at(IndicatorRef::new(IndicatorId::PctFromHigh, Some(3)), 4)).abs() < 1e-9);
    }

    #[test]
    fn rsi_is_100_when_only_rising_and_ema_starts_at_first_value() {
        let b = bars_from(&(1..=30).map(|v| v as f64).collect::<Vec<_>>());
        let f = Frame::new(&b, None);
        assert!((f.at(IndicatorRef::new(IndicatorId::Rsi, Some(14)), 29) - 100.0).abs() < 1e-9);
        assert_eq!(f.at(IndicatorRef::new(IndicatorId::Ema, Some(5)), 0), 1.0);
        assert!(f.at(IndicatorRef::new(IndicatorId::MacdDif, None), 29) > 0.0, "持续上涨时 DIF 为正");
    }

    #[test]
    fn limit_up_follows_board_rules() {
        let mut b = bars_from(&[10.0, 11.0, 12.1]);
        b[2].high = 12.1;
        let main = StockMeta { board: "main".into(), is_st: false, list_date: None };
        let f = Frame::new(&b, Some(&main));
        let l = f.limits();
        assert!(l.limit_up[1] && l.limit_up[2], "主板涨 10% 封涨停");
        let st = StockMeta { board: "main".into(), is_st: true, list_date: None };
        let f = Frame::new(&b, Some(&st));
        assert!((f.limits().up_price[1] - 10.5).abs() < 1e-9, "ST 涨跌幅 5%");
        let star = StockMeta { board: "star".into(), is_st: false, list_date: None };
        let f = Frame::new(&b, Some(&star));
        assert!(!f.limits().limit_up[1], "科创板 20% 才涨停");
    }
}
