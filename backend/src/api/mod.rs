//! REST API：`/api/v1/*`。接口规范由 utoipa 从代码生成，地址 `/api/v1/openapi.json`。

pub mod accounts;
pub mod auth;
pub mod insight;
pub mod market;
pub mod metrics;
pub mod operation_log;
pub mod precompute;
pub mod research;
pub mod setup;
pub mod sync;
pub mod system;
pub mod users;
pub mod watchlist;
mod web;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderName, Request};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::compression::CompressionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::ApiResult;
use crate::sync::Engine;

/// 请求处理共享的状态：同步引擎（数据集、规划）、连接池和分析结果缓存
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub pool: sqlx::PgPool,
    pub cache: Arc<crate::cache::Cache>,
    /// 登录状态的签名密钥，第一次用到时从数据库读取
    pub session_secret: Arc<tokio::sync::OnceCell<Vec<u8>>>,
    /// 登录失败计数
    pub login_guard: Arc<crate::auth::LoginGuard>,
    /// GitHub 上最新版本的查询结果
    pub latest: Arc<system::LatestCache>,
    /// 市场概况，有新日线后失效
    pub overview: Arc<crate::analysis::market::OverviewCache>,
    /// 请求计数和耗时，`GET /api/v1/metrics` 用
    pub metrics: Arc<metrics::Metrics>,
}

/// 依赖复权和权威涨跌停数据的页面结果共用一个可重建版本键。
pub(crate) fn derived_cache_key(
    name: &str,
    version: &crate::analysis::limit_pool::CacheVersions,
    suffix: &str,
) -> String {
    format!(
        "niulai:v1:{name}:adj={:?}:limit={:?}:{suffix}",
        version.adj_factor, version.limit_pool
    )
}

#[derive(OpenApi)]
#[openapi(
    info(title = "niulai API", version = "0.1.0", description = "A 股数据工作台：行情、复权、多因子研究、账户调仓建议与数据同步"),
    tags(
        (name = "system", description = "健康检查、版本与一键升级"),
        (name = "auth", description = "登录与改密码"),
        (name = "operation-log", description = "操作日志（管理员）"),
        (name = "users", description = "用户管理（管理员）"),
        (name = "setup", description = "首次登录引导：配置 API Key"),
        (name = "market", description = "证券、日线、指数与市场环境"),
        (name = "watchlist", description = "自选股"),
        (name = "accounts", description = "我的账户、持仓与每日调仓建议"),
        (name = "research", description = "因子研究：因子检验与多因子组合回测"),
        (name = "sync", description = "数据集与同步运行"),
    )
)]
pub struct ApiDoc;

// @project-doc docs/interfaces/rest_api.md#conventions
pub fn state(engine: Arc<Engine>) -> AppState {
    AppState {
        pool: engine.pool.clone(),
        cache: engine.cache.clone(),
        engine,
        session_secret: Arc::default(),
        login_guard: Arc::default(),
        latest: Arc::default(),
        overview: Arc::default(),
        metrics: Arc::default(),
    }
}

pub fn router(engine: Arc<Engine>) -> Router {
    router_with_state(state(engine))
}

pub fn router_with_state(state: AppState) -> Router {
    let api_timeout_s = state.engine.settings.api_timeout_s;
    let (api, openapi) = routes().split_for_parts();
    let openapi = Arc::new(openapi);
    let request_id = HeaderName::from_static("x-request-id");
    let web_dir = state.engine.settings.web_dir.clone();
    let api = api
        .route(
            "/openapi.json",
            axum::routing::get(move || async move { Json(openapi.as_ref().clone()) }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_login,
        ))
        // 指标接口不经过登录：它自己校验 NIULAI_METRICS_TOKEN，方便外部监控服务读取
        .route("/metrics", axum::routing::get(metrics::handler))
        // 放在最后，前面所有接口（包括指标本身）的请求都会被计数
        .layer(axum::middleware::from_fn_with_state(
            state.metrics.clone(),
            metrics::track,
        ));
    let app = Router::new().nest("/api/v1", api);
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
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(api_timeout_s),
        ))
        // 页面脚本和接口的 JSON 都按浏览器的 Accept-Encoding 压缩，前端脚本从约 600 KB 降到约 190 KB
        .layer(CompressionLayer::new())
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
}

// @project-doc docs/interfaces/rest_api.md#openapi
/// 全部接口及其规范。新增接口模块时只在这里注册一次，路由和 OpenAPI 规范同时生效
fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health))
        .routes(routes!(ready))
        .merge(auth::routes())
        .merge(operation_log::routes())
        .merge(users::routes())
        .merge(system::routes())
        .merge(setup::routes())
        .merge(market::routes())
        .merge(insight::routes())
        .merge(watchlist::routes())
        .merge(accounts::routes())
        .merge(research::routes())
        .merge(sync::routes())
}

pub fn openapi_json() -> String {
    routes()
        .split_for_parts()
        .1
        .to_pretty_json()
        .unwrap_or_default()
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
