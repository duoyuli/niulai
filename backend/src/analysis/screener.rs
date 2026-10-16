//! 选股与每日巡检：在某个交易日对全市场执行一个或多个策略。
//! 全市场的日线按股票逐只读入，每只股票上依次判断所有策略，所以同时运行多个策略的耗时和运行一个差不多。
//! 规则见 docs/domains/strategies.md#screening。

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::indicators::{Frame, IndicatorId, IndicatorRef};
use super::market::{self, calendar_days_for};
use super::prices;
use super::strategy::Definition;

/// 入选的一只股票
#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct Hit {
    pub thscode: String,
    pub name: String,
    /// 收盘价（未复权）
    pub close: f64,
    /// 涨跌幅（%）
    pub change_pct: f64,
    /// 成交额（元）
    pub amount: f64,
    /// 排序指标的值
    pub sort_value: f64,
    /// 命中条件的白话说明
    pub reasons: Vec<String>,
}

pub struct StrategyResult {
    /// 满足条件的股票数（截断前）
    pub matched: usize,
    /// 按排序截取后的结果
    pub hits: Vec<Hit>,
}

pub struct Outcome {
    pub trade_date: NaiveDate,
    /// 当天有日线的股票数
    pub scanned: usize,
    /// 与传入的策略一一对应
    pub results: Vec<StrategyResult>,
}

// @project-doc docs/domains/strategies.md#screening
/// 对 date（为空时取最新交易日）执行这些策略。没有日线时返回 None
pub async fn screen(pool: &PgPool, defs: &[&Definition], date: Option<NaiveDate>) -> anyhow::Result<Option<Outcome>> {
    let Some(date) = market::resolve_date(pool, date).await? else { return Ok(None) };
    let sec = market::securities(pool).await?;
    // 强势板块过滤：每个策略预先算好允许的股票
    let mut sector_sets: Vec<Option<HashSet<String>>> = vec![];
    for d in defs {
        sector_sets.push(match &d.universe.sector {
            Some(s) => Some(market::stocks_in_top_sectors(pool, &s.category, s.days, s.top_pct, date).await?),
            None => None,
        });
    }
    let warmup = defs.iter().map(|d| d.warmup()).max().unwrap_or(30) + 5;
    let mut found: Vec<Vec<Hit>> = vec![vec![]; defs.len()];
    let mut scanned = 0usize;
    prices::for_each_stock(pool, date - chrono::Duration::days(calendar_days_for(warmup)), date, |code, bars| {
        let Some(last) = bars.last() else { return };
        let Some(meta) = sec.meta.get(code) else { return };
        if last.date != date {
            return; // 当天停牌
        }
        scanned += 1;
        let f = Frame::new(bars, Some(meta));
        let i = bars.len() - 1;
        for (k, d) in defs.iter().enumerate() {
            if !d.in_universe(&f, meta, i) || sector_sets[k].as_ref().is_some_and(|s| !s.contains(code)) {
                continue;
            }
            let e = d.eval(&f, i);
            if e.matched {
                found[k].push(Hit {
                    thscode: code.to_string(),
                    name: sec.names.get(code).cloned().unwrap_or_default(),
                    close: last.close,
                    change_pct: f.at(IndicatorRef::new(IndicatorId::ChangePct, None), i),
                    amount: last.amount,
                    sort_value: f.at(d.sort.by, i),
                    reasons: e.reasons,
                });
            }
        }
    })
    .await?;
    let results = defs
        .iter()
        .zip(found)
        .map(|(d, mut hits)| {
            let matched = hits.len();
            // 排序值缺失（NaN）的排在最后
            hits.sort_by(|a, b| {
                let (x, y) = (a.sort_value, b.sort_value);
                match (x.is_nan(), y.is_nan()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ if d.sort.desc => y.total_cmp(&x),
                    _ => x.total_cmp(&y),
                }
            });
            hits.truncate(d.limit as usize);
            StrategyResult { matched, hits }
        })
        .collect();
    Ok(Some(Outcome { trade_date: date, scanned, results }))
}

/// 开启巡检的全部策略
pub async fn screening_strategies(pool: &PgPool) -> anyhow::Result<Vec<(i64, Definition)>> {
    let rows: Vec<(i64, serde_json::Value)> =
        sqlx::query_as("SELECT strategy_id, definition FROM app.strategy WHERE screening ORDER BY strategy_id").fetch_all(pool).await?;
    let mut out = vec![];
    for (id, v) in rows {
        match serde_json::from_value::<Definition>(v) {
            Ok(d) => out.push((id, d)),
            Err(e) => tracing::warn!(strategy_id = id, error = %e, "策略定义无法解析，跳过"),
        }
    }
    Ok(out)
}

/// 巡检的汇总
#[derive(Serialize, ToSchema)]
pub struct ScreeningSummary {
    pub trade_date: Option<NaiveDate>,
    pub strategies: usize,
    pub scanned: usize,
    pub ms: u64,
}

/// 执行巡检：对开启巡检的策略（或只对 only 指定的策略）在 date 上选股，并保存结果。
/// 同一策略同一天重复执行时覆盖；与该策略上一次巡检比较，标出新入选的股票（第一次巡检不标）
pub async fn run_and_store(pool: &PgPool, date: Option<NaiveDate>, only: Option<i64>) -> anyhow::Result<ScreeningSummary> {
    let started = std::time::Instant::now();
    let mut list = screening_strategies(pool).await?;
    if let Some(id) = only {
        list = match load_strategy(pool, id).await? {
            Some(d) => vec![(id, d)],
            None => vec![],
        };
    }
    if list.is_empty() {
        return Ok(ScreeningSummary { trade_date: None, strategies: 0, scanned: 0, ms: 0 });
    }
    let defs: Vec<&Definition> = list.iter().map(|(_, d)| d).collect();
    let Some(out) = screen(pool, &defs, date).await? else {
        return Ok(ScreeningSummary { trade_date: None, strategies: 0, scanned: 0, ms: 0 });
    };
    let mut tx = pool.begin().await?;
    for ((id, _), res) in list.iter().zip(&out.results) {
        // 第一次巡检没有可比较的上一次，不标新入选
        let prev_run: Option<i64> = sqlx::query_scalar(
            "SELECT run_id FROM app.screen_run WHERE strategy_id = $1 AND trade_date < $2 ORDER BY trade_date DESC LIMIT 1",
        )
        .bind(id)
        .bind(out.trade_date)
        .fetch_optional(&mut *tx)
        .await?;
        let prev: Option<HashSet<String>> = match prev_run {
            Some(r) => Some(
                sqlx::query_scalar("SELECT thscode FROM app.screen_hit WHERE run_id = $1").bind(r).fetch_all(&mut *tx).await?.into_iter().collect(),
            ),
            None => None,
        };
        let run_id: i64 = sqlx::query_scalar(
            "INSERT INTO app.screen_run (strategy_id, trade_date, scanned, matched) VALUES ($1, $2, $3, $4)
             ON CONFLICT (strategy_id, trade_date) DO UPDATE SET scanned = EXCLUDED.scanned, matched = EXCLUDED.matched, ran_at = now()
             RETURNING run_id",
        )
        .bind(id)
        .bind(out.trade_date)
        .bind(out.scanned as i32)
        .bind(res.matched as i32)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM app.screen_hit WHERE run_id = $1").bind(run_id).execute(&mut *tx).await?;
        for (rank, h) in res.hits.iter().enumerate() {
            let metrics = serde_json::json!({ "name": h.name, "close": h.close, "change_pct": finite(h.change_pct), "amount": h.amount, "sort_value": finite(h.sort_value) });
            sqlx::query("INSERT INTO app.screen_hit (run_id, thscode, rank, is_new, reasons, metrics) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(run_id)
                .bind(&h.thscode)
                .bind(rank as i32 + 1)
                .bind(prev.as_ref().is_some_and(|p| !p.contains(&h.thscode)))
                .bind(serde_json::json!(h.reasons))
                .bind(metrics)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    let ms = started.elapsed().as_millis() as u64;
    tracing::info!(trade_date = %out.trade_date, strategies = list.len(), scanned = out.scanned, ms, "每日巡检完成");
    Ok(ScreeningSummary { trade_date: Some(out.trade_date), strategies: list.len(), scanned: out.scanned, ms })
}

/// JSON 不能表示 NaN
fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

pub async fn load_strategy(pool: &PgPool, id: i64) -> anyhow::Result<Option<Definition>> {
    let v: Option<serde_json::Value> = sqlx::query_scalar("SELECT definition FROM app.strategy WHERE strategy_id = $1").bind(id).fetch_optional(pool).await?;
    Ok(match v {
        Some(v) => Some(serde_json::from_value(v)?),
        None => None,
    })
}

/// 第一次启动时，把内置模板复制为普通策略并开启巡检，让「选股」页不是空的。只做一次：删掉的不会再出现
pub async fn seed_templates(pool: &PgPool) -> anyhow::Result<()> {
    let seeded: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM app.setting WHERE name = 'strategies_seeded')").fetch_one(pool).await?;
    if seeded {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for t in super::strategy::templates() {
        sqlx::query("INSERT INTO app.strategy (name, description, definition, template) VALUES ($1, $2, $3, $4)")
            .bind(t.name)
            .bind(t.description)
            .bind(serde_json::to_value(&t.definition)?)
            .bind(t.key)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO app.setting (name, value) VALUES ('strategies_seeded', 'v1') ON CONFLICT DO NOTHING").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// 已保存的巡检结果中，每只股票最近入选了哪些策略（个股诊断用）
pub async fn latest_hits_for(pool: &PgPool, code: &str) -> sqlx::Result<HashMap<i64, NaiveDate>> {
    let rows: Vec<(i64, NaiveDate)> = sqlx::query_as(
        "SELECT r.strategy_id, max(r.trade_date) FROM app.screen_hit h JOIN app.screen_run r USING (run_id) WHERE h.thscode = $1 GROUP BY 1",
    )
    .bind(code)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}
