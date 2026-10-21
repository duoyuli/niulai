# ADR：只在发布版本标签时构建镜像

- 状态：已采纳（accepted），2026-10-16。当前发布版本格式由 [ADR：版本号按发布日期定](calendar_versioning.md) 规定；本 ADR 保留「只有发布标签构建正式镜像」的边界。部分取代 [ADR：CI 构建镜像，部署只交付服务本身](ci_cd.md) 的决策第 1 条和后果「合并到 `main` 得到 `latest`」。
- 相关文档：[CI：构建镜像](../operations/runbook.md#release_image)、[部署](../operations/runbook.md#deployment)。

## 背景

原来 `main` 每次推送都构建 amd64 和 arm64 镜像并更新 `latest`。开发期间一天推送多次，每次都要在两种架构上编译 Rust，耗时长；`latest` 也跟着每次提交变化，服务器拉到的可能是还没准备好发布的代码。用户要求只有发标签时才打包构建镜像。

## 决策

1. 推送到 `main` 和 PR 只运行检查（clippy、后端测试、前端类型检查、代码检查和测试），不构建镜像。
2. 推送符合 `vYYYY.MM.DD` 或 `vYYYY.MM.DD.N` 的标签时，检查通过后构建多架构镜像，打去掉 `v` 的日期版本、`latest` 和 `sha-<提交>` 标签。
3. `latest` 指向最近一次正式发布的日期版本。格式不符合规则的 `v*` 标签直接失败，不构建镜像；项目不使用预发布标签。

## 后果

- 发布新版本：按 [版本号按发布日期定](calendar_versioning.md) 生成日期标签并推送。合并到 `main` 不再产生正式镜像。
- `deploy/compose.yml` 默认的 `latest` 是最近一次发布的版本，不是 `main` 的最新代码；服务器上仍建议在 `deploy/.env` 中固定日期版本号。
- 想在发布前试用 `main` 的代码，在本机构建镜像（`docker build -f deploy/Dockerfile -t niulai:local .`）或用源码部署。
