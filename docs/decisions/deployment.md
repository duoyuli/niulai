# ADR：部署方案（已被取代）

- 状态：已被取代（superseded），2026-10-16。由 [ADR：CI 构建镜像，部署只交付服务本身](ci_cd.md) 取代：去掉了 Caddy、基本认证登录和运维脚本，项目只交付服务本身。此前决策第 4、5 条的一部分已被 [ADR：API Key 在首次登录引导中配置](api_key_setup.md) 取代。
- 相关文档：[部署](../operations/runbook.md#deployment)、[部署配置](../interfaces/configuration.md#deploy_env)、[ADR：单进程服务](single_process_service.md)、[ADR：限速在进程内](in_process_rate_limit.md)。

## 背景

用户要求项目能用 Docker 一键部署到任意服务器上：镜像推送到 GitHub，在服务器上用 Docker Compose 启动；服务通过公网域名访问，需要登录；新服务器从零回补数据。

此前的部署只适合开发机：镜像在服务器上从源码构建，编译 Rust release 版本需要约 4 GB 内存；PostgreSQL 要另外准备；服务只监听本机，而程序本身没有用户体系；没有备份和升级流程。开发机是 arm64，云服务器大多是 amd64。

## 决策

1. **镜像**：GitHub Actions 在推送 `main` 和 `v*` 标签时构建镜像，推送到 GHCR（`ghcr.io/<所有者>/niulai`）。amd64 和 arm64 各在对应架构的构建机上原生构建，再合并为一个多架构镜像。服务器只拉取镜像，不需要源码和编译环境。
2. **编排**：一个 `deploy/compose.yml` 包含内置 PostgreSQL 18.6、niulai 和 Caddy，后两个可选服务用 Compose profile（`db`、`proxy`）开关。已有数据库的服务器去掉 `db`；不对公网开放的服务器去掉 `proxy`，用 SSH 隧道访问。niulai 和 PostgreSQL 都不对外暴露端口。
3. **登录**：由 Caddy 的 HTTP 基本认证完成，覆盖全部页面和接口；Caddy 同时自动申请和续期 HTTPS 证书。密码只保存 bcrypt 哈希（`deploy/caddy/auth.caddy`），不写进 `.env`。
4. **配置**：部署相关的全部配置集中在 `deploy/.env` 一个文件中。`install.sh` 首次运行时询问镜像、API Key 和域名，随机生成数据库密码和登录密码。
5. **运维脚本**：`install.sh`（可重复执行，首次安装时规划全量回补）、`upgrade.sh`（先备份再升级）、`backup.sh`（pg_dump，保留最近 7 份）、`restore.sh`（先恢复到临时库再互换，失败时当前库不变）。

## 取舍

- **GHCR 与其他镜像仓库**：代码放在 GitHub 上，GHCR 与 Actions 集成，不需要额外的账号和密钥。代价是国内服务器访问 GHCR 可能较慢或不稳定，需要时可以在工作流中再推送一份到国内镜像仓库，或用 `docker save` 导出离线包。
- **原生多架构构建与 QEMU 模拟**：在 amd64 机器上模拟 arm64 编译 Rust 要慢很多倍。代价是工作流多一个合并步骤；私有仓库用不了 arm64 构建机时，只发布 amd64。
- **基本认证与程序内登录**：基本认证不需要改程序，适合少数几个人共用。它的限制是：只有一组共享账号，没有退出登录，浏览器弹出原生登录框，也区分不了谁做了什么操作。需要多用户、权限或审计时，再在程序内实现登录，或换用 oauth2-proxy 这类认证代理。
- **内置 PostgreSQL 与外部数据库**：内置数据库让一台空服务器一条命令就能跑起来。代价是数据库与应用在同一台机器上，备份由 `backup.sh` 负责，没有托管数据库的高可用和自动备份；需要时改用外部数据库，只改 `.env`。
- **恢复时互换数据库与 `pg_restore --clean`**：`--clean` 在分区表（`raw.record`、`ops.api_call`）上会失败，并且失败时会让服务处于停止状态。恢复到临时库再改名，需要建库权限和一份额外的磁盘空间，换来任何一步失败都不影响当前库。

## 验证（2026-10-16）

在本机用独立的 Compose 项目模拟一台新服务器：`install.sh -y` 生成配置并启动三个服务，迁移在全新的 PostgreSQL 18.6 上执行成功；不带凭据和凭据错误时返回 401，正确凭据访问页面和接口返回 200，HTTP 跳转到 HTTPS；备份后恢复，数据回到备份时的状态；用损坏的文件恢复时报错退出，当前库和服务不受影响；删除凭据文件后重新安装，旧密码失效、新密码生效。GitHub Actions 工作流通过 actionlint 检查，尚未在 GitHub 上实际运行。

## 后果

- 每台服务器是一套独立的部署：自己的数据库、配置和备份。同一个 API Key 不要同时在两台服务器上执行同步，数据库锁只能防止同一个库上的重复执行。
- 新服务器从零回补约需 51 小时；需要立即可用时，从另一台服务器的备份恢复。
- 发布新版本要打 `v*` 标签，服务器上改 `NIULAI_IMAGE` 后执行 `upgrade.sh`；迁移只能向前，回滚要同时恢复数据库。
