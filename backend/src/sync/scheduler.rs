//! 定时调度：按同步计划对到期的数据集发起增量运行。时间都是上海时间，可以在网页上修改。
//!
//! - 每日收盘后：每个交易日到点同步日线、估值等每天都变的数据。
//! - 每周：每周固定一天到点同步交易日历、代码表、财务报表等；每月的数据跟每月第一次每周同步一起。
//! - 财报季的数据：财报季随每日同步，其余月份随每周同步。
//!
//! 多个进程同时运行时，用 ops.scheduler 上的条件 UPDATE 保证每个周期只触发一次。
//! 是否为交易日，以触发时现取的交易日历为准。

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::watch;
use utoipa::ToSchema;

use super::Engine;
use super::calendar;
use super::planner;
use super::run::RunKind;
use super::spec::{DatasetSpec, Schedule};
use crate::time::{cn_to_utc, now_cn};

/// 财报季：一季报（4 月）、半年报（7–8 月）、三季报（10 月）、年报与业绩预告（1 月，年报主要在 4 月）
const REPORT_SEASON_MONTHS: [u32; 5] = [1, 4, 7, 8, 10];

/// app.setting 中保存同步时间的行
const SETTING: &str = "sync_schedule";

pub async fn run(engine: Arc<Engine>, mut stop: watch::Receiver<bool>) -> anyhow::Result<()> {
    match load_times(&engine).await {
        Ok(times) => tracing::info!(daily = %times.daily_time, weekly = %times.weekly_rule(), "调度器已启动（上海时间）"),
        // 读不到时照常运行：每次检查都会重新读取
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "调度器已启动，同步时间暂时读不到"),
    }
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        // 还没有配置 API Key 时不发起增量；配置后当天到点照常触发
        if engine.client.has_api_key() {
            if let Err(e) = tick(&engine).await {
                tracing::error!(error = %format!("{e:#}"), "调度失败");
            }
            let today = now_cn().date();
            if let Err(e) = catch_up(&engine, today).await {
                tracing::error!(error = %format!("{e:#}"), "补发每日同步失败");
            }
            if let Err(e) = super::audit::auto_repair(&engine, today).await {
                tracing::error!(error = %format!("{e:#}"), "兜底补偿失败");
            }
            // 核检要几十秒，放到后台，不耽误下一次检查；同时只会有一次核检
            let e = engine.clone();
            tokio::spawn(async move {
                if let Err(err) = super::check::auto(&e).await {
                    tracing::error!(error = %format!("{err:#}"), "数据核检失败");
                }
            });
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            _ = stop.changed() => {}
        }
    }
}

// @project-doc docs/architecture/sync_pipeline.md#scheduling
/// 网页上可调的同步时间（上海时间）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ScheduleTimes {
    /// 每日收盘后：交易日的几点，HH:MM，例：18:00
    pub daily_time: String,
    /// 每周：星期几，1 = 周一 … 7 = 周日
    pub weekly_weekday: u8,
    /// 每周：几点，HH:MM，例：08:00
    pub weekly_time: String,
}

impl ScheduleTimes {
    /// 网页上没有改过时：每个交易日 18:00（收盘后 3 小时，当天数据基本都已出齐），每周六 08:00（周末供应商空闲，不和每日同步抢请求额度）
    fn defaults() -> Self {
        ScheduleTimes { daily_time: "18:00".into(), weekly_weekday: 6, weekly_time: "08:00".into() }
    }

    pub fn daily_at(&self) -> anyhow::Result<NaiveTime> {
        parse_hm(&self.daily_time).context("每日同步时间格式应为 HH:MM")
    }

    pub fn weekly_at(&self) -> anyhow::Result<NaiveTime> {
        parse_hm(&self.weekly_time).context("每周同步时间格式应为 HH:MM")
    }

    pub fn weekday(&self) -> anyhow::Result<Weekday> {
        match self.weekly_weekday {
            1..=7 => Ok(Weekday::try_from(self.weekly_weekday - 1).expect("0..=6")),
            n => anyhow::bail!("星期几应为 1 到 7：{n}"),
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.daily_at()?;
        self.weekly_at()?;
        self.weekday()?;
        Ok(())
    }

    fn weekly_rule(&self) -> String {
        format!("每{} {}", weekday_name(self.weekly_weekday), self.weekly_time)
    }
}

fn parse_hm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

fn weekday_name(n: u8) -> &'static str {
    ["周一", "周二", "周三", "周四", "周五", "周六", "周日"].get(usize::from(n.max(1)) - 1).copied().unwrap_or("周六")
}

/// 读取同步时间：网页上保存的优先，没有或无法解析时用默认值
pub async fn load_times(engine: &Engine) -> anyhow::Result<ScheduleTimes> {
    let saved: Option<String> = sqlx::query_scalar("SELECT value FROM app.setting WHERE name = $1").bind(SETTING).fetch_optional(&engine.pool).await?;
    Ok(saved
        .and_then(|v| serde_json::from_str::<ScheduleTimes>(&v).ok())
        .filter(|t| t.validate().is_ok())
        .unwrap_or_else(ScheduleTimes::defaults))
}

pub async fn save_times(pool: &PgPool, times: &ScheduleTimes) -> anyhow::Result<()> {
    times.validate()?;
    let normalized = ScheduleTimes {
        daily_time: times.daily_at()?.format("%H:%M").to_string(),
        weekly_weekday: times.weekly_weekday,
        weekly_time: times.weekly_at()?.format("%H:%M").to_string(),
    };
    sqlx::query(
        "INSERT INTO app.setting (name, value) VALUES ($1, $2)
         ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(SETTING)
    .bind(serde_json::to_string(&normalized)?)
    .execute(pool)
    .await?;
    Ok(())
}

fn in_report_season(d: NaiveDate) -> bool {
    REPORT_SEASON_MONTHS.contains(&d.month())
}

/// 每日同步包括的数据集：每天的，以及财报季里的财报季数据
pub fn due_daily(schedule: Schedule, today: NaiveDate) -> bool {
    match schedule {
        Schedule::Daily => true,
        Schedule::ReportSeason => in_report_season(today),
        _ => false,
    }
}

/// 每周同步包括的数据集：每周的、财报季以外月份的财报季数据，以及本月第一次每周同步时的每月数据
pub fn due_weekly(schedule: Schedule, today: NaiveDate, monthly: bool) -> bool {
    match schedule {
        Schedule::Weekly => true,
        Schedule::ReportSeason => !in_report_season(today),
        Schedule::Monthly => monthly,
        _ => false,
    }
}

/// 抢占前作业的水位。抢到之后规划失败时恢复它，下一次检查会重试，不会因为一次数据库抖动丢掉这个周期
struct Mark(Vec<(String, NaiveDate, Option<i64>, DateTime<Utc>)>, Vec<String>);

async fn mark(pool: &PgPool, jobs: &[&str]) -> anyhow::Result<Mark> {
    let jobs: Vec<String> = jobs.iter().map(|j| j.to_string()).collect();
    let rows = sqlx::query_as("SELECT job, last_fired, run_id, fired_at FROM ops.scheduler WHERE job = ANY($1)").bind(&jobs).fetch_all(pool).await?;
    Ok(Mark(rows, jobs))
}

impl Mark {
    async fn restore(self, pool: &PgPool) {
        let res = async {
            let mut tx = pool.begin().await?;
            sqlx::query("DELETE FROM ops.scheduler WHERE job = ANY($1)").bind(&self.1).execute(&mut *tx).await?;
            for (job, last_fired, run_id, fired_at) in &self.0 {
                sqlx::query("INSERT INTO ops.scheduler (job, last_fired, run_id, fired_at) VALUES ($1, $2, $3, $4)")
                    .bind(job)
                    .bind(last_fired)
                    .bind(run_id)
                    .bind(fired_at)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            anyhow::Ok(())
        }
        .await;
        if let Err(e) = res {
            tracing::error!(jobs = ?self.1, error = %format!("{e:#}"), "恢复调度水位失败，这个周期的同步可能要手动发起");
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

/// 抢占一个定时时刻：上次触发早于这个时刻时，只有一个调用者能成功。
/// 第一次（还没有这一行）只记下现在，不触发：刚部署的系统正在做首次回补，不必马上再同步一遍
async fn claim_slot(pool: &PgPool, job: &str, slot: NaiveDateTime) -> anyhow::Result<bool> {
    let row: Option<bool> = sqlx::query_scalar(
        "INSERT INTO ops.scheduler (job, last_fired) VALUES ($1, $2)
         ON CONFLICT (job) DO UPDATE SET last_fired = EXCLUDED.last_fired, fired_at = now()
         WHERE ops.scheduler.fired_at < $3
         RETURNING xmax::text <> '0'",
    )
    .bind(job)
    .bind(slot.date())
    .bind(cn_to_utc(slot))
    .fetch_optional(pool)
    .await?;
    Ok(row == Some(true))
}

/// 不晚于 now 的最近一个每周时刻
pub fn last_weekly_slot(now: NaiveDateTime, weekday: Weekday, at: NaiveTime) -> NaiveDateTime {
    let back = (now.weekday().num_days_from_monday() + 7 - weekday.num_days_from_monday()) % 7;
    let slot = (now.date() - chrono::Duration::days(i64::from(back))).and_time(at);
    if slot > now { slot - chrono::Duration::days(7) } else { slot }
}

async fn tick(engine: &Engine) -> anyhow::Result<()> {
    let times = load_times(engine).await?;
    let now = now_cn();
    if now.time() >= times.daily_at()? {
        daily_tick(engine, now.date()).await?;
    }
    weekly_tick(engine, &times, now).await
}

/// 每日收盘后：交易日到点触发一次
async fn daily_tick(engine: &Engine, today: NaiveDate) -> anyhow::Result<()> {
    if today.weekday().number_from_monday() > 5 {
        return Ok(());
    }
    let before = mark(&engine.pool, &["daily"]).await?;
    if !claim(&engine.pool, "daily", today, "day").await? {
        return Ok(());
    }
    let res = fire_daily(engine, today).await;
    if res.is_err() {
        before.restore(&engine.pool).await;
    }
    res
}

async fn fire_daily(engine: &Engine, today: NaiveDate) -> anyhow::Result<()> {
    if !is_trading_day(engine, today).await? {
        tracing::info!(%today, "非交易日，跳过");
        return Ok(());
    }
    let due: Vec<&DatasetSpec> = engine.registry.datasets.iter().filter(|d| d.enabled && due_daily(d.schedule, today)).collect();
    let opts = planner::PlanOptions { trade_date: Some(today), trigger: Some("daily".into()), ..Default::default() };
    let (run_id, summary) = engine.planner().plan_run(RunKind::Incremental, &due, opts).await?;
    sqlx::query("UPDATE ops.scheduler SET run_id = $2 WHERE job = $1").bind("daily").bind(run_id).execute(&engine.pool).await?;
    let tasks: usize = summary.iter().map(|(_, n)| n).sum();
    tracing::info!(run_id, datasets = due.len(), tasks, "每日同步已规划");
    Ok(())
}

/// 每周：到了每周的时刻、这个时刻还没触发过就触发，不论是不是交易日。服务停机错过时，启动后立即补上
async fn weekly_tick(engine: &Engine, times: &ScheduleTimes, now: NaiveDateTime) -> anyhow::Result<()> {
    let slot = last_weekly_slot(now, times.weekday()?, times.weekly_at()?);
    let before = mark(&engine.pool, &["weekly", "monthly"]).await?;
    if !claim_slot(&engine.pool, "weekly", slot).await? {
        return Ok(());
    }
    let res = fire_weekly(engine, now.date()).await;
    if res.is_err() {
        before.restore(&engine.pool).await;
    }
    res
}

async fn fire_weekly(engine: &Engine, today: NaiveDate) -> anyhow::Result<()> {
    let monthly = claim(&engine.pool, "monthly", today, "month").await?;
    // 运行的交易日取最近一个交易日：按天积累的数据（例如指数成分）记在这一天
    let trade_date: Option<NaiveDate> =
        sqlx::query_scalar("SELECT max(cal_date) FROM ref.trading_day WHERE cal_date <= $1").bind(today).fetch_one(&engine.pool).await?;
    let due: Vec<&DatasetSpec> = engine.registry.datasets.iter().filter(|d| d.enabled && due_weekly(d.schedule, today, monthly)).collect();
    let trigger = if monthly { "weekly+monthly" } else { "weekly" };
    let opts = planner::PlanOptions { trade_date: Some(trade_date.unwrap_or(today)), trigger: Some(trigger.into()), ..Default::default() };
    let (run_id, summary) = engine.planner().plan_run(RunKind::Incremental, &due, opts).await?;
    sqlx::query("UPDATE ops.scheduler SET run_id = $1 WHERE job = 'weekly' OR (job = 'monthly' AND $2)").bind(run_id).bind(monthly).execute(&engine.pool).await?;
    let tasks: usize = summary.iter().map(|(_, n)| n).sum();
    tracing::info!(run_id, datasets = due.len(), tasks, monthly, "每周同步已规划");
    Ok(())
}

/// 补发错过的每日同步：上一个交易日的每日同步没有发起过（例如那天到点后服务一直没有运行），立即补发。
/// 只补每日数据集；只有当天值的数据能不能取到由交易日校验决定，取不到时记为「已错过」。
/// 只往前看一个交易日：更早的日子，只有当天值的数据已经无法获取，其余数据由每周同步和完整性核对补齐
async fn catch_up(engine: &Engine, today: NaiveDate) -> anyhow::Result<()> {
    let prev: Option<NaiveDate> = sqlx::query_scalar("SELECT max(cal_date) FROM ref.trading_day WHERE cal_date < $1 AND cal_date >= $1 - 10")
        .bind(today)
        .fetch_one(&engine.pool)
        .await?;
    let Some(prev) = prev else { return Ok(()) };
    // 系统还没开始每日同步（例如刚部署、还在首次回补）时不补
    let last: Option<NaiveDate> = sqlx::query_scalar("SELECT last_fired FROM ops.scheduler WHERE job = 'daily'").fetch_optional(&engine.pool).await?;
    if last.is_none_or(|l| l >= prev) {
        return Ok(());
    }
    let before = mark(&engine.pool, &["daily"]).await?;
    if !claim(&engine.pool, "daily", prev, "day").await? {
        return Ok(());
    }
    let due: Vec<&DatasetSpec> = engine.registry.datasets.iter().filter(|d| d.enabled && due_daily(d.schedule, prev)).collect();
    let opts = planner::PlanOptions { trade_date: Some(prev), trigger: Some("catch_up".into()), ..Default::default() };
    let (run_id, summary) = match engine.planner().plan_run(RunKind::Incremental, &due, opts).await {
        Ok(r) => r,
        Err(e) => {
            before.restore(&engine.pool).await;
            return Err(e);
        }
    };
    let tasks: usize = summary.iter().map(|(_, n)| n).sum();
    tracing::warn!(run_id, trade_date = %prev, last_fired = ?last, tasks, "上一个交易日的每日同步没有发起过，已补发");
    Ok(())
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

/// 同步计划总览：时间、规则、包含的数据集、下次和上次同步
#[derive(Serialize, ToSchema)]
pub struct SyncSchedule {
    pub times: ScheduleTimes,
    pub plans: Vec<SyncPlan>,
}

#[derive(Serialize, ToSchema)]
pub struct SyncPlan {
    /// daily / weekly / monthly / report_season / manual
    pub key: String,
    pub name: String,
    /// 什么时候同步，白话
    pub rule: String,
    /// 包含的已开启数据集
    pub datasets: Vec<String>,
    /// 下次自动同步的时刻；不自动同步时为空
    pub next_at: Option<DateTime<Utc>>,
    /// 上次自动同步发起的时刻和运行
    pub last_at: Option<DateTime<Utc>>,
    pub last_run_id: Option<i64>,
    /// 上次同步（含后续波次）的状态：running / succeeded / partial
    pub last_status: Option<String>,
}

pub async fn overview(engine: &Engine) -> anyhow::Result<SyncSchedule> {
    let times = load_times(engine).await?;
    let now = now_cn();
    let (daily_at, weekly_at, weekday) = (times.daily_at()?, times.weekly_at()?, times.weekday()?);

    let fired_today: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ops.scheduler WHERE job = 'daily' AND last_fired >= $1)")
        .bind(now.date())
        .fetch_one(&engine.pool)
        .await?;
    let from = if now.time() < daily_at && !fired_today { now.date() } else { now.date() + chrono::Duration::days(1) };
    let next_daily = next_trading_day(&engine.pool, from).await?.and_time(daily_at);
    let next_weekly = last_weekly_slot(now, weekday, weekly_at) + chrono::Duration::days(7);
    let monthly_fired: Option<NaiveDate> = sqlx::query_scalar("SELECT last_fired FROM ops.scheduler WHERE job = 'monthly'").fetch_optional(&engine.pool).await?;
    let mut next_monthly = next_weekly;
    while monthly_fired.is_some_and(|m| (m.year(), m.month()) == (next_monthly.year(), next_monthly.month())) {
        next_monthly += chrono::Duration::days(7);
    }
    let next_season = if in_report_season(next_daily.date()) && next_daily < next_weekly { next_daily } else { next_weekly };
    let weekly_rule = times.weekly_rule();

    let mut plans = vec![];
    for plan in Schedule::ALL {
        let (rule, next, job) = match plan {
            Schedule::Daily => (format!("每个交易日 {}", times.daily_time), Some(next_daily), Some("daily")),
            Schedule::Weekly => (weekly_rule.clone(), Some(next_weekly), Some("weekly")),
            Schedule::Monthly => (format!("每月第一次每周同步时（当月第一个{} {}）", weekday_name(times.weekly_weekday), times.weekly_time), Some(next_monthly), Some("monthly")),
            Schedule::ReportSeason => ("1、4、7、8、10 月随每日同步，其余月份随每周同步".to_string(), Some(next_season), None),
            Schedule::Manual => ("不自动同步，需要时手动发起".to_string(), None, None),
        };
        let datasets: Vec<String> = engine.registry.datasets.iter().filter(|d| d.enabled && d.schedule == plan).map(|d| d.id.clone()).collect();
        let (last_at, last_run_id, last_status) = match job {
            Some(job) => last_fired(&engine.pool, job).await?,
            None => (None, None, None),
        };
        plans.push(SyncPlan {
            key: plan.key().into(),
            name: plan.name().into(),
            rule,
            datasets,
            next_at: next.map(cn_to_utc),
            last_at,
            last_run_id,
            last_status,
        });
    }
    Ok(SyncSchedule { times, plans })
}

/// 不早于 from 的第一个交易日：本地日历没有覆盖到时按周一到周五算
async fn next_trading_day(pool: &PgPool, from: NaiveDate) -> anyhow::Result<NaiveDate> {
    let known: Option<NaiveDate> = sqlx::query_scalar("SELECT min(cal_date) FROM ref.trading_day WHERE cal_date >= $1").bind(from).fetch_one(pool).await?;
    if let Some(d) = known {
        return Ok(d);
    }
    let mut d = from;
    while d.weekday().number_from_monday() > 5 {
        d += chrono::Duration::days(1);
    }
    Ok(d)
}

type LastFired = (Option<DateTime<Utc>>, Option<i64>, Option<String>);

async fn last_fired(pool: &PgPool, job: &str) -> anyhow::Result<LastFired> {
    let row: Option<(DateTime<Utc>, Option<i64>)> = sqlx::query_as("SELECT fired_at, run_id FROM ops.scheduler WHERE job = $1").bind(job).fetch_optional(pool).await?;
    let Some((at, run_id)) = row else { return Ok((None, None, None)) };
    let status = match run_id {
        Some(id) => sqlx::query_scalar(
            "SELECT CASE WHEN bool_or(status = 'running') THEN 'running' WHEN bool_or(status = 'partial') THEN 'partial' ELSE 'succeeded' END
             FROM ops.run WHERE run_id = $1 OR (params->>'root_run')::bigint = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await?,
        None => None,
    };
    Ok((Some(at), run_id, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn report_season_runs_daily_in_season_and_weekly_otherwise() {
        let oct = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let nov = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        assert!(due_daily(Schedule::ReportSeason, oct));
        assert!(!due_weekly(Schedule::ReportSeason, oct, false));
        assert!(!due_daily(Schedule::ReportSeason, nov));
        assert!(due_weekly(Schedule::ReportSeason, nov, false));
        assert!(!due_daily(Schedule::Weekly, nov));
        assert!(!due_weekly(Schedule::Monthly, nov, false));
        assert!(due_weekly(Schedule::Monthly, nov, true));
        assert!(!due_daily(Schedule::Manual, nov) && !due_weekly(Schedule::Manual, nov, true));
    }

    #[test]
    fn weekly_slot_is_the_latest_one_not_after_now() {
        let t = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        // 2026-10-17 是周六
        assert_eq!(last_weekly_slot(at("2026-10-17 09:00"), Weekday::Sat, t), at("2026-10-17 08:00"));
        assert_eq!(last_weekly_slot(at("2026-10-17 07:59"), Weekday::Sat, t), at("2026-10-10 08:00"));
        assert_eq!(last_weekly_slot(at("2026-10-17 08:00"), Weekday::Sat, t), at("2026-10-17 08:00"));
        assert_eq!(last_weekly_slot(at("2026-10-19 10:00"), Weekday::Sat, t), at("2026-10-17 08:00"));
        assert_eq!(last_weekly_slot(at("2026-10-19 10:00"), Weekday::Mon, t), at("2026-10-19 08:00"));
    }

    #[test]
    fn times_are_validated() {
        let t = |d: &str, w: u8, wt: &str| ScheduleTimes { daily_time: d.into(), weekly_weekday: w, weekly_time: wt.into() };
        assert!(t("18:00", 6, "08:00").validate().is_ok());
        assert!(t("25:00", 6, "08:00").validate().is_err());
        assert!(t("18:00", 0, "08:00").validate().is_err());
        assert!(t("18:00", 8, "08:00").validate().is_err());
        assert!(t("18:00", 7, "8点").validate().is_err());
    }
}
