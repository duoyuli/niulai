//! 权威涨跌停池：供应商提供的涨停、跌停、炸板名单，只看 `raw.record`（没有类型化表）。市场概况用当天的涨停、跌停名单。
//!
//! 涨停池从 2020-07-22 起每个交易日齐全，也是判断「池这一天有没有数据」的权威交易日历；
//! 跌停池只有 2025-10-16 起，更早的日期和北交所（池里没有）回退到 `market::limit_flags` 的自算规则。
//! 为什么以池为准、两段分界和各处分界见 docs/decisions/authoritative_limits.md，口径见 docs/domains/market_data_rules.md#limits。

use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;

/// 涨停池：历史最深（2020-07-22 起），同时用作权威交易日历
pub const UP: &str = "special.limit_up_pool";
/// 跌停池：2025-10-16 起
pub const DOWN: &str = "special.limit_down_pool";
/// 炸板池：2025-10-16 起
pub const BREAK: &str = "special.limit_break_pool";
/// 三个池的数据集 ID：同步运行结束后据此判断 `limit_pool` 版本是否要更新
pub const DATASETS: [&str; 3] = [UP, DOWN, BREAK];

/// 某一天的权威名单。为空表示这一天池没有覆盖，按交易所规则自算
#[derive(Default, Clone, Debug)]
pub struct DayLimits {
    /// 涨停的股票；涨停池这一天有数据（是权威交易日）时才有
    pub up: Option<HashSet<String>>,
    /// 跌停的股票；这一天是权威交易日，且在跌停池的数据范围内时才有
    pub down: Option<HashSet<String>>,
}

impl DayLimits {
    pub async fn load(pool: &PgPool, date: NaiveDate) -> sqlx::Result<Self> {
        // 涨停池每天有 1 行 thscode 为空：按它判断这一天池有没有数据，不算进名单
        let up_rows: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT thscode FROM raw.record WHERE dataset_id = $1 AND obs_date = $2",
        )
        .bind(UP)
        .bind(date)
        .fetch_all(pool)
        .await?;
        if up_rows.is_empty() {
            return Ok(Self::default());
        }
        let (from, through): (Option<NaiveDate>, Option<NaiveDate>) = sqlx::query_as(
            "SELECT min(obs_date), max(obs_date) FROM raw.record WHERE dataset_id = $1",
        )
        .bind(DOWN)
        .fetch_one(pool)
        .await?;
        let down = match (from, through) {
            (Some(f), Some(t)) if f <= date && date <= t => Some(
                sqlx::query_scalar(
                    "SELECT thscode FROM raw.record WHERE dataset_id = $1 AND obs_date = $2 AND thscode IS NOT NULL",
                )
                .bind(DOWN)
                .bind(date)
                .fetch_all(pool)
                .await?
                .into_iter()
                .collect(),
            ),
            _ => None,
        };
        Ok(Self {
            up: Some(up_rows.into_iter().flatten().collect()),
            down,
        })
    }
}

/// 进程内缓存用的数据版本：adj_factor 和 limit_pool 中任一更新，依赖它的结果就不再命中。
/// `limit_pool` 行由迁移 20261018000013 加入；升级前的库里查不到，按 None 处理
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CacheVersions {
    pub adj_factor: Option<DateTime<Utc>>,
    pub limit_pool: Option<DateTime<Utc>>,
}

impl CacheVersions {
    pub async fn load(pool: &PgPool) -> sqlx::Result<Self> {
        let rows: Vec<(String, DateTime<Utc>)> =
            sqlx::query_as("SELECT name, updated_at FROM ops.derived_version WHERE name = ANY($1)")
                .bind(&["adj_factor", "limit_pool"][..])
                .fetch_all(pool)
                .await?;
        let mut v = Self::default();
        for (name, at) in rows {
            match name.as_str() {
                "adj_factor" => v.adj_factor = Some(at),
                "limit_pool" => v.limit_pool = Some(at),
                _ => {}
            }
        }
        Ok(v)
    }
}
