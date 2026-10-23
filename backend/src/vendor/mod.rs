//! 同花顺金融数据服务的客户端与进程内限速。
//!
//! 只有一个供应商，同步层直接使用 [`HithinkClient`]，不再包一层只有一个实现的 trait。端点名、分页、错误码和
//! 导出文件格式本来就散布在 `sync::fetch`、`sync::sinks` 和 `datasets.yaml` 中，换数据源时要连同它们一起评估，
//! 见 docs/decisions/hithink_data_source.md。

pub mod client;
pub mod ratelimit;

pub use client::{CallError, CallResult, HithinkClient};
pub use ratelimit::RateLimiter;
