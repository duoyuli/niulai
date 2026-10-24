//! 集成测试：在临时数据库上验证迁移、写入、复权、任务语义和 API。不调用供应商接口。
//!
//! 需要一个有建库权限的 PostgreSQL 连接串：
//!   NIULAI_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/postgres cargo test --test integration
//! 未设置时测试直接跳过。每个测试创建并在结束时删除自己的数据库。

mod common;

mod accounts;
mod auth;
mod market;
mod research;
mod sync;
