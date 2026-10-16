//! REST API：`/api/v1/*`。接口规范由 utoipa 从代码生成，地址 `/api/v1/openapi.json`。

pub mod market;
pub mod setup;
pub mod backtest;
pub mod sync;
pub mod watchlist;
mod web;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderName, Request};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::analysis::scan::ScanCache;
use crate::sync::Engine;

/// 请求处理共享的状态：同步引擎（数据集、规划）、连接池和分析结果缓存
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub pool: sqlx::PgPool,
    pub scans: Arc<ScanCache>,
}

#[derive(OpenApi)]
#[openapi(
    info(title = "niulai API", version = "0.1.0", description = "A 股数据工作台：行情、复权、均线信号与数据同步"),
    tags(
        (name = "system", description = "健康检查"),
        (name = "setup", description = "首次登录引导：配置 API Key"),
        (name = "market", description = "证券、日线、指数与均线信号"),
        (name = "watchlist", description = "自选股与快速诊断"),
        (name = "backtest", description = "策略回测"),
        (name = "sync", description = "数据集与同步运行"),
    )
)]
pub struct ApiDoc;

// @project-doc docs/interfaces/rest_api.md#conventions
pub fn router(engine: Arc<Engine>) -> Router {
    let state = AppState { pool: engine.pool.clone(), engine, scans: Arc::new(ScanCache::default()) };
    let (api, openapi) = routes().split_for_parts();
    let openapi = Arc::new(openapi);
    let request_id = HeaderName::from_static("x-request-id");
    let web_dir = state.engine.settings.web_dir.clone();
    let app = Router::new().nest("/api/v1", api.route("/openapi.json", axum::routing::get(move || async move { Json(openapi.as_ref().clone()) })));
    let app = match web_dir {
        Some(dir) => app.fallback_service(web::service(&dir)),
        None => app,
    };
    app.with_state(state)
        .layer(
            TraceLayer::new_for_http().make_span_with(|req: &Request<_>| {
                let id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or("-");
                tracing::info_span!("http", method = %req.method(), uri = %req.uri(), request_id = %id)
            }),
        )
        .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::REQUEST_TIMEOUT, Duration::from_secs(120)))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
}

// @project-doc docs/interfaces/rest_api.md#openapi
/// 全部接口及其规范。新增接口模块时只在这里注册一次，路由和 OpenAPI 规范同时生效
fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health))
        .routes(routes!(ready))
        .merge(setup::routes())
        .merge(market::routes())
        .merge(watchlist::routes())
        .merge(backtest::routes())
        .merge(sync::routes())
}

pub fn openapi_json() -> String {
    routes().split_for_parts().1.to_pretty_json().unwrap_or_default()
}

#[derive(Serialize, ToSchema)]
pub struct Health {
    pub status: &'static str,
}

/// 存活检查
#[utoipa::path(get, path = "/health", tag = "system", responses((status = 200, description = "成功", body = Health)))]
async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

/// 就绪检查：能连上数据库
#[utoipa::path(get, path = "/ready", tag = "system", responses((status = 200, description = "成功", body = Health), (status = 500, description = "数据库不可用", body = crate::error::ErrorBody)))]
async fn ready(State(state): State<AppState>) -> ApiResult<Json<Health>> {
    sqlx::query("SELECT 1").execute(&state.pool).await?;
    Ok(Json(Health { status: "ready" }))
}
