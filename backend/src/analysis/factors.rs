//! 横截面因子：目录与逐只证券的取值。因子只描述数值特征，检验、合成和组合回测在 `factor_lab`。
//! 每个因子只用当天收盘以前已经知道的数据；财务因子按披露截止日推断可用日，见 docs/domains/factor_research.md。

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::prices::DailyBar;

// @project-doc docs/domains/factor_research.md#catalog
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FactorId {
    Momentum,
    Reversal,
    Momentum12m,
    Volatility,
    IdioVol,
    MaxReturn,
    Liquidity,
    AbnormalTurnover,
    Illiquidity,
    Roe,
    ProfitGrowth,
    Accruals,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FactorCategory {
    /// 价量：只用日线
    PriceVolume,
    /// 财务：按披露截止日推断可用日
    Fundamental,
}

#[derive(Serialize, ToSchema, Clone, Copy)]
pub struct FactorInfo {
    pub id: FactorId,
    pub name: &'static str,
    pub description: &'static str,
    pub category: FactorCategory,
    /// A 股历史研究中数值越大、后续收益越高为真；合成和分组的「好的一端」按它定方向
    pub higher_is_better: bool,
    /// 为什么可能有效（或无效）的白话依据
    pub rationale: &'static str,
}

pub const CATALOG: &[FactorInfo] = &[
    FactorInfo {
        id: FactorId::Reversal,
        name: "短期反转",
        description: "过去 5 个交易日涨跌幅的相反数",
        category: FactorCategory::PriceVolume,
        higher_is_better: true,
        rationale: "A 股散户占比高、追涨杀跌明显，短期涨多的往往回吐、跌多的往往修复",
    },
    FactorInfo {
        id: FactorId::Momentum,
        name: "20 日动量",
        description: "过去 20 个交易日的涨跌幅",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "海外市场的动量效应在 A 股多数时期不成立，20 日涨幅大的股票之后反而偏弱",
    },
    FactorInfo {
        id: FactorId::Momentum12m,
        name: "12-1 月动量",
        description: "250 个交易日前到 20 个交易日前的涨跌幅（跳过最近一个月）",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "经典的中期动量；A 股研究普遍发现它不显著或为负，保留用于对照",
    },
    FactorInfo {
        id: FactorId::Volatility,
        name: "波动率",
        description: "过去 20 个交易日日收益率的标准差",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "低波动异象：高波动股票被当作彩票追捧，长期收益反而偏低",
    },
    FactorInfo {
        id: FactorId::IdioVol,
        name: "特质波动率",
        description: "过去 20 个交易日对全市场等权收益回归后残差的标准差",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "剔除大盘涨跌后的个股波动；A 股文献中是最稳健的定价错误特征之一",
    },
    FactorInfo {
        id: FactorId::MaxReturn,
        name: "最大日涨幅",
        description: "过去 20 个交易日中最大的单日涨幅",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "彩票效应：近期出现过大涨（常见于涨停）的股票被高估，之后收益偏低",
    },
    FactorInfo {
        id: FactorId::Liquidity,
        name: "成交额（规模代理）",
        description: "过去 20 个交易日平均成交额的对数",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "没有历史股本时用成交额近似规模；小盘溢价在 A 股明显，但其中一部分是壳价值",
    },
    FactorInfo {
        id: FactorId::AbnormalTurnover,
        name: "异常成交",
        description: "近 5 日平均成交额与近 120 日平均成交额之比的对数",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "成交突然放大说明关注度和情绪过热，A 股中之后收益通常偏低",
    },
    FactorInfo {
        id: FactorId::Illiquidity,
        name: "非流动性（Amihud）",
        description: "过去 20 个交易日每亿元成交额带来的平均绝对涨跌幅",
        category: FactorCategory::PriceVolume,
        higher_is_better: true,
        rationale: "越难交易的股票要求越高的补偿收益；与规模高度相关",
    },
    FactorInfo {
        id: FactorId::Roe,
        name: "ROE（TTM）",
        description: "最近四个季度归母净利润除以最新股东权益",
        category: FactorCategory::Fundamental,
        higher_is_better: true,
        rationale: "盈利能力强的公司长期收益更高（质量因子）",
    },
    FactorInfo {
        id: FactorId::ProfitGrowth,
        name: "单季利润增速",
        description: "最新单季归母净利润相对去年同季的增长率",
        category: FactorCategory::Fundamental,
        higher_is_better: true,
        rationale: "业绩加速的公司在公告后仍有持续的超额收益（盈余公告后漂移）",
    },
    FactorInfo {
        id: FactorId::Accruals,
        name: "应计利润",
        description: "最近四个季度（归母净利润 − 经营现金流）除以总资产",
        category: FactorCategory::Fundamental,
        higher_is_better: false,
        rationale: "利润里现金含量低（应计多）的公司盈利质量差，之后收益偏低",
    },
];

pub fn info(id: FactorId) -> &'static FactorInfo {
    CATALOG
        .iter()
        .find(|f| f.id == id)
        .expect("目录覆盖全部因子")
}

/// 计算价量因子最多需要往前看多少根日线
pub const WARMUP_BARS: usize = 260;

/// 一只股票的价量因子取值器：前缀和让每个取值是 O(1) 或 O(窗口)
pub struct PriceFactors<'a> {
    bars: &'a [DailyBar],
    /// 第 i 个元素是前 i 根日线成交额之和
    amount_sum: Vec<f64>,
}

impl<'a> PriceFactors<'a> {
    pub fn new(bars: &'a [DailyBar]) -> Self {
        let mut amount_sum = Vec::with_capacity(bars.len() + 1);
        amount_sum.push(0.0);
        let mut acc = 0.0;
        for b in bars {
            acc += b.amount.max(0.0);
            amount_sum.push(acc);
        }
        PriceFactors { bars, amount_sum }
    }

    fn ret(&self, i: usize) -> f64 {
        let (a, b) = (self.bars[i - 1].q_close, self.bars[i].q_close);
        if a > 0.0 { b / a - 1.0 } else { f64::NAN }
    }

    fn change(&self, i: usize, back: usize) -> f64 {
        if i < back {
            return f64::NAN;
        }
        let (a, b) = (self.bars[i - back].q_close, self.bars[i].q_close);
        if a > 0.0 { b / a - 1.0 } else { f64::NAN }
    }

    fn avg_amount(&self, i: usize, n: usize) -> f64 {
        if i + 1 < n {
            return f64::NAN;
        }
        (self.amount_sum[i + 1] - self.amount_sum[i + 1 - n]) / n as f64
    }

    /// 20 日平均成交额的对数，也用作中性化时的规模变量
    pub fn size(&self, i: usize) -> f64 {
        let a = self.avg_amount(i, 20);
        if a > 0.0 { a.ln() } else { f64::NAN }
    }

    /// 价量因子在第 i 根日线收盘时的值；财务因子和特质波动率返回 NaN（由调用方另算）
    pub fn value(&self, id: FactorId, i: usize) -> f64 {
        const N: usize = 20;
        match id {
            FactorId::Reversal => -self.change(i, 5),
            FactorId::Momentum => self.change(i, N),
            FactorId::Momentum12m => {
                if i < 250 {
                    return f64::NAN;
                }
                let (a, b) = (self.bars[i - 250].q_close, self.bars[i - N].q_close);
                if a > 0.0 { b / a - 1.0 } else { f64::NAN }
            }
            FactorId::Volatility => {
                if i < N {
                    return f64::NAN;
                }
                std_dev(&(i + 1 - N..=i).map(|k| self.ret(k)).collect::<Vec<_>>())
            }
            FactorId::MaxReturn => {
                if i < N {
                    return f64::NAN;
                }
                (i + 1 - N..=i)
                    .map(|k| self.ret(k))
                    .filter(|r| r.is_finite())
                    .fold(f64::NAN, f64::max)
            }
            FactorId::Liquidity => self.size(i),
            FactorId::AbnormalTurnover => {
                let (short, long) = (self.avg_amount(i, 5), self.avg_amount(i, 120));
                if short > 0.0 && long > 0.0 {
                    (short / long).ln()
                } else {
                    f64::NAN
                }
            }
            FactorId::Illiquidity => {
                if i < N {
                    return f64::NAN;
                }
                let v: Vec<f64> = (i + 1 - N..=i)
                    .filter(|&k| self.bars[k].amount > 0.0)
                    .map(|k| self.ret(k).abs() / self.bars[k].amount * 1e8)
                    .filter(|v| v.is_finite())
                    .collect();
                if v.len() < N / 2 {
                    f64::NAN
                } else {
                    v.iter().sum::<f64>() / v.len() as f64
                }
            }
            FactorId::IdioVol | FactorId::Roe | FactorId::ProfitGrowth | FactorId::Accruals => {
                f64::NAN
            }
        }
    }
}

/// 样本标准差；有效值少于一半时为 NaN
pub fn std_dev(values: &[f64]) -> f64 {
    let v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.len() < 3 || v.len() * 2 < values.len() {
        return f64::NAN;
    }
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

/// 特质波动率：个股日收益对市场日收益做一元回归，残差的标准差。两组长度相同，NaN 的位置成对丢弃
pub fn idio_vol(stock: &[f64], market: &[f64]) -> f64 {
    let pairs: Vec<(f64, f64)> = stock
        .iter()
        .zip(market)
        .filter(|(s, m)| s.is_finite() && m.is_finite())
        .map(|(s, m)| (*s, *m))
        .collect();
    if pairs.len() < 10 || pairs.len() * 2 < stock.len() {
        return f64::NAN;
    }
    let n = pairs.len() as f64;
    let (ms, mm) = (
        pairs.iter().map(|p| p.0).sum::<f64>() / n,
        pairs.iter().map(|p| p.1).sum::<f64>() / n,
    );
    let var_m = pairs.iter().map(|p| (p.1 - mm).powi(2)).sum::<f64>();
    let beta = if var_m > 0.0 {
        pairs.iter().map(|p| (p.0 - ms) * (p.1 - mm)).sum::<f64>() / var_m
    } else {
        0.0
    };
    let resid: Vec<f64> = pairs
        .iter()
        .map(|p| (p.0 - ms) - beta * (p.1 - mm))
        .collect();
    (resid.iter().map(|r| r * r).sum::<f64>() / (n - 2.0)).sqrt()
}

/// 一期财报（年内累计口径，Q2 是半年报），金额单位元
#[derive(Clone, Debug)]
pub struct Report {
    pub fiscal_year: i32,
    /// 1 到 4
    pub quarter: u8,
    /// 供应商记录的报告日期。同一期数据会被之后的报告（作为上年同期数）覆盖，日期也随之变晚
    pub report_date: Option<NaiveDate>,
    pub parent_net_profit: Option<f64>,
    pub equity: Option<f64>,
    pub assets: Option<f64>,
    pub operating_cash_flow: Option<f64>,
}

impl Report {
    /// 法定披露截止日：一季报 4 月 30 日、半年报 8 月 31 日、三季报 10 月 31 日、年报次年 4 月 30 日
    pub fn deadline(&self) -> NaiveDate {
        let (y, m, d) = match self.quarter {
            1 => (self.fiscal_year, 4, 30),
            2 => (self.fiscal_year, 8, 31),
            3 => (self.fiscal_year, 10, 31),
            _ => (self.fiscal_year + 1, 4, 30),
        };
        NaiveDate::from_ymd_opt(y, m, d).expect("固定日期有效")
    }

    /// 这期数据最晚什么时候能被市场知道：报告日期不晚于截止日时它就是首次披露日，
    /// 晚于截止日说明这条记录被之后的报告覆盖过，首次披露不会晚于截止日（逾期披露极少），按截止日算
    pub fn available(&self) -> NaiveDate {
        let deadline = self.deadline();
        self.report_date
            .filter(|d| *d <= deadline)
            .unwrap_or(deadline)
    }

    fn key(&self) -> (i32, u8) {
        (self.fiscal_year, self.quarter)
    }
}

/// 某一期财报算出的财务因子，以及最早可以使用的日期
#[derive(Clone, Debug)]
pub struct FundamentalSnapshot {
    pub available: NaiveDate,
    pub period_end: NaiveDate,
    pub roe: f64,
    pub profit_growth: f64,
    pub accruals: f64,
}

fn period_end(fy: i32, q: u8) -> NaiveDate {
    let (m, d) = match q {
        1 => (3, 31),
        2 => (6, 30),
        3 => (9, 30),
        _ => (12, 31),
    };
    NaiveDate::from_ymd_opt(fy, m, d).expect("固定日期有效")
}

/// 一只股票全部财报 → 按可用日排序的财务因子快照。TTM 和单季值由累计值换算，
/// 用到的每期财报都必须已经可用，快照的可用日取其中最晚的一个
pub fn fundamental_snapshots(mut reports: Vec<Report>) -> Vec<FundamentalSnapshot> {
    reports.sort_by_key(Report::key);
    reports.dedup_by_key(|r| r.key());
    let find = |fy: i32, q: u8| reports.iter().find(|r| r.key() == (fy, q));
    let cum = |r: &Report, f: fn(&Report) -> Option<f64>| f(r);
    // 单季值：Q1 就是累计值，其余季度是本期累计减上期累计
    let single = |fy: i32, q: u8, f: fn(&Report) -> Option<f64>| -> Option<(f64, NaiveDate)> {
        let cur = find(fy, q)?;
        let v = cum(cur, f)?;
        if q == 1 {
            return Some((v, cur.available()));
        }
        let prev = find(fy, q - 1)?;
        Some((v - cum(prev, f)?, cur.available().max(prev.available())))
    };
    // TTM：年报直接用；其他季度 = 本期累计 + 上年年报 − 上年同期累计
    let ttm = |fy: i32, q: u8, f: fn(&Report) -> Option<f64>| -> Option<(f64, NaiveDate)> {
        let cur = find(fy, q)?;
        let v = cum(cur, f)?;
        if q == 4 {
            return Some((v, cur.available()));
        }
        let (annual, same) = (find(fy - 1, 4)?, find(fy - 1, q)?);
        Some((
            v + cum(annual, f)? - cum(same, f)?,
            cur.available()
                .max(annual.available())
                .max(same.available()),
        ))
    };
    let mut out = vec![];
    for r in &reports {
        let (fy, q) = r.key();
        let np = ttm(fy, q, |r| r.parent_net_profit);
        let cfo = ttm(fy, q, |r| r.operating_cash_flow);
        let roe = match (np, r.equity) {
            (Some((v, _)), Some(e)) if e > 0.0 => v / e,
            _ => f64::NAN,
        };
        let accruals = match (np, cfo, r.assets) {
            (Some((n, _)), Some((c, _)), Some(a)) if a > 0.0 => (n - c) / a,
            _ => f64::NAN,
        };
        let growth = match (
            single(fy, q, |r| r.parent_net_profit),
            single(fy - 1, q, |r| r.parent_net_profit),
        ) {
            (Some((now, _)), Some((before, _))) if before.abs() > 0.0 => {
                ((now - before) / before.abs()).clamp(-10.0, 10.0)
            }
            _ => f64::NAN,
        };
        let available = [
            Some(r.available()),
            np.map(|x| x.1),
            cfo.map(|x| x.1),
            single(fy, q, |r| r.parent_net_profit).map(|x| x.1),
            single(fy - 1, q, |r| r.parent_net_profit).map(|x| x.1),
        ]
        .into_iter()
        .flatten()
        .max()
        .expect("至少有本期的可用日");
        out.push(FundamentalSnapshot {
            available,
            period_end: period_end(fy, q),
            roe,
            profit_growth: growth,
            accruals,
        });
    }
    out.sort_by_key(|s| (s.available, s.period_end));
    out
}

/// 在 date 收盘时可以用的财务因子：可用日早于 date 的快照里报告期最新的一期；
/// 最新一期距今超过 15 个月（停止披露或数据缺失）时视为没有
pub fn fundamental_at(
    snaps: &[FundamentalSnapshot],
    date: NaiveDate,
) -> Option<&FundamentalSnapshot> {
    let usable = snaps.partition_point(|s| s.available < date);
    let best = snaps[..usable].iter().max_by_key(|s| s.period_end)?;
    let months = (date.year() - best.period_end.year()) * 12 + date.month() as i32
        - best.period_end.month() as i32;
    (months <= 15).then_some(best)
}

pub fn fundamental_value(s: &FundamentalSnapshot, id: FactorId) -> f64 {
    match id {
        FactorId::Roe => s.roe,
        FactorId::ProfitGrowth => s.profit_growth,
        FactorId::Accruals => s.accruals,
        _ => f64::NAN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::prices::bars_from;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn report(fy: i32, q: u8, date: NaiveDate, np: f64) -> Report {
        Report {
            fiscal_year: fy,
            quarter: q,
            report_date: Some(date),
            parent_net_profit: Some(np),
            equity: Some(1000.0),
            assets: Some(2000.0),
            operating_cash_flow: Some(np / 2.0),
        }
    }

    #[test]
    fn availability_uses_deadline_when_record_was_restated() {
        // 2025 半年报被 2026 半年报作为上年同期数覆盖，记录的日期变成 2026-08-14
        let r = report(2025, 2, d(2026, 8, 14), 1.0);
        assert_eq!(r.available(), d(2025, 8, 31));
        let first = report(2026, 2, d(2026, 8, 14), 1.0);
        assert_eq!(first.available(), d(2026, 8, 14));
    }

    #[test]
    fn ttm_and_single_quarter_from_cumulative_values() {
        let reports = vec![
            report(2024, 1, d(2024, 4, 20), 10.0),
            report(2024, 2, d(2024, 8, 20), 25.0),
            report(2024, 3, d(2024, 10, 20), 40.0),
            report(2024, 4, d(2025, 4, 20), 60.0),
            report(2025, 1, d(2025, 4, 25), 20.0),
        ];
        let snaps = fundamental_snapshots(reports);
        let last = snaps
            .iter()
            .find(|s| s.period_end == d(2025, 3, 31))
            .unwrap();
        // TTM = 20 + 60 − 10 = 70；单季增速 = (20 − 10) / 10
        assert!((last.roe - 0.07).abs() < 1e-12);
        assert!((last.profit_growth - 1.0).abs() < 1e-12);
        // 2025-04-25 当天还不能用（收盘后才知道），次日可以
        assert_ne!(
            fundamental_at(&snaps, d(2025, 4, 25)).unwrap().period_end,
            d(2025, 3, 31)
        );
        assert_eq!(
            fundamental_at(&snaps, d(2025, 4, 26)).unwrap().period_end,
            d(2025, 3, 31)
        );
        // 太久没有新财报时视为没有
        assert!(fundamental_at(&snaps, d(2026, 12, 1)).is_none());
    }

    #[test]
    fn price_factors_only_look_back() {
        let closes: Vec<f64> = (0..300).map(|i| 10.0 + (i as f64 * 0.37).sin()).collect();
        let bars = bars_from(&closes);
        let full = PriceFactors::new(&bars);
        let cut = PriceFactors::new(&bars[..281]);
        for id in [
            FactorId::Reversal,
            FactorId::Momentum,
            FactorId::Momentum12m,
            FactorId::Volatility,
            FactorId::MaxReturn,
            FactorId::Liquidity,
            FactorId::AbnormalTurnover,
            FactorId::Illiquidity,
        ] {
            let (a, b) = (full.value(id, 280), cut.value(id, 280));
            assert!(a.is_finite(), "{id:?}");
            assert_eq!(a.to_bits(), b.to_bits(), "{id:?} 用到了未来的数据");
        }
        assert!((full.value(FactorId::Reversal, 280) + full.change(280, 5)).abs() < 1e-12);
    }

    #[test]
    fn idio_vol_removes_market_component() {
        let market: Vec<f64> = (0..20).map(|i| ((i as f64) * 0.9).sin() * 0.02).collect();
        // 个股 = 1.5 倍市场，没有特质部分
        let stock: Vec<f64> = market.iter().map(|m| 1.5 * m).collect();
        assert!(idio_vol(&stock, &market) < 1e-12);
        let noisy: Vec<f64> = stock
            .iter()
            .enumerate()
            .map(|(i, s)| s + if i % 2 == 0 { 0.01 } else { -0.01 })
            .collect();
        assert!(idio_vol(&noisy, &market) > 0.005);
    }

    #[test]
    fn catalog_covers_every_factor() {
        for f in CATALOG {
            assert_eq!(info(f.id).id, f.id);
        }
        assert_eq!(CATALOG.len(), 12);
    }
}
