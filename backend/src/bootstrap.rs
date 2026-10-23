//! 应用启动和进程内领域事件编排。

use sqlx::PgPool;
use tokio::sync::{mpsc, watch};

use crate::analysis::compute::{self, Budget};
use crate::analysis::{factor_lab, limit_pool};
use crate::config::Settings;
use crate::sync::events::DomainEvent;

/// 执行只属于应用启动的初始化，不让同步引擎承担鉴权的副作用。
pub async fn bootstrap(pool: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    crate::auth::init_first_admin(pool, settings).await?;
    Ok(())
}

// @project-doc docs/architecture/sync_pipeline.md#postprocess
/// 消费同步领域事件，并编排调仓建议。事件本身只是「去看看」的提示：要处理什么以数据库里
/// `ready` 阶段的事件为准，所以进程重启、事件丢失或处理失败后，下一次提示或维护会补上。
pub async fn run_events(
    mut events: mpsc::UnboundedReceiver<DomainEvent>,
    pool: PgPool,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    replay_ready_events(&pool).await;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
            event = events.recv() => match event {
                Some(DomainEvent::SyncRunCompleted { .. }) => replay_ready_events(&pool).await,
                Some(DomainEvent::MaintenanceTick) => {
                    replay_ready_events(&pool).await;
                    if let Err(e) = compute::run(Budget::Background, crate::advice::maintain(&pool)).await {
                        tracing::error!(error = %format!("{e:#}"), "调仓建议的维护失败");
                    }
                }
                None => return Ok(()),
            }
        }
    }
}

/// 处理全部 `ready` 的同步完成事件。积压了多个时合在一起：汇总所有运行的影响（行情、涨跌停池、因子输入），
/// 需要时只刷新一次建议，然后只确认这一批里的运行，不按编号范围整段清除
pub async fn replay_ready_events(pool: &PgPool) {
    let ready = match crate::sync::run::ready_sync_events(pool).await {
        Ok(events) => events,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "读取待处理同步事件失败");
            return;
        }
    };
    if ready.is_empty() {
        return;
    }
    let run_ids: Vec<i64> = ready.iter().map(|(id, _)| *id).collect();
    let price_changed = ready.iter().any(|(_, changed)| *changed);
    match handle_runs(pool, &run_ids, price_changed).await {
        Ok(()) => {
            if let Err(e) = crate::sync::run::ack_sync_events(pool, &run_ids).await {
                tracing::error!(error = %format!("{e:#}"), "确认已处理的同步完成事件失败");
            }
        }
        Err(e) => {
            tracing::error!(?run_ids, error = %format!("{e:#}"), "同步完成事件处理失败，将在后续维护中重试")
        }
    }
}

async fn handle_runs(pool: &PgPool, run_ids: &[i64], price_changed: bool) -> anyhow::Result<()> {
    let pool_changed =
        bump_if_changed(pool, "limit_pool", run_ids, &limit_pool::DATASETS, &[]).await?;
    if pool_changed {
        tracing::info!(?run_ids, "权威涨跌停池有变化，市场概况的缓存已失效");
    }
    let inputs_changed = bump_if_changed(
        pool,
        "factor_input",
        run_ids,
        &factor_lab::INPUT_DATASETS,
        &factor_lab::INPUT_TABLES,
    )
    .await?;
    if inputs_changed {
        tracing::info!(?run_ids, "财报、行业或证券信息有变化，因子排名的缓存已失效");
    }
    if price_changed || pool_changed || inputs_changed {
        compute::run(Budget::Background, crate::advice::refresh_all(pool, true)).await?;
    }
    Ok(())
}

/// 这些运行里，列出的数据集或写入列出的表的数据集有没有写入变化；有就更新 `ops.derived_version` 的 `name` 行
async fn bump_if_changed(
    pool: &PgPool,
    name: &str,
    run_ids: &[i64],
    datasets: &[&str],
    tables: &[&str],
) -> anyhow::Result<bool> {
    let changed: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM ops.task t
           WHERE t.run_id = ANY($1) AND t.status = 'done' AND t.changed > 0
             AND (t.dataset_id = ANY($2)
                  OR t.dataset_id IN (SELECT dataset_id FROM ops.dataset WHERE spec->'sink' ?| $3)))",
    )
    .bind(run_ids)
    .bind(datasets)
    .bind(tables)
    .fetch_one(pool)
    .await?;
    if changed {
        sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = $1")
            .bind(name)
            .execute(pool)
            .await?;
    }
    Ok(changed)
}
