//! 信号的过去表现：最近几年全市场每次出现之后 5、10、20 个交易日的涨跌，和同期所有股票对比。
//! 只是统计描述，不代表以后。规则见 docs/domains/strategies.md#signal_stats。

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::indicators::Frame;
use super::market::{self, calendar_days_for};
use super::prices;
use super::strategy::{Condition, Definition, SignalLib, Universe};

/// 统计哪几个持有期
pub const HORIZONS: [usize; 3] = [5, 10, 20];
/// 返回最近的多少次出现
const RECENT: usize = 30;

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct HorizonStat {
    /// 出现后第几个交易日
    pub days: u32,
    /// 有这么多天后续数据的出现次数
    pub samples: u64,
    /// 平均涨跌（%）
    pub avg_pct: Option<f64>,
    /// 涨跌的中位数（%）
    pub median_pct: Option<f64>,
    /// 上涨的比例（0.55 表示 55%）
    pub up_ratio: Option<f64>,
    /// 同期所有股票、所有交易日的平均涨跌和上涨比例，作为对比
    pub market_avg_pct: Option<f64>,
    pub market_up_ratio: Option<f64>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct Occurrence {
    pub trade_date: NaiveDate,
    pub thscode: String,
    pub name: String,
    /// 出现后 20 个交易日的涨跌（%）；还不满 20 个交易日时为空
    pub after_20d_pct: Option<f64>,
}

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct SignalStats {
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// 出现的次数：连续几天都满足只算第一天
    pub occurrences: u64,
    /// 出现过的股票数
    pub stocks: u64,
    pub horizons: Vec<HorizonStat>,
    /// 最近的几次出现，新的在前
    pub recent: Vec<Occurrence>,
    /// 统计时用的股票池（白话）
    pub universe: Vec<String>,
}

#[derive(Default)]
struct Acc {
    returns: Vec<f64>,
}

impl Acc {
    fn push(&mut self, r: f64) {
        self.returns.push(r);
    }
}

/// 市场基准只需要和与计数，不保存每一笔
#[derive(Default, Clone, Copy)]
struct Base {
    n: u64,
    sum: f64,
    up: u64,
}

// @project-doc docs/domains/strategies.md#signal_stats
/// 统计一个条件（通常是「当天出现某个信号」）最近 years 年的表现。股票池用默认股票池，排除 ST、低价和成交太少的股票
pub async fn compute(pool: &PgPool, cond: &Condition, lib: &SignalLib, years: u32) -> anyhow::Result<Option<SignalStats>> {
    let Some(end) = market::resolve_date(pool, None).await? else { return Ok(None) };
    let start = end - chrono::Duration::days(365 * years as i64);
    let sec = market::securities(pool).await?;
    let d = Definition {
        universe: Universe::default(),
        all: vec![cond.clone()],
        any: vec![],
        exit: Default::default(),
        sort: Default::default(),
        limit: 50,
        lib: lib.clone(),
    };
    let warmup = d.warmup() + 5;
    let mut accs: Vec<Acc> = HORIZONS.iter().map(|_| Acc::default()).collect();
    let mut base = [Base::default(); HORIZONS.len()];
    let mut occurrences = 0u64;
    let mut stocks = 0u64;
    let mut recent: Vec<Occurrence> = vec![];
    prices::for_each_stock(pool, start - chrono::Duration::days(calendar_days_for(warmup)), end, |code, bars| {
        let Some(meta) = sec.meta.get(code) else { return };
        let Some(first) = bars.iter().position(|b| b.date >= start) else { return };
        let f = Frame::new(bars, Some(meta));
        let s = cond.series(&f, lib);
        let mut hit = false;
        for i in first..bars.len() {
            if !d.in_universe(&f, meta, i) {
                continue;
            }
            let fresh = s[i] && (i == 0 || !s[i - 1]);
            if fresh {
                occurrences += 1;
                hit = true;
            }
            for (h, &days) in HORIZONS.iter().enumerate() {
                let Some(later) = bars.get(i + days) else { continue };
                let r = (later.q_close / bars[i].q_close - 1.0) * 100.0;
                if !r.is_finite() {
                    continue;
                }
                base[h].n += 1;
                base[h].sum += r;
                base[h].up += (r > 0.0) as u64;
                if fresh {
                    accs[h].push(r);
                }
            }
            if fresh {
                let after = bars.get(i + 20).map(|l| (l.q_close / bars[i].q_close - 1.0) * 100.0);
                recent.push(Occurrence { trade_date: bars[i].date, thscode: code.to_string(), name: sec.names.get(code).cloned().unwrap_or_default(), after_20d_pct: after });
                // 只留最近的一批，避免全市场几万次出现都放在内存里
                if recent.len() > RECENT * 20 {
                    recent.sort_by_key(|o| std::cmp::Reverse(o.trade_date));
                    recent.truncate(RECENT);
                }
            }
        }
        stocks += hit as u64;
    })
    .await?;
    recent.sort_by(|a, b| b.trade_date.cmp(&a.trade_date).then_with(|| a.thscode.cmp(&b.thscode)));
    recent.truncate(RECENT);
    let horizons = HORIZONS
        .iter()
        .zip(accs)
        .zip(base)
        .map(|((&days, mut a), b)| {
            let n = a.returns.len();
            a.returns.sort_by(|x, y| x.total_cmp(y));
            let avg = if n > 0 { a.returns.iter().sum::<f64>() / n as f64 } else { f64::NAN };
            let median = if n > 0 { a.returns[n / 2] } else { f64::NAN };
            let up = if n > 0 { a.returns.iter().filter(|r| **r > 0.0).count() as f64 / n as f64 } else { f64::NAN };
            HorizonStat {
                days: days as u32,
                samples: n as u64,
                avg_pct: finite(avg),
                median_pct: finite(median),
                up_ratio: finite(up),
                market_avg_pct: (b.n > 0).then(|| b.sum / b.n as f64),
                market_up_ratio: (b.n > 0).then(|| b.up as f64 / b.n as f64),
            }
        })
        .collect();
    Ok(Some(SignalStats { start, end, occurrences, stocks, horizons, recent, universe: d.describe_universe() }))
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}
