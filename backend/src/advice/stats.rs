//! 策略的过去表现（全部参加巡检的策略）：最近 3 年全市场的每一个买点，按策略自己的卖出规则卖出时赚不赚钱，
//! 以及买点之后 20 个交易日的平均涨跌和同一天「随便买」（默认股票池里当天任意一只）的对比。
//! 涨跌按能做到的算：买点第二天开盘买入（开盘一字涨停买不进的不算），持有到买点之后第 20 个交易日收盘。
//! 超额按交易日分组估计误差：同一天的买点涨跌高度相关，不能当成互不相干的样本。
//! 买卖点用巡检和 K 线上的同一套规则（`points::run`）。全市场 3 年要算十几秒，所以每周统计一次，
//! 策略定义变了也重算，结果存在 `app.strategy_stat`。规则见 docs/domains/accounts.md#strategy_stats。

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::analysis::indicators::Frame;
use crate::analysis::market::{self, calendar_days_for};
use crate::analysis::points;
use crate::analysis::prices;
use crate::analysis::strategy::Definition;

/// 统计最近几年
pub const YEARS: i64 = 3;
/// 买点之后看多少个交易日
pub const HORIZON: usize = 20;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct StrategyStat {
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// 买点次数
    pub signals: u64,
    /// 其中已经按卖出规则卖出的
    pub closed: u64,
    /// 卖出的交易中赚钱的比例（0.55 表示 55%）
    pub win_rate: Option<f64>,
    /// 卖出的交易的平均收益（%，前复权，不扣费用）
    pub avg_return_pct: Option<f64>,
    pub avg_bars_held: Option<f64>,
    /// 买点之后 20 个交易日的平均涨跌（%，第二天开盘买、第 20 个交易日收盘算）
    pub after20_avg_pct: Option<f64>,
    /// 同样那些天随便买的平均涨跌（%）：每个买点日默认股票池里全部股票的平均，按买点数加权
    pub base20_avg_pct: Option<f64>,
    /// 买点之后 20 个交易日上涨的比例
    pub after20_up_ratio: Option<f64>,
    /// 超额的标准误（百分点，按交易日分组）。超额不到它的 2 倍，多半是运气。旧的统计没有，重算后才有
    #[serde(default)]
    pub excess_se_pct: Option<f64>,
    /// 有买点的交易日数
    #[serde(default)]
    pub signal_days: Option<u64>,
}

/// 统计口径的版本：口径变了，已经存下的统计全部重算
pub const STATS_VERSION: &str = "v2-next-open-same-day";

/// 同一天的涨跌：交易日 → (涨跌之和, 次数)
#[derive(Default)]
pub struct DayAcc(HashMap<NaiveDate, (f64, u32)>);

impl DayAcc {
    pub fn add(&mut self, day: NaiveDate, r: f64) {
        let e = self.0.entry(day).or_default();
        e.0 += r;
        e.1 += 1;
    }
}

/// 一组买点相对同日随便买的结果
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub after: f64,
    pub base: f64,
    pub excess: f64,
    pub se: Option<f64>,
    pub n: u64,
    pub days: u64,
}

/// 只看 keep 为真的交易日：超额 = 每个买点的涨跌减去同一天随便买的平均，再平均；标准误按交易日分组算（cluster-robust）
pub fn edge(sig: &DayAcc, base: &DayAcc, keep: impl Fn(NaiveDate) -> bool) -> Option<Edge> {
    let mut days: Vec<(f64, f64, f64)> = vec![];
    for (d, (sum, n)) in &sig.0 {
        let Some((bs, bn)) = base.0.get(d) else {
            continue;
        };
        if !keep(*d) || *n == 0 || *bn == 0 {
            continue;
        }
        let n = f64::from(*n);
        days.push((n, sum / n, bs / f64::from(*bn)));
    }
    let total: f64 = days.iter().map(|(n, _, _)| n).sum();
    if total == 0.0 {
        return None;
    }
    let after = days.iter().map(|(n, a, _)| n * a).sum::<f64>() / total;
    let base_avg = days.iter().map(|(n, _, b)| n * b).sum::<f64>() / total;
    let excess = after - base_avg;
    let k = days.len() as f64;
    let se = (days.len() >= 2).then(|| {
        let v: f64 = days
            .iter()
            .map(|(n, a, b)| (n * (a - b - excess)).powi(2))
            .sum();
        (k / (k - 1.0) * v).sqrt() / total
    });
    Some(Edge {
        after,
        base: base_avg,
        excess,
        se,
        n: total as u64,
        days: days.len() as u64,
    })
}

/// 第 s 天收盘出现买点：第二天开盘买入、第 s + HORIZON 天收盘的涨跌（%，前复权）。
/// 第二天开盘一字涨停（全天一个价且比前一天收盘高）买不进，不算。
/// 判断用前复权价，和 `points::run` 的口径一致：除权日的未复权价会被除低，拿未复权价比较会漏判。
pub fn forward_return(bars: &[prices::DailyBar], s: usize) -> Option<f64> {
    let entry = bars.get(s + 1)?;
    let exit = bars.get(s + HORIZON)?;
    if entry.q_high == entry.q_low && entry.q_open > bars[s].q_close {
        return None;
    }
    Some((exit.q_close / entry.q_open - 1.0) * 100.0).filter(|r| r.is_finite())
}

/// 定义加上统计口径的版本：两者有一个变了就重算
pub fn definition_hash(d: &Definition) -> String {
    hash_json(&format!(
        "{STATS_VERSION}{}",
        serde_json::to_string(d).unwrap_or_default()
    ))
}

pub fn rules_hash(p: &crate::account::rules::Rules) -> String {
    hash_json(&serde_json::to_string(p).unwrap_or_default())
}

fn hash_json(json: &str) -> String {
    Sha256::digest(json.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Default)]
struct Acc {
    signals: u64,
    closed: u64,
    wins: u64,
    ret_sum: f64,
    held_sum: f64,
    after_n: u64,
    after_up: u64,
    days: DayAcc,
}

// @project-doc docs/domains/accounts.md#strategy_stats
/// 一次读全市场，同时统计这些策略
pub async fn compute(
    pool: &PgPool,
    defs: &[(i64, Definition)],
) -> anyhow::Result<HashMap<i64, StrategyStat>> {
    let Some(end) = market::resolve_date(pool, None).await? else {
        return Ok(HashMap::new());
    };
    let start = end - chrono::Duration::days(365 * YEARS);
    let sec = market::securities(pool).await?;
    let asset_types: Vec<String> = defs
        .iter()
        .flat_map(|(_, d)| d.universe.asset_types.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let base_def = Definition::any_assets(asset_types.clone());
    let warmup = defs.iter().map(|(_, d)| d.warmup()).max().unwrap_or(30) + 5;
    let lo = start - chrono::Duration::days(calendar_days_for(warmup));
    // 权威涨跌停池：策略过去表现里的买卖点同样以供应商的名单为准
    let limits = crate::analysis::limit_pool::LimitPool::load(pool, lo, end).await?;
    let mut accs: Vec<Acc> = defs.iter().map(|_| Acc::default()).collect();
    let mut base = DayAcc::default();
    prices::for_each_assets(
        pool,
        lo,
        end,
        asset_types.iter().any(|a| a == "stock"),
        asset_types.iter().any(|a| a == "etf"),
        |code, bars| {
            let Some(meta) = sec.meta.get(code) else {
                return;
            };
            let Some(first) = bars.iter().position(|b| b.date >= start) else {
                return;
            };
            let history = sec.st_history.get(code).map(Vec::as_slice).unwrap_or(&[]);
            let f = Frame::with_authority_and_st_history(bars, Some(meta), &limits, code, history);
            for (acc, (_, d)) in accs.iter_mut().zip(defs) {
                let p = points::run(bars, &f, meta, d, first);
                for t in &p.trades {
                    acc.signals += 1;
                    if t.exit_date.is_some() {
                        acc.closed += 1;
                        acc.wins += (t.return_pct > 0.0) as u64;
                        acc.ret_sum += t.return_pct;
                        acc.held_sum += t.bars_held as f64;
                    }
                    if let Ok(s) = bars.binary_search_by_key(&t.signal_date, |b| b.date)
                        && let Some(r) = forward_return(bars, s)
                    {
                        acc.after_n += 1;
                        acc.after_up += (r > 0.0) as u64;
                        acc.days.add(t.signal_date, r);
                    }
                }
            }
            for i in first..bars.len() {
                if base_def.in_universe(&f, meta, i)
                    && let Some(r) = forward_return(bars, i)
                {
                    base.add(bars[i].date, r);
                }
            }
        },
    )
    .await?;
    Ok(defs
        .iter()
        .zip(accs)
        .map(|((id, _), a)| {
            let ratio = |x: f64, n: u64| (n > 0).then(|| x / n as f64);
            let e = edge(&a.days, &base, |_| true);
            let stat = StrategyStat {
                start,
                end,
                signals: a.signals,
                closed: a.closed,
                win_rate: ratio(a.wins as f64, a.closed),
                avg_return_pct: ratio(a.ret_sum, a.closed),
                avg_bars_held: ratio(a.held_sum, a.closed),
                after20_avg_pct: e.map(|e| e.after),
                base20_avg_pct: e.map(|e| e.base),
                after20_up_ratio: ratio(a.after_up as f64, a.after_n),
                excess_se_pct: e.and_then(|e| e.se),
                signal_days: e.map(|e| e.days),
            };
            (*id, stat)
        })
        .collect())
}

pub async fn load(pool: &PgPool, ids: &[i64]) -> anyhow::Result<HashMap<i64, StrategyStat>> {
    let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
        "SELECT strategy_id, stats FROM app.strategy_stat WHERE strategy_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, v)| Ok((id, serde_json::from_value(v)?)))
        .collect()
}

pub async fn store(pool: &PgPool, id: i64, hash: &str, stat: &StrategyStat) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO app.strategy_stat (strategy_id, definition_hash, end_date, stats) VALUES ($1, $2, $3, $4)
         ON CONFLICT (strategy_id) DO UPDATE SET definition_hash = EXCLUDED.definition_hash, end_date = EXCLUDED.end_date, stats = EXCLUDED.stats, computed_at = now()",
    )
    .bind(id)
    .bind(hash)
    .bind(stat.end)
    .bind(serde_json::to_value(stat)?)
    .execute(pool)
    .await?;
    Ok(())
}

/// 参加巡检的策略中有需要重算的：还没有统计、定义变了，或已经超过 7 天
pub async fn due(pool: &PgPool) -> anyhow::Result<bool> {
    let list = crate::analysis::screener::screening_strategies(pool).await?;
    let rows: Vec<(i64, String, bool)> =
        sqlx::query_as("SELECT strategy_id, definition_hash, computed_at < now() - interval '7 days' FROM app.strategy_stat").fetch_all(pool).await?;
    let known: HashMap<i64, (String, bool)> = rows
        .into_iter()
        .map(|(id, h, old)| (id, (h, old)))
        .collect();
    Ok(list.iter().any(|(id, d)| {
        known
            .get(id)
            .is_none_or(|(h, old)| *old || *h != definition_hash(d))
    }))
}

/// 重算全部参加巡检的策略并保存
pub async fn refresh(pool: &PgPool) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let defs = crate::analysis::screener::screening_strategies(pool).await?;
    let stats = compute(pool, &defs).await?;
    for (id, d) in &defs {
        if let Some(s) = stats.get(id) {
            store(pool, *id, &definition_hash(d), s).await?;
        }
    }
    tracing::info!(
        strategies = defs.len(),
        ms = started.elapsed().as_millis() as u64,
        "策略的过去表现已统计"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_day_excess_and_clustered_error() {
        let d = |i: u32| NaiveDate::from_ymd_opt(2026, 1, i).unwrap();
        let (mut sig, mut base) = (DayAcc::default(), DayAcc::default());
        // 第 1 天大盘涨 10%，策略的两个买点各涨 11%；第 2 天大盘跌 2%，策略的买点跌 1%
        for r in [11.0, 11.0] {
            sig.add(d(1), r);
        }
        sig.add(d(2), -1.0);
        for r in [9.0, 11.0] {
            base.add(d(1), r);
        }
        base.add(d(2), -2.0);
        base.add(d(3), 50.0); // 没有买点的那天不影响
        let e = edge(&sig, &base, |_| true).unwrap();
        assert!(
            (e.excess - 1.0).abs() < 1e-9,
            "每个买点都比同一天多涨 1 个百分点：{e:?}"
        );
        assert_eq!((e.n, e.days), (3, 2));
        assert!(e.se.unwrap() < 1e-9, "每天的超额都一样，没有误差");
        assert!(edge(&sig, &base, |x| x == d(2)).is_some_and(|e| e.n == 1));
    }
}
