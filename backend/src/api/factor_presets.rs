//! 因子预设：当前用户把多因子组合（因子和权重、持有只数、缓冲区）存成一个名字，
//! 账户设置和因子研究页一键套用。规则见 docs/domains/accounts.md#presets。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::auth::CurrentUser;
use crate::account::rules::{FactorWeight, validate_factors, validate_holding};
use crate::error::{ApiError, ApiResult};

/// 每个用户最多保存的预设数
const MAX_PRESETS: i64 = 50;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_presets, save_preset))
        .routes(routes!(delete_preset))
}

#[derive(Serialize, ToSchema)]
pub struct FactorPreset {
    pub preset_id: i64,
    pub name: String,
    pub factors: Vec<FactorWeight>,
    pub top_n: u32,
    pub buffer: f64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
pub struct FactorPresetInput {
    /// 1 到 30 个字；同名（不区分大小写）时覆盖原来的
    pub name: String,
    pub factors: Vec<FactorWeight>,
    /// 持有只数，5 到 50（和账户规则相同）
    pub top_n: u32,
    /// 缓冲区，0 到 2
    pub buffer: f64,
}

type Row = (
    i64,
    String,
    sqlx::types::Json<Vec<FactorWeight>>,
    i32,
    f64,
    DateTime<Utc>,
);

fn to_preset((preset_id, name, factors, top_n, buffer, updated_at): Row) -> FactorPreset {
    FactorPreset {
        preset_id,
        name,
        factors: factors.0,
        top_n: top_n.max(0) as u32,
        buffer,
        updated_at,
    }
}

/// 当前用户保存的预设，最近改过的在前
#[utoipa::path(get, path = "/research/presets", tag = "research",
    responses((status = 200, description = "成功", body = [FactorPreset])))]
async fn list_presets(
    State(s): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<FactorPreset>>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT preset_id, name, factors, top_n, buffer, updated_at FROM app.factor_preset
         WHERE user_id = $1 ORDER BY updated_at DESC, preset_id DESC",
    )
    .bind(me.user_id)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows.into_iter().map(to_preset).collect()))
}

/// 保存一个预设；同名时覆盖
#[utoipa::path(put, path = "/research/presets", tag = "research", request_body = FactorPresetInput,
    responses((status = 200, description = "已保存", body = FactorPreset),
              (status = 400, description = "名字、因子或持有参数不合法，或预设太多", body = crate::error::ErrorBody)))]
async fn save_preset(
    State(s): State<AppState>,
    me: CurrentUser,
    Json(input): Json<FactorPresetInput>,
) -> ApiResult<Json<FactorPreset>> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 30 {
        return Err(ApiError::BadRequest("预设名应是 1 到 30 个字".into()));
    }
    validate_factors(&input.factors).map_err(ApiError::BadRequest)?;
    validate_holding(input.top_n, input.buffer).map_err(ApiError::BadRequest)?;
    let mut tx = s.pool.begin().await?;
    // 锁住用户行，同一用户的保存串行，数量上限不会被并发请求绕过
    sqlx::query("SELECT 1 FROM app.user WHERE user_id = $1 FOR UPDATE")
        .bind(me.user_id)
        .execute(&mut *tx)
        .await?;
    let (count, exists): (i64, bool) = sqlx::query_as(
        "SELECT count(*), bool_or(lower(name) = lower($2)) IS TRUE FROM app.factor_preset WHERE user_id = $1",
    )
    .bind(me.user_id)
    .bind(name)
    .fetch_one(&mut *tx)
    .await?;
    if !exists && count >= MAX_PRESETS {
        return Err(ApiError::BadRequest(format!(
            "最多保存 {MAX_PRESETS} 个预设，先删掉不用的"
        )));
    }
    let row: Row = sqlx::query_as(
        "INSERT INTO app.factor_preset (user_id, name, factors, top_n, buffer) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, lower(name)) DO UPDATE
           SET name = EXCLUDED.name, factors = EXCLUDED.factors, top_n = EXCLUDED.top_n,
               buffer = EXCLUDED.buffer, updated_at = now()
         RETURNING preset_id, name, factors, top_n, buffer, updated_at",
    )
    .bind(me.user_id)
    .bind(name)
    .bind(sqlx::types::Json(&input.factors))
    .bind(input.top_n as i32)
    .bind(input.buffer)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(to_preset(row)))
}

/// 删除一个预设（不存在或不属于当前用户时也返回成功）
#[utoipa::path(delete, path = "/research/presets/{preset_id}", tag = "research", params(("preset_id" = i64, Path)),
    responses((status = 204, description = "已删除")))]
async fn delete_preset(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    sqlx::query("DELETE FROM app.factor_preset WHERE user_id = $1 AND preset_id = $2")
        .bind(me.user_id)
        .bind(id)
        .execute(&s.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
