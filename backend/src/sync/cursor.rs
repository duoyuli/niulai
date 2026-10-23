//! 进度游标（ops.cursor）：执行器推进，规划器在增量运行中读取。
//!
//! - `synced_to`：security_window 分区按证券记录（part_key = `thscode=<代码>`），trading_day 分区记录在 `*` 上；只沿连续成功前沿推进。
//! - `planned_since`：from_dataset 分区记录在 `_plan` 上，增量规划只看这个时刻之后变化的来源记录。

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{Map, Value};
use sqlx::PgPool;

use super::spec::{DatasetSpec, Partition};

// @project-doc docs/architecture/sync_pipeline.md#record_writes
/// 任务成功后，在写入记录的同一个事务里推进游标
pub async fn advance_in_tx(
    conn: &mut sqlx::PgConnection,
    spec: &DatasetSpec,
    task_id: i64,
    vars: &Map<String, Value>,
) -> anyhow::Result<()> {
    let s = |k: &str| vars.get(k).and_then(Value::as_str).map(str::to_string);
    let (part, candidate_start, candidate_end, overlap_days) = match &spec.partition {
        Partition::SecurityWindow { .. } => {
            let Some(code) = s("thscode") else {
                return Ok(());
            };
            let Some(start) =
                s("start").and_then(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok())
            else {
                return Ok(());
            };
            let Some(end) = s("end")
                .or_else(|| s("today"))
                .and_then(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok())
            else {
                return Ok(());
            };
            let overlap = match spec.incremental {
                super::spec::Incremental::SinceCursor { overlap_days } => overlap_days,
                _ => 0,
            };
            (format!("thscode={code}"), start, end, overlap)
        }
        Partition::TradingDay { .. } => {
            let Some(date) = s("date").and_then(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok())
            else {
                return Ok(());
            };
            ("*".to_string(), date, date, 0)
        }
        _ => return Ok(()),
    };

    // The task is still `running` when this function is called, so include it as a
    // successful interval while computing the contiguous frontier.
    let current: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT (state->>'synced_to')::date FROM ops.cursor WHERE dataset_id = $1 AND part_key = $2",
    )
    .bind(&spec.id)
    .bind(&part)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    let code = vars.get("thscode").and_then(Value::as_str).unwrap_or("*");
    let min_start: Option<NaiveDate> = if matches!(spec.partition, Partition::TradingDay { .. }) {
        sqlx::query_scalar(
            "SELECT min((params->'vars'->>'date')::date) FROM ops.task
             WHERE dataset_id = $1",
        )
        .bind(&spec.id)
        .fetch_one(&mut *conn)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT min((params->'vars'->>'start')::date) FROM ops.task
             WHERE dataset_id = $1 AND params->'vars'->>'thscode' = $2",
        )
        .bind(&spec.id)
        .bind(code)
        .fetch_one(&mut *conn)
        .await?
    };
    let mut frontier = match current {
        Some(d) => d,
        None => min_start
            .map(|d| d - Duration::days(1))
            .unwrap_or(candidate_start - Duration::days(1)),
    };
    let rows: Vec<(NaiveDate, NaiveDate)> = if matches!(
        spec.partition,
        Partition::TradingDay { .. }
    ) {
        sqlx::query_as(
            "SELECT (params->'vars'->>'date')::date, (params->'vars'->>'date')::date FROM ops.task
             WHERE dataset_id = $1 AND (status IN ('done', 'empty') OR task_id = $2) ORDER BY 1",
        )
        .bind(&spec.id)
        .bind(task_id)
        .fetch_all(&mut *conn)
        .await?
    } else {
        sqlx::query_as(
            "SELECT (params->'vars'->>'start')::date, COALESCE((params->'vars'->>'end')::date, (params->'vars'->>'date')::date)
             FROM ops.task
             WHERE dataset_id = $1 AND params->'vars'->>'thscode' = $2
               AND (status IN ('done', 'empty') OR task_id = $3) ORDER BY 1, 2",
        )
        .bind(&spec.id)
        .bind(code)
        .bind(task_id)
        .fetch_all(&mut *conn)
        .await?
    };
    for (start, end) in rows {
        if end <= frontier || start > frontier + Duration::days(overlap_days + 1) {
            continue;
        }
        if start <= frontier + Duration::days(overlap_days + 1) {
            frontier = frontier.max(end);
        }
    }
    if frontier < candidate_end || current.is_some_and(|d| frontier <= d) {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO ops.cursor (dataset_id, part_key, state, task_id) VALUES ($1, $2, jsonb_build_object('synced_to', $3::text), $4)
         ON CONFLICT (dataset_id, part_key) DO UPDATE
           SET state = jsonb_set(ops.cursor.state, '{synced_to}', to_jsonb($3::text)),
               task_id = EXCLUDED.task_id, updated_at = now()",
    )
    .bind(&spec.id)
    .bind(&part)
    .bind(frontier)
    .bind(task_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// 各分区已同步到的日期
pub async fn synced_to(
    pool: &PgPool,
    dataset: &str,
) -> anyhow::Result<BTreeMap<String, NaiveDate>> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT part_key, state->>'synced_to' FROM ops.cursor WHERE dataset_id = $1",
    )
    .bind(dataset)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(k, v)| {
            v.and_then(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok())
                .map(|d| (k, d))
        })
        .collect())
}

pub async fn planned_since(pool: &PgPool, dataset: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
    let v: Option<String> = sqlx::query_scalar("SELECT state->>'planned_since' FROM ops.cursor WHERE dataset_id = $1 AND part_key = '_plan'")
        .bind(dataset)
        .fetch_optional(pool)
        .await?
        .flatten();
    Ok(v.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|t| t.to_utc()))
}

/// 记录本次规划读取来源数据的时刻；应在任务写入成功之后调用
pub async fn save_planned_since(
    pool: &PgPool,
    dataset: &str,
    at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO ops.cursor (dataset_id, part_key, state) VALUES ($1, '_plan', jsonb_build_object('planned_since', $2::text))
         ON CONFLICT (dataset_id, part_key) DO UPDATE SET state = EXCLUDED.state, updated_at = now()",
    )
    .bind(dataset)
    .bind(at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string())
    .execute(pool)
    .await?;
    Ok(())
}
