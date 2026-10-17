use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{AssertSqlSafe, Executor};

use crate::config::Settings;

/// 程序里所有日期都按上海时间（A 股交易所的时间）理解，与服务器、数据库所在的时区无关
pub const TIME_ZONE: &str = "Asia/Shanghai";

pub async fn connect(settings: &Settings) -> anyhow::Result<PgPool> {
    pool(&settings.database_url, settings.db_max_connections).await
}

// @project-doc docs/interfaces/configuration.md#time_zone
/// 连接池。每个连接建立后把会话时区设为上海：SQL 中的 current_date、now()::date、date_trunc 都按上海的日期计算。
/// sqlx 连接时会自己发送 TimeZone=UTC，连接参数里的 timezone 会被覆盖，所以在连接建立后再设置
pub async fn pool(url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(|conn, _| {
            Box::pin(async move {
                // 审计：TIME_ZONE 是代码里的常量
                conn.execute(AssertSqlSafe(format!("SET TIME ZONE '{TIME_ZONE}'"))).await?;
                Ok(())
            })
        })
        .connect(url)
        .await?;
    Ok(pool)
}

/// 执行 `backend/migrations` 下尚未执行的迁移（编译时嵌入二进制）。
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
