# 运维手册

本文说明 niulai 的日常操作：本地运行、首次回补、每日增量、重拉指定证券、查看进度、处理失败任务、部署和测试。除非另有说明，命令都在 `backend/` 目录下执行；本地开发时，`niulai` 写作 `cargo run --`。同步机制见 [同步流水线](../architecture/sync_pipeline.md)，环境变量见 [配置](../interfaces/configuration.md)。

## 章节导航

- [本地运行](#local_setup)：第一次在本机启动时阅读。
- [首次回补](#backfill)：新建数据库后拉取历史数据时阅读。
- [每日增量](#daily_sync)：确认或手动触发每日同步时阅读。
- [重拉指定数据](#rerun)：某只证券或某个数据集的数据需要重新拉取时阅读。
- [查看进度](#monitoring)：确认同步是否正常时阅读。
- [处理失败任务](#failure_handling)：出现 `dead` 任务时阅读。
- [修改配置](#config_changes)：修改 `datasets.yaml` 或环境变量时阅读。
- [部署](#deployment)：CI 构建镜像，Docker 部署或源码部署（直接编译运行），升级与回滚，备份与恢复时阅读。
- [测试](#testing)：提交代码前阅读。

<a id="local_setup"></a>
## 本地运行

前提条件：
- 已安装 Rust 1.99.0（`backend/rust-toolchain.toml` 会自动选择）。
- PostgreSQL 16 或以上可以连接。本机开发使用 dev-infra 中的 `dev-postgresql`（PostgreSQL 17），数据库名为 `niulai`。

步骤：
1. 复制配置模板：`cp .env.example .env`。
2. 在 `.env` 中填写 `DATABASE_URL`。`HITHINK_FINANCE_API_KEY` 可以填，也可以留空，启动后在网页的引导页中配置。
3. 执行迁移：`niulai migrate`。预期输出「迁移完成」。
4. 启动服务：`niulai serve`。预期日志出现「niulai 服务已启动」，`curl http://127.0.0.1:8080/api/v1/ready` 返回 `{"status":"ready"}`。

`niulai serve` 在一个进程中运行 API、同步任务执行和定时调度。只需要其中一部分时，用 `--no-api`、`--no-worker`、`--no-scheduler` 关闭其他部分。同一时刻只能有一个进程执行同步任务：`serve` 运行时再执行 `niulai work` 会报错退出，需要先停止 `serve` 或让它以 `--no-worker` 运行。

前端开发（在 `frontend/` 下执行，需要 Node.js 24 和 pnpm 12）：
1. 安装依赖：`pnpm install`。
2. 启动开发服务器：`pnpm dev`，打开 <http://localhost:5173/>。`/api` 请求转发到 `http://127.0.0.1:8080`；后端在别处时，用 `NIULAI_API_URL=http://主机:端口 pnpm dev`。
3. 后端接口变更后，按 [接口客户端](../architecture/frontend.md#api_client) 重新生成客户端。

<a id="backfill"></a>
## 首次回补

前提条件：数据库已迁移；`niulai serve` 正在运行，或准备用 `niulai work` 在前台执行。同一时刻只能有一个进程执行同步任务，`serve` 的执行循环在运行时 `niulai work` 会报错退出。

警告：P0 加 P1 的全量回补约 31.8 万次请求（其中财务指标约 21.9 万次），按默认限速约需 51 小时；不含财务指标约 10 万次、16 小时。每日增量任务优先执行，回补期间日常同步照常进行。各数据集的请求量见 [数据目录](../interfaces/dataset_catalog.md#request_volume)。

第一次在网页上配置 API Key 时会自动规划这一步（数据库里还没有回补运行时），见 [首次登录引导](../interfaces/rest_api.md#setup)。以下手动步骤用于 Key 来自环境变量、或需要重新回补的情况。

步骤：
1. 规划回补：`niulai plan backfill --priority P0 --priority P1`。预期输出各数据集的任务数和第一个波次的 `run_id`。
2. 执行任务。二选一：
   - `niulai serve` 已在运行时，执行循环会自动领取任务，不需要操作。
   - 只想执行完就退出时，运行 `niulai work --until-idle`。队列中没有未完成任务时，程序退出。
3. 查看进度：`niulai status`，或 `GET /api/v1/sync/runs`。第一个波次结束后，会自动出现下一个波次的运行。
4. 确认完成：最后一个波次的运行状态为 `succeeded`。如果状态为 `partial`，按 [处理失败任务](#failure_handling) 检查。

当天已经做过全量回补时，当天的每日增量没有必要再执行。手动把调度标记为已触发，可以避免重复拉取：

```sql
INSERT INTO ops.scheduler (job, last_fired) VALUES ('after_close', current_date), ('weekly', current_date), ('monthly', current_date)
ON CONFLICT (job) DO UPDATE SET last_fired = EXCLUDED.last_fired, fired_at = now();
```

财务指标 `a_share.fin_indicator` 请求量很大（只取年报也约 6.7 万次），可以单独规划：`niulai plan backfill -d a_share.fin_indicator`。

<a id="daily_sync"></a>
## 每日增量

`niulai serve` 在每个交易日 18:00（`NIULAI_SCHEDULE_AFTER_CLOSE`）之后自动规划一次增量，不需要人工操作。触发条件见 [定时调度](../architecture/sync_pipeline.md#scheduling)。

手动触发：`niulai daily`。它会立即规划一次增量，选择的数据集与调度器相同，但不包括每周和每月的数据集。在当天 15:30 之前或供应商尚未出当天数据时触发，快照类任务会自动推迟到数据就绪，不会写入错误日期的数据。

确认当天已同步：

```sql
SELECT run_id, status, tasks_total, tasks_done, tasks_failed, started_at FROM ops.run WHERE kind = 'incremental' ORDER BY run_id DESC LIMIT 5;
SELECT max(trade_date) FROM mkt.stock_bar_1d;
```

<a id="rerun"></a>
## 重拉指定数据

- 重拉某只证券的日线：`niulai plan backfill -d a_share.bar_1d --thscode 600519.SH`。可以重复 `--thscode` 指定多只。
- 重拉某个数据集：`niulai plan backfill -d special.limit_up_pool`。
- 通过 API：`POST /api/v1/sync/runs`，请求体 `{"kind": "backfill", "datasets": ["a_share.bar_1d"], "thscodes": ["600519.SH"]}`。

重复规划是安全的：同一分区已有未完成任务时，不会重复创建；写入都是 UPSERT。重拉日线或公司行动后，运行结束时会自动重算复权因子；需要立即重算时，执行 `niulai recompute`（约 3 秒）。

<a id="monitoring"></a>
## 查看进度

- 页面：<http://127.0.0.1:8080/sync> 显示待执行和失败任务数、最近的运行和各数据集状态。
- 最近的运行：`niulai status`，或 `GET /api/v1/sync/runs`。
- 某次运行的各数据集进度和失败任务：`GET /api/v1/sync/runs/{run_id}`。
- 各数据集的状态：`GET /api/v1/datasets`，其中 `dead_tasks` 大于 0 的需要检查。

常用 SQL：

```sql
-- 最近 10 分钟的请求速率和失败数
SELECT count(*) AS calls, round(count(*) / 600.0, 2) AS rps, count(*) FILTER (WHERE api_code IS DISTINCT FROM 0) AS non_ok
FROM ops.api_call WHERE called_at > now() - interval '10 minutes';

-- 某次运行各数据集的进度
SELECT * FROM ops.v_progress WHERE run_id = 5;

-- 按错误码汇总失败任务
SELECT dataset_id, last_api_code, left(last_error, 80) AS error, count(*)
FROM ops.task WHERE status = 'dead' GROUP BY 1, 2, 3 ORDER BY 4 DESC;
```

<a id="failure_handling"></a>
## 处理失败任务

`dead` 任务不会自动重试。按错误码判断原因：

| 错误 | 常见原因 | 处理 |
| --- | --- | --- |
| 1002 `Unknown thscode` | 证券已退市，或不是该接口支持的证券类型（例如龙虎榜中的可转债） | 确认后忽略；如果是一类证券都不支持，在数据集的 `universe` 或 `filter` 中排除 |
| 1002 `Invalid parameter format` | 日期超出接口支持的历史范围 | 调整数据集分区的起点，见 [接口限制与已知问题](../interfaces/hithink_api.md#endpoint_quirks) |
| 1003 | 窗口过长、分页过大、日期超出一年等 | 调整 `window_days`、`paging.size` 或分区起点 |
| 2001、2003 | API Key 缺失、无效或无权限 | 在供应商的 Key 管理页面 <https://fuyao.aicubes.cn/admin/> 确认 Key 有效，然后在页面「设置」中更换；Key 来自环境变量时修改 `HITHINK_FINANCE_API_KEY` 后重启 |
| 2004 | 同花顺 AI 客户端专用接口 | 关闭该数据集 |
| 无错误码，报「模板变量不存在」 | `key`、`thscode` 或 `obs_date` 模板引用了响应中没有的字段 | 修改 `datasets.yaml` 中的模板 |
| 可重试错误用尽（4001、5xxx、网络错误） | 供应商暂时不可用或被限流 | 等待后重新规划 |

重新执行失败的分区：对同一数据集重新规划，例如 `niulai plan backfill -d special.dragon_tiger_institution`。规划器只为没有未完成任务的分区创建新任务，已成功的分区会再请求一次，结果不变。

某些分区确认无需执行时（例如超出接口范围的日期），把它们标为 `skipped`，运行的状态就不会因此变成 `partial`：

```sql
UPDATE ops.task SET status = 'skipped', last_error = '超出接口支持的历史范围'
WHERE run_id = 5 AND dataset_id = 'special.limit_down_pool' AND status IN ('pending', 'failed', 'dead');
```

执行者异常退出时，`running` 状态的任务会在 10 分钟后被自动放回队列，不需要手动处理。

<a id="config_changes"></a>
## 修改配置

1. 修改 `backend/config/datasets.yaml` 或环境变量。
2. 运行 `cargo test`，确认配置能被解析（修改 `datasets.yaml` 时）。
3. 停止 `niulai serve`：发送 Ctrl+C 或 SIGTERM。程序不再领取新任务，等正在执行的任务结束后退出。
4. 重新启动 `niulai serve`。启动时会把新配置同步到 `ops.dataset`。

已规划的任务按规划时的参数执行，不受配置修改影响。需要按新配置执行时，处理掉旧任务后重新规划。

<a id="deployment"></a>
## 部署

分两部分：CI 在 GitHub 上检查代码并构建镜像；CD 是把服务运行起来，有两种方式：
- **Docker 部署**（默认）：使用 CI 构建的镜像，用 `deploy/compose.yml` 运行内置 PostgreSQL 和 niulai 两个容器，配置在 `deploy/.env`。
- **源码部署**：不用 Docker，在服务器上编译前端和后端，直接运行编译出的程序 `niulai`，配置在 `backend/.env`，数据库自己准备。

两种方式运行的是同一个程序，行为相同。环境变量见 [配置](../interfaces/configuration.md)，取舍见 [ADR：CI 构建镜像，部署只交付服务本身](../decisions/ci_cd.md)。

服务对外只有一个 HTTP 端口，默认 `127.0.0.1:8080`（Docker 部署在 `.env` 的 `NIULAI_LISTEN_ADDR`、`NIULAI_PORT` 中修改，源码部署是 `NIULAI_API_BIND`）：

| 路径 | 内容 |
| --- | --- |
| `/` | 网页 |
| `/api/v1/*` | REST API，见 [REST API](../interfaces/rest_api.md) |
| `/api/v1/health` | 存活检查，进程在运行就返回 200 |
| `/api/v1/ready` | 就绪检查，能连上数据库时返回 200 |

打开网页先要登录，账号密码是 `deploy/.env` 中的 `NIULAI_ADMIN_USER`、`NIULAI_ADMIN_PASSWORD`，没有设置时都是 `niulai`。部署后先改掉默认密码；登录状态在浏览器保存 30 天，见 [登录](../interfaces/rest_api.md#auth)。

第一次打开网页时进入引导页：到 <https://fuyao.aicubes.cn/admin/> 生成 API Key，粘贴后点「验证并保存」。验证通过后同步开始，并自动规划全量回补（约 51 小时，期间每日同步照常进行）。在此之前同步处于暂停状态。

<a id="release_image"></a>
### CI：构建镜像

工作流是 `.github/workflows/ci.yml`：
- 每次 PR 和推送到 `main`：只做检查，包括后端 clippy（不允许警告）、单元测试和集成测试，前端类型检查、代码检查和测试。不构建镜像。
- 发布版本：`git tag v0.1.0 && git push origin v0.1.0`。检查通过后构建 amd64 和 arm64 镜像，推送为 `0.1.0`、`0.1` 和 `latest`。预发布标签（例如 `v0.2.0-rc.1`）只推送完整版本号，不更新 `latest`。服务器上建议固定版本号。为什么只在发标签时构建，见 [ADR：只在发布版本标签时构建镜像](../decisions/image_on_tag.md)。

预期结果：GitHub Actions 中标签触发的 `ci` 工作流成功；`docker buildx imagetools inspect ghcr.io/duoyuli/niulai:0.1.0` 列出 `linux/amd64` 和 `linux/arm64`。镜像包是公开的，拉取不需要 `docker login`。

<a id="server_install"></a>
### CD：Docker 部署

前提条件：Docker Engine 和 Compose 插件 2.20 或以上；内存 2 GB 以上；磁盘 20 GB 以上（全量数据约 4 GB，之后持续增长）。

步骤：
1. 把 `deploy/compose.yml`、`deploy/upgrader.sh` 和 `deploy/.env.example` 放到服务器的一个目录中，例如 `/opt/niulai/`。
2. `cp .env.example .env`。其中都是可以直接使用的默认值，按需修改，例如端口 `NIULAI_PORT`、镜像版本 `NIULAI_IMAGE`。变量见 [部署配置](../interfaces/configuration.md#deploy_env)。
3. `docker compose up -d`。
4. 预期结果：`docker compose ps` 中 `niulai` 显示 `healthy`；`curl http://127.0.0.1:8080/api/v1/ready` 返回 `{"status":"ready"}`。
5. 打开网页，按引导配置 API Key。

常用命令（在 `compose.yml` 所在目录执行）：
- 状态和日志：`docker compose ps`、`docker compose logs -f niulai`。
- 运维命令：`docker compose exec niulai niulai status`，其他子命令同理。
- 停机：`docker compose down`。niulai 有 2 分钟的停机等待，等正在执行的任务结束，未完成的任务留在队列中，下次启动继续执行。

使用本机构建的镜像：在仓库根目录执行 `docker build -f deploy/Dockerfile -t niulai:local .`，把 `.env` 中的 `NIULAI_IMAGE` 改成 `niulai:local`，再执行 `docker compose up -d`。

<a id="source_deploy"></a>
### CD：源码部署

不用 Docker，直接编译运行。前提条件：
- Rust 工具链：安装 rustup，版本由 `backend/rust-toolchain.toml` 固定（1.99.0），首次编译时自动安装。
- Node.js 24 和 pnpm 12（`corepack enable` 后自动使用 `frontend/package.json` 中指定的版本）。
- PostgreSQL 16 或以上，建好一个空库。
- 编译需要约 4 GB 内存，并能访问 npm 和 crates.io。

步骤：
1. 获取源码：`git clone https://github.com/duoyuli/niulai.git && cd niulai`。
2. 编译前端：`cd frontend && pnpm install --frozen-lockfile && pnpm build`，产物在 `frontend/dist/`。
3. 编译后端：`cd ../backend && cargo build --release --locked`，产物是 `backend/target/release/niulai`，数据库迁移已编译在里面。
4. 配置：`cp .env.example .env`，填写 `DATABASE_URL`，并设置 `NIULAI_WEB_DIR=../frontend/dist`（让程序同时提供网页）。其他变量见 [环境变量](../interfaces/configuration.md#settings)。
5. 运行：在 `backend/` 下执行 `./target/release/niulai serve`。启动时自动执行数据库迁移。
6. 预期结果：日志出现「niulai 服务已启动」；`curl http://127.0.0.1:8080/api/v1/ready` 返回 `{"status":"ready"}`。打开网页，按引导配置 API Key。

程序要在 `backend/` 下运行：`.env`、`config/datasets.yaml` 和 `NIULAI_WEB_DIR` 的相对路径都按当前目录查找。长期运行时交给系统的进程管理（例如 systemd）：工作目录设为 `backend/`，启动命令是 `target/release/niulai serve`，异常退出时自动重启；停止时发送 SIGTERM，并留出 2 分钟，等正在执行的任务结束。

更新：`git pull`，重新执行第 2、3 步，再重启程序。

<a id="one_click_upgrade"></a>
### 一键升级（Docker 部署）

页面左上角 logo 后面是当前版本号。niulai 每 6 小时查一次 GitHub 上最新的 `vX.Y.Z` 标签（`NIULAI_UPDATE_REPO`，默认 `duoyuli/niulai`），有新版本时亮红点；点开后点「一键升级」。

执行升级的是 `compose.yml` 中的 `upgrader` 容器（`docker:cli` 镜像，运行同目录的 `upgrader.sh`），由 `.env` 中的 `COMPOSE_PROFILES=upgrader` 启用。niulai 和 upgrader 通过卷 `upgrade`（niulai 中是 `NIULAI_UPGRADE_DIR=/upgrade`）交换文件：
- niulai 写 `request`，内容只是版本号，例如 `1.2.0`；upgrader 只接受 `X.Y.Z` 格式。
- upgrader 每 3 秒写一次 `heartbeat`；30 秒内没有更新时，页面显示「没有启用一键升级」和手动升级的命令。
- upgrader 把进度写在 `status`，输出写在 `log`，页面每 2 秒刷新一次，失败时显示日志的最后几行。

upgrader 收到请求后依次执行：
1. 下载新镜像 `ghcr.io/<所有者>/niulai:<版本>`。下载失败时什么都不改。
2. `DATABASE_URL` 指向内置数据库时，用 `pg_dump` 备份到 `backups/niulai-before-<版本>-<时间>.dump`。备份失败时不升级。
3. 把 `.env` 中的 `NIULAI_IMAGE` 改成新版本，执行 `docker compose up -d --no-deps niulai`，最多等 5 分钟让 niulai 恢复 `healthy`。
4. 新版本没有恢复 `healthy` 时，把 `NIULAI_IMAGE` 改回原来的版本并重建。新版本可能已经执行了数据库迁移，旧版本起不来时用第 2 步的备份恢复，见下一节的回滚。

页面发现运行的版本变成目标版本后自动刷新。整个过程中 niulai 有一次停机，最长 2 分钟的停机等待加上启动时间。

要求和限制：
- `NIULAI_IMAGE` 要是发布的镜像（`ghcr.io/<所有者>/niulai:...`）；本机构建的 `niulai:local` 不能一键升级，开发版也不检查更新。
- upgrader 挂载了 `/var/run/docker.sock`，能管理这台机器上的所有容器。niulai 没有这个权限，网页上能做的只是提交一个版本号。不需要一键升级时，删掉 `.env` 中的 `COMPOSE_PROFILES=upgrader`，再执行 `docker compose up -d --remove-orphans`。
- 一键升级只更换 niulai 镜像，不更新 `compose.yml`、`upgrader.sh` 和 `.env.example`。新版本需要改这些文件时，发布说明中会写明，按下一节手动更新。
- 为什么这样设计，见 [ADR：Docker 部署的一键升级](../decisions/one_click_upgrade.md)。

<a id="upgrade"></a>
### 手动升级与回滚

1. 先备份数据库，见下一节。
2. Docker 部署：把 `.env` 中的 `NIULAI_IMAGE` 改成新版本号，执行 `docker compose pull && docker compose up -d`。源码部署：`git pull`，重新编译前端和后端，再重启程序。
3. 新版本启动时自动执行数据库迁移。预期结果：`niulai` 恢复为 `healthy`。

回滚：迁移只能向前执行，旧版本程序不一定能用新的表结构，所以要同时回滚数据库：把 `NIULAI_IMAGE` 改回旧版本，再用升级前的备份恢复。

<a id="backup_restore"></a>
### 备份与恢复

以下命令在 `compose.yml` 所在目录执行，适用于 Docker 部署的内置数据库。源码部署时，对 `DATABASE_URL` 指向的库直接执行同样的 `pg_dump`、`psql`、`pg_restore`，停止和启动 niulai 换成停止和启动程序。

备份（pg_dump 自定义格式，已压缩）：

```bash
mkdir -p backups
docker compose exec -T postgres pg_dump -U niulai -Fc niulai > backups/niulai-$(date +%Y%m%d-%H%M%S).dump
```

每天执行一次，例如 crontab 中加入 `30 3 * * * cd /opt/niulai && docker compose exec -T postgres pg_dump -U niulai -Fc niulai > backups/niulai-$(date +\%Y\%m\%d).dump`，并定期把备份拷到别处、清理旧文件。

恢复：会覆盖当前数据库，先备份当前库。

```bash
docker compose stop niulai
docker compose exec -T postgres psql -U niulai -d postgres -c 'DROP DATABASE niulai WITH (FORCE)' -c 'CREATE DATABASE niulai'
docker compose exec -T postgres pg_restore -U niulai -d niulai --no-owner < backups/<文件>.dump
docker compose start niulai
```

不要用 `pg_restore --clean` 覆盖已有的库：`raw.record`、`ops.api_call` 是分区表，`--clean` 会失败。迁到另一台服务器时，在新服务器上完成 Docker 部署的第 1 至 3 步，先不要在网页上配置 Key，直接按上面的步骤恢复；备份里有已保存的 Key 和回补记录，恢复后不会重复回补。

<a id="testing"></a>
## 测试

1. 运行单元测试：`cargo test`。不需要数据库。
2. 运行集成测试：`NIULAI_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/postgres cargo test`。连接串需要有建库权限；每个测试创建一个临时数据库，结束时删除。未设置这个变量时，集成测试会跳过。
3. 运行静态检查：`cargo clippy --all-targets`。提交前应没有警告。
4. 前端（在 `frontend/` 下）：`pnpm typecheck`、`pnpm lint`、`pnpm test`，都应通过。

集成测试覆盖：迁移、写入幂等、等比复权与同花顺口径、任务去重与超时回收、运行结束判定与续租、记录修订历史、执行进程的独占锁、主要 API（含自选股、全市场扫描缓存、回测）、前端页面的路由回退与缓存头。测试不调用供应商接口。
