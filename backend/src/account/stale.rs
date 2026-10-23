//! 账户变了、建议还没跟上：持仓、现金、规则或设置改动时，在同一个事务里记下时刻（`app.account.advice_dirty_at`）。
//!
//! 这是一条持久化的待办，不是进程内的状态：保存请求提交了，标记就一定在；进程退出、生成失败都不会丢。
//! 消费者（`advice::regen`）生成成功后按读到的时刻确认，期间又有修改时时刻已经变了，确认不生效，会再生成一次；
//! 失败时保留标记、记下原因并推迟重试。哪些写操作要标记见各函数的说明，规则见 docs/domains/accounts.md#generation。

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

/// 重新生成失败后多久再试
pub const RETRY_AFTER: &str = "1 minute";

/// 在账户写操作的事务里标记建议需要重新生成
pub async fn mark_in_tx(tx: &mut Transaction<'_, Postgres>, account_id: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE app.account SET advice_dirty_at = clock_timestamp(), advice_retry_at = NULL, advice_error = NULL
         WHERE account_id = $1",
    )
    .bind(account_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 等着重新生成的账户（到了重试时间的），按标记时刻先后：(账户, 标记时刻)
pub async fn pending(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<(i64, DateTime<Utc>)>> {
    sqlx::query_as(
        "SELECT account_id, advice_dirty_at FROM app.account
         WHERE advice_dirty_at IS NOT NULL AND (advice_retry_at IS NULL OR advice_retry_at <= now())
         ORDER BY advice_dirty_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// 按 `at` 时刻的账户生成成功：标记没变时清掉；期间又改过的不清，等下一次
pub async fn settle(pool: &PgPool, account_id: i64, at: DateTime<Utc>) -> sqlx::Result<bool> {
    Ok(sqlx::query(
        "UPDATE app.account SET advice_dirty_at = NULL, advice_retry_at = NULL, advice_error = NULL
         WHERE account_id = $1 AND advice_dirty_at = $2",
    )
    .bind(account_id)
    .bind(at)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

/// 生成失败：保留标记（没有标记时补上），记下原因，推迟 [`RETRY_AFTER`] 再试
pub async fn defer(pool: &PgPool, account_id: i64, error: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE app.account SET advice_dirty_at = coalesce(advice_dirty_at, clock_timestamp()),
                advice_retry_at = now() + $3::interval, advice_error = $2
         WHERE account_id = $1",
    )
    .bind(account_id)
    .bind(error)
    .bind(RETRY_AFTER)
    .execute(pool)
    .await?;
    Ok(())
}

/// (是否还在等重新生成, 上次失败的原因)
pub async fn status(pool: &PgPool, account_id: i64) -> sqlx::Result<(bool, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT advice_dirty_at IS NOT NULL, advice_error FROM app.account WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or((false, None)))
}
