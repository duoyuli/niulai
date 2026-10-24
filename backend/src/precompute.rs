//! 首页常用全市场结果的后台预热：市场概况、行业和概念板块近 5 日排名、今日热点。
//!
//! 由 `server` 作为常驻组件启动（只在配置了 Redis 时）。计算在 `analysis`，键和降级在 `cache`，
//! 和接口按需计算走同一个键，所以预热写入的结果接口能直接命中。见 docs/architecture/read_cache.md。

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::watch;

use crate::analysis::market::{self, OverviewCache};
use crate::analysis::versions::CacheVersions;
use crate::analysis::{compute, hotspots};
use crate::cache::{Cache, derived_key};

const WARM_INTERVAL: Duration = Duration::from_secs(5 * 60);
const WARM_SECTOR_DAYS: u32 = 5;

pub struct Warmer {
    pub pool: PgPool,
    pub cache: Arc<Cache>,
    pub overview: Arc<OverviewCache>,
}

impl Warmer {
    pub async fn run(self, mut stop: watch::Receiver<bool>) -> anyhow::Result<()> {
        let mut tick = tokio::time::interval(WARM_INTERVAL);
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        return Ok(());
                    }
                }
                _ = tick.tick() => {
                    // 预热是后台任务，不和页面请求抢前台的全市场计算槽位
                    if let Err(e) = compute::run(compute::Budget::Background, self.warm_once()).await {
                        tracing::warn!(error = %format!("{e:#}"), "首页结果预热失败，下一轮继续");
                    }
                }
            }
        }
    }

    async fn warm_once(&self) -> anyhow::Result<()> {
        let Some(date) = market::resolve_date(&self.pool, None).await? else {
            return Ok(());
        };
        let version = CacheVersions::load(&self.pool).await?.market_token();

        let key = derived_key("market:overview", &version, &date.to_string());
        self.cache
            .warm_json(&key, || async {
                self.overview.get(&self.pool, Some(date)).await
            })
            .await?;

        for category in ["industry", "cn_concept"] {
            let key = derived_key(
                "market:sectors",
                &version,
                &format!("{category}:{WARM_SECTOR_DAYS}:{date}"),
            );
            self.cache
                .warm_json(&key, || async {
                    Ok(Some(
                        market::sector_strength(&self.pool, category, WARM_SECTOR_DAYS, date)
                            .await?,
                    ))
                })
                .await?;
        }

        if let Some(hot_date) = hotspots::trade_date(&self.pool, None).await? {
            let key = derived_key("market:hot", &version, &hot_date.to_string());
            self.cache
                .warm_json(&key, || async {
                    Ok(Some(hotspots::compute(&self.pool, hot_date).await?))
                })
                .await?;
        }
        tracing::debug!(date = %date, "首页结果预热完成");
        Ok(())
    }
}
