//! 行情分析：证券信息、价格读取、市场环境和横截面因子研究。
//!
//! 这里只做计算和读取价格，HTTP 参数校验和响应格式在 `api` 中。因子在 Rust 中计算而不是 SQL 窗口函数，
//! 理由见 docs/decisions/indicator_compute.md。

pub mod costs;
pub mod factor_lab;
pub mod factors;
pub mod limit_pool;
pub mod market;
pub mod prices;
pub mod research;
pub mod securities;
pub mod universe;
