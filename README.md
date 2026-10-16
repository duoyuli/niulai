# 牛来（niulai）

A 股个股诊断工作台。把同花顺金融数据服务的行情、公司行动、财务和特色数据同步到本地 PostgreSQL，在此基础上提供复权行情、技术信号、选股策略与每日巡检、个股诊断和策略回测，通过网页和 REST API 使用。

面向没有金融背景的使用者：页面上的术语（复权、金叉、最大回撤等）都附有一句白话说明。数据仅供研究，不构成投资建议。

## 能做什么

| 功能 | 说明 |
| --- | --- |
| 股票与 K 线 | 搜索沪深京 A 股，查看日 K 线和成交量；支持等比前复权、后复权、不复权，以及同花顺口径（用于对账） |
| 均线信号 | 任意快慢线周期（默认 MA15 / MA30），标出金叉、死叉 |
| 全市场扫描 | 找出某个交易日发生均线交叉的全部股票 |
| 个股诊断 | 趋势、动量、量能、位置各一句白话结论，最近出现的信号（MACD 金叉、放量、创新高等 25 种），支撑和压力价位 |
| 自选 | 收藏股票，一眼看到最新涨跌、均线状态、最近一次交叉和最近的信号 |
| 选股 | 用白话条件组合策略（例如「5 日均线上穿 20 日均线，并且成交额不低于 5000 万」），内置 10 个模板；每天收盘数据同步完后自动巡检全市场，标出新入选的股票；附市场涨跌概况和板块强弱排名 |
| 回测 | 检验「金叉买、死叉卖」或任意已保存的策略：单只股票按真实价格成交、计入分红送转和手续费，与一直持有对比；也可以模拟每天按策略在全市场选股、最多同时持有几只，与沪深 300 对比 |
| 指数与板块 | 交易所指数和同花顺行业、概念、地域板块，含成分股 |
| 数据同步 | 首次拉取 2005 年以来的全部历史，之后每个交易日收盘后自动增量；页面上可以看进度和失败原因 |

## 快速开始

需要 Docker（含 Compose 插件 2.20 以上）。

```bash
mkdir niulai && cd niulai
curl -O https://raw.githubusercontent.com/duoyuli/niulai/main/deploy/compose.yml
curl -o .env https://raw.githubusercontent.com/duoyuli/niulai/main/deploy/.env.example
# .env 中都是可以直接使用的默认值；端口等按需修改
docker compose up -d
```

打开 <http://127.0.0.1:8080/>。第一次打开时会进入引导页，按提示到 <https://fuyao.aicubes.cn/admin/> 生成同花顺金融数据服务的 API Key（免费即可），粘贴保存后自动开始同步。全量历史约需 51 小时，期间可以正常使用，数据陆续出现。

服务默认只监听 `127.0.0.1:8080`，本身没有登录和访问控制。不用 Docker、直接编译运行（源码部署），以及升级、备份与恢复，见 [运维手册](docs/operations/runbook.md#deployment)。

## 组成

```
backend/    Rust 后端（Axum、SQLx、Tokio）：一个程序 niulai，niulai serve 同时提供网页、REST API、同步任务执行和定时调度
frontend/   React 前端（TypeScript、Vite、TanStack Query、Tailwind CSS、Lightweight Charts）
deploy/     Dockerfile 和 Docker Compose 部署文件
docs/       工程文档：架构、业务规则、接口、运维手册、架构决策记录
.github/    CI：检查代码，main 有合并时构建多架构镜像并推送到 ghcr.io/duoyuli/niulai
```

数据只依赖 PostgreSQL。复权采用业内通用的等比复权，均线、信号和收益都基于它计算，理由见 [ADR：采用等比复权](docs/decisions/equal_ratio_adjustment.md)。

## 开发

```bash
# 后端（在 backend/ 下）
cp .env.example .env          # 填写 DATABASE_URL；API Key 可以填，也可以启动后在网页上配置
cargo run -- serve            # http://127.0.0.1:8080/api/v1/health
cargo test && cargo clippy --all-targets

# 前端（在 frontend/ 下，需要 Node.js 24 和 pnpm 12）
pnpm install
pnpm dev                      # http://localhost:5173/，/api 转发到 127.0.0.1:8080
pnpm typecheck && pnpm lint && pnpm test
```

工程文档从 [docs/index.md](docs/index.md) 开始；常用命令和工作约定见 [AGENTS.md](AGENTS.md)。
