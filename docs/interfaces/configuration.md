# 配置

本文说明 `niulai` 的配置来源、环境变量、Docker 部署用的 `deploy/.env` 和配置文件。数据集配置的格式见 [数据集注册表格式](dataset_registry.md)。实现在 `backend/src/config.rs`。

<a id="sources"></a>
## 配置来源

优先级从低到高：

1. 内置默认值。
2. `.env` 文件：从当前目录开始，依次查找 `.env` 和 `backend/.env`，找不到就到上一级目录继续找，一共查 3 层（当前目录和往上两层）。所以在仓库根目录或 `backend/` 下运行都能读到 `backend/.env`。
3. 进程环境变量，覆盖 `.env` 中的同名项。

`.env` 每行是 `KEY=VALUE`；忽略空行和 `#` 开头的注释，去掉值两侧的引号。模板是 `backend/.env.example`。`.env` 含密码和 API Key，不提交到版本库；生产环境用部署平台的环境变量或密钥管理注入。

<a id="time_zone"></a>
## 时区

程序里的时间一律按上海时间（`Asia/Shanghai`，A 股交易所的时间）理解，与服务器、容器、数据库所在的时区无关，部署在哪里都一样：

- **代码：** 「今天」「现在」用 `time::today_cn`、`time::now_cn`（UTC 加 8 小时），定时同步的时间、收盘时间 15:30 都按它判断。
- **数据库：** 连接池的每个连接建立后执行 `SET TIME ZONE 'Asia/Shanghai'`（`db::pool`），SQL 中的 `current_date`、`now()::date`、`date_trunc` 都按上海的日期计算。sqlx 连接时会自己发送 `TimeZone=UTC`，写在连接串里的时区参数会被覆盖，所以不能靠 `DATABASE_URL` 设置。`timestamptz` 列存的是时间点，不受影响。
- **日志：** 时间带 `+08:00`。
- **镜像：** 设置了 `TZ=Asia/Shanghai`，容器里其他用到本地时间的地方（例如升级前备份的文件名）也一致。
- **页面：** 前端显示时间和计算「今天」时指定 `Asia/Shanghai`，与浏览器所在的时区无关。

不需要、也没有配置项可以改时区。

<a id="settings"></a>
## 环境变量

| 变量 | 必填 | 默认值 | 含义 |
| --- | --- | --- | --- |
| `DATABASE_URL` | 是 | — | PostgreSQL 连接串，例如 `postgres://user:pass@127.0.0.1:5432/niulai`。需要 PostgreSQL 16 或以上 |
| `REDIS_URL` | 否 | 无 | Redis 共享读取缓存，例如 `redis://127.0.0.1:6379`。未配置或暂时不可用时回退到进程内缓存和 PostgreSQL；缓存内容可以随时删除 |
| `HITHINK_FINANCE_API_KEY` | 否 | — | 同花顺金融数据服务 API Key（在 <https://fuyao.aicubes.cn/admin/> 生成）。一般不设置，在网页的首次登录引导中配置，保存在数据库；设置后优先使用它，网页上不能修改，见 [ADR：API Key 在首次登录引导中配置](../decisions/api_key_setup.md)。空值按未设置处理 |
| `NIULAI_ADMIN_USER` | 否 | `niulai` | 第一个管理员的用户名。只在第一次启动、第一个管理员还没有密码时用一次，之后在网页上管理用户，见 [登录与权限](rest_api.md#auth) |
| `NIULAI_ADMIN_PASSWORD` | 否 | `niulai` | 第一个管理员的初始密码，同样只用一次；之后修改它没有效果，密码在网页「我的」页面改。用默认值时启动日志有警告，登录后页面顶部提醒修改 |
| `NIULAI_UPGRADE_DIR` | 否 | 无；Docker 部署的 `compose.yml` 中是 `/upgrade` | 与 upgrader 容器共享的目录，为空时不能一键升级，见 [一键升级](../operations/runbook.md#one_click_upgrade) |
| `NIULAI_UPDATE_REPO` | 否 | `duoyuli/niulai` | 到哪个 GitHub 仓库查新版本；设为空时不检查 |
| `NIULAI_METRICS_TOKEN` | 否 | 无 | 设置后启用 `GET /api/v1/metrics`（Prometheus 文本格式），请求要带 `Authorization: Bearer <令牌>`（不接受写在网址里）；不设置时接口返回 404。令牌只在启动时读一次，见 [监控指标](../operations/runbook.md#metrics) |
| `NIULAI_API_BIND` | 否 | `127.0.0.1:8080` | API 监听地址；Docker 镜像中默认 `0.0.0.0:8080` |
| `NIULAI_API_TIMEOUT_S` | 否 | `300` | API 请求最长执行秒数；因子检验、组合回测等长任务仍支持前端取消 |
| `NIULAI_DATASETS_PATH` | 否 | 依次查找 `config/datasets.yaml`、`backend/config/datasets.yaml` | 数据集注册表路径；Docker 镜像中是 `/app/config/datasets.yaml` |
| `NIULAI_DB_MAX_CONNECTIONS` | 否 | `20` | 数据库连接池上限；API、同步和后台计算共用连接池，资源较小的机器可以调低 |
| `NIULAI_WORKER_CONCURRENCY` | 否 | `3` | 单个进程同时执行的同步任务数，必须大于 0 |
| `NIULAI_RATE_LIMIT_RPS` | 否 | `2` | 每秒请求供应商的次数，必须大于 0。同一时刻只有一个进程执行同步任务，所以这就是全局上限，见 [限速与单一执行进程](../architecture/sync_pipeline.md#rate_limit) |
| `NIULAI_HTTP_TIMEOUT_S` | 否 | `60` | 请求供应商的超时秒数；下载导出文件固定为 600 秒 |
| `NIULAI_VENDOR_BASE_URL` | 否 | `https://fuyao.aicubes.cn` | 供应商地址 |
| `NIULAI_WEB_DIR` | 否 | 无；Docker 镜像中是 `/app/web` | 前端构建产物目录（`frontend/dist`）。配置后 `niulai serve` 同时提供前端页面，见 [由 niulai serve 提供](../architecture/frontend.md#serving) |
| `NIULAI_SCHEDULER_ENABLED` | 否 | `true` | 是否在 `niulai serve` 中运行定时调度；`--no-scheduler` 效果相同。同步时间在同步页上设置，见 [定时调度](../architecture/sync_pipeline.md#scheduling) |
| `RUST_LOG` | 否 | `info,sqlx=warn,tower_http=info` | 日志级别 |
| `NIULAI_LOG_FORMAT` | 否 | 文本 | 设为 `json` 时输出结构化日志；Docker 镜像中默认 `json` |
| `HOSTNAME` | 否 | `local` | 用于生成执行者标识（`ops.task.locked_by`） |
| `NIULAI_TEST_DATABASE_URL` | 仅测试 | — | 集成测试使用的、有建库权限的连接串，见 [运维手册](../operations/runbook.md#testing) |

启动时校验：缺少 `DATABASE_URL`、并发或限速为 0，都会让程序报错退出。同步时间保存在数据库（`app.setting` 的 `sync_schedule`），在同步页修改时校验，不在启动时检查。没有 API Key 不是错误：服务照常启动，同步暂停，直到在网页上配置。

<a id="deploy_env"></a>
## 部署配置

Docker 部署的全部配置写在 `deploy/.env` 中，模板 `deploy/.env.example` 列出了全部常用项，默认值可以直接使用。这个文件有两个用途：Compose 用它展开 `compose.yml` 中的变量（镜像、端口、内置数据库的密码），niulai 容器也把它作为环境变量读入，所以 `DATABASE_URL` 和上表中的 `NIULAI_*` 都写在这里。只在 Docker 部署时使用的变量：

| 变量 | 默认值 | 含义 |
| --- | --- | --- |
| `COMPOSE_PROFILES` | `upgrader` | 启用一键升级助手 upgrader 容器；删掉这一行即关闭，见 [一键升级](../operations/runbook.md#one_click_upgrade) |
| `NIULAI_IMAGE` | `ghcr.io/duoyuli/niulai:latest` | 镜像地址；建议固定版本号，例如 `ghcr.io/duoyuli/niulai:2026.10.18`。一键升级会改写这一行 |
| `NIULAI_LISTEN_ADDR`、`NIULAI_PORT` | `127.0.0.1`、`8080` | niulai 在宿主机上监听的地址和端口；端口被占用时改 `NIULAI_PORT`，要从其他机器访问时把地址改成 `0.0.0.0` |
| `POSTGRES_PASSWORD` | `niulai` | 内置数据库的密码，要与 `DATABASE_URL` 中的一致；只在第一次启动（创建数据目录）之前修改才生效 |
| `POSTGRES_DATA` | `./data/postgres` | 内置数据库的数据目录，默认在部署目录下，见 [数据目录](../operations/runbook.md#data_dir) |
| `REDIS_URL` | `redis://redis:6379` | Compose 内置 Redis 的地址；只保存可重建的页面结果 |

`DATABASE_URL` 默认是 `postgres://niulai:niulai@postgres:5432/niulai`，指向同一个 Compose 中的内置数据库。容器内的监听地址、前端目录和数据集配置路径由镜像固定（`NIULAI_API_BIND=0.0.0.0:8080`、`NIULAI_WEB_DIR`、`NIULAI_DATASETS_PATH`），不要在 `.env` 中修改。

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
| `.github/workflows/ci.yml` | CI：PR 和推送到 `main` 只检查代码，推送 `v*` 标签时才构建 amd64、arm64 镜像并推送到 GHCR，见 [CI：构建镜像](../operations/runbook.md#release_image) |
| `deploy/compose.yml` | Docker 部署：内置 PostgreSQL、Redis（共享读取缓存）、niulai 和一键升级助手 upgrader（`niulai upgrader`，见 [一键升级](../operations/runbook.md#one_click_upgrade)），配置读取 `deploy/.env` |
| `deploy/.env`、`deploy/.env.example` | Docker 部署的配置和模板，见 [部署配置](#deploy_env)；`.env` 含密码，不提交 |
