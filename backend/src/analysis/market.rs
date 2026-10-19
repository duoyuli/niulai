//! 市场环境：某个交易日全市场的涨跌家数、涨跌停数、站上 20 日均线的比例，以及行业、概念的强弱排名。
//! 口径见 docs/domains/strategies.md#market。

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::indicators::{AssetType, Frame, IndicatorId, IndicatorRef, StPeriod, StockMeta};
use super::prices;

/// 股票的静态信息和简称
pub struct Securities {
    pub meta: HashMap<String, StockMeta>,
    pub names: HashMap<String, String>,
    pub st_history: HashMap<String, Vec<StPeriod>>,
}

type InstrumentRow = (String, String, String, Option<String>, Option<NaiveDate>);

pub async fn securities(pool: &PgPool) -> sqlx::Result<Securities> {
    let rows: Vec<InstrumentRow> =
        sqlx::query_as("SELECT thscode, name, board, asset_type, list_date FROM ref.instrument")
            .fetch_all(pool)
            .await?;
    let mut meta = HashMap::new();
    let mut names = HashMap::new();
    for (code, name, board, asset_type, list_date) in rows {
        meta.insert(
            code.clone(),
            StockMeta {
                board,
                is_st: name.contains("ST"),
                list_date,
                asset_type: AssetType::from_db(asset_type.as_deref()),
            },
        );
        names.insert(code, name);
    }
    let rows: Vec<(String, NaiveDate, Option<NaiveDate>, bool)> = sqlx::query_as(
        "SELECT thscode, valid_from, valid_to, is_st FROM ref.security_name_hist ORDER BY thscode, valid_from",
    )
    .fetch_all(pool)
    .await?;
    let mut st_history: HashMap<String, Vec<StPeriod>> = HashMap::new();
    for (code, valid_from, valid_to, is_st) in rows {
        st_history.entry(code).or_default().push(StPeriod {
            valid_from,
            valid_to,
            is_st,
        });
    }
    Ok(Securities {
        meta,
        names,
        st_history,
    })
}

/// 本地日线中不晚于 date 的最新交易日；date 为空时取最新
pub async fn resolve_date(
    pool: &PgPool,
    date: Option<NaiveDate>,
) -> sqlx::Result<Option<NaiveDate>> {
    sqlx::query_scalar(
        "SELECT max(trade_date) FROM (
             SELECT trade_date FROM mkt.stock_bar_1d
             UNION
             SELECT trade_date FROM mkt.etf_bar_1d
         ) d WHERE ($1::date IS NULL OR trade_date <= $1)",
    )
    .bind(date)
    .fetch_one(pool)
    .await
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
    let Some(date) = resolve_date(pool, date).await? else {
        return Ok(None);
    };
    let sec = securities(pool).await?;
    let lo = date - chrono::Duration::days(calendar_days_for(25));
    // 涨停、跌停数以供应商的名单为准；名单没有覆盖的日期回退自算
    let limits = super::limit_pool::LimitPool::load(pool, lo, date).await?;
    let (mut up, mut down, mut flat, mut lu, mut ld, mut above, mut with_ma) =
        (0, 0, 0, 0, 0, 0usize, 0usize);
    let mut changes = vec![];
    prices::for_each_stock(pool, lo, date, |code, bars| {
        let Some(last) = bars.last() else { return };
        if last.date != date || bars.len() < 2 {
            return;
        }
        let history = sec.st_history.get(code).map(Vec::as_slice).unwrap_or(&[]);
        let f =
            Frame::with_authority_and_st_history(bars, sec.meta.get(code), &limits, code, history);
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
        above_ma20: if with_ma > 0 {
            above as f64 / with_ma as f64
        } else {
            0.0
        },
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
pub async fn sector_strength(
    pool: &PgPool,
    category: &str,
    days: u32,
    date: NaiveDate,
) -> sqlx::Result<Vec<SectorStrength>> {
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
        .filter_map(|(thscode, name, ch, members)| {
            ch.map(|change_pct| SectorStrength {
                thscode,
                name,
                change_pct,
                rank: 0,
                members,
            })
        })
        .collect();
    out.sort_by(|a, b| b.change_pct.total_cmp(&a.change_pct));
    for (i, s) in out.iter_mut().enumerate() {
        s.rank = i + 1;
    }
    Ok(out)
}

/// 属于排名前 top_pct% 的板块的股票（按当前成分）
pub async fn stocks_in_top_sectors(
    pool: &PgPool,
    category: &str,
    days: u32,
    top_pct: f64,
    date: NaiveDate,
) -> sqlx::Result<HashSet<String>> {
    let ranks = sector_strength(pool, category, days, date).await?;
    let keep = ((ranks.len() as f64 * top_pct / 100.0).ceil() as usize).max(1);
    let codes: Vec<&str> = ranks
        .iter()
        .take(keep)
        .map(|s| s.thscode.as_str())
        .collect();
    let members: Vec<String> = sqlx::query_scalar("SELECT DISTINCT thscode FROM ref.index_member WHERE index_code = ANY($1) AND valid_to IS NULL")
        .bind(&codes)
        .fetch_all(pool)
        .await?;
    Ok(members.into_iter().collect())
}

/// 市场概况的缓存：要把全市场的日线算一遍（约 1 秒），结果只在有新日线或新的涨跌停名单后才变。
/// 以交易日和 `ops.derived_version` 中 adj_factor、limit_pool 的时间戳为键，任一时间戳更新，旧结果不再命中
#[derive(Default)]
pub struct OverviewCache {
    entries: std::sync::Mutex<Vec<OverviewEntry>>,
}

/// (交易日, 数据版本, 结果)
type OverviewEntry = (NaiveDate, super::limit_pool::CacheVersions, Overview);

impl OverviewCache {
    pub async fn get(
        &self,
        pool: &PgPool,
        date: Option<NaiveDate>,
    ) -> anyhow::Result<Option<Overview>> {
        let Some(date) = resolve_date(pool, date).await? else {
            return Ok(None);
        };
        let version = super::limit_pool::CacheVersions::load(pool).await?;
        if let Some(hit) = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(d, v, _)| *d == date && *v == version)
        {
            return Ok(Some(hit.2.clone()));
        }
        let Some(o) = overview(pool, Some(date)).await? else {
            return Ok(None);
        };
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|(_, v, _)| *v == version);
        entries.push((date, version, o.clone()));
        if entries.len() > 16 {
            entries.remove(0);
        }
        Ok(Some(o))
    }
}
