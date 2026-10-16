//! 行情分析：均线与交叉信号、全市场扫描、策略回测。
//!
//! 这里只做计算和读取价格，HTTP 参数校验和响应格式在 `api` 中。指标在 Rust 中计算而不是 SQL 窗口函数，
//! 理由见 docs/decisions/indicator_compute.md。

pub mod backtest;
pub mod ma;
pub mod prices;
pub mod scan;
