//! 本人的推送设置：群机器人地址（企业微信、飞书、钉钉），每个交易日的运营建议生成后推送。见 `crate::notify`。

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::notify;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_notify, set_notify)).routes(routes!(test_notify))
}

#[derive(Serialize, ToSchema)]
pub struct NotifySettings {
    /// 群机器人地址；为空表示不推送
    pub webhook: Option<String>,
    /// 哪一家：企业微信、飞书、钉钉
    pub channel: Option<String>,
    /// 最近一次推送失败的原因；成功后清空
    pub last_error: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct NotifyInput {
    /// 群机器人地址；为空或 null 表示不再推送
    pub webhook: Option<String>,
}

fn channel_name(url: &str) -> Option<String> {
    notify::channel(url).ok().map(|c| {
        match c {
            notify::Channel::WeCom => "企业微信",
            notify::Channel::Feishu => "飞书",
            notify::Channel::DingTalk => "钉钉",
        }
        .to_string()
    })
}

async fn load(s: &AppState, user_id: i64) -> ApiResult<NotifySettings> {
    let (webhook, last_error): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT notify_webhook, notify_error FROM app.user WHERE user_id = $1").bind(user_id).fetch_one(&s.pool).await?;
    Ok(NotifySettings { channel: webhook.as_deref().and_then(channel_name), webhook, last_error })
}

/// 本人的推送设置
#[utoipa::path(get, path = "/me/notify", tag = "auth", responses((status = 200, description = "成功", body = NotifySettings)))]
async fn get_notify(State(s): State<AppState>, me: CurrentUser) -> ApiResult<Json<NotifySettings>> {
    Ok(Json(load(&s, me.user_id).await?))
}

/// 设置或清除群机器人地址
#[utoipa::path(put, path = "/me/notify", tag = "auth", request_body = NotifyInput,
    responses((status = 200, description = "已保存", body = NotifySettings),
              (status = 400, description = "不是企业微信、飞书、钉钉群机器人的 https 地址", body = crate::error::ErrorBody)))]
async fn set_notify(State(s): State<AppState>, me: CurrentUser, Json(input): Json<NotifyInput>) -> ApiResult<Json<NotifySettings>> {
    let url = input.webhook.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    if let Some(u) = &url {
        notify::channel(u).map_err(ApiError::BadRequest)?;
    }
    sqlx::query("UPDATE app.user SET notify_webhook = $2, notify_error = NULL, updated_at = now() WHERE user_id = $1")
        .bind(me.user_id)
        .bind(&url)
        .execute(&s.pool)
        .await?;
    Ok(Json(load(&s, me.user_id).await?))
}

/// 往保存的地址发一条试试
#[utoipa::path(post, path = "/me/notify/test", tag = "auth",
    responses((status = 200, description = "已发出", body = NotifySettings),
              (status = 400, description = "还没有设置地址，或机器人返回了错误", body = crate::error::ErrorBody)))]
async fn test_notify(State(s): State<AppState>, me: CurrentUser) -> ApiResult<Json<NotifySettings>> {
    let cur = load(&s, me.user_id).await?;
    let url = cur.webhook.ok_or_else(|| ApiError::BadRequest("还没有设置群机器人地址".into()))?;
    notify::send(&url, "【牛来】推送测试：以后每个交易日的运营建议生成后会发到这里。")
        .await
        .map_err(|e| ApiError::BadRequest(format!("发送失败：{e:#}")))?;
    Ok(Json(load(&s, me.user_id).await?))
}
