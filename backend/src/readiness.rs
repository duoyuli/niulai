//! 跨应用模块共享的数据就绪判断：业务模块（`advice`、`api`）不依赖同步层，要知道「数据齐没齐、同步在不在跑」时
//! 只问这里，不直接查 `ops.*` 的运行状态表。
//!
//! 核心数据是写入个股日线和公司行动的数据集。这里不靠进程启动时写入的全局变量判断，而是直接按
//! `ops.dataset` 中各数据集的写入目标查：任何入口（服务、命令行、测试里自己建的连接池）得到的结论都一样，
//! 不会因为没经过某个初始化步骤就默认「已就绪」。

use sqlx::PgPool;

/// 核心数据所在的表：个股日线和公司行动，也就是影响复权价的表（`sync::spec::PRICE_TABLES`）
pub const CORE_TABLES: [&str; 2] = ["mkt.stock_bar_1d", "mkt.corp_action"];

/// 有没有增量同步正在运行：这时日线可能只有一部分，后台补建议要等它结束
pub async fn incremental_sync_running(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ops.run WHERE kind = 'incremental' AND status = 'running')",
    )
    .fetch_one(pool)
    .await?)
}

/// 核心数据的首次回补还剩多少个任务；0 表示核心数据已经齐了
pub async fn core_backfill_left(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM ops.task t JOIN ops.run r USING (run_id)
         WHERE r.kind = 'backfill' AND r.status = 'running' AND t.status IN ('pending', 'running', 'failed')
           AND t.dataset_id IN (SELECT dataset_id FROM ops.dataset WHERE spec->'sink' ?| $1)",
    )
    .bind(&CORE_TABLES[..])
    .fetch_one(pool)
    .await?)
}
