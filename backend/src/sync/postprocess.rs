//! 运行结束后的计算：行情或公司行动有变化时，补全交易日、重算复权分段。
//! 新的一天的日线落在最后一段里，不用重算就有复权价；但回补时补回来的历史日线可能正好在除权日，
//! 要重算才能得到正确的因子，所以长时间的回补运行中也定期重算（一次约 2 秒）。

use std::time::Instant;

use sqlx::PgPool;

/// 运行进行中时，两次重算之间至少间隔多久
pub const REFRESH_INTERVAL: &str = "5 minutes";
/// ops.scheduler 中记录上次重算时刻的作业名
const JOB: &str = "adj_rebuild";

/// 返回行情或公司行动是否有变化（有变化时才重算，调用方据此决定是否重新生成调仓建议）
/// price_datasets：写入影响复权价的表的数据集（[`crate::sync::spec::Registry::writing_to`] 加 [`crate::sync::spec::PRICE_TABLES`]）
pub async fn after_run(
    pool: &PgPool,
    run_id: i64,
    price_datasets: &[String],
) -> anyhow::Result<bool> {
    let changed: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(changed), 0) FROM ops.task WHERE run_id = $1 AND dataset_id = ANY($2) AND status = 'done'",
    )
    .bind(run_id)
    .bind(price_datasets)
    .fetch_one(pool)
    .await?;
    if changed == 0 {
        return Ok(false);
    }
    recompute_adjustments(pool, run_id).await?;
    Ok(true)
}

// @project-doc docs/architecture/sync_pipeline.md#postprocess
/// 自上次重算以来有新的行情或公司行动写入，且距上次重算超过 REFRESH_INTERVAL 时重算。
/// 多个进程同时调用时，只有抢到 ops.scheduler 作业的那个执行。返回是否执行了重算。
pub async fn refresh_if_stale(pool: &PgPool, price_datasets: &[String]) -> anyhow::Result<bool> {
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ops.task WHERE dataset_id = ANY($1) AND status = 'done' AND changed > 0
                          AND finished_at > coalesce((SELECT fired_at FROM ops.scheduler WHERE job = $2), '-infinity'))",
    )
    .bind(price_datasets)
    .bind(JOB)
    .fetch_one(pool)
    .await?;
    if !pending {
        return Ok(false);
    }
    let claimed = sqlx::query_scalar::<_, String>(
        "INSERT INTO ops.scheduler (job, last_fired, fired_at) VALUES ($1, current_date, now())
         ON CONFLICT (job) DO UPDATE SET last_fired = EXCLUDED.last_fired, fired_at = EXCLUDED.fired_at
         WHERE ops.scheduler.fired_at < now() - $2::interval
         RETURNING job",
    )
    .bind(JOB)
    .bind(REFRESH_INTERVAL)
    .fetch_optional(pool)
    .await?
    .is_some();
    if claimed && let Err(error) = recompute_adjustments(pool, 0).await {
        // 抢占时间只用于并发互斥；重算失败时把水位退回，让下一次维护继续重试。
        sqlx::query("UPDATE ops.scheduler SET fired_at = now() - $2::interval WHERE job = $1")
            .bind(JOB)
            .bind(REFRESH_INTERVAL)
            .execute(pool)
            .await?;
        return Err(error);
    }
    Ok(claimed)
}

// @project-doc docs/domains/price_adjustment.md#equal_ratio_factor
pub async fn recompute_adjustments(pool: &PgPool, run_id: i64) -> anyhow::Result<()> {
    let t = Instant::now();
    let mut tx = pool.begin().await?;
    // 供应商日历只有最近一年，更早的交易日从行情中推导
    let derived = sqlx::query(
        "INSERT INTO ref.trading_day (cal_date, source)
         SELECT DISTINCT trade_date, 'derived' FROM (
             SELECT trade_date FROM mkt.stock_bar_1d
             UNION
             SELECT trade_date FROM mkt.etf_bar_1d
         ) bars ON CONFLICT (cal_date) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    sqlx::query("CALL mkt.rebuild_adj_segment($1)")
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    // 复权价变了：让依赖它的缓存失效，见 docs/decisions/indicator_compute.md
    sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = 'adj_factor'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO ops.scheduler (job, last_fired, fired_at) VALUES ($1, current_date, now())
         ON CONFLICT (job) DO UPDATE SET last_fired = EXCLUDED.last_fired, fired_at = EXCLUDED.fired_at",
    )
    .bind(JOB)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(
        run_id,
        derived_trading_days = derived,
        ms = t.elapsed().as_millis() as u64,
        "复权因子已重算"
    );
    Ok(())
}
