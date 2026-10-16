//! API 错误：对外返回稳定的错误码和请求标识，内部错误只记日志、不暴露细节。

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    /// 与当前状态冲突，例如 Key 由环境变量提供时在网页上修改
    #[error("{0}")]
    Conflict(String),
    /// 依赖的外部服务暂时不可用
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(e.into())
    }
}

/// 错误响应体
#[derive(Serialize, ToSchema)]
pub struct ErrorBody {
    /// 稳定错误码：not_found / bad_request / conflict / unavailable / internal
    pub code: &'static str,
    pub message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.clone()),
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            ApiError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable", m.clone()),
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", "服务器内部错误".to_string())
            }
        };
        (status, Json(ErrorBody { code, message })).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
