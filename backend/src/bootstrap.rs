//! 应用启动和进程内领域事件编排。

use sqlx::PgPool;
use tokio::sync::{mpsc, watch};

use crate::config::Settings;
use crate::sync::events::DomainEvent;

/// 执行只属于应用启动的初始化，不让同步引擎承担鉴权的副作用。
pub async fn bootstrap(pool: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    crate::auth::init_first_admin(pool, settings).await?;
    Ok(())
}

/// 消费同步领域事件，并编排调仓建议。
pub async fn run_events(
    mut events: mpsc::UnboundedReceiver<DomainEvent>,
    pool: PgPool,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    replay_pending_events(&pool).await;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
            event = events.recv() => match event {
                Some(DomainEvent::SyncRunCompleted { .. }) => replay_pending_events(&pool).await,
                Some(DomainEvent::MaintenanceTick) => {
                    replay_pending_events(&pool).await;
                    if let Err(e) = crate::analysis::prices::in_background(crate::advice::maintain(&pool)).await {
                        tracing::error!(error = %format!("{e:#}"), "调仓建议的维护失败");
                    }
                }
                None => return Ok(()),
            }
        }
    }
}

async fn handle_sync_run_completed(
    pool: &PgPool,
    run_id: i64,
    price_changed: bool,
) -> anyhow::Result<()> {
    let pool_changed = mark_limit_pool_changed(pool, run_id).await?;
    if !(price_changed || pool_changed) {
        crate::sync::run::clear_sync_event_pending(pool, run_id).await?;
        return Ok(());
    }
    let analysis_pool = pool.clone();
    crate::analysis::prices::in_background(async move {
        crate::advice::refresh_all(&analysis_pool, true).await
    })
    .await?;
    crate::sync::run::clear_sync_event_pending(pool, run_id).await?;
    Ok(())
}

async fn replay_pending_events(pool: &PgPool) {
    let pending = match crate::sync::run::pending_sync_events(pool).await {
        Ok(events) => events,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "读取待处理同步事件失败");
            return;
        }
    };
    let Some((run_id, _)) = pending.last().copied() else {
        return;
    };
    let price_changed = pending.iter().any(|(_, changed)| *changed);
    if let Err(e) = handle_sync_run_completed(pool, run_id, price_changed).await {
        tracing::error!(run_id, error = %format!("{e:#}"), "同步完成事件处理失败，将在后续维护中重试");
    } else if let Err(e) = crate::sync::run::clear_sync_event_pending_through(pool, run_id).await {
        tracing::error!(run_id, error = %format!("{e:#}"), "清理已覆盖的同步完成事件失败");
    }
}

async fn mark_limit_pool_changed(pool: &PgPool, run_id: i64) -> anyhow::Result<bool> {
    let changed: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(changed), 0) FROM ops.task WHERE run_id = $1 AND dataset_id = ANY($2) AND status = 'done'",
    )
    .bind(run_id)
    .bind(crate::analysis::limit_pool::DATASETS.as_slice())
    .fetch_one(pool)
    .await?;
    if changed > 0 {
        sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = 'limit_pool'")
            .execute(pool)
            .await?;
        tracing::info!(run_id, changed, "权威涨跌停池有变化，市场概况的缓存已失效");
    }
    Ok(changed > 0)
}
