//! 定期清理：已经结束很久的任务和调用日志。
//!
//! 任务队列和调用日志每天都在增长（一次全量回补几十万个任务，每次请求一行日志），不清理的话一年就是几百万行，
//! 同步页、完整性核对的查询会越来越慢。只删不再有用的行：
//! - 结束超过 `TASK_KEEP_DAYS` 天的成功、没有数据、跳过的任务，但每个数据集最近一个成功的任务保留（「最近同步」从它来）；
//! - 结束超过 `TASK_KEEP_DAYS` 天、之后同一分区已经成功过的失败任务（已经解决了）；还没解决的失败一直保留，直到补上；
//! - 超过 `API_CALL_KEEP_DAYS` 天的调用日志。
//!
//! 进行中的运行里的任务不删。完整性核对只看最近 20 个交易日，数据核检只看发现缺口之后的任务，都在保留期内。

use sqlx::PgPool;

/// 结束的任务保留多少天
pub const TASK_KEEP_DAYS: i32 = 60;
/// 调用日志保留多少天
pub const API_CALL_KEEP_DAYS: i32 = 30;
/// 每批删多少行：分批删，不长时间锁表
const BATCH: i64 = 20_000;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub tasks: u64,
    pub api_calls: u64,
}

/// 每天一次（抢占 `ops.scheduler` 的 `prune` 作业，多个进程并存时也只做一次）
pub async fn prune_if_due(pool: &PgPool) -> anyhow::Result<()> {
    if !super::scheduler::claim(pool, "prune", crate::time::today_cn(), "day").await? {
        return Ok(());
    }
    let p = prune(pool).await?;
    tracing::info!(tasks = p.tasks, api_calls = p.api_calls, "已清理过期的任务和调用日志");
    Ok(())
}

// @project-doc docs/architecture/sync_pipeline.md#retention
pub async fn prune(pool: &PgPool) -> anyhow::Result<Pruned> {
    let mut out = Pruned::default();
    // 先删已经解决的失败：判断「之后成功过」要用到成功的任务，所以成功的任务后删
    out.tasks += batched(
        pool,
        "DELETE FROM ops.task WHERE task_id IN (
           SELECT d.task_id FROM ops.task d
           WHERE d.status = 'dead' AND coalesce(d.finished_at, d.created_at) < now() - make_interval(days => $1)
             AND EXISTS (SELECT 1 FROM ops.task o WHERE o.dataset_id = d.dataset_id AND o.part_key = d.part_key
                           AND o.task_id > d.task_id AND o.status IN ('done', 'empty'))
           LIMIT $2)",
        TASK_KEEP_DAYS,
    )
    .await?;
    out.tasks += batched(
        pool,
        "DELETE FROM ops.task WHERE task_id IN (
           SELECT t.task_id FROM ops.task t JOIN ops.run r USING (run_id)
           WHERE r.status <> 'running' AND t.status IN ('done', 'empty', 'skipped')
             AND t.finished_at < now() - make_interval(days => $1)
             AND t.task_id NOT IN (SELECT max(task_id) FROM ops.task WHERE status IN ('done', 'empty') GROUP BY dataset_id)
           LIMIT $2)",
        TASK_KEEP_DAYS,
    )
    .await?;
    out.api_calls += batched(
        pool,
        "DELETE FROM ops.api_call WHERE call_id IN (SELECT call_id FROM ops.api_call WHERE called_at < now() - make_interval(days => $1) LIMIT $2)",
        API_CALL_KEEP_DAYS,
    )
    .await?;
    Ok(out)
}

/// 反复执行一条带 `LIMIT $2` 的删除，直到删完
async fn batched(pool: &PgPool, sql: &'static str, days: i32) -> anyhow::Result<u64> {
    let mut total = 0;
    loop {
        let n = sqlx::query(sql).bind(days).bind(BATCH).execute(pool).await?.rows_affected();
        total += n;
        if n < BATCH as u64 {
            return Ok(total);
        }
    }
}
