//! 首次登录引导接口：查看和配置同花顺金融数据服务的 API Key。逻辑在 `crate::setup`。

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::error::{ApiError, ApiResult};
use crate::setup::{self, KeySource, SetupError};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_setup))
        .routes(routes!(put_api_key))
}

#[derive(Serialize, ToSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeySource {
    /// 环境变量 HITHINK_FINANCE_API_KEY，网页上不能修改
    Env,
    /// 在网页上配置，保存在数据库中
    Database,
}

#[derive(Serialize, ToSchema)]
pub struct SetupStatus {
    /// 是否已经有可用的 API Key；为 false 时同步暂停，前端显示引导页
    pub api_key_configured: bool,
    pub api_key_source: Option<ApiKeySource>,
    /// 只露出末尾 4 位，例如 ••••3456
    pub api_key_hint: Option<String>,
    /// 生成 API Key 的页面
    pub api_key_url: &'static str,
    /// 是否已经规划过全量回补
    pub backfill_planned: bool,
}

/// 首次登录引导的状态
#[utoipa::path(get, path = "/setup", tag = "setup", responses((status = 200, description = "成功", body = SetupStatus)))]
async fn get_setup(State(s): State<AppState>) -> ApiResult<Json<SetupStatus>> {
    let status = setup::key_status(&s.engine).await?;
    Ok(Json(SetupStatus {
        api_key_configured: status.is_some(),
        api_key_source: status.as_ref().map(|(src, _)| match src {
            KeySource::Env => ApiKeySource::Env,
            KeySource::Database => ApiKeySource::Database,
        }),
        api_key_hint: status.map(|(_, h)| h),
        api_key_url: setup::API_KEY_URL,
        backfill_planned: setup::backfill_planned(&s.pool).await?,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct ApiKeyInput {
    pub api_key: String,
}

#[derive(Serialize, ToSchema)]
pub struct ApiKeySaved {
    /// 第一次配置时规划的全量回补（第一个波次的运行 ID）；之前已经回补过时为空
    pub backfill_run_id: Option<i64>,
}

/// 验证并保存 API Key，立即生效
#[utoipa::path(put, path = "/setup/api-key", tag = "setup", request_body = ApiKeyInput,
    responses((status = 200, description = "已保存", body = ApiKeySaved),
              (status = 400, description = "Key 为空或无效", body = crate::error::ErrorBody),
              (status = 409, description = "Key 由环境变量提供，不能在网页上修改", body = crate::error::ErrorBody),
              (status = 503, description = "暂时连不上同花顺接口，无法验证", body = crate::error::ErrorBody)))]
async fn put_api_key(
    State(s): State<AppState>,
    Json(body): Json<ApiKeyInput>,
) -> ApiResult<Json<ApiKeySaved>> {
    match setup::configure_api_key(&s.engine, &body.api_key).await {
        Ok(backfill_run_id) => Ok(Json(ApiKeySaved { backfill_run_id })),
        Err(e @ SetupError::FromEnv) => Err(ApiError::Conflict(e.to_string())),
        Err(e @ SetupError::Invalid(_)) => Err(ApiError::BadRequest(e.to_string())),
        Err(e @ SetupError::Unreachable(_)) => Err(ApiError::Unavailable(e.to_string())),
        Err(SetupError::Other(e)) => Err(ApiError::Internal(e)),
    }
}
