use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::config::Settings;

pub async fn connect(settings: &Settings) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(settings.db_max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&settings.database_url)
        .await?;
    Ok(pool)
}

/// 执行 `backend/migrations` 下尚未执行的迁移（编译时嵌入二进制）。
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}
