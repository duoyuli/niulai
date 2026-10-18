//! 通用同步框架：注册表 → 规划器 → 任务队列 → 执行器 → raw.record / 类型化表。
//!
//! 模块分层（上层依赖下层，同层之间不互相依赖）：
//! - 入口：scheduler（定时触发）、executor（执行循环与单个任务的编排）、audit（完整性核对与兜底补偿）、check（按证券 × 交易日的数据核检）
//! - 规划：planner（分区展开为任务）
//! - 执行步骤：fetch（HTTP 取数）、records（记录与 raw.record）、sinks（类型化表）、postprocess（运行结束后的计算）
//! - 状态：queue（ops.task）、run（ops.run）、cursor（ops.cursor）、calendar（ref.trading_day 与交易日校验）
//! - 配置：spec、template、registry

pub mod audit;
pub mod calendar;
pub mod check;
pub mod cursor;
pub mod executor;
pub mod fetch;
pub mod planner;
pub mod postprocess;
pub mod queue;
pub mod records;
pub mod registry;
pub mod retention;
pub mod run;
pub mod scheduler;
pub mod sinks;
pub mod spec;
pub mod template;

use std::sync::Arc;
use std::time::Duration;

use sqlx::{PgConnection, PgPool};

use crate::config::Settings;
use crate::vendor::{HithinkClient, RateLimiter};
use spec::Registry;

/// 同步引擎：`niulai serve` 中的 API、执行循环、调度器，以及运维子命令共用
pub struct Engine {
    pub settings: Settings,
    pub pool: PgPool,
    pub client: Arc<HithinkClient>,
    pub registry: Arc<Registry>,
}

impl Engine {
    pub async fn new(settings: Settings, pool: PgPool) -> anyhow::Result<Arc<Self>> {
        let registry = Registry::load(&settings.datasets_file()?)?;
        registry::sync_to_db(&pool, &registry).await?;
        let _ = audit::CORE_DATASETS.set(registry.writing_to(&spec::PRICE_TABLES).iter().map(|d| d.id.clone()).collect());
        crate::analysis::screener::sync_builtin_strategies(&pool).await?;
        crate::auth::init_first_admin(&pool, &settings).await?;
        let limiter = RateLimiter::new(settings.rate_limit_rps);
        // 环境变量优先，其次是网页上保存的；都没有时等用户在首次登录引导中配置
        let api_key = match &settings.hithink_finance_api_key {
            Some(k) => Some(k.clone()),
            None => crate::setup::stored_api_key(&pool).await?,
        };
        if api_key.is_none() {
            tracing::warn!("还没有配置 API Key：同步暂停，打开网页按引导配置");
        }
        tracing::info!(datasets = registry.datasets.len(), rps = settings.rate_limit_rps, "同步引擎已初始化");
        let client = HithinkClient::new(&settings.vendor_base_url, api_key.as_deref(), Duration::from_secs(settings.http_timeout_s), limiter)?;
        Ok(Arc::new(Engine { settings, pool, client: Arc::new(client), registry: Arc::new(registry) }))
    }

    pub fn executor(&self, worker_id: String) -> Arc<executor::Executor> {
        Arc::new(executor::Executor::new(self.pool.clone(), self.client.clone(), self.registry.clone(), worker_id))
    }

    // @project-doc docs/architecture/sync_pipeline.md#rate_limit
    /// 取得执行同步任务的独占权，直到返回值被丢弃。限速在进程内（见 vendor::ratelimit），
    /// 两个进程同时执行任务会让请求数翻倍，所以 `serve` 的执行循环和 `niulai work` 同一时刻只能有一个在运行
    pub async fn lock_executor(&self) -> anyhow::Result<ExecutorLock> {
        match self.try_lock_executor().await? {
            Some(lock) => Ok(lock),
            None => anyhow::bail!("另一个 niulai 进程（serve 或 work）正在执行同步任务，同一时刻只能有一个。先停止它，或用 `niulai serve --no-worker`"),
        }
    }

    /// 同 [`Engine::lock_executor`]，被别的进程占着时返回 None
    pub async fn try_lock_executor(&self) -> anyhow::Result<Option<ExecutorLock>> {
        // 会话级 advisory lock 跟着连接走：用一个脱离连接池的独立连接持有，连接关闭（进程退出）时自动释放
        let mut conn = self.pool.acquire().await?.detach();
        let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtext('niulai.executor'))").fetch_one(&mut conn).await?;
        Ok(got.then_some(ExecutorLock(conn)))
    }

    pub fn planner(&self) -> planner::Planner<'_> {
        planner::Planner::new(&self.pool, &self.client, &self.registry)
    }

    /// 按 ID 或优先级选择数据集。ids 为空时，按 priorities 选择已开启的数据集。
    pub fn select(&self, ids: &[String], priorities: &[String]) -> anyhow::Result<Vec<&spec::DatasetSpec>> {
        if !ids.is_empty() {
            return ids.iter().map(|id| self.registry.get(id).ok_or_else(|| anyhow::anyhow!("未知的数据集：{id}"))).collect();
        }
        Ok(self.registry.datasets.iter().filter(|d| d.enabled && (priorities.is_empty() || priorities.contains(&d.priority))).collect())
    }
}

/// 执行同步任务的独占权，见 `Engine::lock_executor`
pub struct ExecutorLock(PgConnection);

impl ExecutorLock {
    /// 确认持锁的连接还在。连接断了（数据库重启、网络中断）锁已经被数据库释放，别的进程可能接手，
    /// 这时要停止执行任务，否则两个进程同时请求，限速翻倍
    pub async fn alive(&mut self) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(10), sqlx::query("SELECT 1").execute(&mut self.0))
            .await
            .map_err(|_| anyhow::anyhow!("10 秒没有响应"))??;
        Ok(())
    }
}
