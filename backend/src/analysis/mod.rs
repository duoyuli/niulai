//! 行情分析：证券信息、价格读取、全市场计算的资源预算、市场环境、今日热点、横截面因子研究和研究运行记录。
//!
//! 这里只做计算和读取价格，HTTP 参数校验和响应格式在 `api` 中。因子在 Rust 中计算而不是 SQL 窗口函数，
//! 理由见 docs/decisions/indicator_compute.md。

pub mod compute;
pub mod costs;
pub mod factor_lab;
pub mod factors;
pub mod hotspots;
pub mod limit_pool;
pub mod market;
pub mod prices;
pub mod research;
pub mod research_run;
pub mod securities;
pub mod universe;
