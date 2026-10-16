# ADR：CI 构建镜像，部署只交付服务本身

- 状态：已采纳（accepted），2026-10-16。取代 [ADR：部署方案](deployment.md)。同日修订：决策第 3 条从「源码部署也用 Docker 构建镜像」改为「直接编译运行，不用 Docker」，删除了 `deploy/compose.source.yml`；决策第 2 条改为数据库始终内置、`.env.example` 列出全部配置并给出可直接使用的默认值。决策第 1 条中「`main` 有合并时构建镜像」和后果「合并到 `main` 得到 `latest`」已被 [ADR：只在发布版本标签时构建镜像](image_on_tag.md) 取代。决策第 4 条「项目中不包含登录」、取舍「去掉内置登录」和后果「服务没有登录和访问控制」已被 [ADR：网页登录](web_login.md) 取代；HTTPS 和域名仍不在项目范围内。决策第 5 条中的升级部分已被 [ADR：Docker 部署的一键升级](one_click_upgrade.md) 取代：Docker 部署可以在网页上一键升级，由 `niulai upgrader` 执行；仍然不提供运维脚本。
- 相关文档：[部署](../operations/runbook.md#deployment)、[部署配置](../interfaces/configuration.md#deploy_env)、[ADR：API Key 在首次登录引导中配置](api_key_setup.md)、[ADR：单进程服务](single_process_service.md)。

## 背景

上一版部署方案在 `deploy/` 中放了 Caddy（HTTPS 和基本认证登录）、安装、升级、备份、恢复共 5 个脚本和 3 个 Compose 文件。用户认为认知成本太高，要求：

1. 分清 CI 和 CD。CI：`main` 分支有合并时，由 GitHub 自动构建镜像并推送到镜像仓库。CD：分为源码部署和 Docker 部署，Docker 部署要有一个默认的 Compose 文件。
2. 项目只负责服务本身，在 Docker 中明确暴露哪个端口和端点；公网访问、证书不在本项目的范围内。
3. 源码部署是直接编译、直接运行程序，不是另一种 Docker 部署。

## 决策

1. **CI**：`.github/workflows/ci.yml`。每次 PR 和推送都运行检查：后端 clippy（不允许警告）、单元测试和集成测试（临时 PostgreSQL），前端类型检查、代码检查和测试。`main` 有合并或推送 `v*` 标签时，检查通过后构建 amd64 和 arm64 镜像（各在对应架构的机器上原生构建），合并为多架构镜像推送到 `ghcr.io/<所有者>/niulai`。
2. **CD：Docker 部署**（默认）：`deploy/compose.yml` 运行内置 PostgreSQL 18.6 和 niulai 两个容器，配置只有 `deploy/.env` 一个文件。`.env.example` 列出端口、数据库连接、同步参数、日志等全部常用配置，默认值可以直接使用，每台服务器按需修改。命令是 `cp .env.example .env`、`docker compose up -d`。
3. **CD：源码部署**：不用 Docker。在服务器上编译前端（`pnpm build`）和后端（`cargo build --release`），在 `backend/` 下直接运行 `niulai serve`，配置在 `backend/.env`，数据库自己准备。
4. **对外接口**：只暴露 niulai 的一个 HTTP 端口，默认 `127.0.0.1:8080`（`NIULAI_LISTEN_ADDR`、`NIULAI_PORT`）。页面在 `/`，接口在 `/api/v1/*`，健康检查是 `/api/v1/health`（存活）和 `/api/v1/ready`（能连上数据库）。项目中不包含 HTTPS、域名和登录的配置。
5. **不提供运维脚本**：升级、备份、恢复都是几条标准的 `docker compose` 命令，写在运维手册中。

## 取舍

- **去掉 Caddy 和内置登录**：部署只包含服务本身，更简单。代价是服务本身没有任何认证；默认只监听本机，防止误把端口暴露到网络上。
- **去掉脚本**：少了「升级前自动备份」「恢复时先恢复到临时库再互换」这类保护，改为在运维手册中写明顺序：先备份，再升级；恢复前先备份当前库。换来的是没有需要理解和维护的自定义脚本。
- **源码部署直接编译运行**：不依赖 Docker，适合想直接管理程序的环境。代价是要自己安装 Rust、Node.js 和 PostgreSQL，并用系统的进程管理（例如 systemd）负责常驻和重启。两种部署运行的是同一个程序。

## 后果

- 服务没有登录和访问控制；默认只监听 `127.0.0.1:8080`。
- 发布新版本：合并到 `main` 得到 `latest`；打 `v*` 标签得到固定版本号，服务器上建议用固定版本。
- 同一个 API Key 仍然不要同时用在两套部署上。
