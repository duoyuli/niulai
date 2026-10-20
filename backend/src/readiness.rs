//! 跨应用模块共享的数据就绪判断。

use sqlx::PgPool;

/// 启动时由注册表写入，核心数据包括个股日线和公司行动。
pub static CORE_DATASETS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// 核心数据的首次回补还剩多少个任务；0 表示核心数据已经齐了。
pub async fn core_backfill_left(pool: &PgPool) -> anyhow::Result<i64> {
    let Some(ids) = CORE_DATASETS.get() else {
        return Ok(0);
    };
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM ops.task t JOIN ops.run r USING (run_id)
         WHERE r.kind = 'backfill' AND r.status = 'running' AND t.dataset_id = ANY($1) AND t.status IN ('pending', 'running', 'failed')",
    )
    .bind(ids)
    .fetch_one(pool)
    .await?)
}
