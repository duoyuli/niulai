//! 进度游标（ops.cursor）：执行器推进，规划器在增量运行中读取。
//!
//! - `synced_to`：security_window 分区按证券记录（part_key = `thscode=<代码>`），trading_day 分区记录在 `*` 上；只前进不后退。
//! - `planned_since`：from_dataset 分区记录在 `_plan` 上，增量规划只看这个时刻之后变化的来源记录。

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value};
use sqlx::PgPool;

use super::spec::{DatasetSpec, Partition};

// @project-doc docs/architecture/sync_pipeline.md#record_writes
/// 任务成功后推进游标
pub async fn advance(pool: &PgPool, spec: &DatasetSpec, task_id: i64, vars: &Map<String, Value>) -> anyhow::Result<()> {
    let s = |k: &str| vars.get(k).and_then(Value::as_str).map(str::to_string);
    let (part, synced_to) = match &spec.partition {
        Partition::SecurityWindow { .. } => {
            let Some(code) = s("thscode") else { return Ok(()) };
            (format!("thscode={code}"), s("end").or_else(|| s("today")))
        }
        Partition::TradingDay { .. } => ("*".to_string(), s("date")),
        _ => return Ok(()),
    };
    let Some(synced_to) = synced_to else { return Ok(()) };
    sqlx::query(
        "INSERT INTO ops.cursor (dataset_id, part_key, state, task_id) VALUES ($1, $2, jsonb_build_object('synced_to', $3::text), $4)
         ON CONFLICT (dataset_id, part_key) DO UPDATE
           SET state = jsonb_set(ops.cursor.state, '{synced_to}', to_jsonb(greatest(coalesce(ops.cursor.state->>'synced_to', ''), $3::text))),
               task_id = EXCLUDED.task_id, updated_at = now()",
    )
    .bind(&spec.id)
    .bind(&part)
    .bind(&synced_to)
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 各分区已同步到的日期
pub async fn synced_to(pool: &PgPool, dataset: &str) -> anyhow::Result<BTreeMap<String, NaiveDate>> {
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT part_key, state->>'synced_to' FROM ops.cursor WHERE dataset_id = $1").bind(dataset).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(k, v)| v.and_then(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok()).map(|d| (k, d)))
        .collect())
}

pub async fn planned_since(pool: &PgPool, dataset: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
    let v: Option<String> = sqlx::query_scalar("SELECT state->>'planned_since' FROM ops.cursor WHERE dataset_id = $1 AND part_key = '_plan'")
        .bind(dataset)
        .fetch_optional(pool)
        .await?
        .flatten();
    Ok(v.and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|t| t.to_utc()))
}

/// 记录本次规划读取来源数据的时刻；应在任务写入成功之后调用
pub async fn save_planned_since(pool: &PgPool, dataset: &str, at: DateTime<Utc>) -> anyhow::Result<()> {
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
