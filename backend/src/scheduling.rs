//! 应用级周期抢占。同步调度器和业务维护共用，避免业务层依赖 sync 内部实现。

use chrono::NaiveDate;
use sqlx::{AssertSqlSafe, PgPool};

/// 每个周期只有一个调用者能把 last_fired 推进到新周期，返回 true 的那个负责触发。
pub(crate) async fn claim(
    pool: &PgPool,
    job: &str,
    today: NaiveDate,
    period: &str,
) -> anyhow::Result<bool> {
    let sql = format!(
        "INSERT INTO ops.scheduler (job, last_fired) VALUES ($1, $2)
         ON CONFLICT (job) DO UPDATE SET last_fired = EXCLUDED.last_fired, fired_at = now()
         WHERE date_trunc('{period}', ops.scheduler.last_fired) < date_trunc('{period}', EXCLUDED.last_fired)
         RETURNING job"
    );
    // 审计：period 只会是代码中的 day / week / month
    Ok(sqlx::query_scalar::<_, String>(AssertSqlSafe(sql))
        .bind(job)
        .bind(today)
        .fetch_optional(pool)
        .await?
        .is_some())
}

/// 业务动作失败时撤销周期抢占，让下一次维护可以重试。
pub(crate) async fn release_claim(pool: &PgPool, job: &str, day: NaiveDate) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM ops.scheduler WHERE job = $1 AND last_fired = $2")
        .bind(job)
        .bind(day)
        .execute(pool)
        .await?;
    Ok(())
}
