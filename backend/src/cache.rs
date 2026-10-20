//! Redis 共享读取缓存。
//!
//! Redis 只保存可重建的页面结果，PostgreSQL 仍是唯一事实来源。未配置或暂时连不上 Redis
//! 时，调用方应继续走进程内缓存或数据库查询，不能因为缓存故障影响页面请求。

use std::sync::Arc;

use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use serde::Serialize;
use serde::de::DeserializeOwned;

const DEFAULT_TTL_SECONDS: u64 = 15 * 60;
const CACHE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);

#[derive(Clone)]
pub struct Cache {
    client: Option<redis::Client>,
    connection: Arc<tokio::sync::Mutex<Option<ConnectionManager>>>,
}

impl Cache {
    pub fn new(url: Option<&str>) -> anyhow::Result<Self> {
        let client = url
            .map(redis::Client::open)
            .transpose()
            .map_err(|e| anyhow::anyhow!("Redis URL 无效：{e}"))?;
        if client.is_some() {
            tracing::info!("Redis 共享读取缓存已配置");
        }
        Ok(Self {
            client,
            connection: Arc::default(),
        })
    }

    pub fn enabled(&self) -> bool {
        self.client.is_some()
    }

    async fn connection(&self) -> anyhow::Result<Option<ConnectionManager>> {
        let Some(client) = &self.client else {
            return Ok(None);
        };
        let mut guard = self.connection.lock().await;
        if let Some(connection) = guard.as_ref() {
            return Ok(Some(connection.clone()));
        }
        let connection = ConnectionManager::new(client.clone())
            .await
            .map_err(|e| anyhow::anyhow!("连接 Redis 失败：{e}"))?;
        *guard = Some(connection.clone());
        Ok(Some(connection))
    }

    async fn timed_connection(&self) -> anyhow::Result<Option<ConnectionManager>> {
        tokio::time::timeout(CACHE_TIMEOUT, self.connection())
            .await
            .map_err(|_| anyhow::anyhow!("连接 Redis 超时"))?
    }

    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>> {
        let Some(mut connection) = self.timed_connection().await? else {
            return Ok(None);
        };
        let raw: Option<String> = tokio::time::timeout(CACHE_TIMEOUT, connection.get(key))
            .await
            .map_err(|_| anyhow::anyhow!("读取 Redis 缓存超时"))?
            .map_err(|e| anyhow::anyhow!("读取 Redis 缓存失败：{e}"))?;
        raw.map(|value| {
            serde_json::from_str(&value).map_err(|e| anyhow::anyhow!("解析 Redis 缓存失败：{e}"))
        })
        .transpose()
    }

    pub async fn set_json<T: Serialize>(&self, key: &str, value: &T) -> anyhow::Result<()> {
        self.set_json_with_ttl(key, value, DEFAULT_TTL_SECONDS)
            .await
    }

    pub async fn set_json_with_ttl<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl_seconds: u64,
    ) -> anyhow::Result<()> {
        let Some(mut connection) = self.timed_connection().await? else {
            return Ok(());
        };
        let raw = serde_json::to_string(value)?;
        tokio::time::timeout(
            CACHE_TIMEOUT,
            connection.set_ex::<_, _, ()>(key, raw, ttl_seconds),
        )
        .await
        .map_err(|_| anyhow::anyhow!("写入 Redis 缓存超时"))?
        .map_err(|e| anyhow::anyhow!("写入 Redis 缓存失败：{e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_without_url_is_disabled_and_safe() {
        let cache = Cache::new(None).unwrap();
        assert!(!cache.enabled());
    }

    #[test]
    fn invalid_url_is_rejected_without_logging_credentials() {
        let result = Cache::new(Some("not a redis url"));
        assert!(result.is_err());
        assert!(
            result
                .err()
                .is_some_and(|error| error.to_string().contains("Redis URL 无效"))
        );
    }

    #[tokio::test]
    #[ignore = "需要显式配置 NIULAI_REDIS_TEST_URL"]
    async fn redis_round_trip() {
        let url = std::env::var("NIULAI_REDIS_TEST_URL").expect("NIULAI_REDIS_TEST_URL");
        let cache = Cache::new(Some(&url)).unwrap();
        let key = format!("niulai:test:cache:{}", std::process::id());
        let value = serde_json::json!({"status": "ok", "n": 1});
        cache.set_json_with_ttl(&key, &value, 30).await.unwrap();
        assert_eq!(
            cache.get_json::<serde_json::Value>(&key).await.unwrap(),
            Some(value)
        );
    }
}
