//! 登录：只有一个管理员账户，账号密码来自配置（默认都是 niulai）。登录后在浏览器保存 30 天。
//! 除健康检查和登录本身，所有 `/api/v1/*` 接口都要先登录。规则见 docs/interfaces/rest_api.md#auth。

use axum::extract::{Request, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::config::{DEFAULT_ADMIN_PASSWORD, DEFAULT_ADMIN_USER};
use crate::error::{ApiError, ApiResult};

/// 浏览器中保存登录状态的 Cookie
pub const SESSION_COOKIE: &str = "niulai_session";
/// 登录状态保存 30 天
const MAX_AGE_SECS: i64 = 30 * 24 * 3600;
/// 不用登录就能访问的接口（路径相对于 `/api/v1`）
const PUBLIC_PATHS: &[&str] = &["/health", "/ready", "/auth/login", "/auth/logout", "/openapi.json"];

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(login)).routes(routes!(logout)).routes(routes!(me))
}

#[derive(Deserialize, ToSchema)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct AuthStatus {
    pub username: String,
    /// 仍在使用默认账号密码（niulai / niulai）：页面上提醒修改
    pub default_credentials: bool,
}

fn status(s: &AppState) -> AuthStatus {
    let st = &s.engine.settings;
    AuthStatus {
        username: st.admin_user.clone(),
        default_credentials: st.admin_user == DEFAULT_ADMIN_USER && st.admin_password == DEFAULT_ADMIN_PASSWORD,
    }
}

/// 登录：账号密码正确时设置 Cookie，保存 30 天
#[utoipa::path(post, path = "/auth/login", tag = "auth", request_body = LoginInput,
    responses((status = 200, description = "登录成功，响应带 Set-Cookie", body = AuthStatus),
              (status = 401, description = "账号或密码不对", body = crate::error::ErrorBody)))]
async fn login(State(s): State<AppState>, Json(input): Json<LoginInput>) -> ApiResult<Response> {
    let secret = secret(&s).await?;
    let st = &s.engine.settings;
    // 两边都做 HMAC 再比较，比较时间与内容无关
    let given = mac(&secret, &format!("{}\n{}", input.username, input.password));
    let ok = given.verify_slice(&mac(&secret, &format!("{}\n{}", st.admin_user, st.admin_password)).finalize().into_bytes()).is_ok();
    if !ok {
        // 放慢猜密码的速度
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        return Err(ApiError::Unauthorized("账号或密码不对".into()));
    }
    let expires = chrono::Utc::now().timestamp() + MAX_AGE_SECS;
    let token = format!("{expires}.{}", sign(&secret, &st.admin_user, &st.admin_password, expires));
    let cookie = format!("{SESSION_COOKIE}={token}; Path=/; Max-Age={MAX_AGE_SECS}; HttpOnly; SameSite=Lax");
    let mut resp = Json(status(&s)).into_response();
    resp.headers_mut().insert(SET_COOKIE, HeaderValue::from_str(&cookie).map_err(anyhow::Error::from)?);
    Ok(resp)
}

/// 退出登录：清除 Cookie
#[utoipa::path(post, path = "/auth/logout", tag = "auth", responses((status = 204, description = "已退出")))]
async fn logout() -> Response {
    let cookie = format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax");
    (StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]).into_response()
}

/// 当前登录状态；没有登录时返回 401
#[utoipa::path(get, path = "/auth/me", tag = "auth",
    responses((status = 200, description = "已登录", body = AuthStatus),
              (status = 401, description = "没有登录或登录已过期", body = crate::error::ErrorBody)))]
async fn me(State(s): State<AppState>) -> Json<AuthStatus> {
    Json(status(&s))
}

// @project-doc docs/interfaces/rest_api.md#auth
/// 中间件：没有有效登录状态的请求返回 401
pub async fn require_login(State(s): State<AppState>, req: Request, next: Next) -> Response {
    if PUBLIC_PATHS.contains(&req.uri().path()) {
        return next.run(req).await;
    }
    match logged_in(&s, req.headers()).await {
        Ok(true) => next.run(req).await,
        Ok(false) => ApiError::Unauthorized("请先登录".into()).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

async fn logged_in(s: &AppState, headers: &HeaderMap) -> sqlx::Result<bool> {
    let Some(token) = cookie(headers, SESSION_COOKIE) else { return Ok(false) };
    let Some((expires, sig)) = token.split_once('.') else { return Ok(false) };
    let Ok(expires) = expires.parse::<i64>() else { return Ok(false) };
    if expires <= chrono::Utc::now().timestamp() {
        return Ok(false);
    }
    let Some(sig) = from_hex(sig) else { return Ok(false) };
    let secret = secret(s).await?;
    let st = &s.engine.settings;
    Ok(mac(&secret, &payload(&st.admin_user, &st.admin_password, expires)).verify_slice(&sig).is_ok())
}

/// 签名内容包含账号和密码：改了任何一个，之前的登录状态都失效
fn payload(user: &str, password: &str, expires: i64) -> String {
    format!("session\n{user}\n{password}\n{expires}")
}

fn sign(secret: &[u8], user: &str, password: &str, expires: i64) -> String {
    to_hex(&mac(secret, &payload(user, password, expires)).finalize().into_bytes())
}

fn mac(secret: &[u8], data: &str) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC 接受任意长度的密钥");
    m.update(data.as_bytes());
    m
}

/// 签名密钥：第一次用到时随机生成并保存在 `app.setting`，重启后登录状态仍然有效
async fn secret(s: &AppState) -> sqlx::Result<Vec<u8>> {
    s.session_secret
        .get_or_try_init(|| async {
            // gen_random_uuid() 来自 PostgreSQL 的强随机数，两个共 244 位随机
            sqlx::query(
                "INSERT INTO app.setting (name, value) VALUES ('session_secret', replace(gen_random_uuid()::text || gen_random_uuid()::text, '-', ''))
                 ON CONFLICT (name) DO NOTHING",
            )
            .execute(&s.pool)
            .await?;
            let v: String = sqlx::query_scalar("SELECT value FROM app.setting WHERE name = 'session_secret'").fetch_one(&s.pool).await?;
            Ok(v.into_bytes())
        })
        .await
        .cloned()
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(name)?.strip_prefix('='))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_depends_on_credentials_and_expiry() {
        let k = b"secret";
        let a = sign(k, "niulai", "niulai", 100);
        assert_eq!(a, sign(k, "niulai", "niulai", 100));
        assert_ne!(a, sign(k, "niulai", "changed", 100), "改了密码，旧的登录状态失效");
        assert_ne!(a, sign(k, "niulai", "niulai", 101));
        assert_eq!(from_hex(&a).map(|v| to_hex(&v)), Some(a));
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn reads_cookie_among_others() {
        let mut h = HeaderMap::new();
        h.insert(COOKIE, HeaderValue::from_static("a=1; niulai_session=123.abc; b=2"));
        assert_eq!(cookie(&h, SESSION_COOKIE), Some("123.abc"));
        assert_eq!(cookie(&h, "niulai"), None);
    }
}
