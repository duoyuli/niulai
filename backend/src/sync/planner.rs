//! 规划器：把数据集的分区规则展开成 ops.task。
//!
//! 一次运行（ops.run）包含多个数据集的任务。任务按 (dataset_id, part_key) 去重：
//! 同一分区已有未完成任务时不会重复创建，重复规划是安全的。
//!
//! 任务的 params 字段包含：
//! - `vars`：分区变量（thscode、date、start_ms……），用于填充请求参数和主键模板
//! - `query`：已展开的请求参数
//!
//! 证券范围（universe）是写在配置文件里的 SQL 条件，作用于 ops.v_universe。配置文件属于受信任的代码。

use std::collections::BTreeMap;

use anyhow::{Context, bail};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde_json::{Map, Value, json};
use sqlx::{AssertSqlSafe, PgConnection, PgPool};

use super::run::{self, RunKind};
use super::spec::{DatasetSpec, Incremental, Partition, Registry, Schedule};
use super::template::{self, Scope};
use super::{calendar, cursor};
use crate::time::{cn_date_end_ms, cn_date_start_ms, today_cn};
use crate::vendor::HithinkClient;

#[derive(Debug)]
pub struct TaskSeed {
    pub part_key: String,
    pub vars: Map<String, Value>,
}

pub struct PlanContext<'a> {
    pub pool: &'a PgPool,
    pub client: &'a HithinkClient,
    pub today: NaiveDate,
    pub kind: RunKind,
    /// 只规划这些证券（为空表示不限）；作用于按证券展开的分区
    pub only: &'a [String],
    /// 只取这些窗口（为空表示按规则展开）；作用于 security_window 分区，数据核检用它重取缺口
    pub windows: &'a [Window],
}

/// 一只证券的一段日期（含两端）
pub type Window = (String, NaiveDate, NaiveDate);

/// 规划选项
#[derive(Debug, Default, Clone)]
pub struct PlanOptions {
    pub trade_date: Option<NaiveDate>,
    /// 只同步指定证券，例如重拉单只股票
    pub only_thscodes: Vec<String>,
    /// 谁发起的（写进运行的 params.trigger，同步记录据此显示来源）
    pub trigger: Option<String>,
    /// 只取这些窗口，见 [`PlanContext::windows`]。只作用于第一个波次
    pub windows: Vec<Window>,
}

/// API 交给后台规划的第一批运行。`run_id` 已经可供页面轮询，规划完成后才会出现任务。
pub struct PlanningJob {
    pub run_id: i64,
    kind: RunKind,
    trade_date: NaiveDate,
    datasets: Vec<String>,
    pending: Vec<Vec<String>>,
    only: Vec<String>,
    trigger: Option<String>,
}

/// 依赖层级（波次）：
/// - 0：不依赖本地数据的数据集（once、enum，以及按自然日或接口日期列表展开的 trading_day）
/// - 1：按本地交易日历展开的 trading_day（排在交易日历之后）
/// - 1：按证券范围展开的数据集（依赖代码表等）
/// - n+1：from_dataset，取决于来源数据集所在的层级
pub fn wave_of(spec: &DatasetSpec, registry: &Registry) -> usize {
    fn depth(spec: &DatasetSpec, registry: &Registry, guard: usize) -> usize {
        match &spec.partition {
            // 按本地日历展开的交易日分区排在交易日历（once）之后：第一次同步时日历还是空的，
            // 那时规划会把节假日当成交易日
            Partition::TradingDay {
                natural_calendar: false,
                dates_from: None,
                ..
            } => 1,
            Partition::Once
            | Partition::Enum(_)
            | Partition::TradingDay { .. }
            | Partition::Unsupported(_) => 0,
            Partition::Security { .. }
            | Partition::SecurityBatch { .. }
            | Partition::SecurityWindow { .. }
            | Partition::ReportPeriod { .. } => 1,
            Partition::FromDataset { dataset, .. } => match registry.get(dataset) {
                Some(src) if guard < 8 => depth(src, registry, guard + 1) + 1,
                _ => 1,
            },
        }
    }
    depth(spec, registry, 0)
}

/// 一个波次：一个运行包含的数据集，以及之后还要规划的波次
struct Wave<'a> {
    kind: RunKind,
    trade_date: NaiveDate,
    datasets: Vec<String>,
    pending: Vec<Vec<String>>,
    root_run: Option<i64>,
    /// 接着哪个运行规划：和新运行在同一个事务里清空它的 pending_waves，保证下一个波次只规划一次
    prev_run: Option<i64>,
    only: &'a [String],
    trigger: Option<&'a str>,
    windows: &'a [Window],
}

/// 一个数据集规划出的任务种子
struct Planned<'s> {
    spec: &'s DatasetSpec,
    seeds: Vec<TaskSeed>,
    /// from_dataset 分区读取来源数据的时刻，任务写入成功后保存为游标
    since_mark: Option<DateTime<Utc>>,
}

pub struct Planner<'a> {
    pool: &'a PgPool,
    client: &'a HithinkClient,
    registry: &'a Registry,
}

impl<'a> Planner<'a> {
    pub fn new(pool: &'a PgPool, client: &'a HithinkClient, registry: &'a Registry) -> Self {
        Planner {
            pool,
            client,
            registry,
        }
    }

    // @project-doc docs/architecture/sync_pipeline.md#waves
    /// 规划一次运行。数据集按依赖层级分成多个波次，每个波次是一个 ops.run；
    /// 前一个波次结束后，执行器调用 [`Planner::continue_chain`] 规划下一个波次。
    /// 返回第一个波次的 run_id 和各数据集的任务数。
    pub async fn plan_run(
        &self,
        kind: RunKind,
        specs: &[&DatasetSpec],
        opts: PlanOptions,
    ) -> anyhow::Result<(i64, Vec<(String, usize)>)> {
        let mut waves: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for s in specs {
            waves
                .entry(wave_of(s, self.registry))
                .or_default()
                .push(s.id.clone());
        }
        let mut waves: Vec<Vec<String>> = waves.into_values().collect();
        if waves.is_empty() {
            bail!("没有可规划的数据集");
        }
        let datasets = waves.remove(0);
        let trade_date = opts.trade_date.unwrap_or_else(today_cn);
        let wave = Wave {
            kind,
            trade_date,
            datasets,
            pending: waves,
            root_run: None,
            prev_run: None,
            only: &opts.only_thscodes,
            trigger: opts.trigger.as_deref(),
            windows: &opts.windows,
        };
        self.plan_wave(wave)
            .await?
            .context("没有前一个运行时总会创建运行")
    }

    /// 先创建一个带 planning 标记的运行，给 API 立即返回；实际展开任务由调用方在后台执行。
    pub async fn create_planning_job(
        &self,
        kind: RunKind,
        specs: &[&DatasetSpec],
        opts: PlanOptions,
    ) -> anyhow::Result<PlanningJob> {
        let mut waves: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for s in specs {
            waves
                .entry(wave_of(s, self.registry))
                .or_default()
                .push(s.id.clone());
        }
        let mut waves: Vec<Vec<String>> = waves.into_values().collect();
        if waves.is_empty() {
            bail!("没有可规划的数据集");
        }
        let datasets = waves.remove(0);
        let trade_date = opts.trade_date.unwrap_or_else(today_cn);
        let trigger = opts.trigger.clone();
        let mut tx = self.pool.begin().await?;
        let new_run = run::NewRun {
            kind,
            trade_date,
            datasets: &datasets,
            pending_waves: &waves,
            root_run: None,
            only_thscodes: &opts.only_thscodes,
            trigger: trigger.as_deref(),
            planning: true,
        };
        let run_id = run::create(&mut tx, &new_run).await?;
        tx.commit().await?;
        Ok(PlanningJob {
            run_id,
            kind,
            trade_date,
            datasets,
            pending: waves,
            only: opts.only_thscodes,
            trigger,
        })
    }

    /// 在后台展开已创建的 planning 运行；失败时把运行标为 failed，页面仍可展示原因。
    pub async fn run_planning_job(&self, job: PlanningJob) -> anyhow::Result<()> {
        let PlanningJob {
            run_id,
            kind,
            trade_date,
            datasets,
            pending,
            only,
            trigger,
        } = job;
        let heartbeat_pool = self.pool.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                match run::touch_planning(&heartbeat_pool, run_id).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => tracing::warn!(run_id, error = %error, "刷新规划心跳失败"),
                }
            }
        });
        let windows: Vec<Window> = vec![];
        let wave = Wave {
            kind,
            trade_date,
            datasets,
            pending,
            root_run: None,
            prev_run: None,
            only: &only,
            trigger: trigger.as_deref(),
            windows: &windows,
        };
        let result = self.plan_wave_existing(wave, run_id).await;
        heartbeat.abort();
        if let Err(e) = result {
            sqlx::query("UPDATE ops.run SET status = 'failed', error = $2, finished_at = now(), params = params - 'planning' - 'planning_heartbeat' WHERE run_id = $1")
                .bind(run_id)
                .bind(format!("规划失败：{e:#}"))
                .execute(self.pool)
                .await?;
            return Err(e);
        }
        Ok(())
    }

    /// 某个波次结束后，规划下一个波次（如果有）。返回新的 run_id；已经被别人接上时返回 None。
    /// 可以重复调用：结束后处理中途失败、进程退出时，执行器的定期维护会对还没接上的运行再调一次（[`run::unchained`]）
    pub async fn continue_chain(&self, finished_run: i64) -> anyhow::Result<Option<i64>> {
        let row: Option<(String, NaiveDate, Value)> =
            sqlx::query_as("SELECT kind, trade_date, params FROM ops.run WHERE run_id = $1")
                .bind(finished_run)
                .fetch_optional(self.pool)
                .await?;
        let Some((kind, trade_date, params)) = row else {
            return Ok(None);
        };
        let list = |k: &str| params.get(k).cloned().unwrap_or(Value::Null);
        let mut pending: Vec<Vec<String>> =
            serde_json::from_value(list("pending_waves")).unwrap_or_default();
        if pending.is_empty() {
            return Ok(None);
        }
        let datasets = pending.remove(0);
        let root = params
            .get("root_run")
            .and_then(Value::as_i64)
            .unwrap_or(finished_run);
        let only: Vec<String> = serde_json::from_value(list("only_thscodes")).unwrap_or_default();
        let trigger = params.get("trigger").and_then(Value::as_str);
        let wave = Wave {
            kind: kind.parse()?,
            trade_date,
            datasets,
            pending,
            root_run: Some(root),
            prev_run: Some(finished_run),
            only: &only,
            trigger,
            windows: &[],
        };
        Ok(self.plan_wave(wave).await?.map(|(run_id, _)| run_id))
    }

    /// 先算出全部任务种子，再在一个事务里创建运行和任务：执行者看到运行时，它的任务已经齐全，
    /// 不会出现前几个数据集的任务做完、后面的还没写入，运行就被判定结束的情况。
    async fn plan_wave(
        &self,
        wave: Wave<'_>,
    ) -> anyhow::Result<Option<(i64, Vec<(String, usize)>)>> {
        self.plan_wave_inner(wave, None).await
    }

    async fn plan_wave_existing(
        &self,
        wave: Wave<'_>,
        run_id: i64,
    ) -> anyhow::Result<Option<(i64, Vec<(String, usize)>)>> {
        self.plan_wave_inner(wave, Some(run_id)).await
    }

    async fn plan_wave_inner(
        &self,
        wave: Wave<'_>,
        existing_run: Option<i64>,
    ) -> anyhow::Result<Option<(i64, Vec<(String, usize)>)>> {
        let specs: Vec<&DatasetSpec> = wave
            .datasets
            .iter()
            .filter_map(|id| self.registry.get(id))
            .collect();
        let ctx = PlanContext {
            pool: self.pool,
            client: self.client,
            today: wave.trade_date,
            kind: wave.kind,
            only: wave.only,
            windows: wave.windows,
        };
        let mut planned = Vec::with_capacity(specs.len());
        let mut failed = vec![];
        for spec in specs {
            match seeds_for(&ctx, spec).await {
                Ok((seeds, since_mark)) => planned.push(Planned {
                    spec,
                    seeds,
                    since_mark,
                }),
                Err(e) => {
                    tracing::error!(dataset = %spec.id, error = %format!("{e:#}"), "规划失败");
                    failed.push(spec.id.clone());
                }
            }
        }

        let mut tx = self.pool.begin().await?;
        if let Some(prev) = wave.prev_run
            && !run::take_pending_waves(&mut tx, prev).await?
        {
            return Ok(None);
        }
        let run_id = if let Some(run_id) = existing_run {
            let changed = sqlx::query("UPDATE ops.run SET params = params - 'planning' - 'planning_heartbeat' WHERE run_id = $1 AND status = 'running'")
                .bind(run_id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if changed == 0 {
                return Ok(None);
            }
            run_id
        } else {
            let new_run = run::NewRun {
                kind: wave.kind,
                trade_date: wave.trade_date,
                datasets: &wave.datasets,
                pending_waves: &wave.pending,
                root_run: wave.root_run,
                only_thscodes: wave.only,
                trigger: wave.trigger,
                planning: false,
            };
            run::create(&mut tx, &new_run).await?
        };
        let mut summary = vec![];
        let mut total = 0usize;
        for p in &planned {
            let n = insert_tasks(
                &mut tx,
                run_id,
                p.spec,
                &p.seeds,
                self.registry.max_attempts,
                task_priority(p.spec, wave.kind),
                wave.trade_date,
            )
            .await?;
            total += n;
            summary.push((p.spec.id.clone(), n));
        }
        if !failed.is_empty() {
            let message = format!("部分数据集规划失败：{}", failed.join(", "));
            sqlx::query("UPDATE ops.run SET error = $2, params = jsonb_set(params, '{planning_error}', 'true'::jsonb, true) WHERE run_id = $1")
                .bind(run_id)
                .bind(message)
                .execute(&mut *tx)
                .await?;
        }
        summary.extend(failed.into_iter().map(|id| (id, 0)));
        let marks: serde_json::Map<String, Value> = planned
            .iter()
            .filter_map(|p| {
                p.since_mark
                    .map(|at| (p.spec.id.clone(), Value::String(at.to_rfc3339())))
            })
            .collect();
        if !marks.is_empty() {
            sqlx::query("UPDATE ops.run SET params = jsonb_set(params, '{planned_since_marks}', $2::jsonb, true) WHERE run_id = $1")
                .bind(run_id)
                .bind(Value::Object(marks))
                .execute(&mut *tx)
                .await?;
        }
        run::set_total(&mut tx, run_id, total).await?;
        tx.commit().await?;
        tracing::info!(
            run_id,
            kind = wave.kind.as_str(),
            tasks = total,
            datasets = summary.len(),
            "已规划"
        );
        if total == 0 {
            // 本波次没有任务，运行已直接结束：继续规划下一个波次
            Box::pin(self.continue_chain(run_id)).await?;
        }
        Ok(Some((run_id, summary)))
    }
}

// @project-doc docs/architecture/sync_pipeline.md#task_lifecycle
/// 任务优先级（越小越先执行），按急迫程度分层，同一层再按数据集的 P0–P3 排序：
/// 每日数据（0 层）→ 兜底补偿（50 层）→ 每周等其他增量（100 层）→ 历史回补（200 层）。
/// 只有当天值的数据（估值、异动原因等）错过当天就补不回来，不能排在每周增量的几万个任务后面
pub fn task_priority(spec: &DatasetSpec, kind: RunKind) -> i16 {
    let tier = match kind {
        RunKind::Backfill => 200,
        RunKind::Repair => 50,
        RunKind::Incremental if spec.schedule == Schedule::Daily => 0,
        RunKind::Incremental => 100,
    };
    tier + spec.priority_rank()
}

async fn insert_tasks(
    conn: &mut PgConnection,
    run_id: i64,
    spec: &DatasetSpec,
    seeds: &[TaskSeed],
    max_attempts: i16,
    priority: i16,
    trade_date: NaiveDate,
) -> anyhow::Result<usize> {
    // 主键里带运行交易日的数据集（按天积累的快照），分区键也带上交易日：
    // 不同交易日的任务互不去重，否则排队中的旧任务会挡住当天的增量
    let per_day = spec.key.contains("{_run.trade_date}");
    let mut keys = Vec::with_capacity(seeds.len());
    let mut params = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let query = render_query(spec, &seed.vars)?;
        keys.push(if per_day {
            format!("{}|day={trade_date}", seed.part_key)
        } else {
            seed.part_key.clone()
        });
        params.push(json!({"vars": seed.vars, "query": query}));
    }
    let mut inserted = 0usize;
    for (k, p) in keys.chunks(5000).zip(params.chunks(5000)) {
        let res = sqlx::query(
            "INSERT INTO ops.task (run_id, dataset_id, part_key, params, priority, max_attempts)
             SELECT $1, $2, k, p, $5, $6 FROM unnest($3::text[], $4::jsonb[]) AS t(k, p)
             ON CONFLICT (dataset_id, part_key) WHERE status IN ('pending', 'running', 'failed') DO NOTHING",
        )
        .bind(run_id)
        .bind(&spec.id)
        .bind(k)
        .bind(p)
        .bind(priority)
        .bind(max_attempts)
        .execute(&mut *conn)
        .await?;
        inserted += res.rows_affected() as usize;
    }
    Ok(inserted)
}

/// 用分区变量展开请求参数。增量规则 latest_n 带 params 时，去掉时间窗口参数，换成覆盖参数。
pub fn render_query(
    spec: &DatasetSpec,
    vars: &Map<String, Value>,
) -> anyhow::Result<Map<String, Value>> {
    let window_vars = ["start_ms", "end_ms", "start", "end"];
    let latest_override = match (&spec.incremental, vars.get("_latest")) {
        (Incremental::LatestN { params, .. }, Some(Value::Bool(true))) if !params.is_empty() => {
            Some(params)
        }
        _ => None,
    };
    let scope = Scope {
        part: vars,
        record: None,
        run_trade_date: None,
        extra: &[],
    };
    let mut out = Map::new();
    for (k, tpl) in &spec.params {
        if latest_override.is_some()
            && template::variables(tpl)
                .iter()
                .any(|v| window_vars.contains(v))
        {
            continue;
        }
        out.insert(
            k.clone(),
            Value::String(template::render(tpl, &scope).with_context(|| format!("参数 {k}"))?),
        );
    }
    if let Some(over) = latest_override {
        for (k, v) in over {
            out.insert(k.clone(), Value::String(v.clone()));
        }
    }
    Ok(out)
}

// @project-doc docs/interfaces/dataset_registry.md#partitions
async fn seeds_for(
    ctx: &PlanContext<'_>,
    spec: &DatasetSpec,
) -> anyhow::Result<(Vec<TaskSeed>, Option<DateTime<Utc>>)> {
    if let Partition::FromDataset {
        dataset,
        fields,
        filter,
    } = &spec.partition
    {
        return from_dataset_seeds(ctx, spec, dataset, fields, filter.as_deref()).await;
    }
    Ok((window_seeds(ctx, spec).await?, None))
}

async fn window_seeds(ctx: &PlanContext<'_>, spec: &DatasetSpec) -> anyhow::Result<Vec<TaskSeed>> {
    let today = ctx.today;
    let base_vars = || base_vars(today);
    let incremental = ctx.kind != RunKind::Backfill;
    match &spec.partition {
        Partition::Unsupported(what) => bail!("规划器暂不支持 {what}"),
        Partition::Once => Ok(vec![TaskSeed {
            part_key: "*".into(),
            vars: base_vars(),
        }]),
        Partition::Enum(dims) => {
            let mut combos: Vec<Map<String, Value>> = vec![base_vars()];
            for (k, vals) in dims {
                combos = combos
                    .into_iter()
                    .flat_map(|c| {
                        vals.iter().map(move |v| {
                            let mut c = c.clone();
                            c.insert(k.clone(), json!(v));
                            c
                        })
                    })
                    .collect();
            }
            Ok(combos
                .into_iter()
                .map(|vars| {
                    let part_key = dims
                        .keys()
                        .map(|k| format!("{k}={}", template::value_to_string(&vars[k])))
                        .collect::<Vec<_>>()
                        .join("|");
                    TaskSeed { part_key, vars }
                })
                .collect())
        }
        Partition::Security { universe } => {
            let rows = universe_rows(ctx, universe).await?;
            Ok(rows
                .into_iter()
                .map(|(code, _)| {
                    let mut vars = base_vars();
                    vars.insert("thscode".into(), json!(code));
                    TaskSeed {
                        part_key: format!("thscode={code}"),
                        vars,
                    }
                })
                .collect())
        }
        Partition::SecurityBatch { universe, size } => {
            let rows = universe_rows(ctx, universe).await?;
            Ok(rows
                .chunks(*size)
                .enumerate()
                .map(|(i, chunk)| {
                    let codes: Vec<&str> = chunk.iter().map(|(c, _)| c.as_str()).collect();
                    let mut vars = base_vars();
                    vars.insert("thscodes".into(), json!(codes.join(",")));
                    TaskSeed {
                        part_key: format!("batch={i:04}|{}", codes[0]),
                        vars,
                    }
                })
                .collect())
        }
        Partition::SecurityWindow { window_days, .. } if !ctx.windows.is_empty() => Ok(ctx
            .windows
            .iter()
            .flat_map(|(code, from, to)| window_range_seeds(code, *from, *to, *window_days, today))
            .collect()),
        Partition::SecurityWindow {
            universe,
            window_days,
            floor,
            start_from_list_date,
        } => {
            let floor = floor.resolve(today);
            let rows = universe_rows(ctx, universe).await?;
            let cursors = if incremental {
                cursor::synced_to(ctx.pool, &spec.id).await?
            } else {
                BTreeMap::new()
            };
            let mut seeds = vec![];
            for (code, list_date) in rows {
                if list_date.is_some_and(|d| d > today) {
                    continue; // 尚未上市
                }
                let part = format!("thscode={code}");
                // 增量：latest_n 带覆盖参数时，每只证券一个"最近 N 期"任务
                if incremental
                    && let Incremental::LatestN { params, .. } = &spec.incremental
                    && !params.is_empty()
                {
                    let mut vars = base_vars();
                    vars.insert("thscode".into(), json!(code));
                    vars.insert("_latest".into(), json!(true));
                    seeds.push(TaskSeed {
                        part_key: format!("{part}|latest"),
                        vars,
                    });
                    continue;
                }
                let mut start = floor;
                if *start_from_list_date && let Some(ld) = list_date {
                    start = start.max(ld);
                }
                if incremental
                    && let (Incremental::SinceCursor { overlap_days }, Some(synced)) =
                        (&spec.incremental, cursors.get(&part))
                {
                    start = start.max(*synced - Duration::days(*overlap_days));
                }
                seeds.extend(window_range_seeds(&code, start, today, *window_days, today));
            }
            Ok(seeds)
        }
        Partition::TradingDay {
            from,
            natural_calendar,
            dates_from,
        } => {
            let mut start = from.resolve(today);
            if incremental
                && let Incremental::SinceCursor { overlap_days } = &spec.incremental
                && let Some(synced) = cursor::synced_to(ctx.pool, &spec.id).await?.get("*")
            {
                start = start.max(*synced - Duration::days(*overlap_days));
            }
            let dates = if let Some(endpoint) = dates_from {
                available_dates(ctx.client, endpoint).await?
            } else if *natural_calendar {
                start.iter_days().take_while(|d| *d <= today).collect()
            } else {
                calendar::trading_days(ctx.pool, start, today).await?
            };
            Ok(dates
                .into_iter()
                .filter(|d| *d >= start && *d <= today)
                .map(|d| {
                    let mut vars = base_vars();
                    vars.insert("date".into(), json!(d.to_string()));
                    vars.insert("date_ms".into(), json!(cn_date_start_ms(d)));
                    TaskSeed {
                        part_key: format!("date={d}"),
                        vars,
                    }
                })
                .collect())
        }
        Partition::ReportPeriod {
            universe,
            from_year,
            from_quarter,
            quarters,
        } => {
            let mut periods = report_periods(*from_year, *from_quarter, quarters, today);
            if incremental && let Incremental::LatestN { n: Some(n), .. } = &spec.incremental {
                let keep = periods.len().saturating_sub(*n);
                periods.drain(..keep);
            }
            let rows = universe_rows(ctx, universe).await?;
            let mut seeds = vec![];
            for (code, list_date) in rows {
                for (y, q) in &periods {
                    // 上市前一年以前的报告期没有数据，跳过（科创板等新股在上市前一年就有财务数据）
                    if list_date.is_some_and(|ld| ld.year() > *y + 1) {
                        continue;
                    }
                    let mut vars = base_vars();
                    vars.insert("thscode".into(), json!(code));
                    vars.insert("report".into(), json!(format!("{y}-{q}")));
                    seeds.push(TaskSeed {
                        part_key: format!("thscode={code}|report={y}-{q}"),
                        vars,
                    });
                }
            }
            Ok(seeds)
        }
        Partition::FromDataset { .. } => unreachable!("from_dataset 由 from_dataset_seeds 处理"),
    }
}

/// from_dataset：来源数据集的每组不同字段值是一个任务。增量运行只看上次规划之后变化的来源记录。
async fn from_dataset_seeds(
    ctx: &PlanContext<'_>,
    spec: &DatasetSpec,
    dataset: &str,
    fields: &BTreeMap<String, String>,
    filter: Option<&str>,
) -> anyhow::Result<(Vec<TaskSeed>, Option<DateTime<Utc>>)> {
    let incremental = ctx.kind != RunKind::Backfill;
    let since = if incremental {
        cursor::planned_since(ctx.pool, &spec.id).await?
    } else {
        None
    };
    // 先取数据库时刻再读来源记录：读取期间变化的记录下次还会被规划，不会漏掉
    let mark: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(ctx.pool)
        .await?;
    let rows = from_dataset_rows(ctx.pool, dataset, fields, filter, since).await?;
    let today = ctx.today;
    let seeds = rows
        .into_iter()
        .map(|vals| {
            let mut vars = base_vars(today);
            let part_key = fields
                .keys()
                .zip(&vals)
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("|");
            for (k, v) in fields.keys().zip(vals) {
                vars.insert(k.clone(), json!(v));
            }
            TaskSeed { part_key, vars }
        })
        .collect();
    // 回补也要记下水位：否则第一次增量没有水位，会把来源数据集的全部记录重新规划一遍
    // （龙虎榜机构明细的第一次每周增量因此重查了一整年、1.6 万次请求）。只同步部分证券时不记，免得漏掉其他证券
    Ok((seeds, (incremental || ctx.only.is_empty()).then_some(mark)))
}

fn base_vars(today: NaiveDate) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("today".into(), json!(today.to_string()));
    m.insert(
        "today+60d".into(),
        json!((today + Duration::days(60)).to_string()),
    );
    m
}

/// 一只证券 from 到 to（含）按窗口长度切成的任务，分区键 `thscode=<代码>|<起>..<止>`
fn window_range_seeds(
    code: &str,
    from: NaiveDate,
    to: NaiveDate,
    window_days: i64,
    today: NaiveDate,
) -> Vec<TaskSeed> {
    let mut seeds = vec![];
    let mut s = from;
    while s <= to {
        let e = (s + Duration::days(window_days - 1)).min(to);
        let mut vars = base_vars(today);
        vars.insert("thscode".into(), json!(code));
        vars.insert("start".into(), json!(s.to_string()));
        vars.insert("end".into(), json!(e.to_string()));
        vars.insert("start_ms".into(), json!(cn_date_start_ms(s)));
        vars.insert("end_ms".into(), json!(cn_date_end_ms(e)));
        seeds.push(TaskSeed {
            part_key: format!("thscode={code}|{s}..{e}"),
            vars,
        });
        s = e + Duration::days(1);
    }
    seeds
}

async fn universe_rows(
    ctx: &PlanContext<'_>,
    universe: &str,
) -> anyhow::Result<Vec<(String, Option<NaiveDate>)>> {
    let sql = format!(
        "SELECT DISTINCT ON (thscode) thscode, list_date FROM ops.v_universe
         WHERE thscode IS NOT NULL AND ({universe}) AND (cardinality($1::text[]) = 0 OR thscode = ANY($1)) ORDER BY thscode"
    );
    let pool = ctx.pool;
    // 审计：universe 来自受信任的配置文件 config/datasets.yaml，不接受外部输入
    sqlx::query_as(AssertSqlSafe(sql))
        .bind(ctx.only)
        .fetch_all(pool)
        .await
        .with_context(|| format!("universe 查询失败：{universe}"))
}

async fn available_dates(client: &HithinkClient, endpoint: &str) -> anyhow::Result<Vec<NaiveDate>> {
    let resp = client
        .get(endpoint, &[])
        .await
        .map_err(|e| anyhow::anyhow!("获取可选日期失败：{e}"))?;
    let dates = resp
        .body
        .pointer("/data/dates")
        .and_then(Value::as_array)
        .context("响应中没有 data.dates")?;
    Ok(dates
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .collect())
}

/// 已经结束的报告期（季度末早于今天），按时间升序
fn report_periods(
    from_year: i32,
    from_quarter: u32,
    quarters: &[u32],
    today: NaiveDate,
) -> Vec<(i32, u32)> {
    let mut out = vec![];
    for y in from_year..=today.year() {
        for q in 1..=4u32 {
            if (y == from_year && q < from_quarter) || !quarters.contains(&q) {
                continue;
            }
            let quarter_end = NaiveDate::from_ymd_opt(y, q * 3, 1)
                .and_then(|d| d.checked_add_months(chrono::Months::new(1)))
                .map(|d| d - Duration::days(1));
            if quarter_end.is_some_and(|e| e < today) {
                out.push((y, q));
            }
        }
    }
    out
}

/// from_dataset：字段写 rkey / thscode / obs_date，或以 $ 开头的 JSONPath（作用在 payload 上，可返回多个值）
async fn from_dataset_rows(
    pool: &PgPool,
    dataset: &str,
    fields: &BTreeMap<String, String>,
    filter: Option<&str>,
    since: Option<DateTime<Utc>>,
) -> anyhow::Result<Vec<Vec<String>>> {
    let mut exprs = vec![];
    for (i, (_, f)) in fields.iter().enumerate() {
        let e = match f.as_str() {
            "rkey" | "thscode" => format!("r.{f}"),
            "obs_date" => "r.obs_date::text".to_string(),
            p if p.starts_with('$') => format!(
                "jsonb_path_query(r.payload, '{}') #>> '{{}}'",
                p.replace('\'', "''")
            ),
            other => {
                bail!("from_dataset 字段只能是 rkey / thscode / obs_date 或 JSONPath：{other}")
            }
        };
        exprs.push(format!("{e} AS c{i}"));
    }
    let cols: Vec<String> = (0..fields.len()).map(|i| format!("c{i}")).collect();
    let sql = format!(
        "SELECT DISTINCT {cols} FROM (SELECT {exprs} FROM raw.record r WHERE r.dataset_id = $1 AND ($2::timestamptz IS NULL OR r.changed_at >= $2) AND ({filter})) s
         WHERE {not_null} ORDER BY {cols}",
        cols = cols.join(", "),
        exprs = exprs.join(", "),
        filter = filter.unwrap_or("true"),
        not_null = cols.iter().map(|c| format!("{c} IS NOT NULL")).collect::<Vec<_>>().join(" AND "),
    );
    // 审计：字段只允许固定列名或 JSONPath，filter 来自受信任的配置文件
    let rows = sqlx::query(AssertSqlSafe(sql))
        .bind(dataset)
        .bind(since)
        .fetch_all(pool)
        .await?;
    use sqlx::Row;
    Ok(rows
        .iter()
        .map(|r| (0..fields.len()).map(|i| r.get::<String, _>(i)).collect())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_data_goes_before_weekly_increments_and_backfill() {
        let reg = Registry::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml"),
        )
        .unwrap();
        let p = |id: &str, kind| task_priority(reg.get(id).unwrap(), kind);
        // 只有当天值的估值（P1）也要排在每周增量的 P0 前面
        assert!(
            p("a_share.valuation", RunKind::Incremental)
                < p("a_share.bar_1d", RunKind::Incremental)
        );
        assert!(p("a_share.bar_1d", RunKind::Repair) < p("a_share.income_q", RunKind::Incremental));
        assert!(
            p("a_share.income_q", RunKind::Incremental) < p("a_share.snapshot", RunKind::Backfill)
        );
    }

    #[test]
    fn calendar_dates_are_planned_after_the_calendar() {
        let reg = Registry::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml"),
        )
        .unwrap();
        let w = |id: &str| wave_of(reg.get(id).unwrap(), &reg);
        // 第一次同步时日历是空的：按本地日历展开的数据集要等日历取完再规划，否则节假日也会被当成交易日
        assert!(w("special.dragon_tiger") > w("a_share.calendar"));
        assert!(w("special.limit_up_stats") > w("a_share.calendar"));
        // 按自然日查询的不依赖日历
        assert_eq!(w("special.hot_stock_daily"), w("a_share.calendar"));
    }

    #[test]
    fn report_periods_only_finished_quarters() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
        let p = report_periods(2025, 3, &[1, 2, 3, 4], today);
        assert_eq!(
            p,
            vec![(2025, 3), (2025, 4), (2026, 1), (2026, 2), (2026, 3)]
        );
        let annual = report_periods(2023, 4, &[4], today);
        assert_eq!(annual, vec![(2023, 4), (2024, 4), (2025, 4)]);
    }
}
