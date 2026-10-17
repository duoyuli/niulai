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
use super::points::{self, Today};
use super::prices;
use super::strategy::Definition;

/// 入选的一只股票：今日买点，或预览时今天满足买入条件
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

/// 今日卖点或持有中的一只股票
#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct PositionHit {
    pub thscode: String,
    pub name: String,
    /// 收盘价（未复权）
    pub close: f64,
    pub change_pct: f64,
    /// 买入日和买入价（未复权开盘价）
    pub entry_date: NaiveDate,
    pub entry_price: f64,
    /// 买入以来的涨跌（%，前复权）
    pub return_pct: f64,
    /// 已持有的交易日数
    pub bars_held: i64,
    /// 卖点的原因（白话）；持有中为空
    pub reason: Option<String>,
}

pub struct StrategyResult {
    /// 今日买点数（预览时是今天满足买入条件的股票数），截断前
    pub matched: usize,
    /// 按排序截取后的买点
    pub hits: Vec<Hit>,
    /// 今日卖点；预览时为空
    pub sells: Vec<PositionHit>,
    /// 持有中；预览时为空
    pub holdings: Vec<PositionHit>,
}

pub struct Outcome {
    pub trade_date: NaiveDate,
    /// 当天有日线的股票数
    pub scanned: usize,
    /// 与传入的策略一一对应
    pub results: Vec<StrategyResult>,
}

/// 买卖点从多少个交易日前开始算（空仓起步）。持有中的股票按这一年内的买卖点推算
pub const POINT_DAYS: usize = 250;
/// 每个策略最多保存多少只持有中的股票
const MAX_HOLDINGS: usize = 300;

/// 怎么选：预览只看今天是否满足买入条件；巡检按买卖点给出今日买点、卖点和持有中
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Preview,
    Points,
}

// @project-doc docs/domains/strategies.md#screening
/// 对 date（为空时取最新交易日）执行这些策略。没有日线时返回 None
pub async fn screen(pool: &PgPool, defs: &[&Definition], date: Option<NaiveDate>, mode: Mode) -> anyhow::Result<Option<Outcome>> {
    let Some(date) = market::resolve_date(pool, date).await? else { return Ok(None) };
    let sec = market::securities(pool).await?;
    // 强势板块过滤：每个策略预先算好允许的股票（只作用于买点）
    let mut sector_sets: Vec<Option<HashSet<String>>> = vec![];
    for d in defs {
        sector_sets.push(match &d.universe.sector {
            Some(s) => Some(market::stocks_in_top_sectors(pool, &s.category, s.days, s.top_pct, date).await?),
            None => None,
        });
    }
    let window = if mode == Mode::Points { POINT_DAYS as u32 } else { 0 };
    let warmup = defs.iter().map(|d| d.warmup()).max().unwrap_or(30) + 5 + window;
    let mut found: Vec<StrategyResult> = defs.iter().map(|_| StrategyResult { matched: 0, hits: vec![], sells: vec![], holdings: vec![] }).collect();
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
        let name = || sec.names.get(code).cloned().unwrap_or_default();
        let change_pct = f.at(IndicatorRef::new(IndicatorId::ChangePct, None), i);
        for (k, d) in defs.iter().enumerate() {
            let sector_ok = sector_sets[k].as_ref().is_none_or(|s| s.contains(code));
            let buy_hit = |reasons| Hit {
                thscode: code.to_string(),
                name: name(),
                close: last.close,
                change_pct,
                amount: last.amount,
                sort_value: f.at(d.sort.by, i),
                reasons,
            };
            if mode == Mode::Preview {
                if sector_ok && d.in_universe(&f, meta, i) {
                    let e = d.eval(&f, i);
                    if e.matched {
                        found[k].hits.push(buy_hit(e.reasons));
                    }
                }
                continue;
            }
            let p = points::run(bars, &f, meta, d, bars.len().saturating_sub(POINT_DAYS));
            let position = |reason: Option<String>| {
                p.trades.last().map(|t| PositionHit {
                    thscode: code.to_string(),
                    name: name(),
                    close: last.close,
                    change_pct,
                    entry_date: t.entry_date,
                    entry_price: t.entry_price,
                    return_pct: t.return_pct,
                    bars_held: t.bars_held,
                    reason,
                })
            };
            match p.today {
                Today::Buy if sector_ok => found[k].hits.push(buy_hit(d.eval(&f, i).reasons)),
                Today::Sell { text, .. } => found[k].sells.extend(position(Some(text))),
                Today::Holding => found[k].holdings.extend(position(None)),
                _ => {}
            }
        }
    })
    .await?;
    let results = defs
        .iter()
        .zip(found)
        .map(|(d, mut r)| {
            r.matched = r.hits.len();
            // 排序值缺失（NaN）的排在最后
            r.hits.sort_by(|a, b| {
                let (x, y) = (a.sort_value, b.sort_value);
                match (x.is_nan(), y.is_nan()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ if d.sort.desc => y.total_cmp(&x),
                    _ => x.total_cmp(&y),
                }
            });
            r.hits.truncate(d.limit as usize);
            r.sells.sort_by(|a, b| a.return_pct.total_cmp(&b.return_pct));
            r.holdings.sort_by(|a, b| b.return_pct.total_cmp(&a.return_pct));
            r.holdings.truncate(MAX_HOLDINGS);
            r
        })
        .collect();
    Ok(Some(Outcome { trade_date: date, scanned, results }))
}

/// 开启巡检的全部策略
pub async fn screening_strategies(pool: &PgPool) -> anyhow::Result<Vec<(i64, Definition)>> {
    let rows: Vec<(i64, serde_json::Value)> =
        sqlx::query_as("SELECT strategy_id, definition FROM app.strategy WHERE screening ORDER BY strategy_id").fetch_all(pool).await?;
    let lib = super::signal_lib::load(pool).await?;
    let mut out = vec![];
    for (id, v) in rows {
        match serde_json::from_value::<Definition>(v) {
            Ok(d) => out.push((id, d.with_lib(lib.clone()))),
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
    let Some(out) = screen(pool, &defs, date, Mode::Points).await? else {
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
                sqlx::query_scalar("SELECT thscode FROM app.screen_hit WHERE run_id = $1 AND kind = 'buy'").bind(r).fetch_all(&mut *tx).await?.into_iter().collect(),
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
        let insert = "INSERT INTO app.screen_hit (run_id, thscode, rank, is_new, reasons, metrics, kind) VALUES ($1, $2, $3, $4, $5, $6, $7)";
        for (rank, h) in res.hits.iter().enumerate() {
            let metrics = serde_json::json!({ "name": h.name, "close": h.close, "change_pct": finite(h.change_pct), "amount": h.amount, "sort_value": finite(h.sort_value) });
            sqlx::query(insert)
                .bind(run_id)
                .bind(&h.thscode)
                .bind(rank as i32 + 1)
                .bind(prev.as_ref().is_some_and(|p| !p.contains(&h.thscode)))
                .bind(serde_json::json!(h.reasons))
                .bind(metrics)
                .bind("buy")
                .execute(&mut *tx)
                .await?;
        }
        for (kind, list) in [("sell", &res.sells), ("holding", &res.holdings)] {
            for (rank, h) in list.iter().enumerate() {
                let metrics = serde_json::json!({
                    "name": h.name, "close": h.close, "change_pct": finite(h.change_pct), "entry_date": h.entry_date,
                    "entry_price": h.entry_price, "return_pct": finite(h.return_pct), "bars_held": h.bars_held,
                });
                sqlx::query(insert)
                    .bind(run_id)
                    .bind(&h.thscode)
                    .bind(rank as i32 + 1)
                    .bind(false)
                    .bind(serde_json::json!(h.reason.iter().collect::<Vec<_>>()))
                    .bind(metrics)
                    .bind(kind)
                    .execute(&mut *tx)
                    .await?;
            }
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
        Some(v) => Some(serde_json::from_value::<Definition>(v)?.with_lib(super::signal_lib::load(pool).await?)),
        None => None,
    })
}

/// 内置策略：每个模板一行（`builtin_key` 是模板的 key），启动时按模板内容同步，只读、总是参加巡检。
/// 账户的调仓建议默认参考全部参加巡检的策略，内置策略保证总有一批不会被改坏的。还没有内置行时，先接管内容和模板完全一样的普通策略
/// （以前第一次启动时由模板复制出来、没有改过的），避免同一个策略出现两份；改过的保持为普通策略。
/// 模板删掉后，对应的行变回普通策略，巡检记录保留
pub async fn sync_builtin_strategies(pool: &PgPool) -> anyhow::Result<()> {
    let list = super::strategy::templates();
    let keys: Vec<&str> = list.iter().map(|t| t.key).collect();
    let mut tx = pool.begin().await?;
    for t in &list {
        let def = serde_json::to_value(&t.definition)?;
        let updated = sqlx::query(
            "UPDATE app.strategy SET name = $2, description = $3, definition = $4, template = $1, screening = true, updated_at = now()
             WHERE builtin_key = $1 AND (name, description, definition, screening) IS DISTINCT FROM ($2, $3, $4, true)",
        )
        .bind(t.key)
        .bind(t.name)
        .bind(t.description)
        .bind(&def)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM app.strategy WHERE builtin_key = $1)").bind(t.key).fetch_one(&mut *tx).await?;
        if exists {
            if updated > 0 {
                tracing::info!(key = t.key, "内置策略已按模板更新");
            }
            continue;
        }
        let adopted = sqlx::query(
            "UPDATE app.strategy SET builtin_key = $1, name = $2, description = $3, screening = true, updated_at = now()
             WHERE strategy_id = (SELECT min(strategy_id) FROM app.strategy WHERE builtin_key IS NULL AND template = $1 AND definition = $4)",
        )
        .bind(t.key)
        .bind(t.name)
        .bind(t.description)
        .bind(&def)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if adopted == 0 {
            sqlx::query("INSERT INTO app.strategy (name, description, definition, template, builtin_key) VALUES ($1, $2, $3, $4, $4)")
                .bind(t.name)
                .bind(t.description)
                .bind(&def)
                .bind(t.key)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query("UPDATE app.strategy SET builtin_key = NULL WHERE builtin_key IS NOT NULL AND NOT (builtin_key = ANY($1))").bind(&keys).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// 内置策略的编号，按模板 key
pub async fn builtin_ids(pool: &PgPool) -> sqlx::Result<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT builtin_key, strategy_id FROM app.strategy WHERE builtin_key IS NOT NULL").fetch_all(pool).await?;
    Ok(rows.into_iter().collect())
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
