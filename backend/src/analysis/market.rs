//! 市场环境：某个交易日全市场的涨跌家数、涨跌停数、站上 20 日均线的比例，以及行业、概念的强弱排名。
//! 口径见 docs/domains/strategies.md#market。

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::indicators::{Frame, IndicatorId, IndicatorRef, StockMeta};
use super::prices;

/// 股票的静态信息和简称
pub struct Securities {
    pub meta: HashMap<String, StockMeta>,
    pub names: HashMap<String, String>,
}

pub async fn securities(pool: &PgPool) -> sqlx::Result<Securities> {
    let rows: Vec<(String, String, String, Option<NaiveDate>)> =
        sqlx::query_as("SELECT thscode, name, board, list_date FROM ref.security").fetch_all(pool).await?;
    let mut meta = HashMap::new();
    let mut names = HashMap::new();
    for (code, name, board, list_date) in rows {
        meta.insert(code.clone(), StockMeta { board, is_st: name.contains("ST"), list_date });
        names.insert(code, name);
    }
    Ok(Securities { meta, names })
}

/// 本地日线中不晚于 date 的最新交易日；date 为空时取最新
pub async fn resolve_date(pool: &PgPool, date: Option<NaiveDate>) -> sqlx::Result<Option<NaiveDate>> {
    sqlx::query_scalar("SELECT max(trade_date) FROM mkt.stock_bar_1d WHERE ($1::date IS NULL OR trade_date <= $1)").bind(date).fetch_one(pool).await
}

/// 读多少个自然日才能覆盖 bars 根日线（交易日约占自然日的 68%，再留出长假）
pub fn calendar_days_for(bars: u32) -> i64 {
    bars as i64 * 3 / 2 + 30
}

#[derive(Serialize, ToSchema, Clone)]
pub struct Overview {
    pub trade_date: NaiveDate,
    /// 当天有日线的股票数
    pub stocks: usize,
    pub up: usize,
    pub down: usize,
    pub flat: usize,
    pub limit_up: usize,
    pub limit_down: usize,
    /// 收盘站在 20 日均线上方的比例（0.6 表示 60%）
    pub above_ma20: f64,
    /// 涨跌幅的中位数（%）
    pub median_change_pct: f64,
}

// @project-doc docs/domains/strategies.md#market
pub async fn overview(pool: &PgPool, date: Option<NaiveDate>) -> anyhow::Result<Option<Overview>> {
    let Some(date) = resolve_date(pool, date).await? else { return Ok(None) };
    let sec = securities(pool).await?;
    let (mut up, mut down, mut flat, mut lu, mut ld, mut above, mut with_ma) = (0, 0, 0, 0, 0, 0usize, 0usize);
    let mut changes = vec![];
    prices::for_each_stock(pool, date - chrono::Duration::days(calendar_days_for(25)), date, |code, bars| {
        let Some(last) = bars.last() else { return };
        if last.date != date || bars.len() < 2 {
            return;
        }
        let f = Frame::new(bars, sec.meta.get(code));
        let i = bars.len() - 1;
        let ch = f.at(IndicatorRef::new(IndicatorId::ChangePct, None), i);
        changes.push(ch);
        if ch > 0.0 {
            up += 1;
        } else if ch < 0.0 {
            down += 1;
        } else {
            flat += 1;
        }
        let l = f.limits();
        lu += l.limit_up[i] as usize;
        ld += l.limit_down[i] as usize;
        let ma20 = f.at(IndicatorRef::new(IndicatorId::Ma, Some(20)), i);
        if !ma20.is_nan() {
            with_ma += 1;
            above += (last.q_close > ma20) as usize;
        }
    })
    .await?;
    changes.sort_by(|a, b| a.total_cmp(b));
    let median = changes.get(changes.len() / 2).copied().unwrap_or(f64::NAN);
    Ok(Some(Overview {
        trade_date: date,
        stocks: changes.len(),
        up,
        down,
        flat,
        limit_up: lu,
        limit_down: ld,
        above_ma20: if with_ma > 0 { above as f64 / with_ma as f64 } else { 0.0 },
        median_change_pct: median,
    }))
}

#[derive(Serialize, ToSchema, Clone)]
pub struct SectorStrength {
    pub thscode: String,
    pub name: String,
    /// 最近 days 个交易日的涨跌幅（%）
    pub change_pct: f64,
    /// 排名（1 表示最强）
    pub rank: usize,
    /// 当前成分股数
    pub members: i64,
}

/// 一类板块（行业或概念）在 date 之前 days 个交易日的涨跌幅排名。板块指数日线从 2022 年起才有
pub async fn sector_strength(pool: &PgPool, category: &str, days: u32, date: NaiveDate) -> sqlx::Result<Vec<SectorStrength>> {
    let rows: Vec<(String, String, Option<f64>, i64)> = sqlx::query_as(
        "WITH b AS (
           SELECT x.thscode, x.close::float8 AS close, row_number() OVER (PARTITION BY x.thscode ORDER BY x.trade_date DESC) AS rn
           FROM mkt.index_bar_1d x JOIN ref.index_info i ON i.thscode = x.thscode
           WHERE i.category = $1 AND x.trade_date <= $2 AND x.trade_date > $2 - ($3 * 2 + 20))
         SELECT i.thscode, i.name,
                max(b.close) FILTER (WHERE b.rn = 1) / nullif(max(b.close) FILTER (WHERE b.rn = $3 + 1), 0) * 100 - 100,
                (SELECT count(*) FROM ref.index_member m WHERE m.index_code = i.thscode AND m.valid_to IS NULL)
         FROM ref.index_info i JOIN b ON b.thscode = i.thscode
         WHERE i.category = $1 GROUP BY i.thscode, i.name",
    )
    .bind(category)
    .bind(date)
    .bind(days as i32)
    .fetch_all(pool)
    .await?;
    let mut out: Vec<SectorStrength> = rows
        .into_iter()
        .filter_map(|(thscode, name, ch, members)| ch.map(|change_pct| SectorStrength { thscode, name, change_pct, rank: 0, members }))
        .collect();
    out.sort_by(|a, b| b.change_pct.total_cmp(&a.change_pct));
    for (i, s) in out.iter_mut().enumerate() {
        s.rank = i + 1;
    }
    Ok(out)
}

/// 属于排名前 top_pct% 的板块的股票（按当前成分）
pub async fn stocks_in_top_sectors(pool: &PgPool, category: &str, days: u32, top_pct: f64, date: NaiveDate) -> sqlx::Result<HashSet<String>> {
    let ranks = sector_strength(pool, category, days, date).await?;
    let keep = ((ranks.len() as f64 * top_pct / 100.0).ceil() as usize).max(1);
    let codes: Vec<&str> = ranks.iter().take(keep).map(|s| s.thscode.as_str()).collect();
    let members: Vec<String> = sqlx::query_scalar("SELECT DISTINCT thscode FROM ref.index_member WHERE index_code = ANY($1) AND valid_to IS NULL")
        .bind(&codes)
        .fetch_all(pool)
        .await?;
    Ok(members.into_iter().collect())
}

/// 市场概况的缓存：要把全市场的日线算一遍（约 1 秒），结果只在有新日线后才变。
/// 以交易日和 `ops.derived_version` 中 adj_factor 的时间戳为键，每次同步写入行情后时间戳更新，旧结果不再命中
#[derive(Default)]
pub struct OverviewCache {
    entries: std::sync::Mutex<Vec<OverviewEntry>>,
}

/// (交易日, 数据版本, 结果)
type OverviewEntry = (NaiveDate, Option<chrono::DateTime<chrono::Utc>>, Overview);

impl OverviewCache {
    pub async fn get(&self, pool: &PgPool, date: Option<NaiveDate>) -> anyhow::Result<Option<Overview>> {
        let Some(date) = resolve_date(pool, date).await? else { return Ok(None) };
        let version: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT updated_at FROM ops.derived_version WHERE name = 'adj_factor'").fetch_optional(pool).await?;
        if let Some(hit) = self.entries.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(d, v, _)| *d == date && *v == version) {
            return Ok(Some(hit.2.clone()));
        }
        let Some(o) = overview(pool, Some(date)).await? else { return Ok(None) };
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|(_, v, _)| *v == version);
        entries.push((date, version, o.clone()));
        if entries.len() > 16 {
            entries.remove(0);
        }
        Ok(Some(o))
    }
}
