# ADR：CI 构建镜像，部署只交付服务本身

- 状态：已采纳（accepted），2026-10-16；发布触发条件见 [ADR：只在发布版本标签时构建镜像](image_on_tag.md)，登录见 [ADR：多用户登录与角色](multi_user_login.md)，升级见 [ADR：Docker 部署的一键升级](one_click_upgrade.md)。
- 当前操作步骤：[运维手册](../operations/runbook.md#deployment)。

## 背景

早期部署同时包含 Caddy、安装脚本、升级脚本、备份脚本和恢复脚本，组件多且边界不清。项目需要保留 Docker 快速启动和源码部署两条路径，并把公网入口、证书和域名留给运行环境管理。

## 决策

1. **CI：** Pull Request 和 `main` 推送只运行检查；符合规则的 `v*` 标签在检查通过后构建 amd64 与 arm64 多架构镜像并推送到 GHCR。
2. **Docker 部署：** `deploy/compose.yml` 运行内置 PostgreSQL 和 niulai；常用配置集中在 `.env`，对外只暴露一个 HTTP 端口。
3. **源码部署：** 在本地或服务器直接构建前端和 Rust 后端，在 `backend/` 下运行 `niulai serve`；数据库由部署环境提供。
4. **服务边界：** 页面在 `/`，接口在 `/api/v1/*`，健康检查是 `/api/v1/health` 和 `/api/v1/ready`。网页登录由 niulai 提供；HTTPS、域名和公网入口不属于项目范围。
5. **运维边界：** 项目不提供独立安装、升级、备份和恢复脚本；Docker 升级助手和标准命令由运维手册说明。

## 取舍

- 使用 CI 预构建镜像降低服务器编译成本，但发布依赖标签和镜像可用性。
- 保留源码部署减少运行时依赖，但实施人员需要自行维护 Rust、Node.js、PostgreSQL 和进程管理。
- 不内置公网入口和证书，部署更简单，但暴露到网络前必须由外部入口提供 HTTPS 和访问边界。

## 后果

- 服务默认监听 `127.0.0.1:8080`，内置多用户登录和角色权限。
- `main` 和 Pull Request 不更新正式镜像；服务器建议固定具体发布版本。
- 同一个 API Key 不要同时在两套部署上执行同步。
