# 接口文档索引

> 上级索引：[工程文档](../index.md)

## 范围

本分类记录对外和对第三方的契约：同花顺金融数据服务的接口约定与限制、可同步的数据、数据集配置格式、本项目的 REST API 和配置项。这些契约背后的同步流程见 [架构](../architecture/index.md)，数据的业务含义见 [领域](../domains/index.md)。

## 文档

- [同花顺接口](hithink_api.md)：认证、响应信封、错误码与重试分类、请求频率、各接口的限制和已知问题、全市场导出、不可用的接口。阅读时机：调用或新增供应商接口、排查 `ops.api_call` 中的错误码、修改 `backend/src/vendor/client.rs`。
- [数据目录](dataset_catalog.md)：86 个数据集的内容、历史深度、同步方式和请求量。阅读时机：想知道某类数据能不能取到、取多深，或评估回补和每日同步的请求量。
- [数据集注册表格式](dataset_registry.md)：`datasets.yaml` 的字段、分区类型、主键模板、写入目标和新增数据集的步骤。阅读时机：新增或修改数据集配置、修改 `backend/src/sync/spec.rs`、`planner.rs`、`template.rs`。
- [REST API](rest_api.md)：API 的通用约定、接口清单、复权参数、错误格式和 OpenAPI。阅读时机：调用或修改 `/api/v1` 接口、生成前端类型。
- [配置](configuration.md)：配置来源与优先级、时区（一律按上海时间）、全部环境变量、部署用的 `deploy/.env`、配置文件。阅读时机：部署、修改 `backend/src/config.rs`、`backend/.env.example`、`deploy/.env.example`，或排查启动时的配置错误。
