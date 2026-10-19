//! 应用启动和进程内领域事件编排。

use sqlx::PgPool;
use tokio::sync::{mpsc, watch};

use crate::config::Settings;
use crate::sync::events::DomainEvent;

/// 执行只属于应用启动的初始化，不让同步引擎承担鉴权和业务模板的副作用。
pub async fn bootstrap(pool: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    crate::auth::init_first_admin(pool, settings).await?;
    crate::analysis::screener::sync_builtin_strategies(pool).await?;
    Ok(())
}

/// 消费同步领域事件，并编排分析与调仓建议。
pub async fn run_events(
    mut events: mpsc::UnboundedReceiver<DomainEvent>,
    pool: PgPool,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
            event = events.recv() => match event {
                Some(DomainEvent::SyncRunCompleted { run_id, price_changed }) => {
                    handle_sync_run_completed(&pool, run_id, price_changed).await;
                }
                Some(DomainEvent::MaintenanceTick) => {
                    if let Err(e) = crate::analysis::prices::in_background(crate::advice::maintain(&pool)).await {
                        tracing::error!(error = %format!("{e:#}"), "调仓建议的维护失败");
                    }
                }
                None => return Ok(()),
            }
        }
    }
}

async fn handle_sync_run_completed(pool: &PgPool, run_id: i64, price_changed: bool) {
    let pool_changed = match mark_limit_pool_changed(pool, run_id).await {
        Ok(changed) => changed,
        Err(e) => {
            tracing::error!(run_id, error = %format!("{e:#}"), "更新权威涨跌停池的版本号失败");
            false
        }
    };
    if !(price_changed || pool_changed) {
        return;
    }
    let pool = pool.clone();
    crate::analysis::prices::in_background(async move {
        match crate::analysis::screener::run_and_store(&pool, None, None).await {
            Ok(_) => {
                if let Err(e) = crate::advice::refresh_all(&pool, true).await {
                    tracing::error!(run_id, error = %format!("{e:#}"), "生成调仓建议失败");
                }
            }
            Err(e) => tracing::error!(run_id, error = %format!("{e:#}"), "每日巡检失败"),
        }
    })
    .await;
}

async fn mark_limit_pool_changed(pool: &PgPool, run_id: i64) -> anyhow::Result<bool> {
    const DATASETS: &[&str] = &[
        "special.limit_up_pool",
        "special.limit_down_pool",
        "special.limit_break_pool",
    ];
    let changed: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(changed), 0) FROM ops.task WHERE run_id = $1 AND dataset_id = ANY($2) AND status = 'done'",
    )
    .bind(run_id)
    .bind(DATASETS)
    .fetch_one(pool)
    .await?;
    if changed > 0 {
        sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = 'limit_pool'")
            .execute(pool)
            .await?;
        tracing::info!(
            run_id,
            changed,
            "权威涨跌停池有变化，市场概况和信号统计的缓存已失效"
        );
    }
    Ok(changed > 0)
}
