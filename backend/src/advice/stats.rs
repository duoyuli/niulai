//! 内置策略的过去表现：最近 3 年全市场的每一个买点，按策略自己的卖出规则卖出时赚不赚钱，
//! 以及买点之后 20 个交易日的平均涨跌和同期「随便买」（默认股票池里任意一天买入）的对比。
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
use crate::analysis::strategy::{Definition, SignalLib, Universe};

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
    /// 买点之后 20 个交易日的平均涨跌（%）
    pub after20_avg_pct: Option<f64>,
    /// 同期默认股票池里任意一天之后 20 个交易日的平均涨跌（%）
    pub base20_avg_pct: Option<f64>,
    /// 买点之后 20 个交易日上涨的比例
    pub after20_up_ratio: Option<f64>,
}

fn move_text(v: f64) -> String {
    if v >= 0.0 { format!("涨 {v:.1}%") } else { format!("跌 {:.1}%", -v) }
}

impl StrategyStat {
    /// 超额：买点之后 20 个交易日的平均涨跌减去随便买的
    pub fn excess20_pct(&self) -> Option<f64> {
        Some(self.after20_avg_pct? - self.base20_avg_pct?)
    }

    /// 一句话，建议的理由里引用
    pub fn describe(&self, name: &str) -> String {
        let mut s = format!("「{name}」过去 {YEARS} 年在全市场出现 {} 次买点", self.signals);
        if let (Some(a), Some(b)) = (self.after20_avg_pct, self.base20_avg_pct) {
            s.push_str(&format!("，之后 20 个交易日平均{}，同期随便买平均{}", move_text(a), move_text(b)));
        }
        if let (Some(w), Some(r), Some(h)) = (self.win_rate, self.avg_return_pct, self.avg_bars_held) {
            s.push_str(&format!("；按它的卖出规则卖出，{:.0}% 的交易赚钱，平均每笔{}、持有 {h:.0} 个交易日", w * 100.0, move_text(r)));
        }
        s.push_str(&format!("（统计到 {}，不扣费用，不代表以后）", self.end));
        s
    }
}

pub fn definition_hash(d: &Definition) -> String {
    hash_json(&serde_json::to_string(d).unwrap_or_default())
}

pub fn params_hash(p: &crate::account::style::StyleParams) -> String {
    hash_json(&serde_json::to_string(p).unwrap_or_default())
}

fn hash_json(json: &str) -> String {
    Sha256::digest(json.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Default)]
struct Acc {
    signals: u64,
    closed: u64,
    wins: u64,
    ret_sum: f64,
    held_sum: f64,
    after_n: u64,
    after_sum: f64,
    after_up: u64,
}

// @project-doc docs/domains/accounts.md#strategy_stats
/// 一次读全市场，同时统计这些策略
pub async fn compute(pool: &PgPool, defs: &[(i64, Definition)]) -> anyhow::Result<HashMap<i64, StrategyStat>> {
    let Some(end) = market::resolve_date(pool, None).await? else { return Ok(HashMap::new()) };
    let start = end - chrono::Duration::days(365 * YEARS);
    let sec = market::securities(pool).await?;
    let base_def = Definition { universe: Universe::default(), all: vec![], any: vec![], exit: Default::default(), sort: Default::default(), limit: 50, lib: SignalLib::default() };
    let warmup = defs.iter().map(|(_, d)| d.warmup()).max().unwrap_or(30) + 5;
    let mut accs: Vec<Acc> = defs.iter().map(|_| Acc::default()).collect();
    let (mut base_n, mut base_sum) = (0u64, 0.0f64);
    prices::for_each_stock(pool, start - chrono::Duration::days(calendar_days_for(warmup)), end, |code, bars| {
        let Some(meta) = sec.meta.get(code) else { return };
        let Some(first) = bars.iter().position(|b| b.date >= start) else { return };
        let f = Frame::new(bars, Some(meta));
        let after = |s: usize| bars.get(s + HORIZON).map(|l| (l.q_close / bars[s].q_close - 1.0) * 100.0).filter(|r| r.is_finite());
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
                    && let Some(r) = after(s)
                {
                    acc.after_n += 1;
                    acc.after_sum += r;
                    acc.after_up += (r > 0.0) as u64;
                }
            }
        }
        for i in first..bars.len() {
            if base_def.in_universe(&f, meta, i)
                && let Some(r) = after(i)
            {
                base_n += 1;
                base_sum += r;
            }
        }
    })
    .await?;
    let base = (base_n > 0).then(|| base_sum / base_n as f64);
    Ok(defs
        .iter()
        .zip(accs)
        .map(|((id, _), a)| {
            let ratio = |x: f64, n: u64| (n > 0).then(|| x / n as f64);
            let stat = StrategyStat {
                start,
                end,
                signals: a.signals,
                closed: a.closed,
                win_rate: ratio(a.wins as f64, a.closed),
                avg_return_pct: ratio(a.ret_sum, a.closed),
                avg_bars_held: ratio(a.held_sum, a.closed),
                after20_avg_pct: ratio(a.after_sum, a.after_n),
                base20_avg_pct: base,
                after20_up_ratio: ratio(a.after_up as f64, a.after_n),
            };
            (*id, stat)
        })
        .collect())
}

pub async fn load(pool: &PgPool, ids: &[i64]) -> anyhow::Result<HashMap<i64, StrategyStat>> {
    let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as("SELECT strategy_id, stats FROM app.strategy_stat WHERE strategy_id = ANY($1)").bind(ids).fetch_all(pool).await?;
    rows.into_iter().map(|(id, v)| Ok((id, serde_json::from_value(v)?))).collect()
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

/// 内置策略中需要重算的：还没有统计、定义变了，或已经超过 7 天
pub async fn due(pool: &PgPool) -> anyhow::Result<bool> {
    let builtin = builtin_definitions(pool).await?;
    let rows: Vec<(i64, String, bool)> =
        sqlx::query_as("SELECT strategy_id, definition_hash, computed_at < now() - interval '7 days' FROM app.strategy_stat").fetch_all(pool).await?;
    let known: HashMap<i64, (String, bool)> = rows.into_iter().map(|(id, h, old)| (id, (h, old))).collect();
    Ok(builtin.iter().any(|(id, d)| known.get(id).is_none_or(|(h, old)| *old || *h != definition_hash(d))))
}

/// 全部内置策略的定义（带自定义信号库），按编号
pub async fn builtin_definitions(pool: &PgPool) -> anyhow::Result<Vec<(i64, Definition)>> {
    let rows: Vec<(i64, serde_json::Value)> =
        sqlx::query_as("SELECT strategy_id, definition FROM app.strategy WHERE builtin_key IS NOT NULL ORDER BY strategy_id").fetch_all(pool).await?;
    let lib = crate::analysis::signal_lib::load(pool).await?;
    rows.into_iter().map(|(id, v)| Ok((id, serde_json::from_value::<Definition>(v)?.with_lib(lib.clone())))).collect()
}

/// 重算全部内置策略并保存
pub async fn refresh(pool: &PgPool) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let defs = builtin_definitions(pool).await?;
    let stats = compute(pool, &defs).await?;
    for (id, d) in &defs {
        if let Some(s) = stats.get(id) {
            store(pool, *id, &definition_hash(d), s).await?;
        }
    }
    tracing::info!(strategies = defs.len(), ms = started.elapsed().as_millis() as u64, "内置策略的过去表现已统计");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_with_numbers_and_excess() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let s = StrategyStat {
            start: d,
            end: d,
            signals: 3412,
            closed: 3300,
            win_rate: Some(0.52),
            avg_return_pct: Some(-0.3),
            avg_bars_held: Some(9.4),
            after20_avg_pct: Some(2.4),
            base20_avg_pct: Some(0.6),
            after20_up_ratio: Some(0.55),
        };
        assert!((s.excess20_pct().unwrap() - 1.8).abs() < 1e-9);
        let t = s.describe("放量突破");
        assert!(t.contains("3412 次买点") && t.contains("平均涨 2.4%") && t.contains("随便买平均涨 0.6%") && t.contains("平均每笔跌 0.3%"), "{t}");
        assert!(StrategyStat { base20_avg_pct: None, ..s }.excess20_pct().is_none());
    }
}
