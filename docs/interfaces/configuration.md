# 配置

本文说明 `niulai` 的配置来源、环境变量、Docker 部署用的 `deploy/.env` 和配置文件。数据集配置的格式见 [数据集注册表格式](dataset_registry.md)。实现在 `backend/src/config.rs`。

<a id="sources"></a>
## 配置来源

优先级从低到高：

1. 内置默认值。
2. `.env` 文件：从当前目录开始，依次查找 `.env` 和 `backend/.env`，找不到就到上一级目录继续找，最多向上 3 层。所以在仓库根目录或 `backend/` 下运行都能读到 `backend/.env`。
3. 进程环境变量，覆盖 `.env` 中的同名项。

`.env` 每行是 `KEY=VALUE`；忽略空行和 `#` 开头的注释，去掉值两侧的引号。模板是 `backend/.env.example`。`.env` 含密码和 API Key，不提交到版本库；生产环境用部署平台的环境变量或密钥管理注入。

<a id="settings"></a>
## 环境变量

| 变量 | 必填 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `DATABASE_URL` | 是 | — | PostgreSQL 连接串，例如 `postgres://user:pass@127.0.0.1:5432/niulai`。需要 PostgreSQL 16 或以上 |
| `HITHINK_FINANCE_API_KEY` | 否 | — | 同花顺金融数据服务 API Key（在 <https://fuyao.aicubes.cn/admin/> 生成）。一般不设置，在网页的首次登录引导中配置，保存在数据库；设置后优先使用它，网页上不能修改，见 [ADR：API Key 在首次登录引导中配置](../decisions/api_key_setup.md)。空值按未设置处理 |
| `NIULAI_API_BIND` | 否 | `127.0.0.1:8080` | API 监听地址；Docker 镜像中默认 `0.0.0.0:8080` |
| `NIULAI_DATASETS_PATH` | 否 | 依次查找 `config/datasets.yaml`、`backend/config/datasets.yaml` | 数据集注册表路径；Docker 镜像中是 `/app/config/datasets.yaml` |
| `NIULAI_DB_MAX_CONNECTIONS` | 否 | `10` | 数据库连接池上限 |
| `NIULAI_WORKER_CONCURRENCY` | 否 | `3` | 单个进程同时执行的同步任务数，必须大于 0 |
| `NIULAI_RATE_LIMIT_RPS` | 否 | `2` | 每秒请求供应商的次数，必须大于 0。同一时刻只有一个进程执行同步任务，所以这就是全局上限，见 [限速与单一执行进程](../architecture/sync_pipeline.md#rate_limit) |
| `NIULAI_HTTP_TIMEOUT_S` | 否 | `60` | 请求供应商的超时秒数；下载导出文件固定为 600 秒 |
| `NIULAI_VENDOR_BASE_URL` | 否 | `https://fuyao.aicubes.cn` | 供应商地址 |
| `NIULAI_SCHEDULE_AFTER_CLOSE` | 否 | `18:00` | 每日增量的触发时间，Asia/Shanghai，`HH:MM` 格式 |
| `NIULAI_WEB_DIR` | 否 | 无；Docker 镜像中是 `/app/web` | 前端构建产物目录（`frontend/dist`）。配置后 `niulai serve` 同时提供前端页面，见 [由 niulai serve 提供](../architecture/frontend.md#serving) |
| `NIULAI_SCHEDULER_ENABLED` | 否 | `true` | 是否在 `niulai serve` 中运行定时调度；`--no-scheduler` 效果相同 |
| `RUST_LOG` | 否 | `info,sqlx=warn,tower_http=info` | 日志级别 |
| `NIULAI_LOG_FORMAT` | 否 | 文本 | 设为 `json` 时输出结构化日志；Docker 镜像中默认 `json` |
| `HOSTNAME` | 否 | `local` | 用于生成执行者标识（`ops.task.locked_by`） |
| `NIULAI_TEST_DATABASE_URL` | 仅测试 | — | 集成测试使用的、有建库权限的连接串，见 [运维手册](../operations/runbook.md#testing) |

启动时校验：缺少 `DATABASE_URL`、并发或限速为 0、调度时间格式不对，都会让程序报错退出。没有 API Key 不是错误：服务照常启动，同步暂停，直到在网页上配置。

<a id="deploy_env"></a>
## 部署配置

用 Docker Compose 部署时，所有配置都写在 `deploy/.env` 中，模板是 `deploy/.env.example`。这个文件同时有两个用途：Compose 用它展开 `compose.yml` 中的变量，niulai 容器也把它作为环境变量读入，所以上面的 `NIULAI_*` 变量都可以写在这里。只在部署时使用的变量：

| 变量 | 必填 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `POSTGRES_PASSWORD` | 使用内置数据库时 | — | 内置数据库的密码；`DATABASE_URL` 未设置时，niulai 用它连接内置数据库 |
| `NIULAI_IMAGE` | 否 | `ghcr.io/duoyuli/niulai:latest` | 镜像地址；服务器上建议固定版本号，例如 `ghcr.io/duoyuli/niulai:0.1.0` |
| `NIULAI_LISTEN` | 否 | `127.0.0.1:8080` | niulai 在宿主机上监听的地址和端口；要从其他机器访问时改成 `0.0.0.0:8080` |
| `COMPOSE_PROFILES` | 否 | 空 | `db` 表示启动内置 PostgreSQL；使用已有的数据库时留空，并设置 `DATABASE_URL` |

API Key 一般不写在这里，而是第一次打开网页时配置，见 [首次登录引导](rest_api.md#setup)。

<a id="config_files"></a>
## 配置文件

| 文件 | 内容 |
| --- | --- |
| `backend/config/datasets.yaml` | 数据集注册表。进程启动时读取一次并同步到 `ops.dataset`；修改后需要重启 `niulai serve` |
| `backend/.env` | 本地环境变量 |
| `backend/rust-toolchain.toml` | 固定 Rust 工具链版本（1.99.0） |
| `frontend/package.json`、`frontend/pnpm-lock.yaml` | 前端依赖，版本精确锁定 |
| `frontend/pnpm-workspace.yaml` | pnpm 的供应链策略：依赖发布满 3 天才安装，只允许 esbuild 执行安装脚本 |
| `frontend/orval.config.ts` | 从 `frontend/openapi.json` 生成接口客户端的配置 |
| `frontend/vite.config.ts` | Vite 配置；开发时把 `/api` 转发到 `NIULAI_API_URL`（默认 `http://127.0.0.1:8080`） |
| `deploy/Dockerfile` | 镜像构建：先构建前端，再编译后端，构建上下文是仓库根目录（`.dockerignore` 在根目录） |
| `.github/workflows/ci.yml` | CI：检查代码，`main` 有合并或推送版本标签时构建 amd64、arm64 镜像并推送到 GHCR，见 [CI：构建镜像](../operations/runbook.md#release_image) |
| `deploy/compose.yml` | Docker 部署：内置 PostgreSQL 和 niulai，读取 `deploy/.env` |
| `deploy/.env` | 部署配置，见 [部署配置](#deploy_env)；含密码，不提交 |
