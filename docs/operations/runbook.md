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
- [部署](#deployment)：发布镜像、在服务器上安装、升级与回滚、备份与恢复，或在本机用 Docker 部署时阅读。
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

镜像由 GitHub Actions 构建并推送到 GitHub 镜像仓库（GHCR），服务器上只需要 Docker 和 `deploy/` 目录，不需要源码和编译环境。`deploy/compose.yml` 编排三个服务：内置 PostgreSQL（`db`）、niulai、Caddy 反向代理（`proxy`，负责 HTTPS 和登录），按 `COMPOSE_PROFILES` 启用。全部配置在 `deploy/.env` 中，变量见 [部署配置](../interfaces/configuration.md#deploy_env)，方案的取舍见 [ADR：部署方案](../decisions/deployment.md)。以下命令除特别说明外，都在服务器的 `deploy/` 目录下执行。

<a id="release_image"></a>
### 发布镜像

前提条件：代码在 GitHub 仓库中，仓库的 Actions 已启用。

1. 推送到 `main`：工作流 `.github/workflows/image.yml` 构建 amd64 和 arm64 两种架构的镜像，推送为 `ghcr.io/<所有者>/niulai:latest`（所有者名转为小写）。
2. 发布版本：`git tag v0.1.0 && git push origin v0.1.0`，得到标签 `0.1.0` 和 `0.1`。服务器上建议固定版本号，不用 `latest`。
3. 预期结果：Actions 中 `image` 工作流成功；`docker buildx imagetools inspect ghcr.io/<所有者>/niulai:0.1.0` 列出 `linux/amd64` 和 `linux/arm64`。
4. 镜像包默认私有。二选一：在 GitHub 的 Packages 页面把 `niulai` 改为公开；或在每台服务器上 `docker login ghcr.io -u <用户名>`，密码用有 `read:packages` 权限的 token。

arm64 构建机（`ubuntu-24.04-arm`）在公开仓库中免费。私有仓库用不了时，删掉工作流 matrix 中的 arm64 一项，只发布 amd64。

<a id="server_install"></a>
### 在服务器上安装

前提条件：
- Linux 服务器，Docker Engine 和 Compose 插件 2.20 或以上。
- 内存 2 GB 以上，磁盘 20 GB 以上：全量数据约 4 GB，之后持续增长。
- 对公网开放时：一个解析到这台服务器的域名，防火墙放行 80、443 端口。
- 同一个 API Key 不要同时在两台服务器上执行同步：两边的请求会叠加，超过供应商的频率上限。

步骤：
1. 把 `deploy/` 中的 `compose.yml`、`Caddyfile`、`.env.example` 和所有 `.sh` 拷到服务器，例如在仓库根目录执行：`rsync -a --exclude .env --exclude caddy/ --exclude backups/ deploy/ <服务器>:/opt/niulai/deploy/`。
2. 在服务器上执行 `cd /opt/niulai/deploy && ./install.sh`，按提示输入镜像地址（例如 `ghcr.io/<所有者>/niulai:0.1.0`）和域名。域名留空表示不对公网开放。也可以用环境变量加 `-y` 非交互执行，见脚本开头的说明。
3. 脚本依次：生成 `.env`（权限 600，数据库密码随机生成）；有域名时生成网页登录的用户名和随机密码（只显示这一次）；拉取镜像，启动服务，等待健康检查通过。
4. 预期结果：输出「安装完成」和访问地址。有域名时打开 `https://<域名>/` 并登录，Caddy 首次访问时自动申请证书；没有域名时，用 `ssh -L 8080:127.0.0.1:8080 <服务器>` 建隧道后打开 <http://127.0.0.1:8080/>。
5. 第一次打开网页时进入引导页：到 <https://fuyao.aicubes.cn/admin/> 生成 API Key，粘贴后点「验证并保存」。验证通过后同步开始，并自动规划 P0、P1 的全量回补（约 51 小时，期间每日同步照常进行）。在此之前同步处于暂停状态。
6. 配置每日备份，见 [备份与恢复](#backup_restore)。

`install.sh` 可以重复执行：已有 `.env` 时不再询问，只启动服务并检查。常用命令：
- 状态和日志：`docker compose ps`、`docker compose logs -f niulai`。
- 运维命令：`docker compose exec niulai niulai status`，其他子命令同理。
- 停机：`docker compose down`。niulai 有 2 分钟的停机等待，等正在执行的任务结束，未完成的任务留在队列中，下次启动继续执行。
- 忘记登录密码：删除 `caddy/auth.caddy`，重新执行 `./install.sh`，会生成新密码并重启 Caddy。

使用已有的 PostgreSQL（16 或以上）：在 `.env` 中把 `COMPOSE_PROFILES` 里的 `db` 去掉，`DATABASE_URL` 改成那个库的连接串（主机要能从容器访问）。库要先建好。

<a id="upgrade"></a>
### 升级与回滚

升级：
1. 把 `.env` 中的 `NIULAI_IMAGE` 改成新版本号。
2. 执行 `./upgrade.sh`：先备份数据库，再拉取镜像、重启服务，等待健康检查通过。新版本启动时自动执行数据库迁移。
3. 预期结果：输出「已升级」和新镜像的摘要。

回滚：迁移只能向前执行，旧版本程序不一定能用新的表结构，所以要同时回滚数据库。把 `NIULAI_IMAGE` 改回旧版本，再执行 `./restore.sh backups/<升级前的备份>.dump`。

<a id="backup_restore"></a>
### 备份与恢复

- 备份：`./backup.sh`，把数据库导出到 `backups/niulai-<时间>.dump`（pg_dump 自定义格式，已压缩），保留最近 7 份（`NIULAI_BACKUP_KEEP`）。内置数据库和外部数据库都适用。每天执行一次，例如 `crontab -e` 中加入 `30 3 * * * /opt/niulai/deploy/backup.sh >> /opt/niulai/deploy/backups/backup.log 2>&1`。备份在服务器本机磁盘上，要定期拷到别处。
- 恢复：`./restore.sh backups/<文件>.dump`。先恢复到临时库，成功后再与当前库互换，服务正常启动后才删除旧库；任何一步失败，当前库都保持原样，niulai 会重新启动。数据库用户需要有建库权限，内置数据库默认满足。
- 迁到另一台服务器：在旧服务器上 `./backup.sh`，把备份文件拷到新服务器，执行 `./install.sh`，先不要在网页上配置 Key，直接 `./restore.sh <备份文件>`。备份里有已保存的 Key 和回补记录，恢复后不会重复回补。

<a id="local_deploy"></a>
### 本机 dev-infra 部署

本机开发环境使用 dev-infra 中已有的 `dev-postgresql`，镜像在本机从源码构建。`deploy/.env` 中：`NIULAI_IMAGE=niulai:latest`，`COMPOSE_PROFILES` 留空（不启用内置数据库和代理），`DATABASE_URL` 的主机写 `dev-postgresql:5432`；API Key 可以写在这里（`HITHINK_FINANCE_API_KEY`），也可以留空后在网页上配置。在仓库根目录执行：

```bash
docker compose -f deploy/compose.yml -f deploy/compose.build.yml -f deploy/compose.dev-infra.yml up -d --build
```

之后的 `ps`、`logs`、`exec`、`down` 也带上这三个 `-f`。页面在 <http://127.0.0.1:8080/>。

<a id="testing"></a>
## 测试

1. 运行单元测试：`cargo test`。不需要数据库。
2. 运行集成测试：`NIULAI_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/postgres cargo test`。连接串需要有建库权限；每个测试创建一个临时数据库，结束时删除。未设置这个变量时，集成测试会跳过。
3. 运行静态检查：`cargo clippy --all-targets`。提交前应没有警告。
4. 前端（在 `frontend/` 下）：`pnpm typecheck`、`pnpm lint`、`pnpm test`，都应通过。

集成测试覆盖：迁移、写入幂等、等比复权与同花顺口径、任务去重与超时回收、运行结束判定与续租、记录修订历史、执行进程的独占锁、主要 API（含自选股、全市场扫描缓存、回测）、前端页面的路由回退与缓存头。测试不调用供应商接口。
