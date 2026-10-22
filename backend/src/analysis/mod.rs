//! 行情分析：技术指标、信号、选股策略与每日巡检、个股诊断、市场环境、回测。
//!
//! 这里只做计算和读取价格，HTTP 参数校验和响应格式在 `api` 中。指标在 Rust 中计算而不是 SQL 窗口函数，
//! 理由见 docs/decisions/indicator_compute.md。

pub mod backtest;
pub mod diagnosis;
pub mod execution;
pub mod factors;
pub mod indicators;
pub mod limit_pool;
pub mod ma;
pub mod market;
pub mod points;
pub mod portfolio;
pub mod prices;
pub mod research;
pub mod screener;
pub mod signal_lib;
pub mod signal_stats;
pub mod signals;
pub mod strategy;
