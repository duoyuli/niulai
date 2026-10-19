//! 运行（ops.run）的生命周期：创建、结束判定。
//!
//! 运行和它的全部任务在同一个事务中创建（见 planner），所以执行者看到一个运行时，它的任务已经齐全，
//! 「没有未完成的任务」就等于「运行已结束」。

use anyhow::bail;
use chrono::NaiveDate;
use serde_json::json;
use sqlx::{PgConnection, PgPool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Backfill,
    Incremental,
    /// 兜底补偿：完整性核对发现缺口后补取，规则与增量相同
    Repair,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Backfill => "backfill",
            RunKind::Incremental => "incremental",
            RunKind::Repair => "repair",
        }
    }
}

impl std::str::FromStr for RunKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "backfill" => Ok(RunKind::Backfill),
            "incremental" => Ok(RunKind::Incremental),
            "repair" => Ok(RunKind::Repair),
            other => bail!("kind 只能是 backfill、incremental 或 repair：{other}"),
        }
    }
}

/// 一个波次对应的运行
pub struct NewRun<'a> {
    pub kind: RunKind,
    pub trade_date: NaiveDate,
    pub datasets: &'a [String],
    /// 本波次之后还要规划的波次
    pub pending_waves: &'a [Vec<String>],
    /// 第一个波次的 run_id；本身就是第一个波次时为空
    pub root_run: Option<i64>,
    pub only_thscodes: &'a [String],
    /// 谁发起的：daily、catch_up、weekly、weekly+monthly、manual、setup、cli、repair
    pub trigger: Option<&'a str>,
    /// true while an API request has handed planning to the background task
    pub planning: bool,
}

pub async fn create(conn: &mut PgConnection, run: &NewRun<'_>) -> anyhow::Result<i64> {
    let single = if run.datasets.len() == 1 {
        Some(run.datasets[0].as_str())
    } else {
        None
    };
    Ok(sqlx::query_scalar("INSERT INTO ops.run (kind, dataset_id, trade_date, params) VALUES ($1, $2, $3, $4) RETURNING run_id")
        .bind(run.kind.as_str())
        .bind(single)
        .bind(run.trade_date)
        .bind(json!({"datasets": run.datasets, "pending_waves": run.pending_waves, "root_run": run.root_run, "only_thscodes": run.only_thscodes, "trigger": run.trigger, "planning": run.planning}))
        .fetch_one(conn)
        .await?)
}

/// 写入任务总数；没有任务的运行直接记为成功
pub async fn set_total(conn: &mut PgConnection, run_id: i64, total: usize) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE ops.run SET tasks_total = $2,
                status = CASE WHEN $2 = 0 AND NOT (params ? 'planning_error') THEN 'succeeded'
                              WHEN $2 = 0 THEN 'partial' ELSE status END,
                finished_at = CASE WHEN $2 = 0 THEN now() END
         WHERE run_id = $1",
    )
    .bind(run_id)
    .bind(total as i32)
    .execute(conn)
    .await?;
    Ok(())
}

// @project-doc docs/architecture/sync_pipeline.md#task_lifecycle
/// 运行没有未完成的任务时，把它标为结束，返回结束状态。
/// 条件 UPDATE 只有一个调用者能成功，其余调用者得到 None，所以后续计算不会重复执行。
pub async fn try_complete(pool: &PgPool, run_id: i64) -> anyhow::Result<Option<String>> {
    // 先用部分索引做廉价判断：运行进行中时（绝大多数情况）不必统计整个运行
    let open: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ops.task WHERE run_id = $1 AND status IN ('pending', 'running', 'failed'))")
        .bind(run_id)
        .fetch_one(pool)
        .await?;
    if open {
        return Ok(None);
    }
    // 结束时按任务表重新统计，纠正进行中累加的计数可能出现的偏差
    Ok(sqlx::query_scalar(
        "UPDATE ops.run r SET
            tasks_done   = s.ok, tasks_failed = s.bad,
            status       = CASE WHEN s.bad > 0 OR r.params ? 'planning_error' THEN 'partial' ELSE 'succeeded' END,
            finished_at  = now()
         FROM (SELECT count(*) FILTER (WHERE status IN ('done', 'empty', 'skipped')) ok,
                      count(*) FILTER (WHERE status = 'dead') bad
               FROM ops.task WHERE run_id = $1) s
         WHERE r.run_id = $1 AND r.status = 'running'
           AND coalesce((r.params->>'planning')::boolean, false) = false
           AND NOT EXISTS (SELECT 1 FROM ops.task t WHERE t.run_id = $1 AND t.status IN ('pending', 'running', 'failed'))
         RETURNING r.status",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?)
}

/// 接上下一个波次之前，清空已结束运行的 pending_waves。条件 UPDATE 只有一个调用者能成功（其余的等行锁、再判断时不满足），
/// 和新运行在同一个事务里，所以下一个波次不会规划两次，也不会清空了却没规划
pub async fn take_pending_waves(conn: &mut PgConnection, run_id: i64) -> anyhow::Result<bool> {
    let n = sqlx::query(
        "UPDATE ops.run SET params = jsonb_set(params, '{pending_waves}', '[]'::jsonb)
         WHERE run_id = $1 AND jsonb_array_length(coalesce(params->'pending_waves', '[]'::jsonb)) > 0",
    )
    .bind(run_id)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(n > 0)
}

/// 已经结束、还有后续波次却没接上的运行：结束后的处理中途失败或进程退出时会留下。
/// 结束 2 分钟以上才算，避免和正在接续的执行者抢（抢了也只会有一个成功）
pub async fn unchained(pool: &PgPool) -> anyhow::Result<Vec<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT run_id FROM ops.run
         WHERE status IN ('succeeded', 'partial') AND finished_at < now() - interval '2 minutes'
           AND jsonb_array_length(coalesce(params->'pending_waves', '[]'::jsonb)) > 0
         ORDER BY run_id",
    )
    .fetch_all(pool)
    .await?)
}

/// 状态仍是 running、但已经没有未完成任务的运行。
/// 例如最后一个任务被超时回收后直接标为 dead：这时没有执行者处理过它，运行不会自己结束。
pub async fn stalled(pool: &PgPool) -> anyhow::Result<Vec<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT run_id FROM ops.run r WHERE status = 'running'
           AND coalesce((params->>'planning')::boolean, false) = false
           AND NOT EXISTS (SELECT 1 FROM ops.task t WHERE t.run_id = r.run_id AND t.status IN ('pending', 'running', 'failed'))
         ORDER BY run_id",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn planned_since_runs(pool: &PgPool) -> anyhow::Result<Vec<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT run_id FROM ops.run WHERE status = 'succeeded'
         AND (SELECT count(*) FROM jsonb_object_keys(coalesce(params->'planned_since_marks', '{}'::jsonb))) > 0
         ORDER BY run_id",
    )
    .fetch_all(pool)
    .await?)
}

/// 运行结束后需要由应用层消费的同步完成事件。标记写入 `ops.run.params`，进程重启后可以重放。
pub async fn pending_sync_events(pool: &PgPool) -> anyhow::Result<Vec<(i64, bool)>> {
    Ok(sqlx::query_as(
        "SELECT run_id, coalesce((params->'sync_event_pending'->>'price_changed')::boolean, true)
         FROM ops.run WHERE params ? 'sync_event_pending' ORDER BY run_id",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn mark_sync_event_pending(
    pool: &PgPool,
    run_id: i64,
    price_changed: bool,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE ops.run SET params = jsonb_set(
             params, '{sync_event_pending}', jsonb_build_object('price_changed', $2::boolean), true
         ) WHERE run_id = $1",
    )
    .bind(run_id)
    .bind(price_changed)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_sync_event_pending(pool: &PgPool, run_id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE ops.run SET params = params - 'sync_event_pending' WHERE run_id = $1")
        .bind(run_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn planned_since_marks(
    pool: &PgPool,
    run_id: i64,
) -> anyhow::Result<Vec<(String, chrono::DateTime<chrono::Utc>)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM ops.run, jsonb_each_text(coalesce(params->'planned_since_marks', '{}'::jsonb)) WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(dataset, at)| at.parse().ok().map(|at| (dataset, at)))
        .collect())
}

pub async fn clear_planned_since_marks(pool: &PgPool, run_id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE ops.run SET params = params - 'planned_since_marks' WHERE run_id = $1")
        .bind(run_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// API 后台规划中进程重启后留下的运行：超过 10 分钟仍带 planning 标记就结束为失败。
pub async fn stale_planning(pool: &PgPool) -> anyhow::Result<Vec<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT run_id FROM ops.run
         WHERE status = 'running'
           AND coalesce((params->>'planning')::boolean, false)
           AND coalesce((params->>'planning_heartbeat')::timestamptz, started_at) < now() - interval '10 minutes'
         ORDER BY run_id",
    )
    .fetch_all(pool)
    .await?)
}

/// 规划仍在执行时刷新心跳；维护按心跳判断是否真的中断。
pub async fn touch_planning(pool: &PgPool, run_id: i64) -> anyhow::Result<bool> {
    Ok(sqlx::query(
        "UPDATE ops.run
         SET params = jsonb_set(params, '{planning_heartbeat}', to_jsonb(now()), true)
         WHERE run_id = $1 AND status = 'running' AND coalesce((params->>'planning')::boolean, false)",
    )
    .bind(run_id)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

/// 标记中断的后台规划，去掉 planning 让运行不再卡住。
pub async fn fail_planning(pool: &PgPool, run_id: i64) -> anyhow::Result<bool> {
    Ok(sqlx::query(
        "UPDATE ops.run SET status = 'failed', error = '规划中断（服务重启），请重新发起', finished_at = now(), params = params - 'planning' - 'planning_heartbeat'
         WHERE run_id = $1 AND status = 'running' AND coalesce((params->>'planning')::boolean, false)",
    )
    .bind(run_id)
    .execute(pool)
    .await?
    .rows_affected() > 0)
}
