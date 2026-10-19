//! niulai：A 股数据工作台后端。
//!
//! - `sync`：通用同步框架。读取 `config/datasets.yaml`，规划任务、执行请求、写入 PostgreSQL。
//! - `vendor`：同花顺金融数据服务（fuyao）客户端与全局限速。
//! - `analysis`：均线与交叉信号、全市场扫描、策略回测。
//! - `api`：对外的 REST API（Axum + utoipa）。
//! - `server`：常驻服务，在一个进程中运行 API、任务执行和定时调度。
//! - `setup`：首次登录引导，在网页上配置 API Key。
//! - `upgrader`：Docker 部署的一键升级助手（`niulai upgrader`）。

pub mod account;
pub mod advice;
pub mod analysis;
pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod instrument;
pub mod server;
pub mod setup;
pub mod sync;
pub mod telemetry;
pub mod time;
pub mod upgrader;
pub mod vendor;
pub mod version;
