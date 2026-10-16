//! 定时调度：每天收盘后（默认 18:00，Asia/Shanghai）对到期的数据集发起一次增量运行。
//!
//! 多个 worker 同时运行时，用 ops.scheduler 上的条件 UPDATE 保证每个周期只触发一次。
//! 是否为交易日，以触发时现取的交易日历为准。

use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::watch;

use super::Engine;
use super::calendar;
use super::planner;
use super::run::RunKind;
use super::spec::{DatasetSpec, Schedule};
use crate::time::now_cn;

/// 财报季：一季报（4 月）、半年报（7–8 月）、三季报（10 月）、年报与业绩预告（1 月，年报主要在 4 月）
const REPORT_SEASON_MONTHS: [u32; 5] = [1, 4, 7, 8, 10];

pub async fn run(engine: Arc<Engine>, mut stop: watch::Receiver<bool>) -> anyhow::Result<()> {
    let at = engine.settings.after_close_time()?;
    tracing::info!(time = %at, "调度器已启动");
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let now = now_cn();
        // 还没有配置 API Key 时不发起增量；配置后当天到点照常触发
        if engine.client.has_api_key() {
            if now.time() >= at
                && let Err(e) = tick(&engine, now.date()).await
            {
                tracing::error!(error = %format!("{e:#}"), "调度失败");
            }
            if let Err(e) = catch_up(&engine, now.date()).await {
                tracing::error!(error = %format!("{e:#}"), "补发每日同步失败");
            }
            if let Err(e) = super::audit::auto_repair(&engine, now.date()).await {
                tracing::error!(error = %format!("{e:#}"), "兜底补偿失败");
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            _ = stop.changed() => {}
        }
    }
}

/// 每个周期只有一个调用者能把 last_fired 推进到新周期，返回 true 的那个负责触发
pub(crate) async fn claim(pool: &PgPool, job: &str, today: NaiveDate, period: &str) -> anyhow::Result<bool> {
    let sql = format!(
        "INSERT INTO ops.scheduler (job, last_fired) VALUES ($1, $2)
         ON CONFLICT (job) DO UPDATE SET last_fired = EXCLUDED.last_fired, fired_at = now()
         WHERE date_trunc('{period}', ops.scheduler.last_fired) < date_trunc('{period}', EXCLUDED.last_fired)
         RETURNING job"
    );
    // 审计：period 只会是代码中的 day / week / month
    Ok(sqlx::query_scalar::<_, String>(AssertSqlSafe(sql)).bind(job).bind(today).fetch_optional(pool).await?.is_some())
}

// @project-doc docs/architecture/sync_pipeline.md#scheduling
async fn tick(engine: &Engine, today: NaiveDate) -> anyhow::Result<()> {
    if today.weekday().number_from_monday() > 5 || !claim(&engine.pool, "after_close", today, "day").await? {
        return Ok(());
    }
    if !is_trading_day(engine, today).await? {
        tracing::info!(%today, "非交易日，跳过");
        return Ok(());
    }
    let weekly = claim(&engine.pool, "weekly", today, "week").await?;
    let monthly = claim(&engine.pool, "monthly", today, "month").await?;
    let due: Vec<&DatasetSpec> = engine
        .registry
        .datasets
        .iter()
        .filter(|d| d.enabled && is_due(d.schedule, today, weekly, monthly))
        .collect();
    let (run_id, summary) = engine.planner().plan_run(RunKind::Incremental, &due, planner::PlanOptions { trade_date: Some(today), ..Default::default() }).await?;
    sqlx::query("UPDATE ops.scheduler SET run_id = $2 WHERE job = $1").bind("after_close").bind(run_id).execute(&engine.pool).await?;
    let tasks: usize = summary.iter().map(|(_, n)| n).sum();
    tracing::info!(run_id, datasets = due.len(), tasks, weekly, monthly, "每日增量已规划");
    Ok(())
}

/// 补发错过的每日同步：上一个交易日的每日同步没有发起过（例如那天 18:00 到现在服务一直没有运行），立即补发。
/// 只补每日数据集；只有当天值的数据能不能取到由交易日校验决定，取不到时记为「已错过」。
/// 只往前看一个交易日：更早的日子，只有当天值的数据已经无法获取，其余数据由每周增量和完整性核对补齐
async fn catch_up(engine: &Engine, today: NaiveDate) -> anyhow::Result<()> {
    let prev: Option<NaiveDate> = sqlx::query_scalar("SELECT max(cal_date) FROM ref.trading_day WHERE cal_date < $1 AND cal_date >= $1 - 10")
        .bind(today)
        .fetch_one(&engine.pool)
        .await?;
    let Some(prev) = prev else { return Ok(()) };
    // 系统还没开始每日同步（例如刚部署、还在首次回补）时不补
    let last: Option<NaiveDate> = sqlx::query_scalar("SELECT last_fired FROM ops.scheduler WHERE job = 'after_close'").fetch_optional(&engine.pool).await?;
    if last.is_none_or(|l| l >= prev) || !claim(&engine.pool, "after_close", prev, "day").await? {
        return Ok(());
    }
    let due: Vec<&DatasetSpec> = engine.registry.datasets.iter().filter(|d| d.enabled && d.schedule.is_daily()).collect();
    let (run_id, summary) = engine.planner().plan_run(RunKind::Incremental, &due, planner::PlanOptions { trade_date: Some(prev), ..Default::default() }).await?;
    let tasks: usize = summary.iter().map(|(_, n)| n).sum();
    tracing::warn!(run_id, trade_date = %prev, last_fired = ?last, tasks, "上一个交易日的每日同步没有发起过，已补发");
    Ok(())
}

pub fn is_due(schedule: Schedule, today: NaiveDate, weekly: bool, monthly: bool) -> bool {
    match schedule {
        Schedule::AfterClose | Schedule::Daily => true,
        Schedule::Weekly => weekly,
        Schedule::Monthly => monthly,
        Schedule::ReportSeason => REPORT_SEASON_MONTHS.contains(&today.month()) || weekly,
        // 开盘后的调度尚未启用：竞价终态在收盘后同样可取
        Schedule::AfterOpen => false,
    }
}

/// 现取交易日历并写入 ref.trading_day，再判断今天是否为交易日
async fn is_trading_day(engine: &Engine, today: NaiveDate) -> anyhow::Result<bool> {
    match calendar::refresh(&engine.pool, &engine.client).await {
        Ok(dates) => Ok(dates.contains(&today)),
        Err(e) => {
            // 日历取不到时退回本地记录；本地也没有，就按交易日处理（执行快照类任务时还会再校验）
            tracing::warn!(error = %format!("{e:#}"), "获取交易日历失败，使用本地记录");
            let known = calendar::is_known_trading_day(&engine.pool, today).await?;
            let any = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM ref.trading_day WHERE cal_date >= $1)").bind(today).fetch_one(&engine.pool).await?;
            Ok(known || !any)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_season_runs_daily_in_season_and_weekly_otherwise() {
        let oct = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let nov = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        assert!(is_due(Schedule::ReportSeason, oct, false, false));
        assert!(!is_due(Schedule::ReportSeason, nov, false, false));
        assert!(is_due(Schedule::ReportSeason, nov, true, false));
        assert!(!is_due(Schedule::Weekly, nov, false, true));
    }
}
