//! 同步管理接口：数据集、运行与进度、通用记录查询。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::AssertSqlSafe;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::error::{ApiError, ApiResult};
use crate::sync::planner;
use crate::sync::run::RunKind;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_datasets))
        .routes(routes!(dataset_records))
        .routes(routes!(list_runs, create_run))
        .routes(routes!(get_run))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct DatasetStatus {
    pub dataset_id: String,
    pub domain: String,
    pub endpoint: String,
    /// P0–P3
    pub priority: String,
    pub enabled: bool,
    pub last_done_at: Option<DateTime<Utc>>,
    pub open_tasks: i64,
    pub dead_tasks: i64,
}

/// 数据集列表及同步状态
#[utoipa::path(get, path = "/datasets", tag = "sync", responses((status = 200, description = "成功", body = [DatasetStatus])))]
async fn list_datasets(State(s): State<AppState>) -> ApiResult<Json<Vec<DatasetStatus>>> {
    let rows = sqlx::query_as(
        "SELECT d.dataset_id, d.domain, d.endpoint, d.priority, d.enabled, t.last_done_at, coalesce(t.open, 0) open_tasks, coalesce(t.dead, 0) dead_tasks
         FROM ops.dataset d
         LEFT JOIN LATERAL (
           SELECT max(finished_at) FILTER (WHERE status IN ('done', 'empty')) last_done_at,
                  count(*) FILTER (WHERE status IN ('pending', 'running', 'failed')) open,
                  count(*) FILTER (WHERE status = 'dead') dead
           FROM ops.task WHERE dataset_id = d.dataset_id) t ON true
         ORDER BY d.priority, d.dataset_id",
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, IntoParams)]
pub struct RecordQuery {
    pub thscode: Option<String>,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    /// 默认 100，最大 1000
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct RecordItem {
    pub rkey: String,
    pub obs_date: Option<NaiveDate>,
    pub thscode: Option<String>,
    /// 供应商原始字段
    #[schema(value_type = Object)]
    pub payload: Value,
    pub changed_at: DateTime<Utc>,
}

/// 通用记录查询：任何已同步数据集的原始记录（字段保持供应商原样）
#[utoipa::path(get, path = "/datasets/{dataset_id}/records", tag = "sync", params(("dataset_id" = String, Path, description = "例：special.limit_up_pool"), RecordQuery),
    responses((status = 200, description = "成功", body = [RecordItem]), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn dataset_records(State(s): State<AppState>, Path(id): Path<String>, Query(q): Query<RecordQuery>) -> ApiResult<Json<Vec<RecordItem>>> {
    if s.engine.registry.get(&id).is_none() {
        return Err(ApiError::NotFound(format!("未知的数据集：{id}")));
    }
    let rows = sqlx::query_as(
        "SELECT rkey, obs_date, thscode, payload, changed_at FROM raw.record
         WHERE dataset_id = $1 AND ($2::text IS NULL OR thscode = $2)
           AND ($3::date IS NULL OR obs_date >= $3) AND ($4::date IS NULL OR obs_date <= $4)
         ORDER BY obs_date DESC NULLS LAST, rkey LIMIT $5",
    )
    .bind(&id)
    .bind(&q.thscode)
    .bind(q.start)
    .bind(q.end)
    .bind(q.limit.unwrap_or(100).clamp(1, 1000))
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct RunSummary {
    pub run_id: i64,
    /// backfill / incremental
    pub kind: String,
    pub trade_date: Option<NaiveDate>,
    pub status: String,
    pub tasks_total: i32,
    pub tasks_done: i32,
    pub tasks_failed: i32,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    #[schema(value_type = Object)]
    pub params: Value,
}

#[derive(Deserialize, IntoParams)]
pub struct RunListQuery {
    pub limit: Option<i64>,
}

/// 固定列清单（拼接进 SQL，已审计）
const RUN_COLUMNS: &str = "run_id, kind, trade_date, status, tasks_total, tasks_done, tasks_failed, started_at, finished_at, params";

/// 最近的同步运行
#[utoipa::path(get, path = "/sync/runs", tag = "sync", params(RunListQuery), responses((status = 200, description = "成功", body = [RunSummary])))]
async fn list_runs(State(s): State<AppState>, Query(q): Query<RunListQuery>) -> ApiResult<Json<Vec<RunSummary>>> {
    let rows = sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS} FROM ops.run ORDER BY run_id DESC LIMIT $1")))
        .bind(q.limit.unwrap_or(20).clamp(1, 200))
        .fetch_all(&s.pool)
        .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateRun {
    /// backfill（首次全量）或 incremental（增量）
    pub kind: String,
    /// 指定数据集；为空时按 priorities 选择已开启的数据集
    #[serde(default)]
    pub datasets: Vec<String>,
    /// 例：["P0", "P1"]
    #[serde(default)]
    pub priorities: Vec<String>,
    /// 只同步这些证券（作用于按证券展开的数据集），例：["600519.SH"]
    #[serde(default)]
    pub thscodes: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedRun {
    /// 第一个波次的运行 ID；后续波次在前一个结束后自动创建，可通过 params.root_run 关联
    pub run_id: i64,
    pub planned: Vec<PlannedDataset>,
}

#[derive(Serialize, ToSchema)]
pub struct PlannedDataset {
    pub dataset_id: String,
    pub tasks: usize,
}

/// 发起一次同步运行。只负责规划任务，由 `niulai serve` 的执行循环执行。
#[utoipa::path(post, path = "/sync/runs", tag = "sync", request_body = CreateRun,
    responses((status = 202, description = "已规划，任务由执行循环执行", body = CreatedRun), (status = 400, description = "参数不合法", body = crate::error::ErrorBody)))]
async fn create_run(State(s): State<AppState>, Json(body): Json<CreateRun>) -> ApiResult<(StatusCode, Json<CreatedRun>)> {
    let kind: RunKind = body.kind.parse().map_err(|e: anyhow::Error| ApiError::BadRequest(e.to_string()))?;
    let specs = s.engine.select(&body.datasets, &body.priorities).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if specs.is_empty() {
        return Err(ApiError::BadRequest("没有选中任何数据集".into()));
    }
    let (run_id, summary) = s.engine.planner().plan_run(kind, &specs, planner::PlanOptions { only_thscodes: body.thscodes, ..Default::default() }).await?;
    let planned = summary.into_iter().map(|(dataset_id, tasks)| PlannedDataset { dataset_id, tasks }).collect();
    Ok((StatusCode::ACCEPTED, Json(CreatedRun { run_id, planned })))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct DatasetProgress {
    pub dataset_id: String,
    pub tasks: i64,
    pub done: i64,
    pub pending: i64,
    pub running: i64,
    pub failed: i64,
    pub records: Option<i64>,
    pub changed: Option<i64>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct TaskFailure {
    pub task_id: i64,
    pub dataset_id: String,
    pub part_key: String,
    pub status: String,
    pub attempts: i16,
    pub last_api_code: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct RunDetail {
    #[serde(flatten)]
    pub run: RunSummary,
    pub progress: Vec<DatasetProgress>,
    /// 最多 50 条失败任务
    pub failures: Vec<TaskFailure>,
}

/// 运行详情：各数据集进度与失败任务
#[utoipa::path(get, path = "/sync/runs/{run_id}", tag = "sync", params(("run_id" = i64, Path)),
    responses((status = 200, description = "成功", body = RunDetail), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn get_run(State(s): State<AppState>, Path(run_id): Path<i64>) -> ApiResult<Json<RunDetail>> {
    let run: RunSummary = sqlx::query_as(AssertSqlSafe(format!("SELECT {RUN_COLUMNS} FROM ops.run WHERE run_id = $1")))
        .bind(run_id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("运行不存在：{run_id}")))?;
    let progress = sqlx::query_as(
        "SELECT dataset_id, tasks, done, pending, running, failed, records, changed FROM ops.v_progress WHERE run_id = $1 ORDER BY dataset_id",
    )
    .bind(run_id)
    .fetch_all(&s.pool)
    .await?;
    let failures = sqlx::query_as(
        "SELECT task_id, dataset_id, part_key, status, attempts, last_api_code, last_error FROM ops.task
         WHERE run_id = $1 AND status IN ('failed', 'dead') ORDER BY task_id LIMIT 50",
    )
    .bind(run_id)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(RunDetail { run, progress, failures }))
}
