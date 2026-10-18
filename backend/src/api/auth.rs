//! 登录、退出、改自己的密码，以及每个请求的登录检查和角色权限。用户由管理员添加（见 `users.rs`），不开放注册。
//! 除健康检查和登录本身，所有 `/api/v1/*` 接口都要先登录；普通用户只能查看，写操作只能动自己的数据。
//! 规则见 docs/interfaces/rest_api.md#auth。

use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::auth::{self, Role, User};
use crate::error::{ApiError, ApiResult};

/// 浏览器中保存登录状态的 Cookie
pub const SESSION_COOKIE: &str = "niulai_session";
/// 登录状态保存 30 天
const MAX_AGE_SECS: i64 = 30 * 24 * 3600;
/// 不用登录就能访问的接口（路径相对于 `/api/v1`）
const PUBLIC_PATHS: &[&str] = &[
    "/health",
    "/ready",
    "/auth/login",
    "/auth/logout",
    "/openapi.json",
];
/// 只有管理员能访问（包括查看）的接口前缀：数据同步、数据源设置、用户管理、升级、手动巡检
const ADMIN_PREFIXES: &[&str] = &[
    "/sync",
    "/datasets",
    "/setup",
    "/users",
    "/system/upgrade",
    "/screening",
];
/// 普通用户可以调用的写操作：只动自己的数据（自选股、账户都按当前用户过滤），或者只计算、不保存。`*` 匹配一段路径。
/// 其余写操作（策略、信号的增改删等）只有管理员能做：新增接口默认不对普通用户开放
const MEMBER_WRITES: &[(Method, &str)] = &[
    (Method::PUT, "/auth/password"),
    (Method::PUT, "/watchlist/*"),
    (Method::DELETE, "/watchlist/*"),
    (Method::POST, "/accounts"),
    (Method::PUT, "/accounts/*"),
    (Method::DELETE, "/accounts/*"),
    (Method::POST, "/accounts/*/checked"),
    (Method::POST, "/accounts/*/positions/parse"),
    (Method::PUT, "/accounts/*/positions"),
    (Method::POST, "/accounts/*/trades"),
    (Method::DELETE, "/accounts/*/trades/*"),
    (
        Method::POST,
        "/accounts/*/corp-actions/rights/*/*/subscribe",
    ),
    (Method::POST, "/accounts/*/corp-actions/rights/*/*/ignore"),
    (Method::POST, "/accounts/*/advice/refresh"),
    (Method::POST, "/accounts/*/advice/*/items/*/execute"),
    (Method::POST, "/accounts/*/advice/*/items/*/skip"),
    (Method::DELETE, "/accounts/*/advice/*/items/*/skip"),
    (Method::POST, "/accounts/*/backtest"),
    (Method::POST, "/signals/stats"),
    (Method::POST, "/signals/today"),
    (Method::POST, "/strategies/preview"),
    (Method::POST, "/securities/*/signal-days"),
];

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(logout))
        .routes(routes!(me))
        .routes(routes!(change_password))
}

/// 当前登录的用户，由登录检查中间件放进请求；接口用它区分「我的」数据和判断角色
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or_else(|| ApiError::Unauthorized("请先登录".into()))
    }
}

#[derive(Deserialize, ToSchema)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct AuthStatus {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
    /// 密码是管理员设置的或默认的：页面提醒本人修改
    pub must_change_password: bool,
}

impl From<&User> for AuthStatus {
    fn from(u: &User) -> Self {
        AuthStatus {
            user_id: u.user_id,
            username: u.username.clone(),
            role: u.role,
            must_change_password: u.must_change_password,
        }
    }
}

/// 登录：账号密码正确时设置 Cookie，保存 30 天。同一用户名连续输错 5 次后暂停 5 分钟
#[utoipa::path(post, path = "/auth/login", tag = "auth", request_body = LoginInput,
    responses((status = 200, description = "登录成功，响应带 Set-Cookie", body = AuthStatus),
              (status = 401, description = "账号或密码不对，或已停用", body = crate::error::ErrorBody),
              (status = 429, description = "输错次数太多，暂停登录", body = crate::error::ErrorBody)))]
async fn login(State(s): State<AppState>, Json(input): Json<LoginInput>) -> ApiResult<Response> {
    if let Some(left) = s.login_guard.locked_for(&input.username) {
        return Err(ApiError::TooManyRequests(format!(
            "输错次数太多，请 {} 分钟后再试",
            left.as_secs().div_ceil(60)
        )));
    }
    let user = auth::find_by_name(&s.pool, &input.username).await?;
    // 用户不存在时也做一次同样耗时的校验，不让人从耗时看出用户名对不对
    let ok = auth::verify_password(
        user.as_ref().map_or("", |u| u.password_hash.as_str()),
        &input.password,
    )
    .await;
    let user = match user {
        Some(u) if ok && !u.disabled => u,
        _ => {
            s.login_guard.failed(&input.username);
            return Err(ApiError::Unauthorized("账号或密码不对".into()));
        }
    };
    s.login_guard.succeeded(&input.username);
    with_session(&s, &user, Json(AuthStatus::from(&user)).into_response()).await
}

/// 给响应加上登录 Cookie
async fn with_session(s: &AppState, user: &User, mut resp: Response) -> ApiResult<Response> {
    let secret = secret(s).await?;
    let token = auth::sign_session(&secret, user, chrono::Utc::now().timestamp() + MAX_AGE_SECS);
    let cookie =
        format!("{SESSION_COOKIE}={token}; Path=/; Max-Age={MAX_AGE_SECS}; HttpOnly; SameSite=Lax");
    resp.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(anyhow::Error::from)?,
    );
    Ok(resp)
}

/// 退出登录：清除 Cookie
#[utoipa::path(post, path = "/auth/logout", tag = "auth", responses((status = 204, description = "已退出")))]
async fn logout() -> Response {
    let cookie = format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax");
    (StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]).into_response()
}

/// 当前登录的用户；没有登录时返回 401
#[utoipa::path(get, path = "/auth/me", tag = "auth",
    responses((status = 200, description = "已登录", body = AuthStatus),
              (status = 401, description = "没有登录或登录已过期", body = crate::error::ErrorBody)))]
async fn me(State(s): State<AppState>, me: CurrentUser) -> ApiResult<Json<AuthStatus>> {
    let user = auth::find(&s.pool, me.user_id)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("请先登录".into()))?;
    Ok(Json(AuthStatus::from(&user)))
}

#[derive(Deserialize, ToSchema)]
pub struct PasswordInput {
    pub current_password: String,
    pub new_password: String,
}

/// 改自己的密码：要先输入现在的密码。改完后其他浏览器上的登录都失效，这次的响应带新的登录 Cookie
#[utoipa::path(put, path = "/auth/password", tag = "auth", request_body = PasswordInput,
    responses((status = 200, description = "已修改，响应带新的 Set-Cookie", body = AuthStatus),
              (status = 400, description = "现在的密码不对，或新密码太短", body = crate::error::ErrorBody)))]
async fn change_password(
    State(s): State<AppState>,
    me: CurrentUser,
    Json(input): Json<PasswordInput>,
) -> ApiResult<Response> {
    let user = auth::find(&s.pool, me.user_id)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("请先登录".into()))?;
    if !auth::verify_password(&user.password_hash, &input.current_password).await {
        return Err(ApiError::BadRequest("现在的密码不对".into()));
    }
    auth::set_password(&s.pool, user.user_id, &input.new_password, true).await?;
    let user = auth::find(&s.pool, me.user_id)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("请先登录".into()))?;
    with_session(&s, &user, Json(AuthStatus::from(&user)).into_response()).await
}

// @project-doc docs/interfaces/rest_api.md#auth
/// 中间件：没有有效登录状态的请求返回 401；普通用户调用管理员接口返回 403。通过时把当前用户放进请求
pub async fn require_login(State(s): State<AppState>, mut req: Request, next: Next) -> Response {
    if PUBLIC_PATHS.contains(&req.uri().path()) {
        return next.run(req).await;
    }
    let user = match current_user(&s, req.headers()).await {
        Ok(Some(u)) => u,
        Ok(None) => return ApiError::Unauthorized("请先登录".into()).into_response(),
        Err(e) => return ApiError::from(e).into_response(),
    };
    if !allowed(user.role, req.method(), req.uri().path()) {
        return ApiError::Forbidden("只有管理员可以这样做".into()).into_response();
    }
    req.extensions_mut().insert(CurrentUser {
        user_id: user.user_id,
        username: user.username,
        role: user.role,
    });
    next.run(req).await
}

async fn current_user(s: &AppState, headers: &HeaderMap) -> anyhow::Result<Option<User>> {
    let Some((user_id, expires, sig)) =
        cookie(headers, SESSION_COOKIE).and_then(auth::parse_session)
    else {
        return Ok(None);
    };
    let Some(user) = auth::find(&s.pool, user_id).await? else {
        return Ok(None);
    };
    let secret = secret(s).await?;
    Ok(auth::session_valid(&secret, &user, expires, &sig).then_some(user))
}

/// 角色能不能调用这个接口：管理员都可以；普通用户不能碰管理员前缀，写操作只能用 `MEMBER_WRITES` 中的
fn allowed(role: Role, method: &Method, path: &str) -> bool {
    if role == Role::Admin {
        return true;
    }
    if ADMIN_PREFIXES.iter().any(|p| {
        path == *p
            || path
                .strip_prefix(p)
                .is_some_and(|rest| rest.starts_with('/'))
    }) {
        return false;
    }
    if matches!(*method, Method::GET | Method::HEAD) {
        return true;
    }
    MEMBER_WRITES
        .iter()
        .any(|(m, pattern)| m == method && path_matches(pattern, path))
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let (p, a): (Vec<&str>, Vec<&str>) = (pattern.split('/').collect(), path.split('/').collect());
    p.len() == a.len()
        && p.iter()
            .zip(&a)
            .all(|(p, a)| (*p == "*" && !a.is_empty()) || p == a)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cookie_among_others() {
        let mut h = HeaderMap::new();
        h.insert(
            COOKIE,
            HeaderValue::from_static("a=1; niulai_session=1.123.abc; b=2"),
        );
        assert_eq!(cookie(&h, SESSION_COOKIE), Some("1.123.abc"));
        assert_eq!(cookie(&h, "niulai"), None);
    }

    #[test]
    fn members_read_and_manage_only_their_own_data() {
        let m = Role::Member;
        assert!(allowed(Role::Admin, &Method::DELETE, "/strategies/3"));
        assert!(
            allowed(m, &Method::GET, "/strategies/3")
                && allowed(m, &Method::GET, "/system/version")
        );
        assert!(
            allowed(m, &Method::PUT, "/watchlist/600519.SH")
                && allowed(m, &Method::PUT, "/auth/password")
        );
        assert!(
            allowed(m, &Method::POST, "/securities/600519.SH/signal-days")
                && allowed(m, &Method::POST, "/signals/stats")
        );
        assert!(
            allowed(m, &Method::POST, "/accounts")
                && allowed(
                    m,
                    &Method::POST,
                    "/accounts/3/advice/9/items/buy-600519.SH/execute"
                )
        );
        for (method, path) in [
            (Method::POST, "/strategies"),
            (Method::PUT, "/strategies/3"),
            (Method::DELETE, "/signals/my/2"),
            (Method::POST, "/screening/run"),
            (Method::POST, "/system/upgrade"),
            (Method::GET, "/sync/runs"),
            (Method::GET, "/datasets"),
            (Method::GET, "/setup"),
            (Method::GET, "/users"),
            (Method::PUT, "/watchlist"),
            (Method::PUT, "/watchlist/a/b"),
        ] {
            assert!(!allowed(m, &method, path), "{method} {path}");
        }
        assert!(allowed(m, &Method::GET, "/syncx"), "前缀按整段匹配");
    }
}
