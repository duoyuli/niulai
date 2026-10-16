# niulai 仓库说明

A 股数据工作台：从同花顺金融数据服务同步行情、财务和特色数据到 PostgreSQL，提供复权、均线信号等查询 API 和网页界面。工程知识见 `docs/index.md`。

## 目录

- `backend/`：Rust 后端（Axum、SQLx、Tokio），只有一个程序 `niulai`：`niulai serve` 是常驻服务，其余子命令是运维命令。迁移在 `backend/migrations/`，数据集配置在 `backend/config/datasets.yaml`，本地配置在 `backend/.env`（不提交）。
- `frontend/`：前端（React 19、TypeScript 6、Vite、TanStack Query、React Router、Tailwind CSS、Lightweight Charts），包管理用 pnpm。接口类型和 hooks 由 orval 从 `frontend/openapi.json` 生成在 `src/api/generated/`，不手工修改。
- `docs/`：Project Doc 文档库；`docs/research/` 是调研笔记，不属于文档库。
- `deploy/`：CD。`Dockerfile`（构建上下文是仓库根目录，同时构建前端和后端）、`compose.yml`（Docker 部署）；部署配置在 `deploy/.env`（不提交）。源码部署不用 Docker，直接编译运行，见运维手册。
- `.github/workflows/ci.yml`：CI。PR 和推送到 `main` 只检查代码；推送 `v*` 标签时才构建 amd64、arm64 镜像并推送到 GHCR。
- `.agents/skills/project-doc/`：本仓库使用的 project-doc skill；`.claude/skills/project-doc` 是指向它的符号链接。
- `CLAUDE.md`：指向本文件的符号链接，供只读取 CLAUDE.md 的工具使用。
- `README.md`：给第一次进入仓库的人看的项目介绍和快速开始。

## 常用命令

在 `backend/` 下执行：

- 构建：`cargo build`
- 单元测试：`cargo test`
- 集成测试：`NIULAI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo test`（会创建并删除临时库）
- 静态检查：`cargo clippy --all-targets`，提交前应无警告
- 运维命令：`cargo run --bin niulai -- --help`

在 `frontend/` 下执行：

- 开发服务器：`pnpm dev`（`/api` 转发到 `127.0.0.1:8080`）
- 类型检查、代码检查、测试：`pnpm typecheck`、`pnpm lint`、`pnpm test`，提交前应全部通过
- 后端接口变更后：`pnpm api:spec && pnpm api:gen`，再 `pnpm typecheck`

在仓库根目录执行：

- 构建镜像：`docker build -f deploy/Dockerfile -t niulai:local .`
- 本机 Docker 部署：`docker compose -f deploy/compose.yml up -d --no-deps niulai`（`deploy/.env` 中 `NIULAI_IMAGE=niulai:local`，`DATABASE_URL` 指向宿主机上的 dev-postgresql，所以不启动内置数据库）
- Docker 部署、源码部署（直接编译运行）、升级、备份恢复：见 `docs/operations/runbook.md` 的「部署」

## 工作约定

- 已执行过的迁移文件不再修改；表结构变更一律新增迁移文件。
- 新增数据集优先只改 `backend/config/datasets.yaml`；只有需要类型化表时才在 `backend/src/sync/sinks.rs` 增加投影。
- 动态拼接的 SQL 用 `sqlx::AssertSqlSafe` 包裹，并在上一行用「审计：」注释说明拼接内容为什么安全。用户输入一律走绑定参数。
- 新增或修改 API 时，`#[utoipa::path]` 的每个响应都写 `description`，否则前端生成客户端失败。
- 所有 `ToSchema` 类型的名字在整个 API 中必须唯一：utoipa 遇到同名类型会静默覆盖，生成的前端类型会错。重名时改类型名或用 `#[schema(as = ...)]`。
- 前端依赖精确锁定版本；`frontend/pnpm-workspace.yaml` 要求依赖发布满 3 天才安装，不要为了装最新版而放宽这条策略。
- API Key、数据库密码只放在 `backend/.env`、`deploy/.env` 或部署环境变量中，不写进代码、日志、文档或提交。
- 中文文档遵循 tech-doc-style-chinese 风格：中文与英文、数字之间加空格，引号用「」，一段一行。

## Project Doc gate

- 执行任何仓库任务之前，先确认当前 agent 会话能够调用名为 `project-doc` 的 skill。
- 如果该 skill 不可用，立即停止。唯一允许的下一步是回复：「当前环境未安装或未加载 `project-doc` skill，仓库规则不允许继续工作。请安装或启用该 skill 后，在新会话中重试。」
- skill 可用时，先使用 `project-doc`。在本仓库的第一个任务中，完整阅读 `docs/index.md`，再按其中的路由处理任务。
- 同一会话中已读过的 skill 说明、索引和章节，只要其内容仍足够完整地保留在上下文中就复用，编辑文件、继续子任务和完成工作时同样适用。内容缺失、相关文件发生变化或路由冲突时，按 skill 的「Reading and context reuse」规则重读受影响的材料。
