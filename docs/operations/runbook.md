# 运维手册

本文是 niulai 的操作步骤：本地运行、同步操作、处理失败、部署、升级、备份和测试。命令默认在 `backend/` 下执行，本地开发时 `niulai` 写作 `cargo run --`。同步机制见 [同步流水线](../architecture/sync_pipeline.md)，环境变量见 [配置](../interfaces/configuration.md)。

## 章节导航

- [本地运行](#local_setup)：第一次在本机启动。
- [首次回补](#backfill)、[日常同步](#daily_sync)、[重拉指定数据](#rerun)、[查看进度](#monitoring)、[处理失败任务](#failure_handling)、[修改配置](#config_changes)：同步相关的操作。
- [用户与密码](#users)：添加用户、设管理员、停用，忘记密码。
- [部署](#deployment)：构建镜像、Docker 部署、探活与告警、监控指标与慢查询、源码部署、一键升级、部署迁移、资源限制与升级助手加固、数据目录、手动升级、备份与恢复。
- [测试](#testing)：提交代码前。

<a id="local_setup"></a>
## 本地运行

前提：Rust 1.99.0（`backend/rust-toolchain.toml` 自动选择）；PostgreSQL 16 或以上（本机开发用 dev-infra 的 `dev-postgresql`，库名 `niulai`）；前端需要 Node.js 24 和 pnpm 12。

1. `cp .env.example .env`，填写 `DATABASE_URL`。API Key 可以留空，启动后在网页上配置。
2. `niulai serve`：启动时执行迁移。预期日志出现「niulai 服务已启动」，`curl http://127.0.0.1:8080/api/v1/ready` 返回 `{"status":"ready"}`。
3. 前端（在 `frontend/` 下）：`pnpm install`，`pnpm dev`，打开 <http://localhost:5173/>。`/api` 转发到 `127.0.0.1:8080`，后端在别处时用 `NIULAI_API_URL=http://主机:端口 pnpm dev`。后端接口变更后按 [接口客户端](../architecture/frontend.md#api_client) 重新生成。

`niulai serve` 在一个进程里运行 API、任务执行和调度，`--no-api`、`--no-worker`、`--no-scheduler` 关闭其中一部分。同一时刻只能有一个进程执行任务：`serve` 运行时再执行 `niulai work` 会报错退出；反过来，`work` 运行时启动的 `serve` 先只提供 API，`work` 退出后自动接手执行任务。

<a id="backfill"></a>
## 首次回补

第一次在网页上配置 API Key 时自动规划 P0、P1 的全量回补，见 [首次登录引导](../interfaces/rest_api.md#setup)。历史测量约 31.8 万次请求（其中财务指标约 21.9 万次），在默认限速下曾约需 51 小时；实际耗时取决于数据集开关、供应商响应和失败重试。回补排在日常同步后面，不影响每天的数据。各数据集的请求量见 [数据目录](../interfaces/dataset_catalog.md#request_volume)。

Key 来自环境变量或需要重新回补时手动规划：

1. `niulai plan backfill --priority P0 --priority P1`，输出各数据集的任务数和第一个波次的 `run_id`。财务指标可以单独规划：`niulai plan backfill -d a_share.fin_indicator`。
2. `niulai serve` 在运行时自动执行；没有运行时用 `niulai work --until-idle`，队列空了就退出。
3. 同步页或 `niulai status` 看进度。最后一个波次为 `succeeded` 即完成；`partial` 时按 [处理失败任务](#failure_handling) 检查。

<a id="daily_sync"></a>
## 日常同步

`niulai serve` 按同步计划自动规划：默认每个交易日 18:00 同步每日的数据，每周六 08:00 同步每周的数据，之后自动核对和补齐缺口、每周做一次数据核检。时间在同步页的「同步计划」里改，规则见 [定时调度](../architecture/sync_pipeline.md#scheduling)。

手动触发：同步页的「立即同步全部」「手动同步」或某个计划的「现在同步一次」；命令行 `niulai daily`（只规划每日的数据集）、`niulai check`（数据核检）。收盘前或供应商还没出当天数据时触发，快照类任务会自动推迟到数据就绪。

<a id="rerun"></a>
## 重拉指定数据

- 某只证券的日线：`niulai plan backfill -d a_share.bar_1d --thscode 600519.SH`（可重复 `--thscode`）。
- 某个数据集：`niulai plan backfill -d special.limit_up_pool`。
- 网页：同步页的「手动同步」选「全部重取」，可以只填几只股票。

重复规划是安全的：同一分区已有未完成任务时不会重复创建，写入都是 UPSERT。重拉日线或公司行动后运行结束时自动重算复权；需要立即重算用 `niulai recompute`（本机全市场约 2 秒，见 [运行结束后的计算](../architecture/sync_pipeline.md#postprocess)）。

<a id="monitoring"></a>
## 查看进度

- 同步页 `/sync`：状态、同步计划、同步记录、需要注意的缺口和失败、数据核检；点一条同步记录看各数据集进度和失败任务。
- 命令行和接口：`niulai status`、`GET /api/v1/sync/runs`、`GET /api/v1/sync/runs/{run_id}`、`GET /api/v1/sync/health`。

常用 SQL：

```sql
-- 最近 10 分钟的请求速率和失败数
SELECT count(*) AS calls, round(count(*) / 600.0, 2) AS rps, count(*) FILTER (WHERE api_code IS DISTINCT FROM 0) AS non_ok
FROM ops.api_call WHERE called_at > now() - interval '10 minutes';

-- 按错误汇总还没解决的失败任务
SELECT dataset_id, last_api_code, left(last_error, 80) AS error, count(*)
FROM ops.task WHERE status = 'dead' GROUP BY 1, 2, 3 ORDER BY 4 DESC;
```

<a id="failure_handling"></a>
## 处理失败任务

`dead` 任务不会自动重试（兜底补偿会把供应商暂时不可用造成的失败再排一次）。同步页「需要注意」按数据集和错误分组，附白话原因。常见的：

| 错误 | 原因 | 处理 |
| --- | --- | --- |
| 已错过 | 只有当天值的数据，供应商已经到了下一个交易日 | 补不回来；有 `recover_with` 的由兜底补偿补 |
| 1002（提示不是交易日）、1003 `date must be within one year` | 日期是节假日，或排队期间超出一年 | 执行前会自动跳过；已经失败的点「立即补齐」改为跳过，见 [过期的日期](../architecture/sync_pipeline.md#stale_dates) |
| 1002 `Unknown thscode` | 证券已退市，或接口不支持这类证券 | 确认后忽略；一类证券都不支持时在 `universe` 或 `filter` 中排除 |
| 1003 其他 | 窗口过长、分页过大 | 调整 `window_days`、`paging.size` |
| 2001、2003 | API Key 无效或无权限 | 在 <https://fuyao.aicubes.cn/admin/> 确认后在「设置」中更换；来自环境变量时改 `HITHINK_FINANCE_API_KEY` 后重启 |
| 2004 | 同花顺 AI 客户端专用接口 | 关闭该数据集 |
| 模板变量不存在 | `key`、`thscode` 或 `obs_date` 引用了响应中没有的字段 | 修改 `datasets.yaml` |
| 可重试错误用尽 | 供应商暂时不可用或限流 | 点「立即补齐」或等每天的自动补偿 |

重新执行失败的分区：对同一数据集重新规划即可。确认不需要执行的分区可以手动标为跳过，运行就不会因此记为 `partial`：

```sql
UPDATE ops.task SET status = 'skipped', last_error = '超出接口支持的历史范围'
WHERE dataset_id = 'special.limit_down_pool' AND status IN ('pending', 'failed', 'dead');
```

执行者异常退出时，`running` 的任务 10 分钟后自动放回队列。

<a id="config_changes"></a>
## 修改配置

修改 `backend/config/datasets.yaml` 后先 `cargo test` 确认能解析，再重启 `niulai serve`（Ctrl+C 或 SIGTERM 后等正在执行的任务结束）。启动时新配置同步到 `ops.dataset`。已规划的任务按规划时的参数执行。

<a id="users"></a>
## 用户与密码

第一次启动时，第一个管理员的用户名和密码来自 `NIULAI_ADMIN_USER`、`NIULAI_ADMIN_PASSWORD`（默认都是 `niulai`）。之后这两个变量不再起作用：

1. 用这个账号登录，点右上角的用户名，在「我的」页面改密码。
2. 在「用户」页添加其他人：不开放注册，用户都由管理员添加。不填密码时系统生成一个，只显示一次，转交本人；本人登录后会被提醒改成自己的密码。新用户默认是普通用户，需要时再「设为管理员」。
3. 不再使用的账号点「停用」：不能登录，已登录的立即退出，数据保留。

忘记密码：请另一个管理员在「用户」页重置。管理员自己也忘了时，在服务器上执行（停用的用户同时恢复）：

```bash
docker compose exec niulai niulai user reset-password <用户名>   # Docker 部署，在部署目录执行
./target/release/niulai user reset-password <用户名>             # 源码部署，在 backend/ 下执行
```

命令打印新密码，登录后立即修改。`niulai user list` 列出全部用户。

<a id="deployment"></a>
## 部署

CI 在 GitHub 上检查代码、发布时构建镜像。服务有两种运行方式，运行的是同一个程序：
- **Docker 部署**（默认）：CI 构建的镜像，`deploy/compose.yml` 运行内置 PostgreSQL、Redis、niulai 和一键升级助手，配置在 `deploy/.env`。Redis 只用于页面读取缓存，删除或重启不会影响数据库数据。
- **源码部署**：在服务器上编译，直接运行 `niulai`，配置在 `backend/.env`，数据库自己准备。

对外只有一个 HTTP 端口，默认 `127.0.0.1:8080`：`/` 是网页，`/api/v1/*` 是 [REST API](../interfaces/rest_api.md)，`/api/v1/health` 是存活检查、`/api/v1/ready` 在能连上数据库时返回 200，这两个检查不需要登录。打开网页先要登录（默认 `niulai` / `niulai`，部署后先改掉），然后按引导配置 API Key。取舍见 [ADR：CI 构建镜像，部署只交付服务本身](../decisions/ci_cd.md)。

<a id="release_image"></a>
### 构建镜像与发布

PR 和推送到 `main` 只做检查（后端 clippy、单元和集成测试，前端类型检查、代码检查和测试）。发布版本：在 `main` 上、改动已推送且 CI 通过后，按今天的上海日期打带说明的标签并推送：

```bash
git tag -l "v$(TZ=Asia/Shanghai date +%Y.%m.%d)*"            # 今天已经发过的版本
git tag -a v2026.10.18 -m "niulai 2026.10.18: 一句话说明"    # 今天发过时用下一个补丁号，例如 v2026.10.18.1
git push origin v2026.10.18
```

检查通过后构建 amd64 和 arm64 镜像，推送为 `ghcr.io/duoyuli/niulai:2026.10.18`、`latest` 和 `sha-<提交>`；格式不对的 `v*` 标签不构建。预期 `docker buildx imagetools inspect ghcr.io/duoyuli/niulai:2026.10.18` 列出两种架构，镜像公开，拉取不需要登录。规则见 [ADR：版本号按发布日期定](../decisions/calendar_versioning.md) 和 [ADR：只在发布版本标签时构建镜像](../decisions/image_on_tag.md)。

<a id="server_install"></a>
### Docker 部署

前提：Docker Engine 和 Compose 插件 2.20 或以上；内存 3 GB 以上（`compose.yml` 的资源上限按 4 vCPU、3.6 GiB 的服务器设置，更小的机器先调低上限，见 [资源限制](#resource_limits)）；部署目录所在的磁盘 20 GB 以上（全量数据约 4 GB，之后持续增长）。

1. 把 `deploy/compose.yml` 和 `deploy/.env.example` 放到服务器的一个目录，例如 `/opt/niulai/`。
2. `cp .env.example .env`，默认值可以直接用，按需改端口、镜像版本、管理员密码，见 [部署配置](../interfaces/configuration.md#deploy_env)。
3. `docker compose up -d`。预期 `docker compose ps` 中 `niulai` 为 `healthy`，`curl http://127.0.0.1:8080/api/v1/ready` 返回 ready。
4. 打开网页，登录后按引导配置 API Key。

常用命令（在部署目录执行）：`docker compose ps`、`docker compose logs -f niulai`、`docker compose exec niulai niulai status`（其他子命令同理）。停机用 `docker compose down`，niulai 等正在执行的任务结束（最多 2 分钟），未完成的任务下次启动继续。数据库数据在部署目录的 `data/postgres`，加 `-v` 也不会删掉，见 [数据目录](#data_dir)。

使用本机构建的镜像：在仓库根目录 `docker build -f deploy/Dockerfile -t niulai:local .`，`.env` 中 `NIULAI_IMAGE=niulai:local`，再 `docker compose up -d`。

<a id="health_monitoring"></a>
### 探活与告警

`GET /api/v1/health` 不需要登录（免登录的只有 `/health`、`/ready`、`/auth/login`、`/auth/logout` 和接口规范本身，见 [哪些接口要登录](../interfaces/rest_api.md#auth)），返回 `{"status":"ok"}`。它只说明进程还在，不查数据库；`GET /api/v1/ready` 也不需要登录，能连上数据库才返回 200，用来区分「服务停了」和「数据库有问题」。

用部署在服务器之外的监控服务定期请求对外地址，失败时发通知。例如每分钟请求 `https://niulai.liduoyu.site/api/v1/health`，连续 3 次超时或返回非 200 就告警：

- 从服务器之外请求，才能发现对外地址失效、机房网络不通这类问题；在服务器上用本机地址探测发现不了它们。
- 用「连续 3 次失败再报」：一键升级会停 niulai 一两分钟，单次失败是正常的；恢复也要等一次成功确认，不因为短暂恢复就撤销告警。
- 同时监控 `/api/v1/ready`：进程活着但数据库连不上时 `/health` 仍是 200，只看它会把「网页打得开但什么都查不到」漏掉。
- 探活只需要这一个地址，不需要账号；监控服务本身的配置不放在本仓库里。

<a id="metrics"></a>
### 监控指标

把环境变量 `NIULAI_METRICS_TOKEN` 设成一个足够长的随机字符串（例如 `openssl rand -hex 24`），重启 niulai 后 `GET /api/v1/metrics` 返回 Prometheus 文本格式的指标；不设置这个变量时接口返回 404，等于关闭。请求要带 `Authorization: Bearer <令牌>`（Prometheus 的 `authorization.credentials`），令牌不对返回 401。不接受写在网址里的 `?token=`：请求日志会原样记下网址。令牌在启动时读一次，改了要重启；它只用于这个接口，不是登录密码。

指标只有进程和队列的聚合数字，不含证券、账户等业务数据：

| 指标 | 含义 |
| --- | --- |
| `niulai_up`、`niulai_info`、`niulai_uptime_seconds` | 进程在运行、运行的版本、已运行的秒数 |
| `niulai_http_requests_total`、`niulai_http_errors_total` | 处理过的 API 请求数、其中返回 5xx 的 |
| `niulai_http_request_duration_seconds_sum` / `_count` | 请求累计耗时和次数，除一下就是平均耗时 |
| `niulai_sync_pending_tasks` | 等待执行的任务数（含等待重试的）；长期不为 0 说明同步卡住了 |
| `niulai_db_pool_connections`、`niulai_db_pool_idle` | 连接池的连接数和空闲数 |

Prometheus 的抓取配置由使用方保存，不放在本仓库；例如 `scrape_configs: [{job_name: niulai, scheme: https, metrics_path: /api/v1/metrics, authorization: {credentials: <令牌>}, static_configs: [{targets: ["niulai.example.com"]}]}]`。没有 Prometheus 时用 `curl -H "Authorization: Bearer $NIULAI_METRICS_TOKEN" https://niulai.example.com/api/v1/metrics` 也能看。

**慢查询：** SQLx 默认会把执行超过 1 秒的语句以 `WARN slow statement` 记到日志里，`elapsed_secs` 是耗时，`db.statement` 是完整 SQL。`docker compose logs niulai | grep "slow statement"` 可以找出慢查询；长时间同步、首次回补时的全市场读取偶尔超过 1 秒是正常的，反复出现或明显变慢才需要排查（缺少索引、数据量变大）。

<a id="source_deploy"></a>
### 源码部署

前提：rustup（版本由 `backend/rust-toolchain.toml` 固定）；Node.js 24 和 pnpm 12（`corepack enable`）；PostgreSQL 16 或以上的空库；编译约需 4 GB 内存，能访问 npm 和 crates.io。

1. `git clone https://github.com/duoyuli/niulai.git && cd niulai`
2. `cd frontend && pnpm install --frozen-lockfile && pnpm build`
3. `cd ../backend && cargo build --release --locked`，产物 `target/release/niulai`，迁移已编译在里面。
4. `cp .env.example .env`，填写 `DATABASE_URL`，设置 `NIULAI_WEB_DIR=../frontend/dist`。
5. 在 `backend/` 下运行 `./target/release/niulai serve`，然后打开网页配置 API Key。

程序要在 `backend/` 下运行（`.env`、`config/datasets.yaml`、`NIULAI_WEB_DIR` 都按当前目录查找）。长期运行交给 systemd 等进程管理：工作目录 `backend/`，异常退出时重启，停止时发 SIGTERM 并留 2 分钟。更新：`git pull`，重做第 2、3 步，重启。

<a id="one_click_upgrade"></a>
### 一键升级（Docker 部署）

niulai 每 6 小时查一次 GitHub 上的版本标签（`NIULAI_UPDATE_REPO`，默认 `duoyuli/niulai`），并确认镜像已经能从 GHCR 拉取（推送标签后要等 CI 构建完），有新版本时页面左上角的版本号亮红点，点开即可升级。版本号是 `vYYYY.MM.DD`，同一天的补丁是 `vYYYY.MM.DD.N`，按发布先后比较，其他写法的标签不认。

执行升级的是 `upgrader` 容器（与 niulai 同一个镜像，运行 `niulai upgrader`，由 `.env` 的 `COMPOSE_PROFILES=upgrader` 启用），它通过 `/var/run/docker.sock` 调用 Docker Engine API。niulai 和它通过卷 `upgrade`（`NIULAI_UPGRADE_DIR=/upgrade`）交换文件：niulai 写 `request`（只有版本号）；upgrader 每 3 秒写 `heartbeat`（30 秒没更新时页面显示手动升级的命令），把进度写在 `status`、输出写在 `log`。

upgrader 收到请求后依次执行，每一步的状态和说明写在 `status` 的 `steps` 中，版本号弹窗里显示进度条和步骤（收起后进度保留）：
1. **下载新版本**，显示百分比和 MB。失败且本机没有这个镜像时什么都不改。
2. **更新升级助手**：用新镜像重建自己，进度写在 `handoff` 交给新的 upgrader 继续。新版本的部署迁移只有新版本知道，所以先换掉自己；换不成时由当前的继续。
3. **备份数据库**：用内置数据库时在 postgres 容器中 `pg_dump` 并校验，写到 `backups/niulai-before-<版本>-<时间>.dump`（保留最近 3 份，见 [备份与恢复](#backup_restore)）。失败时不升级。
4. **部署迁移**：做完新版本带来的、还没做过的 [部署迁移](#deploy_migrations)。失败时恢复原样，不升级。
5. **启动新版本**：`.env` 的 `NIULAI_IMAGE` 改成新版本，停止旧容器（最多 2 分钟），按它的配置用新镜像建新容器，最多等 5 分钟变成 `healthy`（页面显示已等多久），成功后删掉旧容器；失败时换回旧容器并改回 `.env`。新版本可能已经执行了数据库迁移，旧版本起不来时用第 3 步的备份恢复。

所有步骤结束、运行的版本变成目标版本后页面自动刷新；niulai 停机期间页面打不开，重新连上后接着显示进度。`status` 10 分钟没有更新时页面提示查看 `docker compose logs upgrader`。2026-10-17 之前的 upgrader 不写步骤，按旧顺序先升级 niulai 再换掉自己，新的 upgrader 启动后补做部署迁移。

限制：`NIULAI_IMAGE` 要是发布的镜像（`ghcr.io/<所有者>/niulai:...`），`niulai:local` 和开发版不能一键升级。upgrader 能管理这台机器上的所有容器，niulai 只能提交一个版本号；不需要时删掉 `COMPOSE_PROFILES=upgrader`，再 `docker compose up -d --remove-orphans`。设计见 [ADR：Docker 部署的一键升级](../decisions/one_click_upgrade.md) 和 [ADR：部署迁移与升级进度](../decisions/deploy_migrations.md)。

<a id="deploy_migrations"></a>
### 部署迁移

新版本对部署方式（`compose.yml`、`.env`、容器挂载）有不兼容的改动时，写成部署迁移（`backend/src/upgrader/migrate.rs`），由新版本的 upgrader 在启动新版本之前做完：一键升级时是第 4 步，upgrader 启动时也检查一遍还没做过的。

- 每个迁移只做一次，结果记在部署目录的 `.niulai-migrations`（编号、`done` 或 `skipped`、时间、说明）；失败的不记，下次再试。
- 先检查前提，不满足就跳过并写明原因；做到一半失败时恢复原来的容器、`compose.yml` 和 `.env`。
- 没有启用 upgrader 时按下面的手动步骤做。

| 编号 | 内容 |
| --- | --- |
| `2026-10-17-postgres-data-dir` | 数据库数据从 Docker 卷 `pgdata` 移到部署目录的 `data/postgres`，见 [数据目录](#data_dir) |

<a id="resource_limits"></a>
### 资源限制与升级助手加固

2026-10-18 起的 `compose.yml` 给三个服务加了内存和 CPU 上限，给 upgrader 加了安全选项（`no-new-privileges`，放弃全部 capability 后只保留 `CHOWN` 和 `DAC_OVERRIDE`）和健康检查，数值的依据写在文件注释里。这两项没有写成部署迁移，原因：

- 升级助手重建容器时沿用原容器的启动参数（`backend/src/upgrader/mod.rs` 的 `create_body`），只改服务器上的 `compose.yml`，正在运行的容器不会变，要 `docker compose up -d` 重建才生效；由迁移改文件会让人误以为已经生效。
- 内存和 CPU 上限取决于服务器大小，猜错会让数据库或 niulai 被强制杀掉并重启，不适合自动做。

已有部署按下面手动更新（先 [备份](#backup_restore)）：

1. 在服务器上编辑部署目录的 `compose.yml`，参照仓库中的新文件给 `postgres`、`niulai` 加 `mem_limit`、`cpus`，给 `upgrader` 加 `mem_limit`、`cpus`、`security_opt`、`cap_drop`、`cap_add` 和 `healthcheck`。只加这些行，保留自己改过的端口、镜像、挂载等；不要整个替换文件。
2. 在部署目录执行 `docker compose up -d`。参数变了的服务会被重建：数据库和 niulai 会短暂重启（几秒到几十秒），正在执行的同步任务在下次启动后继续，不会丢数据。
3. 验证：`docker compose ps` 中 `upgrader` 为 `healthy`；`docker inspect -f '{{.HostConfig.Memory}} {{.HostConfig.NanoCpus}} {{.HostConfig.CapDrop}} {{.HostConfig.SecurityOpt}}' "$(docker compose ps -q upgrader)"` 显示 `1073741824 500000000 [ALL] [no-new-privileges:true]`。之后一键升级重建容器时会沿用这些参数，不用再改。

只想先加固升级助手、不重启数据库和 niulai 时，执行 `docker compose up -d upgrader`。服务器内存小于 3 GB 时按比例调低上限，例如 `postgres` 1024m、`niulai` 800m，但 `upgrader` 不要低于 512m；大于 4 GiB 时可以观察 `docker stats` 的峰值后适当调高。

**升级助手的内存为什么要留大：** 备份在 postgres 容器里 `pg_dump` 后，会经 Docker 控制接口流式写到部署目录的 `backups/`。写盘产生的脏页会计入 upgrader 容器的内存上限，数据约 650 MB 时，128 MB 的限制会在备份中途把进程 OOM 杀掉（2026-10-18 发布 v2026.10.18.9 时实际发生：pg_dump 写完 `.tmp` 文件、进程被杀、升级停在「备份数据库」，niulai 和数据库本身没有受影响）。现在的版本每写 32 MB 就 sync 一次，把脏页压低，`compose.yml` 里仍留 1 GB 上限；**如果某个部署的 upgrader 在升级或每日备份时卡在「pg_dump」、容器重启过，先看 `docker inspect` 和 `dmesg` 有没有 OOM，并把 `mem_limit` 调大。** 升级卡住后恢复：`docker compose up -d upgrader` 换个内存够的配置，删掉 `backups/` 下残留的 `*.tmp` 和共享卷里的 `status`（`docker compose exec upgrader rm -f /upgrade/status`），再重新点一键升级；数据库和正在运行的服务不需要动。

<a id="data_dir"></a>
### 数据目录

内置数据库的数据在部署目录的 `data/postgres`（`.env` 的 `POSTGRES_DATA`，默认 `./data/postgres`），是宿主机上的普通目录：删除容器、`docker compose down -v`、`docker volume prune` 都不影响，备份和搬迁时直接看得到。不要删除它和旁边的 `backups/`。

2026-10-17 之前的部署，数据在 Docker 卷 `<项目名>_pgdata`（通常是 `niulai_pgdata`）中，清理卷的命令会把它删掉。升级到之后的版本时，部署迁移 `2026-10-17-postgres-data-dir` 自动搬过来：
1. 检查：用内置数据库、数据库挂载的是卷、`compose.yml` 中还是原来的 `volumes: [pgdata:/var/lib/postgresql]`、`data/postgres` 是空的，不满足就跳过。
2. 备份数据库，停止 niulai 和数据库。
3. 用数据库镜像起临时容器：先确认磁盘放得下一整份数据再多 1 GB（不够时什么都不改，提示需要多少），再 `cp -a` 原样复制（含属主和权限），核对文件数和总大小。
4. `compose.yml` 那一行改成 `volumes: ["${POSTGRES_DATA:-./data/postgres}:/var/lib/postgresql"]`，`.env` 写 `POSTGRES_DATA=./data/postgres`。
5. 用新挂载重建数据库容器，等它 `healthy`，再启动 niulai。数据库起不来时换回原来的容器和配置，复制出来的目录改名为 `data/postgres.failed-<时间>`。

原来的卷保留不删，确认数据没问题后可以 `docker volume rm niulai_pgdata`。预期 `.niulai-migrations` 中有 `2026-10-17-postgres-data-dir done`，`data/postgres/18` 存在。

没有启用一键升级时手动迁移（先 [备份](#backup_restore)）：

```bash
docker compose stop niulai postgres
mkdir -p data/postgres
docker run --rm -v niulai_pgdata:/from:ro -v "$PWD/data/postgres":/to postgres:18.6 sh -c 'cp -a /from/. /to/'
# 换成新版本的 compose.yml（或按上面第 4 步改那一行），.env 中加 POSTGRES_DATA=./data/postgres
docker compose up -d
```

不要没迁移就换上新的 `compose.yml`，否则数据库会从空的开始（原来的数据还在卷里，按上面的步骤迁移即可）。

<a id="upgrade"></a>
### 手动升级与回滚

1. 先 [备份](#backup_restore)。
2. Docker 部署：`.env` 的 `NIULAI_IMAGE` 改成新版本，`docker compose pull && docker compose up -d`。源码部署：`git pull`，重新编译，重启。
3. 新版本启动时自动执行数据库迁移，预期 `niulai` 恢复为 `healthy`。

数据库迁移只能向前，回滚要同时回滚数据库：`NIULAI_IMAGE` 改回旧版本，再用升级前的备份恢复。

<a id="backup_restore"></a>
### 备份与恢复

**自动备份：** 启用了一键升级（upgrader）、用内置数据库时，升级助手每天上海时间 3:30 之后备份一次（服务那时没在运行就在启动后补做；超过 30 小时没有备份时不等到点），写到部署目录的 `backups/niulai-daily-<时间>.dump`，保留最近 7 份；升级前的备份 `niulai-before-<版本>-<时间>.dump` 保留最近 3 份（每份大约是数据目录的十分之一，留意磁盘空间）。每份都在 postgres 容器中先导出、用 `pg_restore --list` 读一遍，读得通才写进 `backups/`，所以那里只有完整的备份。失败一小时后重试。版本号弹窗显示最近一次备份；超过 36 小时没有成功时，管理员的每个页面顶部都有提醒。实现在 `backend/src/upgrader/backup.rs`。

**异地副本：** 备份和数据库在同一块磁盘上，防不了坏盘、误删部署目录、重装系统。在宿主机上用 crontab 把 `backups/` 同步到另一台机器或对象存储，例如：

```bash
# 每天 5 点把 backups/ 同步到对象存储（先用 rclone config 配置好名为 remote 的存储）
0 5 * * * rclone sync /opt/niulai/backups remote:niulai-backups --include 'niulai-*.dump'
```

**手动备份与恢复：** 在部署目录执行（源码部署对 `DATABASE_URL` 指向的库执行同样的 `pg_dump`、`psql`、`pg_restore`；源码部署没有自动备份，用 crontab 定时执行下面的备份命令）。

```bash
# 备份（自定义格式，已压缩）
mkdir -p backups
docker compose exec -T postgres pg_dump -U niulai -Fc niulai > backups/niulai-$(date +%Y%m%d-%H%M%S).dump

# 恢复：会覆盖当前数据库，先备份当前库
docker compose stop niulai
docker compose exec -T postgres psql -U niulai -d postgres -c 'DROP DATABASE niulai WITH (FORCE)' -c 'CREATE DATABASE niulai'
docker compose exec -T postgres pg_restore -U niulai -d niulai --no-owner < backups/<文件>.dump
docker compose start niulai
```

恢复之前可以先恢复到一个临时库（`CREATE DATABASE niulai_check`，`pg_restore -d niulai_check`）看看数据对不对，再恢复到 `niulai`。不要用 `pg_restore --clean`：`raw.record`、`ops.api_call` 是分区表，会失败。迁到另一台服务器时，在新服务器完成 Docker 部署的第 1 至 3 步，不配置 Key，直接恢复；备份里有 Key 和回补记录，不会重复回补。

<a id="testing"></a>
## 测试

在 `backend/` 下：
- `cargo test`：单元测试，不需要数据库。
- `NIULAI_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/postgres cargo test`：加上集成测试。连接串要有建库权限，每个测试建一个临时库，结束时删除；不设置时跳过集成测试。
- `cargo clippy --all-targets`：提交前应没有警告。
- `cargo test --lib -- --ignored real_docker_data_dir_migration`：用本机 Docker 真实走一遍数据目录迁移（约 1 分钟），改 `backend/src/upgrader/` 时运行。

在 `frontend/` 下：`pnpm typecheck`、`pnpm lint`、`pnpm test`，都应通过。

集成测试覆盖迁移、写入幂等、复权、任务去重与超时回收、运行结束判定、交易日校验与过期日期、完整性核对与补偿、数据核检、同步计划、定期清理、登录、一键升级的文件交换和主要 API，不调用供应商接口。
