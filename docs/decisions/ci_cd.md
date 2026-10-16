# ADR：CI 构建镜像，部署只交付服务本身

- 状态：已采纳（accepted），2026-10-16。取代 [ADR：部署方案](deployment.md)。
- 相关文档：[部署](../operations/runbook.md#deployment)、[部署配置](../interfaces/configuration.md#deploy_env)、[ADR：API Key 在首次登录引导中配置](api_key_setup.md)、[ADR：单进程服务](single_process_service.md)。

## 背景

上一版部署方案在 `deploy/` 中放了 Caddy（HTTPS 和基本认证登录）、安装、升级、备份、恢复共 5 个脚本和 3 个 Compose 文件。用户认为认知成本太高，要求：

1. 分清 CI 和 CD。CI：`main` 分支有合并时，由 GitHub 自动构建镜像并推送到镜像仓库。CD：分为源码部署和 Docker 部署，Docker 部署要有一个默认的 Compose 文件。
2. 项目只负责服务本身，在 Docker 中明确暴露哪个端口和端点。公网访问由使用者自己的 Nginx 反向代理负责，证书也在那里配置。

## 决策

1. **CI**：`.github/workflows/ci.yml`。每次 PR 和推送都运行检查：后端 clippy（不允许警告）、单元测试和集成测试（临时 PostgreSQL），前端类型检查、代码检查和测试。`main` 有合并或推送 `v*` 标签时，检查通过后构建 amd64 和 arm64 镜像（各在对应架构的机器上原生构建），合并为多架构镜像推送到 `ghcr.io/<所有者>/niulai`。
2. **CD：Docker 部署**（默认）：`deploy/compose.yml` 运行 PostgreSQL 18.6 和 niulai 两个容器，配置只有 `deploy/.env` 一个文件。命令是 `cp .env.example .env`、填写、`docker compose up -d`。
3. **CD：源码部署**：叠加 `deploy/compose.source.yml`，在本机用同一个 `deploy/Dockerfile` 从源码构建镜像，其余与 Docker 部署相同。
4. **对外接口**：只暴露 niulai 的一个 HTTP 端口，默认 `127.0.0.1:8080`（`NIULAI_LISTEN`）。页面在 `/`，接口在 `/api/v1/*`，健康检查是 `/api/v1/health`（存活）和 `/api/v1/ready`（能连上数据库）。HTTPS、域名、登录和访问控制都由前面的反向代理负责，项目中不再包含相关配置。
5. **不提供运维脚本**：升级、备份、恢复都是几条标准的 `docker compose` 命令，写在运维手册中。

## 取舍

- **去掉 Caddy 和内置登录**：部署更简单，也不和使用者已有的 Nginx、证书管理重复。代价是服务本身没有任何认证，必须由反向代理限制访问；默认只监听本机，防止误把端口直接暴露到网络上。
- **去掉脚本**：少了「升级前自动备份」「恢复时先恢复到临时库再互换」这类保护，改为在运维手册中写明顺序：先备份，再升级；恢复前先备份当前库。换来的是没有需要理解和维护的自定义脚本。
- **源码部署仍然通过 Docker**：直接在服务器上安装 Rust、Node.js 编译运行也可行，但要自己管理进程、依赖和升级。用同一个 Dockerfile 构建，源码部署和镜像部署的运行环境完全一致。

## 后果

- 使用者需要自己配置反向代理：转发到 `NIULAI_LISTEN`，在代理上配置证书和访问控制。
- 发布新版本：合并到 `main` 得到 `latest`；打 `v*` 标签得到固定版本号，服务器上建议用固定版本。
- 同一个 API Key 仍然不要同时用在两套部署上。
