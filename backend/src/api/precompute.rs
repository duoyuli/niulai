//! 首页常用全市场结果的后台预热。

use std::time::Duration;

use tokio::sync::watch;

use super::AppState;
use crate::analysis::limit_pool::CacheVersions;
use crate::analysis::market;

const WARM_INTERVAL: Duration = Duration::from_secs(5 * 60);
const WARM_SECTOR_DAYS: u32 = 5;

pub async fn run(state: AppState, mut stop: watch::Receiver<bool>) -> anyhow::Result<()> {
    let mut tick = tokio::time::interval(WARM_INTERVAL);
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
            _ = tick.tick() => {
                if let Err(e) = warm_once(&state).await {
                    tracing::warn!(error = %format!("{e:#}"), "首页结果预热失败，下一轮继续");
                }
            }
        }
    }
}

async fn warm_once(state: &AppState) -> anyhow::Result<()> {
    let Some(date) = market::resolve_date(&state.pool, None).await? else {
        return Ok(());
    };
    let version = CacheVersions::load(&state.pool).await?;

    let overview_key = super::derived_cache_key("market:overview", &version, &date.to_string());
    warm_overview(state, &overview_key, date).await?;

    for category in ["industry", "cn_concept"] {
        let key = super::derived_cache_key(
            "market:sectors",
            &version,
            &format!("{category}:{WARM_SECTOR_DAYS}:{date}"),
        );
        if state
            .cache
            .get_json::<Vec<market::SectorStrength>>(&key)
            .await?
            .is_some()
        {
            continue;
        }
        let value = market::sector_strength(&state.pool, category, WARM_SECTOR_DAYS, date).await?;
        state.cache.set_json(&key, &value).await?;
    }

    if let Some(hot_date) = super::market::hot_trade_date(&state.pool, None).await? {
        let key = super::derived_cache_key("market:hot", &version, &hot_date.to_string());
        if state
            .cache
            .get_json::<super::market::HotMarket>(&key)
            .await?
            .is_none()
        {
            let value = super::market::compute_hot(&state.pool, hot_date)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            state.cache.set_json(&key, &value).await?;
        }
    }
    tracing::debug!(date = %date, "首页结果预热完成");
    Ok(())
}

async fn warm_overview(state: &AppState, key: &str, date: chrono::NaiveDate) -> anyhow::Result<()> {
    if state
        .cache
        .get_json::<market::Overview>(key)
        .await?
        .is_some()
    {
        return Ok(());
    }
    let Some(value) = state.overview.get(&state.pool, Some(date)).await? else {
        return Ok(());
    };
    state.cache.set_json(key, &value).await?;
    Ok(())
}
