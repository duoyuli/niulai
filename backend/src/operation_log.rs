//! 操作日志：目前只记录成功登录，供管理员核对登录用户、来源地址和设备。

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct OperationLog {
    /// 日志编号
    pub log_id: i64,
    /// 用户被删除后为空；username 仍保留当时的用户名
    pub user_id: Option<i64>,
    pub username: String,
    /// 当前只会是 login
    pub action: String,
    pub ip_address: String,
    /// 登录请求的 User-Agent
    pub device: String,
    pub created_at: DateTime<Utc>,
}

type LogRow = (
    i64,
    Option<i64>,
    String,
    String,
    String,
    String,
    DateTime<Utc>,
);

/// 记录一次成功登录。设备信息只保存有限长度，避免异常请求头撑大日志。
pub async fn record_login(
    pool: &PgPool,
    user_id: i64,
    username: &str,
    ip_address: &str,
    device: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO app.operation_log (user_id, username, action, ip_address, device)
         VALUES ($1, $2, 'login', $3, $4)",
    )
    .bind(user_id)
    .bind(username)
    .bind(truncate(ip_address, 128))
    .bind(truncate(device, 512))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &PgPool, limit: i64, offset: i64) -> sqlx::Result<Vec<OperationLog>> {
    let rows: Vec<LogRow> = sqlx::query_as(
        "SELECT log_id, user_id, username, action, ip_address, device, created_at
         FROM app.operation_log
         ORDER BY created_at DESC, log_id DESC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(log_id, user_id, username, action, ip_address, device, created_at)| OperationLog {
                log_id,
                user_id,
                username,
                action,
                ip_address,
                device,
                created_at,
            },
        )
        .collect())
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncates_by_characters() {
        assert_eq!(truncate("设备🙂信息", 3), "设备🙂");
    }
}
