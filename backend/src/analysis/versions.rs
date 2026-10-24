//! 派生数据的版本（`ops.derived_version`）：缓存按它失效，应用层在数据变化后更新它。
//!
//! 进程内缓存（市场概况、账户的因子排名）和 Redis 共享缓存都把相关版本放进键里，版本变了旧结果自然不再命中，
//! 多个进程并存时也不依赖谁去清空。`adj_factor` 行由同步层重算复权后直接更新（同步层不依赖分析模块），
//! 其余两行由应用事件编排器通过 [`bump`] 更新。

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// 应用层负责更新的派生数据版本
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Derived {
    /// 权威涨跌停池
    LimitPool,
    /// 因子排名的非价格输入：财报、行业成分、证券名称和 ST
    FactorInput,
}

impl Derived {
    pub const fn key(self) -> &'static str {
        match self {
            Self::LimitPool => "limit_pool",
            Self::FactorInput => "factor_input",
        }
    }
}

/// 记下这份派生数据变了：依赖它的缓存都不再命中
pub async fn bump(pool: &PgPool, which: Derived) -> sqlx::Result<()> {
    sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = $1")
        .bind(which.key())
        .execute(pool)
        .await?;
    Ok(())
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
                .bind(
                    &[
                        "adj_factor",
                        Derived::LimitPool.key(),
                        Derived::FactorInput.key(),
                    ][..],
                )
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
