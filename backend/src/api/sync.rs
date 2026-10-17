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
        .routes(routes!(sync_health))
        .routes(routes!(sync_repair))
        .routes(routes!(get_schedule, update_schedule))
}

/// 同步是否正常：最近一个交易日的数据是否就绪、最近 20 个交易日的缺口、没有解决的失败、历史回补进度
#[utoipa::path(get, path = "/sync/health", tag = "sync", responses((status = 200, description = "成功", body = crate::sync::audit::SyncHealth)))]
async fn sync_health(State(s): State<AppState>) -> ApiResult<Json<crate::sync::audit::SyncHealth>> {
    Ok(Json(crate::sync::audit::health(&s.engine).await?))
}

/// 立即兜底补偿：能重新取的缺口规划补偿运行，供应商暂时不可用导致的失败重新排队。每天每日同步结束后也会自动执行一次
#[utoipa::path(post, path = "/sync/repair", tag = "sync", responses((status = 202, description = "已规划补偿", body = crate::sync::audit::RepairSummary)))]
async fn sync_repair(State(s): State<AppState>) -> ApiResult<(StatusCode, Json<crate::sync::audit::RepairSummary>)> {
    Ok((StatusCode::ACCEPTED, Json(crate::sync::audit::repair(&s.engine).await?)))
}

/// 同步计划：每日、每周、每月、财报季各在什么时候同步、包括哪些数据、下次和上次同步。时间都是上海时间
#[utoipa::path(get, path = "/sync/schedule", tag = "sync", responses((status = 200, description = "成功", body = crate::sync::scheduler::SyncSchedule)))]
async fn get_schedule(State(s): State<AppState>) -> ApiResult<Json<crate::sync::scheduler::SyncSchedule>> {
    Ok(Json(crate::sync::scheduler::overview(&s.engine).await?))
}

/// 修改同步时间（上海时间），下一次检查（30 秒内）起生效
#[utoipa::path(put, path = "/sync/schedule", tag = "sync", request_body = crate::sync::scheduler::ScheduleTimes,
    responses((status = 200, description = "已保存", body = crate::sync::scheduler::SyncSchedule), (status = 400, description = "时间格式不对", body = crate::error::ErrorBody)))]
async fn update_schedule(State(s): State<AppState>, Json(body): Json<crate::sync::scheduler::ScheduleTimes>) -> ApiResult<Json<crate::sync::scheduler::SyncSchedule>> {
    body.validate().map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    crate::sync::scheduler::save_times(&s.pool, &body).await?;
    Ok(Json(crate::sync::scheduler::overview(&s.engine).await?))
}

#[derive(Serialize, ToSchema)]
pub struct DatasetStatus {
    pub dataset_id: String,
    /// 中文名、分类、一句话说明
    pub title: String,
    pub category: String,
    pub summary: String,
    pub domain: String,
    pub endpoint: String,
    /// P0–P3
    pub priority: String,
    pub enabled: bool,
    /// 更新频率：after_close / daily / weekly / monthly / report_season / after_open
    pub schedule: String,
    /// 所属同步计划：daily / weekly / monthly / report_season / manual
    pub plan: String,
    /// 只有当天的值，错过当天就补不回来
    pub only_current: bool,
    pub last_done_at: Option<DateTime<Utc>>,
    pub open_tasks: i64,
    /// 失败后之后也没有成功过的任务
    pub dead_tasks: i64,
    /// 正在进行的历史回补中，这个数据集的任务数和已结束的任务数；没有回补时都是 0
    pub backfill_total: i64,
    pub backfill_done: i64,
}

type DatasetRow = (String, String, String, String, bool, Option<DateTime<Utc>>, i64, i64, i64, i64);

/// 数据集列表及同步状态
#[utoipa::path(get, path = "/datasets", tag = "sync", responses((status = 200, description = "成功", body = [DatasetStatus])))]
async fn list_datasets(State(s): State<AppState>) -> ApiResult<Json<Vec<DatasetStatus>>> {
    let rows: Vec<DatasetRow> = sqlx::query_as(
        "SELECT d.dataset_id, d.domain, d.endpoint, d.priority, d.enabled, t.last_done_at, coalesce(t.open, 0), coalesce(t.dead, 0),
                coalesce(b.total, 0), coalesce(b.done, 0)
         FROM ops.dataset d
         LEFT JOIN LATERAL (
           SELECT max(finished_at) FILTER (WHERE status IN ('done', 'empty')) last_done_at,
                  count(*) FILTER (WHERE status IN ('pending', 'running', 'failed')) open,
                  count(*) FILTER (WHERE status = 'dead' AND NOT EXISTS (
                    SELECT 1 FROM ops.task o WHERE o.dataset_id = x.dataset_id AND o.part_key = x.part_key
                      AND o.task_id > x.task_id AND o.status IN ('done', 'empty'))) dead
           FROM ops.task x WHERE x.dataset_id = d.dataset_id) t ON true
         LEFT JOIN LATERAL (
           SELECT count(*) total, count(*) FILTER (WHERE y.status NOT IN ('pending', 'running', 'failed')) done
           FROM ops.task y JOIN ops.run r USING (run_id)
           WHERE y.dataset_id = d.dataset_id AND r.kind = 'backfill' AND r.status = 'running') b ON true
         ORDER BY d.priority, d.dataset_id",
    )
    .fetch_all(&s.pool)
    .await?;
    let reg = &s.engine.registry;
    let out = rows
        .into_iter()
        .map(|(dataset_id, domain, endpoint, priority, enabled, last_done_at, open_tasks, dead_tasks, backfill_total, backfill_done)| {
            let spec = reg.get(&dataset_id);
            DatasetStatus {
                title: spec.map_or_else(|| dataset_id.clone(), |s| s.title.clone()),
                category: spec.map_or_else(|| "其他".into(), |s| s.category.clone()),
                summary: spec.map(|s| s.summary.clone()).unwrap_or_default(),
                schedule: spec.map_or("after_close", |s| s.schedule.as_str()).into(),
                plan: spec.map_or("daily", |s| crate::sync::scheduler::Plan::of(s.schedule).key()).into(),
                only_current: spec.is_some_and(|s| s.trade_date_guard && s.recover_with.is_none()),
                dataset_id,
                domain,
                endpoint,
                priority,
                enabled,
                last_done_at,
                open_tasks,
                dead_tasks,
                backfill_total,
                backfill_done,
            }
        })
        .collect();
    Ok(Json(out))
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
    let (run_id, summary) = s.engine.planner().plan_run(kind, &specs, planner::PlanOptions { only_thscodes: body.thscodes, trigger: Some("manual".into()), ..Default::default() }).await?;
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
