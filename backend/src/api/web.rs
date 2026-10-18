//! 前端页面：`NIULAI_WEB_DIR` 指向 frontend/dist 时，由 `niulai serve` 同源提供，不需要另外的 Web 服务器。
//!
//! - 带哈希的静态资源（/assets/*）长期缓存；其余文件（index.html 等）每次协商，发布新版本后立即生效。
//! - 前端是单页应用，不存在的页面路径返回 index.html，由前端路由处理；/assets/ 下不存在的文件返回 404，
//!   不能返回页面（否则旧页面引用的脚本变成一份被长期缓存的 HTML）。
//! - 未匹配的 /api/ 路径仍返回 JSON 404，不返回页面。

use std::convert::Infallible;
use std::path::Path;

use axum::body::Body;
use axum::http::{HeaderValue, Request, header};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower::util::BoxCloneSyncService;
use tower_http::services::{ServeDir, ServeFile};

use crate::error::ApiError;

// @project-doc docs/architecture/frontend.md#serving
pub fn service(dir: &Path) -> BoxCloneSyncService<Request<Body>, Response, Infallible> {
    let assets = ServeDir::new(dir);
    let spa = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
    BoxCloneSyncService::new(tower::service_fn(move |req: Request<Body>| {
        let (spa, assets) = (spa.clone(), assets.clone());
        async move {
            let path = req.uri().path().to_owned();
            if path == "/api" || path.starts_with("/api/") {
                return Ok(ApiError::NotFound(format!("接口不存在：{path}")).into_response());
            }
            let is_asset = path.starts_with("/assets/");
            let mut res = if is_asset { assets.oneshot(req).await.into_response() } else { spa.oneshot(req).await.into_response() };
            if is_asset && !res.status().is_success() {
                return Ok(res);
            }
            let cache = if is_asset { "public, max-age=31536000, immutable" } else { "no-cache" };
            res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
            Ok(res)
        }
    }))
}
