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
    /// 怎么算、数值大小代表什么，白话，带例子
    pub description: &'static str,
    pub category: FactorCategory,
    /// A 股历史研究中数值越大、后续收益越高为真；合成和分组的「好的一端」按它定方向
    pub higher_is_better: bool,
    /// 方向的依据：为什么数值大（或小）的股票之后可能涨得更多，白话，不重复「越大越好」
    pub rationale: &'static str,
}

pub const CATALOG: &[FactorInfo] = &[
    FactorInfo {
        id: FactorId::Reversal,
        name: "短期反转",
        description: "最近 5 个交易日的涨跌幅取相反数：这 5 天跌了 8%，值就是 8%；涨了 8%，值就是 −8%。数值越大，说明最近一周跌得越多",
        category: FactorCategory::PriceVolume,
        higher_is_better: true,
        rationale: "A 股散户多、追涨杀跌明显，短期涨多了容易跌回去，跌多了容易涨回来，所以最近一周跌得多的股票之后往往表现更好",
    },
    FactorInfo {
        id: FactorId::Momentum,
        name: "20 日动量",
        description: "最近 20 个交易日（约一个月）的累计涨跌幅，例如一个月涨了 15%，值就是 15%。数值越大，最近一个月涨得越多",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "在海外市场，近期涨得多的股票往往继续涨（动量效应）；A 股多数时期正好相反，最近一个月涨得多的股票之后反而偏弱",
    },
    FactorInfo {
        id: FactorId::Momentum12m,
        name: "12-1 月动量",
        description: "从约一年前（250 个交易日前）到一个月前（20 个交易日前）的累计涨跌幅，故意跳过最近一个月，避开短期反转的干扰。数值越大，过去一年（不含最近一个月）涨得越多",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "这是海外最经典的动量指标：过去一年涨得多的继续涨。A 股研究普遍发现它没有效果甚至相反，这里主要用来对照，不建议单独使用",
    },
    FactorInfo {
        id: FactorId::Volatility,
        name: "波动率",
        description: "最近 20 个交易日每天涨跌幅的标准差，衡量每天涨跌的幅度有多大、多不稳定。粗略地说，一只股票平时每天涨跌在 ±1% 上下，波动率约 1%；经常一天涨跌 4%～5%，波动率约 4%～5%。数值越大，股价上下颠簸得越厉害",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "A 股里大起大落的股票常被当作「彩票」追捧，价格容易被抬高，之后的收益反而偏低；走势平稳的股票长期收益更好，这叫低波动异象",
    },
    FactorInfo {
        id: FactorId::IdioVol,
        name: "特质波动率",
        description: "先把股价涨跌拆成两部分：跟着大盘一起涨跌的部分，和这只股票自己独有的部分。做法是拿最近 20 个交易日这只股票每天的涨跌幅，和同一天全市场股票的平均涨跌幅对比，算出它平时跟大盘联动的程度，再把跟随大盘的那部分扣掉，剩下部分的标准差就是特质波动率。数值越大，说明股价越多地被公司自己的消息或炒作推动，而不是随大盘涨跌",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "大盘涨跌所有股票都要承受，个股自己额外的大起大落多来自消息和炒作，这类股票之后的收益偏低。它比「波动率」少了大盘的影响，是 A 股研究里最稳定的规律之一",
    },
    FactorInfo {
        id: FactorId::MaxReturn,
        name: "最大日涨幅",
        description: "最近 20 个交易日里涨得最多的那一天的涨幅，例如期间有一天主板涨停，值就是 10%。数值越大，最近一个月出现过越猛的单日大涨",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "近期出现过单日暴涨（常见于涨停）的股票容易吸引短线资金追捧，价格被抬高，之后收益偏低，这叫彩票效应",
    },
    FactorInfo {
        id: FactorId::Liquidity,
        name: "成交额（规模代理）",
        description: "最近 20 个交易日平均每天的成交额，再取对数。取对数让 1 亿和 10 亿、10 亿和 100 亿之间的差距一样大，避免少数大盘股把数值拉得太开。数值越大，交易越活跃，公司通常也越大。本地没有历史股本数据，所以用成交额近似代替市值",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "A 股小公司长期涨得比大公司多（小盘溢价）。其中一部分来自小公司容易被借壳重组的价值，退市新规之后这部分在减弱；小盘股在市场踩踏时回撤也更集中",
    },
    FactorInfo {
        id: FactorId::AbnormalTurnover,
        name: "异常成交",
        description: "最近 5 个交易日的平均成交额 ÷ 最近 120 个交易日（约半年）的平均成交额，再取对数。0 表示最近的成交和半年平均水平一样；大于 0 表示最近成交放大，放大到平时的 2 倍时约为 0.7；小于 0 表示最近成交萎缩",
        category: FactorCategory::PriceVolume,
        higher_is_better: false,
        rationale: "成交突然放大往往说明关注度和情绪过热，短线资金涌入后价格偏高，A 股里这类股票之后收益通常偏低",
    },
    FactorInfo {
        id: FactorId::Illiquidity,
        name: "非流动性",
        description: "最近 20 个交易日，平均每成交 1 亿元能让股价变动多少：每天的涨跌幅（不分涨跌）÷ 当天成交额，再取平均。例如某天成交 5000 万、涨跌 2%，那天的值是 4%。数值越大，说明不多的成交就能推动股价，股票越难大量买卖。这个指标叫 Amihud 非流动性",
        category: FactorCategory::PriceVolume,
        higher_is_better: true,
        rationale: "难以大量买卖的股票，投资者要求更高的回报作为补偿。它和公司规模高度相关，和「成交额」因子同时选用时信息大量重复",
    },
    FactorInfo {
        id: FactorId::Roe,
        name: "净资产收益率",
        description: "最近四个季度归属于母公司股东的净利润之和 ÷ 最新的净资产（股东权益），也就是 ROE。例如净资产 100 亿、一年赚 15 亿，值就是 15%。数值越大，公司用股东的钱赚钱的效率越高；净资产为负时不计算",
        category: FactorCategory::Fundamental,
        higher_is_better: true,
        rationale: "盈利能力强的公司长期收益更高，这是「质量」类因子的代表",
    },
    FactorInfo {
        id: FactorId::ProfitGrowth,
        name: "单季利润增速",
        description: "最新一个季度归属于母公司股东的净利润，比去年同一季度增长了多少。例如去年同季赚 1 亿、今年赚 1.3 亿，值就是 30%。去年同季亏损时按亏损额的绝对值计算；超过 ±1000% 时按 ±1000% 计，避免基数太小时数值失真",
        category: FactorCategory::Fundamental,
        higher_is_better: true,
        rationale: "利润加速增长的公司，在业绩公布后的一段时间里股价往往继续跑赢，因为市场对好消息的反应偏慢（盈余公告后漂移）",
    },
    FactorInfo {
        id: FactorId::Accruals,
        name: "应计利润",
        description: "最近四个季度的（净利润 − 经营活动现金流净额）÷ 总资产。净利润里还没收到现金的部分叫应计利润，例如货卖出去了钱还没收回来。数值越大，利润里的现金含量越低；为负说明收到的现金比账面利润还多",
        category: FactorCategory::Fundamental,
        higher_is_better: false,
        rationale: "账面利润多、实际收到的现金少的公司，利润往往难以持续，之后容易出现业绩下修，股价收益偏低；利润有现金支撑的公司盈利质量更高",
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
