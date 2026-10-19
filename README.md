# 牛来（niulai）

A 股股票和交易所 ETF 诊断工作台。把同花顺金融数据服务的行情、公司行动、财务和特色数据同步到本地 PostgreSQL，在网页上看复权 K 线、技术信号和证券诊断，用自己组合的选股策略每天巡检全市场，并回测策略。

面向没有金融背景的使用者，页面上的术语都附一句白话说明。数据仅供研究，不构成投资建议。

## 功能

- **今日**：大盘涨跌、强势行业、每个策略今天的买点和卖点、自选股的新信号。
- **证券**：股票和 ETF 的 K 线（等比复权，可以叠加 38 个指标、56 个信号和策略的买卖点）、四个维度的诊断、单只证券回测；股票额外提供基本面摘要。
- **策略**：用白话条件组合选股策略（内置 15 个模板），也可以自己定义信号；每天同步完自动巡检全市场；全市场组合回测。
- **数据同步**：首次拉取 2005 年以来的历史，之后按计划自动增量（每个交易日收盘后、每周），并自动核对和补齐缺口。

## 部署

需要 Docker 和 Compose 插件 2.20 以上，部署目录所在的磁盘 20 GB 以上。

```bash
mkdir niulai && cd niulai
curl -O https://raw.githubusercontent.com/duoyuli/niulai/main/deploy/compose.yml
curl -o .env https://raw.githubusercontent.com/duoyuli/niulai/main/deploy/.env.example
docker compose up -d
```

1. 打开 <http://127.0.0.1:8080/>，用第一个管理员登录：账号密码来自 `.env` 的 `NIULAI_ADMIN_USER`、`NIULAI_ADMIN_PASSWORD`，默认都是 `niulai`，登录后在「我的」页面改密码。其他人由管理员在「用户」页添加，不开放注册。
2. 按引导到 <https://fuyao.aicubes.cn/admin/> 生成同花顺金融数据服务的 API Key（免费即可），保存后自动开始同步。全量历史约需 51 小时，期间可以正常使用。

- 数据库数据在部署目录的 `data/postgres`，一键升级前的备份在 `backups/`。不要删除这两个目录。
- 有新版本时，页面左上角的版本号旁边亮红点，点开可以一键升级（先备份数据库，失败时自动换回）。
- 服务默认只监听本机。端口、镜像版本等配置都在 `.env` 中。

源码部署（不用 Docker）、手动升级、备份与恢复见 [运维手册](docs/operations/runbook.md#deployment)。开发和工程文档从 [docs/index.md](docs/index.md) 开始。
