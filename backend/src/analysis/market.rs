//! 市场环境：某个交易日全市场的涨跌家数、涨跌停数、站上 20 日均线的比例，以及行业、概念的强弱排名。
//! 口径见 docs/domains/market_data_rules.md#market。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

use super::limit_pool::DayLimits;
use super::prices::{self, DailyBar};
use super::securities::{self, StPeriod, StockMeta, st_on};
use crate::instrument::AssetType;

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

#[derive(Deserialize, Serialize, ToSchema, Clone)]
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

// @project-doc docs/domains/market_data_rules.md#market
pub async fn overview(pool: &PgPool, date: Option<NaiveDate>) -> anyhow::Result<Option<Overview>> {
    let Some(date) = resolve_date(pool, date).await? else {
        return Ok(None);
    };
    let sec = securities::load(pool).await?;
    let lo = date - chrono::Duration::days(calendar_days_for(25));
    // 涨停、跌停数以供应商的名单为准；名单没有覆盖的日期和北交所回退自算
    let limits = DayLimits::load(pool, date).await?;
    let (mut up, mut down, mut flat, mut lu, mut ld, mut above, mut with_ma) =
        (0, 0, 0, 0, 0, 0usize, 0usize);
    let mut changes = vec![];
    prices::for_each_stock(pool, lo, date, |code, bars| {
        let Some(meta) = sec.meta.get(code) else {
            return;
        };
        let n = bars.len();
        if n < 2 || bars[n - 1].date != date {
            return;
        }
        let (prev, last) = (&bars[n - 2], &bars[n - 1]);
        let ch = (last.q_close / prev.q_close - 1.0) * 100.0;
        changes.push(ch);
        if ch > 0.0 {
            up += 1;
        } else if ch < 0.0 {
            down += 1;
        } else {
            flat += 1;
        }
        let (is_up, is_down) = limit_flags(bars, meta, sec.st_history(code), &limits, code);
        lu += is_up as usize;
        ld += is_down as usize;
        if n >= 20 {
            let ma20 = bars[n - 20..].iter().map(|b| b.q_close).sum::<f64>() / 20.0;
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

// @project-doc docs/domains/market_data_rules.md#limits
/// 最后一根日线是否涨停、跌停。池覆盖的日期以供应商的名单为准（北交所不在池里）；其余按交易所规则自算：
/// 涨跌幅限制按板块和 ST 确定，基准是前一交易日按除权调整后的收盘价，四舍五入到分；新股上市前 5 个交易日不判断
fn limit_flags(
    bars: &[DailyBar],
    meta: &StockMeta,
    st: &[StPeriod],
    pool: &DayLimits,
    code: &str,
) -> (bool, bool) {
    let n = bars.len();
    let (prev, last) = (&bars[n - 2], &bars[n - 1]);
    if meta.asset_type != AssetType::Stock {
        return (false, false);
    }
    // 本地第一根日线就是上市首日时，前 5 根没有涨跌幅限制
    if meta
        .list_date
        .is_some_and(|d| bars[0].date <= d + chrono::Duration::days(3) && n <= 5)
    {
        return (false, false);
    }
    let own = self_limits(prev, last, meta, st);
    if meta.board == "bse" {
        return own;
    }
    (
        pool.up.as_ref().map_or(own.0, |up| up.contains(code)),
        pool.down.as_ref().map_or(own.1, |down| down.contains(code)),
    )
}

fn self_limits(
    prev: &DailyBar,
    last: &DailyBar,
    meta: &StockMeta,
    st: &[StPeriod],
) -> (bool, bool) {
    let chinext_reform = NaiveDate::from_ymd_opt(2020, 8, 24).unwrap();
    let pct = match meta.board.as_str() {
        "bse" => 0.30,
        "star" => 0.20,
        "chinext" if last.date >= chinext_reform => 0.20,
        _ if st_on(meta, st, last.date) => 0.05,
        _ => 0.10,
    };
    // 前复权价之比消去了除权：今天的原价 × 昨天和今天的前复权价之比 = 昨天的收盘价按除权调整后的值
    let base = last.close * prev.q_close / last.q_close;
    let round2 = |v: f64| (v * 100.0).round() / 100.0;
    (
        last.close >= round2(base * (1.0 + pct)) - 0.005,
        last.close <= round2(base * (1.0 - pct)) + 0.005,
    )
}

#[derive(Deserialize, Serialize, ToSchema, Clone)]
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
                (SELECT count(*) FROM ref.index_member m WHERE m.index_code = i.thscode
                    AND m.valid_from <= $2 AND (m.valid_to IS NULL OR m.valid_to > $2))
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
