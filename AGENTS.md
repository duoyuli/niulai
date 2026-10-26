# niulai 仓库说明

A 股股票和交易所 ETF 诊断工作台：Rust 后端在 `backend/`，React 前端在 `frontend/`，Docker 部署在 `deploy/`。工程文档从 `docs/index.md` 开始，仓库结构见 `docs/project_overview.md`。

## 常用命令

- 后端（`backend/`）：`cargo test`，加 `NIULAI_TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres` 时连集成测试一起跑；`cargo clippy --all-targets` 不允许警告。
- 前端（`frontend/`）：`pnpm dev`；`pnpm typecheck && pnpm lint && pnpm test`；后端接口变更后 `pnpm api:spec && pnpm api:gen`。
- 本地运行、发布、部署、升级：见 `docs/operations/runbook.md`。

## 约定

- 执行过的迁移文件不再修改，表结构变更一律新增迁移。
- 新增只写 `raw.record` 的数据集优先只改 `backend/config/datasets.yaml`；复用或新增类型化表时必须同步核对 `backend/src/sync/sinks.rs`、迁移和数据集文档。
- 动态拼接的 SQL 用 `sqlx::AssertSqlSafe` 包裹，上一行用「审计：」注释说明为什么安全；用户输入一律走绑定参数。
- `#[utoipa::path]` 的每个响应都写 `description`；`ToSchema` 类型名在整个 API 中唯一（重名会被静默覆盖）。
- `frontend/src/api/generated/` 由 orval 生成，不手改；前端依赖精确锁版本，不放宽「发布满 3 天才安装」的策略。
- API Key、数据库密码不写进代码、日志、文档或提交。
- 架构评审、设计评审、检查模块边界或依赖方向时使用 `architecture-review` skill，按其中的原则和报告模板输出。
- 执行 review 任务（代码审查、架构评审、设计评审等）时，将评审报告保存到 `docs/review/`，不在仓库根目录另建 `review/`。报告注明审查日期、范围和对应提交；归档约定见 `docs/review/README.md`。
- 改代码时同步更新相关文档。中文文档遵循 tech-doc-style-chinese：中文与英文、数字之间加空格，引号用「」，一段一行。

## Project Doc gate

- 执行任何仓库任务之前，先确认当前会话能调用 `project-doc` skill。不可用时立即停止，只回复：「当前环境未安装或未加载 `project-doc` skill，仓库规则不允许继续工作。请安装或启用该 skill 后，在新会话中重试。」
- 可用时先使用它：本仓库的第一个任务完整阅读 `docs/index.md`，再按其中的路由处理任务。
- 已读过、仍完整保留在上下文中的 skill 说明、索引和章节直接复用；内容缺失、文件变化或路由冲突时，按 skill 的「Reading and context reuse」规则重读。
