//! 操作日志：管理员查看成功登录的用户、来源地址和设备。

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::error::ApiResult;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_operation_logs))
}

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct OperationLogListParams {
    /// 返回条数，默认 100，最大 200
    #[param(minimum = 1, maximum = 200, default = 100)]
    pub limit: Option<i64>,
    /// 跳过的条数，默认 0
    #[param(minimum = 0, default = 0)]
    pub offset: Option<i64>,
}

/// 最近的成功登录日志。登录失败不写入操作日志。
#[utoipa::path(get, path = "/operation-logs", tag = "operation-log", params(OperationLogListParams),
    responses((status = 200, description = "成功", body = [crate::operation_log::OperationLog])))]
async fn list_operation_logs(
    State(s): State<AppState>,
    Query(params): Query<OperationLogListParams>,
) -> ApiResult<Json<Vec<crate::operation_log::OperationLog>>> {
    let limit = params.limit.unwrap_or(100).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);
    Ok(Json(
        crate::operation_log::list(&s.pool, limit, offset).await?,
    ))
}
