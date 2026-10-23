# 牛来（niulai）

> A 股多因子选股的研究和调仓工作台。

## 新闻稿

均线金叉、新高突破这类单只股票的买卖信号，在 A 股扣掉成本后很难赚钱。牛来只用一种方法：每月给全市场股票按几个经过检验的因子（低波动、低特质波动、利润增速、盈利质量、小规模）打分，分散持有排名靠前的一批。它从供应商同步行情、公司行动和财报到本地 PostgreSQL，在网页上检验因子有没有预测力，并给每个账户生成「明天怎么操作」的调仓清单，每条都写清楚为什么。

牛来面向需要自己核对依据的小团队和个人研究者。它以日线和收盘后同步为主，数据和回测结果仅供研究，不构成投资建议，也不连接券商或自动下单。

## 常见问题

### 牛来解决什么问题？

它把「看行情」「验证方法」「按方法调仓」拆成连续的步骤：

- **先看结论：** 首页、热点和指数页面汇总最近交易日的市场状态、账户建议和自选股涨跌。
- **验证方法：** 因子研究页给全市场股票按反转、低波动、小盘、盈利质量等因子打分，用 RankIC、分组收益和过拟合概率检验它们有没有预测力，再做多因子组合回测，见 [因子研究](docs/domains/factor_research.md)。
- **按方法调仓：** 账户保存持仓和流水，每天按因子排名生成调仓建议：每月最后一个交易日整体调仓，没有持满时补仓，每条附带名次和理由，见 [账户与调仓建议](docs/domains/accounts.md)。
- **维护数据：** 首次配置数据源后执行历史回补，之后按计划增量同步、核对缺口并安排可补的数据。

### 数据从哪里来？

当前接入同花顺金融数据服务。原始响应保存在通用记录库，常用行情、公司行动和财务数据再投影到类型化表。股票的因子和收益使用等比前复权价格；交易所 ETF 使用供应商提供的 ETF 行情口径。数据集、覆盖范围和限制见 [数据目录](docs/interfaces/dataset_catalog.md) 与 [同花顺接口](docs/interfaces/hithink_api.md)。

### 回测代表未来收益吗？

不代表。回测是按历史数据重放规则，用于比较方法和检查规则是否符合预期；成交时点、价格口径、交易成本、涨跌停和历史证券覆盖范围都有明确边界，详见 [多因子组合回测](docs/domains/factor_research.md#portfolio)。历史表现不能保证未来结果。

### 如何快速启动？

需要 Docker Engine 和 Compose 插件。下面的命令会下载当前 `main` 的部署文件，并在本机启动 PostgreSQL、niulai 和可选的一键升级助手：

```bash
mkdir niulai && cd niulai
curl -O https://raw.githubusercontent.com/duoyuli/niulai/main/deploy/compose.yml
curl -o .env https://raw.githubusercontent.com/duoyuli/niulai/main/deploy/.env.example
docker compose up -d
```

打开 <http://127.0.0.1:8080/>。首次管理员账号和密码读取 `.env` 中的 `NIULAI_ADMIN_USER`、`NIULAI_ADMIN_PASSWORD`；示例文件提供了可启动的默认值，部署到共享网络前应立即修改。管理员登录后按页面引导配置数据源 API Key，再到「数据同步」查看回补进度。

数据库数据位于部署目录的 `data/postgres`，升级前备份位于 `backups/`。不要删除这两个目录。端口、镜像版本、数据库连接和同步限速都在 `.env` 中配置；完整步骤见 [运维手册](docs/operations/runbook.md#deployment)。

### 可以不用 Docker 吗？

可以。前端使用 Node.js 与 pnpm 构建，后端使用仓库固定的 Rust toolchain 编译，运行时需要 PostgreSQL。源码部署、测试、备份和升级步骤见 [运维手册](docs/operations/runbook.md)。

### 从哪里了解工程？

- [工程文档索引](docs/index.md)：按架构、领域、接口、运维和决策进入当前工程知识。
- [项目概览](docs/project_overview.md)：术语、仓库结构和 `niulai` 子命令。
- [运维手册：本地运行](docs/operations/runbook.md#local_setup)：本地后端、前端和测试入口。

### 这个项目明确不做什么？

牛来不承诺收益，不把排名当成订单，不接收券商委托，也不自动交易。供应商权限、数据覆盖和历史缺口会影响结果；页面上的排名和建议必须结合原始数据、规则定义和回测边界一起阅读。
