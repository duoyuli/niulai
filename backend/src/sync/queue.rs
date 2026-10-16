//! 任务队列（ops.task）：认领、续租、结束、推迟、失败、超时回收。
//!
//! 可靠性约定（见技术栈决策 §4）：
//! - 认领用短事务（FOR UPDATE SKIP LOCKED），HTTP 调用在事务之外执行。
//! - 执行期间定期续租（刷新 locked_at），只有执行者真的消失，任务才会被超时回收。
//! - 所有状态更新都带 locked_by = 当前执行者，超时被回收的旧执行者无法覆盖新结果。

use std::time::Duration;

use chrono::NaiveDate;
use serde_json::Value;
use sqlx::PgPool;

/// 数据尚未就绪时，任务推迟多久再执行
pub const DEFER_DELAY: &str = "10 minutes";
/// 可重试的失败：推迟「这么多分钟 × 已尝试次数」后再试
pub const RETRY_DELAY_MINUTES_PER_ATTEMPT: i32 = 5;
/// 租约：锁定超过这么久没有续租的任务视为执行者已崩溃
pub const LEASE: &str = "10 minutes";
/// 执行期间的续租间隔，必须远小于 LEASE
pub const HEARTBEAT: Duration = Duration::from_secs(60);

#[derive(Debug, sqlx::FromRow)]
pub struct ClaimedTask {
    pub task_id: i64,
    pub run_id: i64,
    pub dataset_id: String,
    pub part_key: String,
    pub params: Value,
    pub attempts: i16,
    pub max_attempts: i16,
    /// 所属运行的交易日
    pub trade_date: Option<NaiveDate>,
}

/// 任务成功结束时的状态
#[derive(Debug, Clone, Copy)]
pub enum Finished {
    Done,
    Empty,
    Skipped,
}

impl Finished {
    fn as_str(self) -> &'static str {
        match self {
            Finished::Done => "done",
            Finished::Empty => "empty",
            Finished::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TaskStats {
    pub pages: i32,
    pub records: usize,
    pub changed: usize,
}

#[derive(Clone)]
pub struct TaskQueue {
    pool: PgPool,
    worker_id: String,
}

impl TaskQueue {
    pub fn new(pool: PgPool, worker_id: String) -> Self {
        TaskQueue { pool, worker_id }
    }

    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    // @project-doc docs/architecture/sync_pipeline.md#task_lifecycle
    pub async fn claim(&self) -> anyhow::Result<Option<ClaimedTask>> {
        Ok(sqlx::query_as(
            "WITH c AS (
               UPDATE ops.task SET status = 'running', locked_by = $1, locked_at = now(), attempts = attempts + 1
               WHERE task_id = (SELECT task_id FROM ops.task
                                WHERE status IN ('pending', 'failed') AND not_before <= now()
                                ORDER BY priority, task_id FOR UPDATE SKIP LOCKED LIMIT 1)
               RETURNING task_id, run_id, dataset_id, part_key, params, attempts, max_attempts)
             SELECT c.*, r.trade_date FROM c JOIN ops.run r USING (run_id)",
        )
        .bind(&self.worker_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// 续租：返回 false 表示任务已不属于本执行者（被回收或被人工改动）
    pub async fn heartbeat(&self, task_id: i64) -> anyhow::Result<bool> {
        let n = sqlx::query("UPDATE ops.task SET locked_at = now() WHERE task_id = $1 AND locked_by = $2 AND status = 'running'")
            .bind(task_id)
            .bind(&self.worker_id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    /// 任务结束，同时累加所属运行的完成数（运行结束时会按任务表重新统计）
    pub async fn finish(&self, task: &ClaimedTask, status: Finished, stats: TaskStats, note: Option<&str>) -> anyhow::Result<()> {
        sqlx::query(
            "WITH t AS (
               UPDATE ops.task SET status = $3, pages = $4, records = $5, changed = $6, last_error = $7, finished_at = now(), locked_by = NULL
               WHERE task_id = $1 AND locked_by = $2 RETURNING run_id)
             UPDATE ops.run r SET tasks_done = tasks_done + 1 FROM t WHERE r.run_id = t.run_id",
        )
        .bind(task.task_id)
        .bind(&self.worker_id)
        .bind(status.as_str())
        .bind(stats.pages)
        .bind(stats.records as i32)
        .bind(stats.changed as i32)
        .bind(note)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 数据尚未就绪：放回队列，推迟 DEFER_DELAY，不计入尝试次数
    pub async fn defer(&self, task: &ClaimedTask, why: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE ops.task SET status = 'pending', attempts = greatest(attempts - 1, 0), not_before = now() + $3::interval,
                    last_error = $4, locked_by = NULL
             WHERE task_id = $1 AND locked_by = $2",
        )
        .bind(task.task_id)
        .bind(&self.worker_id)
        .bind(DEFER_DELAY)
        .bind(why)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 失败：fatal 或尝试次数用尽时标为 dead，否则标为 failed 并退避
    pub async fn fail(&self, task: &ClaimedTask, code: Option<i64>, msg: &str, fatal: bool) -> anyhow::Result<()> {
        let dead = fatal || task.attempts >= task.max_attempts;
        sqlx::query(
            "WITH t AS (
               UPDATE ops.task SET status = $3, last_api_code = $4, last_error = $5, locked_by = NULL,
                      not_before = now() + make_interval(mins => $6 * attempts), finished_at = CASE WHEN $3 = 'dead' THEN now() END
               WHERE task_id = $1 AND locked_by = $2 RETURNING run_id, status)
             UPDATE ops.run r SET tasks_failed = tasks_failed + 1 FROM t WHERE r.run_id = t.run_id AND t.status = 'dead'",
        )
        .bind(task.task_id)
        .bind(&self.worker_id)
        .bind(if dead { "dead" } else { "failed" })
        .bind(code.map(|c| c as i32))
        .bind(msg)
        .bind(RETRY_DELAY_MINUTES_PER_ATTEMPT)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 把租约过期的任务放回队列（尝试次数已达上限的标为 dead），返回回收的数量
    pub async fn requeue_stale(&self) -> anyhow::Result<i32> {
        Ok(sqlx::query_scalar("SELECT ops.requeue_stale($1::interval)").bind(LEASE).fetch_one(&self.pool).await?)
    }

    /// 整个队列是否还有未完成的任务（包括其他执行者正在执行的）
    pub async fn has_open(&self) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ops.task WHERE status IN ('pending', 'running', 'failed'))")
            .fetch_one(&self.pool)
            .await?)
    }
}
