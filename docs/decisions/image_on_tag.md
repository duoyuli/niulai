# ADR：只在发布版本标签时构建镜像

- 状态：已采纳（accepted），2026-10-16。部分取代 [ADR：CI 构建镜像，部署只交付服务本身](ci_cd.md) 的决策第 1 条和后果「合并到 `main` 得到 `latest`」。
- 相关文档：[CI：构建镜像](../operations/runbook.md#release_image)、[部署](../operations/runbook.md#deployment)。

## 背景

原来 `main` 每次推送都构建 amd64 和 arm64 镜像并更新 `latest`。开发期间一天推送多次，每次都要在两种架构上编译 Rust，耗时长；`latest` 也跟着每次提交变化，服务器拉到的可能是还没准备好发布的代码。用户要求只有发标签时才打包构建镜像。

## 决策

1. 推送到 `main` 和 PR 只运行检查（clippy、后端测试、前端类型检查、代码检查和测试），不构建镜像。
2. 推送 `v*` 标签时，检查通过后构建多架构镜像，打 `0.1.0`、`0.1`、`sha-<提交>` 标签。
3. `latest` 指向最近一次正式发布的版本，由 `docker/metadata-action` 的 `latest=auto` 决定：预发布标签（例如 `v0.2.0-rc.1`）只打完整版本号和 `sha`，不更新 `latest` 和 `0.2`。

## 后果

- 发布新版本：`git tag v0.1.0 && git push origin v0.1.0`。合并到 `main` 不再产生新镜像。
- `deploy/compose.yml` 默认的 `latest` 是最近一次发布的版本，不是 `main` 的最新代码；服务器上仍建议在 `deploy/.env` 中固定版本号。
- 想在发布前试用 `main` 的代码，在本机构建镜像（`docker build -f deploy/Dockerfile -t niulai:local .`）或用源码部署。
