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
/// 封板率等统计：2025-10-17 起，只有今日热点用，不影响市场概况
pub const STATS: &str = "special.limit_up_stats";
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

/// 缓存用的数据版本（`ops.derived_version`）：依赖的那几行中任一更新，结果就不再命中。
/// - `adj_factor`：复权价（行情或公司行动有变化、重算复权之后）；
/// - `limit_pool`：权威涨跌停池；
/// - `factor_input`：因子排名的非价格输入（财报、行业成分、证券名称和 ST），迁移 20261023000003 加入。
///
/// 升级前的库里查不到的行按 None 处理。多个进程各自的缓存都按这些版本失效，不依赖进程内的清空调用
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CacheVersions {
    pub adj_factor: Option<DateTime<Utc>>,
    pub limit_pool: Option<DateTime<Utc>>,
    pub factor_input: Option<DateTime<Utc>>,
}

impl CacheVersions {
    pub async fn load(pool: &PgPool) -> sqlx::Result<Self> {
        let rows: Vec<(String, DateTime<Utc>)> =
            sqlx::query_as("SELECT name, updated_at FROM ops.derived_version WHERE name = ANY($1)")
                .bind(&["adj_factor", "limit_pool", "factor_input"][..])
                .fetch_all(pool)
                .await?;
        let mut v = Self::default();
        for (name, at) in rows {
            match name.as_str() {
                "adj_factor" => v.adj_factor = Some(at),
                "limit_pool" => v.limit_pool = Some(at),
                "factor_input" => v.factor_input = Some(at),
                _ => {}
            }
        }
        Ok(v)
    }

    /// 页面结果（市场概况、今日热点、板块排名）的版本：复权价和涨跌停池
    pub fn market_token(&self) -> String {
        format!("adj={:?}:limit={:?}", self.adj_factor, self.limit_pool)
    }

    /// 因子排名的版本：复权价和因子的非价格输入
    pub fn factor_token(&self) -> String {
        format!("adj={:?}:input={:?}", self.adj_factor, self.factor_input)
    }
}
