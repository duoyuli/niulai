//! 执行器：认领任务并编排一个任务的执行步骤（交易日校验 → 取数 → 生成记录 → 写入 → 推进游标），
//! 任务结束后判断运行是否结束，结束时执行后续计算并规划下一个波次。
//!
//! 各步骤的实现分别在 queue（任务状态）、calendar（交易日校验）、fetch（HTTP）、records（记录与 raw.record）、
//! sinks（类型化表）、cursor（游标）、run（运行状态）中。至少执行一次：写入都按主键 UPSERT，重复执行结果不变。

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::NaiveDate;
use serde_json::{Map, Value};
use sqlx::PgPool;
use tokio::sync::watch;

use super::calendar::{self, Readiness, TradeDateGuard};
use super::fetch::{Fetcher, Target};
use super::planner::Planner;
use super::queue::{ClaimedTask, Finished, HEARTBEAT, TaskQueue, TaskStats};
use super::spec::{DatasetSpec, Registry};
use super::{cursor, postprocess, records, run, sinks, template};
use crate::vendor::{CallError, HithinkClient};

/// 队列为空时的轮询间隔
const IDLE_POLL: Duration = Duration::from_secs(2);
/// 0 号执行槽的维护间隔（回收过期租约、结束无人结束的运行、定期重算复权因子）
const MAINTENANCE_EVERY: Duration = Duration::from_secs(60);

pub struct Executor {
    pool: PgPool,
    client: Arc<HithinkClient>,
    registry: Arc<Registry>,
    queue: TaskQueue,
    fetcher: Fetcher,
    guard: TradeDateGuard,
    /// 推送里账户页链接的前缀（`NIULAI_PUBLIC_URL`）
    public_url: Option<String>,
}

/// 一个任务的执行结果（失败以 Err 返回）
enum Outcome {
    Done(TaskStats),
    Empty { pages: i32, note: Option<String> },
    Skipped(String),
    /// 只有当天值的数据已经错过，记为失败
    Missed(String),
    /// 数据尚未就绪：放回队列，推迟后再执行，不计入尝试次数
    Deferred(String),
}

impl Executor {
    pub fn new(pool: PgPool, client: Arc<HithinkClient>, registry: Arc<Registry>, worker_id: String, public_url: Option<String>) -> Self {
        Executor {
            public_url,
            queue: TaskQueue::new(pool.clone(), worker_id),
            fetcher: Fetcher::new(pool.clone(), client.clone(), registry.backoff_s.clone()),
            guard: TradeDateGuard::new(pool.clone(), client.clone()),
            pool,
            client,
            registry,
        }
    }

    /// 写入影响复权价的表（个股日线、公司行动）的数据集
    fn price_datasets(&self) -> Vec<String> {
        self.registry.writing_to(&super::spec::PRICE_TABLES).into_iter().map(|d| d.id.clone()).collect()
    }

    /// 循环认领并执行任务，直到收到停止信号；`until_idle` 为真时，整个队列空了就返回
    pub async fn run_loop(self: Arc<Self>, slot: usize, mut stop: watch::Receiver<bool>, until_idle: bool) -> anyhow::Result<()> {
        // 队列一直有任务时也要定期维护，所以按时间而不是按空闲轮数触发
        let mut last_maintenance: Option<std::time::Instant> = None;
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            if slot == 0 && last_maintenance.is_none_or(|t| t.elapsed() >= MAINTENANCE_EVERY) {
                last_maintenance = Some(std::time::Instant::now());
                if let Err(e) = self.maintain().await {
                    tracing::error!(error = %format!("{e:#}"), "队列维护失败");
                }
            }
            // 还没有配置 API Key：不认领任务，等用户在首次登录引导中配置（维护照常进行）
            if !self.client.has_api_key() {
                if until_idle {
                    anyhow::bail!("还没有配置 API Key：在网页的首次登录引导中配置，或设置环境变量 HITHINK_FINANCE_API_KEY");
                }
                tokio::select! {
                    _ = tokio::time::sleep(IDLE_POLL) => {}
                    _ = stop.changed() => {}
                }
                continue;
            }
            match self.queue.claim().await {
                Ok(Some(task)) => {
                    let task_id = task.task_id;
                    if let Err(e) = self.process(task).await {
                        tracing::error!(task_id, slot, error = %format!("{e:#}"), "任务处理异常");
                    }
                }
                Ok(None) => {
                    // 其他执行槽可能还有任务在跑，确认整个队列都空了再退出
                    if until_idle && !self.queue.has_open().await? {
                        return Ok(());
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(IDLE_POLL) => {}
                        _ = stop.changed() => {}
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "认领任务失败");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// 回收租约过期的任务，结束已经没有未完成任务、却没人结束的运行，补上没接上的波次，在回补期间定期重算复权因子，每天清理一次过期的任务和日志
    async fn maintain(&self) -> anyhow::Result<()> {
        let n = self.queue.requeue_stale().await?;
        if n > 0 {
            tracing::warn!(n, "回收了租约过期的任务");
        }
        for run_id in run::stalled(&self.pool).await? {
            self.after_task(run_id).await?;
        }
        for run_id in run::unchained(&self.pool).await? {
            if let Some(next) = Planner::new(&self.pool, &self.client, &self.registry).continue_chain(run_id).await? {
                tracing::warn!(run_id, next, "运行结束后没有接上下一个波次，已补上");
            }
        }
        postprocess::refresh_if_stale(&self.pool, &self.price_datasets()).await?;
        super::retention::prune_if_due(&self.pool).await?;
        // 调仓建议出错不影响同步
        if let Err(e) = crate::notify::push_new_advice(&self.pool, self.public_url.as_deref()).await {
            tracing::error!(error = %format!("{e:#}"), "推送运营建议失败");
        }
        if let Err(e) = crate::analysis::prices::in_background(crate::advice::maintain(&self.pool)).await {
            tracing::error!(error = %format!("{e:#}"), "调仓建议的维护失败");
        }
        Ok(())
    }

    // @project-doc docs/architecture/sync_pipeline.md#task_lifecycle
    pub async fn process(&self, task: ClaimedTask) -> anyhow::Result<()> {
        let Some(spec) = self.registry.get(&task.dataset_id) else {
            return self.queue.fail(&task, None, "数据集不在当前注册表中", true).await;
        };
        let started = std::time::Instant::now();
        let result = {
            let _lease = self.keep_lease(task.task_id);
            self.execute(&task, spec).await
        };
        match result {
            Ok(Outcome::Done(stats)) => {
                self.queue.finish(&task, Finished::Done, stats, None).await?;
                tracing::info!(task_id = task.task_id, dataset = %task.dataset_id, part = %task.part_key,
                               pages = stats.pages, records = stats.records, changed = stats.changed,
                               ms = started.elapsed().as_millis() as u64, "任务完成");
            }
            Ok(Outcome::Empty { pages, note }) => {
                self.queue.finish(&task, Finished::Empty, TaskStats { pages, ..Default::default() }, note.as_deref()).await?
            }
            Ok(Outcome::Skipped(why)) => {
                tracing::warn!(task_id = task.task_id, dataset = %task.dataset_id, reason = %why, "任务跳过");
                self.queue.finish(&task, Finished::Skipped, TaskStats::default(), Some(&why)).await?
            }
            Ok(Outcome::Missed(why)) => {
                // 只有当天值的数据错过了：记为失败（dead），在同步页和完整性核对中显示，而不是静默跳过
                tracing::error!(task_id = task.task_id, dataset = %task.dataset_id, reason = %why, "已错过当天的数据");
                self.queue.fail(&task, None, &why, true).await?;
            }
            Ok(Outcome::Deferred(why)) => {
                tracing::info!(task_id = task.task_id, dataset = %task.dataset_id, reason = %why, "数据尚未就绪，稍后再试");
                return self.queue.defer(&task, &why).await;
            }
            Err(e) => {
                let (code, retryable) = match e.downcast_ref::<CallError>() {
                    Some(c) => (c.code(), c.is_retryable()),
                    None => (None, false),
                };
                let msg = format!("{e:#}");
                tracing::warn!(task_id = task.task_id, dataset = %task.dataset_id, part = %task.part_key, error = %msg, retryable, "任务失败");
                // 不可重试的接口错误、配置或解析错误：直接标为 dead，等人处理
                self.queue.fail(&task, code, &msg, !retryable).await?;
            }
        }
        self.after_task(task.run_id).await
    }

    /// 这个运行写入了权威涨跌停池（special.limit_up_pool、limit_down_pool、limit_break_pool）的话，
    /// 更新 `ops.derived_version` 的 `limit_pool` 行：市场概况、信号过去表现的进程内缓存以它为版本号。
    /// 与行情是否变化无关：行情没变、只补上了滞后的涨停名单时，涨停数也要跟着变。返回池有没有变化
    async fn mark_limit_pool_changed(&self, run_id: i64) -> anyhow::Result<bool> {
        let changed: i64 = sqlx::query_scalar("SELECT coalesce(sum(changed), 0) FROM ops.task WHERE run_id = $1 AND dataset_id = ANY($2) AND status = 'done'")
            .bind(run_id)
            .bind(crate::analysis::limit_pool::DATASETS.to_vec())
            .fetch_one(&self.pool)
            .await?;
        if changed > 0 {
            sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = 'limit_pool'").execute(&self.pool).await?;
            tracing::info!(run_id, changed, "权威涨跌停池有变化，市场概况和信号统计的缓存已失效");
        }
        Ok(changed > 0)
    }

    /// 执行期间定期续租；返回的句柄被丢弃时停止
    fn keep_lease(&self, task_id: i64) -> AbortOnDrop {
        let queue = self.queue.clone();
        AbortOnDrop(tokio::spawn(async move {
            let mut tick = tokio::time::interval(HEARTBEAT);
            tick.tick().await;
            loop {
                tick.tick().await;
                match queue.heartbeat(task_id).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(task_id, worker = queue.worker_id(), "任务已不属于本执行者，停止续租");
                        return;
                    }
                    Err(e) => tracing::warn!(task_id, error = %e, "续租失败"),
                }
            }
        }))
    }

    // @project-doc docs/architecture/sync_pipeline.md#record_writes
    async fn execute(&self, task: &ClaimedTask, spec: &DatasetSpec) -> anyhow::Result<Outcome> {
        if spec.trade_date_guard {
            match self.guard.check(task.trade_date).await? {
                Readiness::Ready => {}
                Readiness::Skip(why) => return Ok(Outcome::Skipped(why)),
                Readiness::Missed(why) => return Ok(Outcome::Missed(why)),
                Readiness::Defer(why) => return Ok(Outcome::Deferred(why)),
            }
        }

        let empty = Map::new();
        let vars = task.params.get("vars").and_then(Value::as_object).unwrap_or(&empty);
        if let Some(d) = vars.get("date").and_then(Value::as_str).and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            && let Some(why) = calendar::stale_partition_date(&self.pool, spec, d).await?
        {
            return Ok(Outcome::Skipped(why));
        }
        let query: Vec<(String, String)> = task
            .params
            .get("query")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), template::value_to_string(v))).collect())
            .unwrap_or_default();
        let target = Target { task_id: task.task_id, part_key: &task.part_key, spec };
        let fetched = self.fetcher.fetch(&target, &query).await?;
        if fetched.records.is_empty() {
            return Ok(Outcome::Empty { pages: fetched.pages, note: fetched.empty_note });
        }

        let trade_date = task.trade_date.map(|d| d.to_string());
        let rows = records::build_rows(spec, vars, &fetched.records, trade_date.as_deref())?;
        let mut changed = if spec.store_record { records::store(&self.pool, &spec.id, task.task_id, &rows).await? } else { 0 };
        let ctx = sinks::SinkContext { dataset_id: &spec.id, run_id: task.run_id, trade_date: task.trade_date, vars };
        for sink in &spec.sink {
            let n = sinks::apply(&self.pool, sink, &ctx, &rows).await.with_context(|| format!("写入 {sink}"))?;
            if !spec.store_record {
                changed += n;
            }
        }
        cursor::advance(&self.pool, spec, task.task_id, vars).await?;
        Ok(Outcome::Done(TaskStats { pages: fetched.pages, records: rows.len(), changed }))
    }

    // @project-doc docs/architecture/sync_pipeline.md#postprocess
    /// 运行的所有任务都结束后，先规划下一个波次，再执行后续计算。只有结束运行的那个执行者会走到这里。
    /// 先接续：后续计算失败或进程中途退出，也不会让后面的波次停掉；万一接续本身失败，定期维护会重试（[`run::unchained`]）
    async fn after_task(&self, run_id: i64) -> anyhow::Result<()> {
        let Some(status) = run::try_complete(&self.pool, run_id).await? else { return Ok(()) };
        tracing::info!(run_id, %status, "运行结束，开始后处理");
        // 权威涨跌停池只写 raw.record、没有类型化表，不在 PRICE_TABLES 里；单独统计它的变化来更新版本号。
        // 更新失败不影响运行状态，只写日志（下次有变化的运行还会再更新）
        let pool_changed = match self.mark_limit_pool_changed(run_id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(run_id, error = %format!("{e:#}"), "更新权威涨跌停池的版本号失败");
                false
            }
        };
        if let Some(next) = Planner::new(&self.pool, &self.client, &self.registry).continue_chain(run_id).await? {
            tracing::info!(run_id, next, "已规划下一个波次");
        }
        // 行情有变化时，按最新交易日执行每日巡检，再为每个账户生成调仓建议；失败不影响同步。
        // 只有涨跌停池变了（例如当天的名单比行情晚到）也重做一次：之前的巡检用的是自算的涨停，名单到了以名单为准。
        // 已经处理过的建议不会被替换（refresh_all 只替换没处理过的）
        let price_changed = postprocess::after_run(&self.pool, run_id, &self.price_datasets()).await?;
        if price_changed || pool_changed {
            let pool = &self.pool;
            crate::analysis::prices::in_background(async {
                match crate::analysis::screener::run_and_store(pool, None, None).await {
                    Ok(_) => {
                        if let Err(e) = crate::advice::refresh_all(pool, true).await {
                            tracing::error!(run_id, error = %format!("{e:#}"), "生成调仓建议失败");
                        }
                    }
                    Err(e) => tracing::error!(run_id, error = %format!("{e:#}"), "每日巡检失败"),
                }
            })
            .await;
        }
        Ok(())
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
