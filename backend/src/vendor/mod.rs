pub mod client;
pub mod ratelimit;

pub use client::{CallError, CallResult, HithinkClient};
pub use ratelimit::RateLimiter;

use bytes::Bytes;

/// 供应商数据源边界：同步层只依赖取数、下载和 Key 生命周期，不绑定具体供应商实现。
#[async_trait::async_trait]
pub trait MarketDataClient: Send + Sync {
    fn has_api_key(&self) -> bool;
    fn set_api_key(&self, key: Option<String>);
    async fn get(
        &self,
        endpoint: &str,
        query: &[(String, String)],
    ) -> Result<CallResult, CallError>;
    async fn verify_key(&self, api_key: &str, endpoint: &str) -> Result<CallResult, CallError>;
    async fn download(&self, url: &str) -> Result<Bytes, CallError>;
}

#[async_trait::async_trait]
impl MarketDataClient for HithinkClient {
    fn has_api_key(&self) -> bool {
        HithinkClient::has_api_key(self)
    }

    fn set_api_key(&self, key: Option<String>) {
        HithinkClient::set_api_key(self, key)
    }

    async fn get(
        &self,
        endpoint: &str,
        query: &[(String, String)],
    ) -> Result<CallResult, CallError> {
        HithinkClient::get(self, endpoint, query).await
    }

    async fn verify_key(&self, api_key: &str, endpoint: &str) -> Result<CallResult, CallError> {
        HithinkClient::verify_key(self, api_key, endpoint).await
    }

    async fn download(&self, url: &str) -> Result<Bytes, CallError> {
        HithinkClient::download(self, url).await
    }
}
