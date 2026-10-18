//! 用户管理：只有管理员能用（登录检查中间件按 `/users` 前缀拦截）。不开放注册，用户都在这里添加。
//! 规则见 docs/interfaces/rest_api.md#auth。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::auth::CurrentUser;
use crate::auth::{self, Role, User};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_users, add_user))
        .routes(routes!(update_user))
        .routes(routes!(reset_password))
}

/// 全部用户
#[utoipa::path(get, path = "/users", tag = "users", responses((status = 200, description = "成功", body = [User])))]
async fn list_users(State(s): State<AppState>) -> ApiResult<Json<Vec<User>>> {
    Ok(Json(auth::list(&s.pool).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct NewUser {
    pub username: String,
    /// 初始密码，至少 8 个字符；为空时系统生成一个
    pub password: Option<String>,
    /// 不填时是普通用户：管理员权限要明确给
    #[serde(default = "member")]
    pub role: Role,
}

fn member() -> Role {
    Role::Member
}

#[derive(Serialize, ToSchema)]
pub struct UserWithPassword {
    pub user: User,
    /// 系统生成的密码，只在这一次返回，交给本人；自己填了密码时为空
    pub generated_password: Option<String>,
}

/// 添加用户。本人第一次登录后页面提醒修改密码
#[utoipa::path(post, path = "/users", tag = "users", request_body = NewUser,
    responses((status = 201, description = "已添加", body = UserWithPassword),
              (status = 400, description = "用户名或密码不合法", body = crate::error::ErrorBody),
              (status = 409, description = "用户名已经有人用了", body = crate::error::ErrorBody)))]
async fn add_user(
    State(s): State<AppState>,
    Json(input): Json<NewUser>,
) -> ApiResult<(StatusCode, Json<UserWithPassword>)> {
    let (password, generated) = password_or_generated(input.password)?;
    let user = auth::create(&s.pool, &input.username, &password, input.role).await?;
    tracing::info!(
        user_id = user.user_id,
        username = user.username,
        role = user.role.key(),
        "已添加用户"
    );
    Ok((
        StatusCode::CREATED,
        Json(UserWithPassword {
            user,
            generated_password: generated,
        }),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct UserUpdate {
    pub role: Role,
    /// 停用后不能登录，已登录的也立即失效；数据保留
    pub disabled: bool,
}

/// 修改角色、停用或启用。不能修改自己（避免把自己锁在外面），至少要留一个可以登录的管理员
#[utoipa::path(put, path = "/users/{user_id}", tag = "users", request_body = UserUpdate,
    params(("user_id" = i64, Path, description = "用户编号")),
    responses((status = 200, description = "已修改", body = User),
              (status = 400, description = "不能修改自己，或会没有可以登录的管理员", body = crate::error::ErrorBody),
              (status = 404, description = "用户不存在", body = crate::error::ErrorBody)))]
async fn update_user(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(user_id): Path<i64>,
    Json(input): Json<UserUpdate>,
) -> ApiResult<Json<User>> {
    if user_id == me.user_id {
        return Err(ApiError::BadRequest("不能修改自己的角色或停用自己".into()));
    }
    let user = auth::update(&s.pool, user_id, input.role, input.disabled).await?;
    tracing::info!(
        user_id,
        role = user.role.key(),
        disabled = user.disabled,
        by = me.user_id,
        "已修改用户"
    );
    Ok(Json(user))
}

#[derive(Deserialize, ToSchema)]
pub struct PasswordReset {
    /// 新密码，至少 8 个字符；为空时系统生成一个
    pub password: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ResetResult {
    /// 系统生成的密码，只在这一次返回；自己填了密码时为空
    pub generated_password: Option<String>,
}

/// 重置别人的密码：这个用户已登录的浏览器都要重新登录，登录后页面提醒修改。改自己的密码用 `PUT /auth/password`
#[utoipa::path(post, path = "/users/{user_id}/password", tag = "users", request_body = PasswordReset,
    params(("user_id" = i64, Path, description = "用户编号")),
    responses((status = 200, description = "已重置", body = ResetResult),
              (status = 400, description = "密码太短，或想重置自己的密码", body = crate::error::ErrorBody),
              (status = 404, description = "用户不存在", body = crate::error::ErrorBody)))]
async fn reset_password(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(user_id): Path<i64>,
    Json(input): Json<PasswordReset>,
) -> ApiResult<Json<ResetResult>> {
    if user_id == me.user_id {
        return Err(ApiError::BadRequest(
            "改自己的密码请到「我的」页面，需要输入现在的密码".into(),
        ));
    }
    let (password, generated) = password_or_generated(input.password)?;
    auth::set_password(&s.pool, user_id, &password, false).await?;
    tracing::info!(user_id, by = me.user_id, "已重置密码");
    Ok(Json(ResetResult {
        generated_password: generated,
    }))
}

/// 填了密码就用填的，没填就生成一个（同时返回给调用方）
fn password_or_generated(password: Option<String>) -> ApiResult<(String, Option<String>)> {
    match password.filter(|p| !p.is_empty()) {
        Some(p) => Ok((p, None)),
        None => {
            let p = auth::generate_password()?;
            Ok((p.clone(), Some(p)))
        }
    }
}
