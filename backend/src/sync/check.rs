//! 数据核检：按证券 × 交易日检查个股日线和指数日线有没有缺。
//!
//! 每日的完整性核对（audit）只看最近 20 个交易日、全市场条数，个别股票缺了某天、更早的历史有洞都查不出来。
//! 这里逐只证券、逐个交易日对照：
//! 1. 从上市日（或数据集的起点）到最近一个交易日，交易日历里有、日线表里没有的日子就是缺口。
//! 2. 停牌日供应商本来就不返回日线，单看缺口分不清是漏了还是停牌，所以发现缺口后用历史接口把那几天重新取一次。
//! 3. 下次核检时：补上了就删掉；重取的任务成功结束、接口仍然没有这一天，确认为停牌，以后不再报。
//!
//! 正在同步这只证券（有未完成任务）时先不查它，等它同步完。

use std::collections::BTreeMap;

use anyhow::Context;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{AssertSqlSafe, PgConnection, PgPool};
use utoipa::ToSchema;

use super::Engine;
use super::planner::{PlanOptions, Window};
use super::run::RunKind;
use super::spec::{DatasetSpec, Partition};
use crate::time::today_cn;

/// 同时只做一次核检的独占权：数据库咨询锁 `niulai.data_check`，用一个脱离连接池的独立连接持有，
/// 所以多个进程（`serve`、`serve --no-worker`、`niulai check`）之间也只有一个在核检；进程退出时连接关闭、锁自动释放
pub struct CheckLock(PgConnection);

/// 取核检的独占权；另一次核检正在进行时返回 None
pub async fn try_lock(pool: &PgPool) -> anyhow::Result<Option<CheckLock>> {
    let mut conn = pool.acquire().await?.detach();
    let got: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtext('niulai.data_check'))")
            .fetch_one(&mut conn)
            .await?;
    Ok(got.then_some(CheckLock(conn)))
}

/// 有没有核检正在进行（任何进程）：试着取一下独占权，取到了立即放回
pub async fn is_running(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT CASE WHEN pg_try_advisory_lock(hashtext('niulai.data_check'))
                     THEN NOT pg_advisory_unlock(hashtext('niulai.data_check')) ELSE true END",
    )
    .fetch_one(pool)
    .await?)
}

impl CheckLock {
    async fn release(self) {
        use sqlx::Connection;
        // 关闭连接就释放了会话级的咨询锁
        if let Err(e) = self.0.close().await {
            tracing::warn!(error = %e, "关闭核检独占连接失败");
        }
    }
}

/// 一个数据集的核检结果
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CheckResult {
    pub dataset_id: String,
    pub title: String,
    /// 查了多少只证券、多少个「证券 × 交易日」
    pub securities: i64,
    pub security_days: i64,
    /// 正在同步、这次先不查的证券
    pub busy: i64,
    /// 这次新发现的缺口
    pub new_gaps: i64,
    /// 以前发现的缺口，这次已经有数据了
    pub filled: i64,
    /// 这次确认为停牌的，以及累计确认为停牌的
    pub confirmed_now: i64,
    pub suspended: i64,
    /// 还缺、正在或将要重取的
    pub open: i64,
    /// 这次为缺口安排的重取：运行编号和任务数
    pub refetch_run: Option<i64>,
    pub refetch_tasks: i64,
}

// @project-doc docs/architecture/sync_pipeline.md#data_check
/// 执行一次核检。另一次核检正在进行时返回 None
pub async fn run(engine: &Engine, trigger: &str) -> anyhow::Result<Option<i64>> {
    let Some(lock) = try_lock(&engine.pool).await? else {
        return Ok(None);
    };
    run_with(engine, trigger, lock).await.map(Some)
}

/// 已经取得独占权时执行一次核检（例如接口先取锁、再交给后台任务），结束后释放
pub async fn run_with(engine: &Engine, trigger: &str, lock: CheckLock) -> anyhow::Result<i64> {
    let out = run_locked(engine, trigger).await;
    lock.release().await;
    out
}

async fn run_locked(engine: &Engine, trigger: &str) -> anyhow::Result<i64> {
    let end: Option<NaiveDate> =
        sqlx::query_scalar("SELECT max(cal_date) FROM ref.trading_day WHERE cal_date < $1")
            .bind(today_cn())
            .fetch_one(&engine.pool)
            .await?;
    // 上一次核检没有做完（程序中途退出）：记下来，页面上不再显示为进行中
    sqlx::query("UPDATE ops.data_check SET finished_at = now(), error = '没有做完：程序中途退出了' WHERE finished_at IS NULL")
        .execute(&engine.pool)
        .await?;
    let check_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.data_check (trigger, end_date) VALUES ($1, $2) RETURNING check_id",
    )
    .bind(trigger)
    .bind(end)
    .fetch_one(&engine.pool)
    .await?;
    let started = std::time::Instant::now();
    let mut results = vec![];
    let mut error = None;
    if let Some(end) = end {
        // 核检的是按证券和时间窗口取历史日线的数据集（个股日线、指数日线），由配置的分区和 sink 决定
        for (spec, table) in engine
            .registry
            .bar_histories()
            .into_iter()
            .filter(|(s, _)| s.enabled)
        {
            let id = &spec.id;
            match check_dataset(engine, spec, table, end).await {
                Ok(r) => results.push(r),
                Err(e) => {
                    tracing::error!(dataset = id, error = %format!("{e:#}"), "数据核检失败");
                    error = Some(format!("{id}：{e:#}"));
                }
            }
        }
    }
    sqlx::query("UPDATE ops.data_check SET finished_at = now(), result = $2, error = $3 WHERE check_id = $1")
        .bind(check_id)
        .bind(json!(results))
        .bind(&error)
        .execute(&engine.pool)
        .await?;
    tracing::info!(check_id, trigger, ms = started.elapsed().as_millis() as u64, results = %json!(results), "数据核检完成");
    Ok(check_id)
}

async fn check_dataset(
    engine: &Engine,
    spec: &DatasetSpec,
    table: &str,
    end: NaiveDate,
) -> anyhow::Result<CheckResult> {
    let pool = &engine.pool;
    let Partition::SecurityWindow {
        universe,
        floor,
        start_from_list_date,
        ..
    } = &spec.partition
    else {
        anyhow::bail!("{} 不是按证券和时间窗口同步的数据集", spec.id);
    };
    let floor = floor.resolve(today_cn());
    let busy: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT split_part(split_part(part_key, '|', 1), '=', 2) FROM ops.task
         WHERE dataset_id = $1 AND status IN ('pending', 'running', 'failed')",
    )
    .bind(&spec.id)
    .fetch_all(pool)
    .await?;

    // 每只证券的检查起点：数据集起点和上市日中较晚的；不知道上市日（指数）时从第一条日线开始，一条都没有就不查。
    // 终点：已推断退市的证券查到退市日前一天，之后供应商不再认这个代码（1002），算成缺口只会每次核检都重取失败
    let ranges = format!(
        "WITH sec AS (
           SELECT u.thscode,
                  greatest($2::date, st.base) lo,
                  least($1::date, coalesce(i.delisted_on - 1, $1::date)) hi
           FROM (SELECT DISTINCT ON (thscode) thscode, list_date FROM ops.v_universe WHERE thscode IS NOT NULL AND ({universe})) u
           LEFT JOIN ref.instrument i ON i.thscode = u.thscode
           LEFT JOIN LATERAL (SELECT min(b.trade_date) first FROM {table} b WHERE b.thscode = u.thscode) f ON true
           -- 上市日和第一条日线都没有（例如还没上市的新股）时 base 为空，不查；greatest 会忽略空值，所以先单独算
           CROSS JOIN LATERAL (SELECT CASE WHEN $3 THEN coalesce(u.list_date, f.first) ELSE f.first END base) st
           WHERE st.base IS NOT NULL AND NOT (u.thscode = ANY($4)))"
    );
    // 审计：universe 来自受信任的配置文件 config/datasets.yaml；table 是本文件中的常量
    let (securities, security_days): (i64, Option<i64>) = sqlx::query_as(AssertSqlSafe(format!(
        "{ranges}
         SELECT count(*), sum((SELECT count(*) FROM ref.trading_day d WHERE d.cal_date BETWEEN s.lo AND s.hi))::bigint
         FROM sec s WHERE s.lo IS NOT NULL AND s.lo <= s.hi"
    )))
    .bind(end)
    .bind(floor)
    .bind(*start_from_list_date)
    .bind(&busy)
    .fetch_one(pool)
    .await
    .with_context(|| format!("统计 {} 的检查范围失败", spec.id))?;
    // 审计：同上
    let gaps: Vec<(String, NaiveDate)> = sqlx::query_as(AssertSqlSafe(format!(
        "{ranges}
         SELECT s.thscode, d.cal_date FROM sec s JOIN ref.trading_day d ON d.cal_date BETWEEN s.lo AND s.hi
         WHERE NOT EXISTS (SELECT 1 FROM {table} b WHERE b.thscode = s.thscode AND b.trade_date = d.cal_date)
         ORDER BY 1, 2"
    )))
    .bind(end)
    .bind(floor)
    .bind(*start_from_list_date)
    .bind(&busy)
    .fetch_all(pool)
    .await
    .with_context(|| format!("查找 {} 的缺口失败", spec.id))?;
    let codes: Vec<&str> = gaps.iter().map(|(c, _)| c.as_str()).collect();
    let dates: Vec<NaiveDate> = gaps.iter().map(|(_, d)| *d).collect();

    let mut tx = pool.begin().await?;
    // 以前的缺口这次已经有数据了（或证券已经不在范围内）：删掉。正在同步的证券先不动
    let filled = sqlx::query(
        "DELETE FROM ops.bar_gap g WHERE g.dataset_id = $1 AND NOT (g.thscode = ANY($4))
           AND NOT EXISTS (SELECT 1 FROM unnest($2::text[], $3::date[]) x(c, d) WHERE x.c = g.thscode AND x.d = g.trade_date)",
    )
    .bind(&spec.id)
    .bind(&codes)
    .bind(&dates)
    .bind(&busy)
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;
    let new_gaps = sqlx::query(
        "INSERT INTO ops.bar_gap (dataset_id, thscode, trade_date, status)
         SELECT $1, c, d, 'open' FROM unnest($2::text[], $3::date[]) x(c, d)
         ON CONFLICT DO NOTHING",
    )
    .bind(&spec.id)
    .bind(&codes)
    .bind(&dates)
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;
    // 发现缺口之后，有覆盖这一天的任务成功结束，这一天仍然没有数据：接口确实没有，是停牌
    let confirmed_now = sqlx::query(
        "UPDATE ops.bar_gap g SET status = 'suspended'
         FROM (SELECT split_part(split_part(part_key, '|', 1), '=', 2) code,
                      (params->'vars'->>'start')::date s, (params->'vars'->>'end')::date e, finished_at
               FROM ops.task
               WHERE dataset_id = $1 AND status IN ('done', 'empty') AND params->'vars' ? 'start'
                 AND finished_at > (SELECT min(found_at) FROM ops.bar_gap WHERE dataset_id = $1 AND status = 'open')) t
         WHERE g.dataset_id = $1 AND g.status = 'open' AND t.code = g.thscode
           AND g.trade_date BETWEEN t.s AND t.e AND t.finished_at > g.found_at",
    )
    .bind(&spec.id)
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;
    tx.commit().await?;

    // 还缺、没有安排重取（或上次重取的运行已经结束但没能确认，例如请求失败）的，安排重取
    let todo: Vec<(String, NaiveDate)> = sqlx::query_as(
        "SELECT g.thscode, g.trade_date FROM ops.bar_gap g
         WHERE g.dataset_id = $1 AND g.status = 'open'
           AND (g.refetch_run IS NULL OR NOT EXISTS (SELECT 1 FROM ops.run r WHERE r.run_id = g.refetch_run AND r.status = 'running'))
         ORDER BY 1, 2",
    )
    .bind(&spec.id)
    .fetch_all(pool)
    .await?;
    let windows = merge_windows(&todo);
    let (refetch_run, refetch_tasks) = if windows.is_empty() {
        (None, 0)
    } else {
        let opts = PlanOptions {
            windows: windows.clone(),
            trigger: Some("data_check".into()),
            ..Default::default()
        };
        let (run_id, summary) = engine
            .planner()
            .plan_run(RunKind::Repair, &[spec], opts)
            .await?;
        let wcodes: Vec<&str> = todo.iter().map(|(c, _)| c.as_str()).collect();
        let wdates: Vec<NaiveDate> = todo.iter().map(|(_, d)| *d).collect();
        sqlx::query(
            "UPDATE ops.bar_gap g SET refetch_run = $4 FROM unnest($2::text[], $3::date[]) x(c, d)
             WHERE g.dataset_id = $1 AND g.thscode = x.c AND g.trade_date = x.d",
        )
        .bind(&spec.id)
        .bind(&wcodes)
        .bind(&wdates)
        .bind(run_id)
        .execute(pool)
        .await?;
        (Some(run_id), summary.iter().map(|(_, n)| *n as i64).sum())
    };

    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, count(*) FROM ops.bar_gap WHERE dataset_id = $1 GROUP BY status",
    )
    .bind(&spec.id)
    .fetch_all(pool)
    .await?;
    let count = |s: &str| counts.iter().find(|(k, _)| k == s).map_or(0, |(_, n)| *n);
    Ok(CheckResult {
        dataset_id: spec.id.clone(),
        title: spec.title.clone(),
        securities,
        security_days: security_days.unwrap_or(0),
        busy: busy.len() as i64,
        new_gaps,
        filled,
        confirmed_now,
        suspended: count("suspended"),
        open: count("open"),
        refetch_run,
        refetch_tasks,
    })
}

/// 同一只证券的缺口合并成一个重取窗口：从最早的缺口到最晚的缺口。
/// 一次请求最多能取 10 年（超过的由规划器按窗口长度切开），按天数算请求量很小；
/// 散落的停牌日如果一天一个窗口，全市场第一次核检要上万次请求。窗口里已有的日线重写一遍，值不变
pub fn merge_windows(gaps: &[(String, NaiveDate)]) -> Vec<Window> {
    let mut by_code: BTreeMap<&str, (NaiveDate, NaiveDate)> = BTreeMap::new();
    for (c, d) in gaps {
        let w = by_code.entry(c).or_insert((*d, *d));
        w.0 = w.0.min(*d);
        w.1 = w.1.max(*d);
    }
    by_code
        .into_iter()
        .map(|(c, (lo, hi))| (c.to_string(), lo, hi))
        .collect()
}

/// 定时核检：每周同步全部结束后一次；之后重取的运行结束了，再核检一次确认结果（至少隔 10 分钟）
pub async fn auto(engine: &Engine) -> anyhow::Result<()> {
    // 先取独占权再抢每周的作业：别的核检正在进行时不消耗这周的机会
    let Some(lock) = try_lock(&engine.pool).await? else {
        return Ok(());
    };
    if weekly_finished(&engine.pool).await?
        && super::scheduler::claim(&engine.pool, "data_check", today_cn(), "week").await?
    {
        if let Err(e) = run_with(engine, "weekly", lock).await {
            if let Err(release_error) =
                super::scheduler::release_claim(&engine.pool, "data_check", today_cn()).await
            {
                tracing::error!(error = %format!("{release_error:#}"), "撤销数据核检调度标记失败");
            }
            return Err(e);
        }
        return Ok(());
    }
    let followup: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ops.bar_gap g JOIN ops.run r ON r.run_id = g.refetch_run
                        WHERE g.status = 'open' AND r.status <> 'running' AND r.finished_at > coalesce((SELECT max(started_at) FROM ops.data_check), '-infinity'))
           AND NOT EXISTS (SELECT 1 FROM ops.data_check WHERE started_at > now() - interval '10 minutes')",
    )
    .fetch_one(&engine.pool)
    .await?;
    if followup {
        run_with(engine, "followup", lock).await?;
    } else {
        lock.release().await;
    }
    Ok(())
}

/// 最近 7 天内发起的每周同步全部结束了（各批都结束，也没有还没规划的批次）
async fn weekly_finished(pool: &PgPool) -> anyhow::Result<bool> {
    let root: Option<i64> =
        sqlx::query_scalar("SELECT run_id FROM ops.scheduler WHERE job = 'weekly' AND run_id IS NOT NULL AND fired_at > now() - interval '7 days'")
            .fetch_optional(pool)
            .await?
            .flatten();
    let Some(root) = root else { return Ok(false) };
    let waves: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT status, params FROM ops.run WHERE run_id = $1 OR (params->>'root_run')::bigint = $1 ORDER BY run_id")
            .bind(root)
            .fetch_all(pool)
            .await?;
    let more = waves
        .last()
        .and_then(|(_, p)| p.get("pending_waves"))
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    Ok(!waves.is_empty() && !more && waves.iter().all(|(s, _)| s != "running"))
}

/// 最近一次核检
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct DataCheckRun {
    pub check_id: i64,
    /// weekly / followup / manual
    pub trigger: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// 查到哪个交易日为止
    pub end_date: Option<NaiveDate>,
    #[sqlx(json(nullable))]
    pub result: Option<Vec<CheckResult>>,
    pub error: Option<String>,
}

/// 一只证券还缺的日子
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct OpenGap {
    pub dataset_id: String,
    pub thscode: String,
    pub name: Option<String>,
    pub days: i64,
    /// 最早的 10 天
    pub dates: Vec<NaiveDate>,
    /// 正在重取的运行（第一批的编号）
    pub refetch_run: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct DataCheckStatus {
    /// 正在核检
    pub running: bool,
    pub last: Option<DataCheckRun>,
    /// 还缺数据的证券，缺的天数多的在前，最多 50 只
    pub open: Vec<OpenGap>,
}

pub async fn status(pool: &PgPool) -> anyhow::Result<DataCheckStatus> {
    let last = sqlx::query_as("SELECT check_id, trigger, started_at, finished_at, end_date, result, error FROM ops.data_check ORDER BY check_id DESC LIMIT 1")
        .fetch_optional(pool)
        .await?;
    let open = sqlx::query_as(
        "SELECT g.dataset_id, g.thscode, coalesce(s.name, i.name) name, count(*) days,
                (array_agg(g.trade_date ORDER BY g.trade_date))[1:10] dates, max(g.refetch_run) refetch_run
         FROM ops.bar_gap g
         LEFT JOIN ref.security s ON s.thscode = g.thscode
         LEFT JOIN ref.index_info i ON i.thscode = g.thscode
         WHERE g.status = 'open' GROUP BY 1, 2, 3 ORDER BY 4 DESC, 2 LIMIT 50",
    )
    .fetch_all(pool)
    .await?;
    Ok(DataCheckStatus {
        running: is_running(pool).await?,
        last,
        open,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn each_security_gets_one_window() {
        let gaps = vec![
            ("A".to_string(), d("2020-01-02")),
            ("A".to_string(), d("2020-01-03")),
            ("A".to_string(), d("2020-02-20")),
            ("A".to_string(), d("2021-06-01")),
            ("B".to_string(), d("2024-05-06")),
        ];
        assert_eq!(
            merge_windows(&gaps),
            vec![
                ("A".to_string(), d("2020-01-02"), d("2021-06-01")),
                ("B".to_string(), d("2024-05-06"), d("2024-05-06")),
            ]
        );
    }
}
