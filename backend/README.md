# niulai 后端

Rust（Axum、SQLx、Tokio）实现的数据同步、分析与查询服务。只有一个程序 `niulai`：`niulai serve` 是常驻服务，在一个进程中运行 REST API、前端页面、同步任务执行和定时调度；其余子命令是运维命令（`cargo run -- --help` 查看）。

工程文档在 [`docs/index.md`](../docs/index.md)。常用入口：

- [运维手册](../docs/operations/runbook.md)：本地运行、首次回补、每日增量、处理失败任务、部署、测试。
- [同步流水线](../docs/architecture/sync_pipeline.md)、[数据集注册表格式](../docs/interfaces/dataset_registry.md)：同步框架和新增数据集。
- [REST API](../docs/interfaces/rest_api.md)、[配置](../docs/interfaces/configuration.md)：接口和环境变量。

## 快速开始

```bash
cd backend
cp .env.example .env               # 填 DATABASE_URL、HITHINK_FINANCE_API_KEY
cargo run -- serve                 # 启动时自动执行迁移；http://127.0.0.1:8080/api/v1/health
```

## 检查

```bash
cargo test                                                                          # 单元测试
NIULAI_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/postgres cargo test   # 加上集成测试（建临时库，结束后删除）
cargo clippy --all-targets                                                          # 提交前应无警告
```
