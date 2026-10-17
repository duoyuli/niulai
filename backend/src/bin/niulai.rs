//! niulai：唯一的可执行程序。`serve` 是常驻服务，其余子命令是执行完就退出的运维命令。
//!
//! 常用：
//!   niulai serve                                # 常驻服务：API + 任务执行 + 定时调度
//!   niulai migrate
//!   niulai plan backfill --priority P0          # 首次全量（只规划，由 serve 或 work 执行）
//!   niulai work --until-idle                    # 在前台执行任务，队列空了就退出
//!   niulai daily                                # 立即发起一次每日增量（不等调度时间）
//!   niulai status
//!   niulai user reset-password <用户名>         # 忘记密码时：生成一个新密码
//!   niulai upgrader                             # Docker 部署的一键升级助手（upgrader 容器中）

use std::sync::Arc;

use clap::{Parser, Subcommand};
use niulai::config::Settings;
use niulai::sync::run::RunKind;
use niulai::sync::{Engine, planner, postprocess, scheduler};
use niulai::server::{self, ServeOptions};
use niulai::{api, db, telemetry};

#[derive(Parser)]
#[command(name = "niulai", about = "niulai：A 股数据同步与查询服务")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 常驻服务：在一个进程中运行 REST API、任务执行和定时调度
    Serve {
        /// 不启动 REST API
        #[arg(long)]
        no_api: bool,
        /// 不执行同步任务
        #[arg(long)]
        no_worker: bool,
        /// 不运行定时调度（也可用 NIULAI_SCHEDULER_ENABLED=false）
        #[arg(long)]
        no_scheduler: bool,
    },
    /// 执行数据库迁移
    Migrate,
    /// 校验 datasets.yaml 并同步到 ops.dataset
    Registry,
    /// 规划一次同步运行
    Plan {
        /// backfill 或 incremental
        kind: String,
        /// 指定数据集（可多次）
        #[arg(long = "dataset", short = 'd')]
        datasets: Vec<String>,
        /// 按优先级选择已开启的数据集，例如 --priority P0 --priority P1
        #[arg(long = "priority", short = 'p')]
        priorities: Vec<String>,
        /// 只同步指定证券（可多次），例如 --thscode 600519.SH
        #[arg(long = "thscode")]
        thscodes: Vec<String>,
    },
    /// 在前台执行任务队列（`serve` 的执行循环在运行时不能同时使用）
    Work {
        /// 队列空了就退出
        #[arg(long)]
        until_idle: bool,
        #[arg(long)]
        concurrency: Option<usize>,
    },
    /// 立即发起一次每日增量（按调度规则选择到期的数据集）
    Daily,
    /// 数据核检：按证券 × 交易日检查个股日线和指数日线有没有缺，发现缺口就安排重取（由执行循环执行）
    Check,
    /// 重算复权因子和同花顺分段系数
    Recompute,
    /// 查看最近的运行
    Status {
        #[arg(long, default_value_t = 10)]
        limit: i64,
    },
    /// 网页用户：列出用户，或在忘记密码时重置（不开放注册，平时在网页上由管理员添加）
    User {
        #[command(subcommand)]
        cmd: UserCmd,
    },
    /// 输出 OpenAPI 规范（JSON）
    Openapi,
    /// 一键升级助手：在 Docker 部署的 upgrader 容器中运行，不连接数据库
    Upgrader {
        /// 与 niulai 共享的目录
        #[arg(long, default_value = "/upgrade")]
        upgrade_dir: std::path::PathBuf,
        /// compose.yml 和 .env 所在目录
        #[arg(long, default_value = "/deploy")]
        deploy_dir: std::path::PathBuf,
        /// Docker 的控制接口
        #[arg(long, default_value = "/var/run/docker.sock")]
        socket: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum UserCmd {
    /// 列出全部用户
    List,
    /// 重置一个用户的密码并打印新密码；停用的用户同时恢复，登录后提醒本人修改
    ResetPassword { username: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Cmd::Openapi = cli.cmd {
        println!("{}", api::openapi_json());
        return Ok(());
    }
    telemetry::init();
    if let Cmd::Upgrader { upgrade_dir, deploy_dir, socket } = cli.cmd {
        return niulai::upgrader::run(niulai::upgrader::Options { upgrade_dir, deploy_dir, socket }).await;
    }
    let settings = Settings::load()?;
    let pool = db::connect(&settings).await?;
    if let Cmd::Migrate = cli.cmd {
        db::migrate(&pool).await?;
        println!("迁移完成");
        return Ok(());
    }
    db::migrate(&pool).await?;
    let engine = Engine::new(settings, pool).await?;
    match cli.cmd {
        Cmd::Migrate | Cmd::Openapi | Cmd::Upgrader { .. } => unreachable!(),
        Cmd::Serve { no_api, no_worker, no_scheduler } => {
            let opts = ServeOptions { api: !no_api, worker: !no_worker, scheduler: !no_scheduler && engine.settings.scheduler_enabled };
            server::serve(engine, opts).await?;
        }
        Cmd::Registry => {
            for d in &engine.registry.datasets {
                println!("{:<4} {:<5} {:<42} {}", d.priority, if d.enabled { "on" } else { "off" }, d.id, d.endpoint);
            }
            println!("共 {} 个数据集，已同步到 ops.dataset", engine.registry.datasets.len());
        }
        Cmd::Plan { kind, datasets, priorities, thscodes } => {
            let specs = engine.select(&datasets, &priorities)?;
            let opts = planner::PlanOptions { only_thscodes: thscodes, trigger: Some("cli".into()), ..Default::default() };
            let (run_id, summary) = engine.planner().plan_run(kind.parse()?, &specs, opts).await?;
            for (id, n) in &summary {
                println!("{n:>8}  {id}");
            }
            println!("run_id = {run_id}（第一个波次；后续波次在前一个结束后自动规划）");
        }
        Cmd::Work { until_idle, concurrency } => work(engine, until_idle, concurrency).await?,
        Cmd::Daily => {
            let today = niulai::time::today_cn();
            let specs: Vec<_> = engine
                .registry
                .datasets
                .iter()
                .filter(|d| d.enabled && scheduler::due_daily(d.schedule, today))
                .collect();
            let (run_id, summary) = engine.planner().plan_run(RunKind::Incremental, &specs, planner::PlanOptions { trade_date: Some(today), trigger: Some("cli".into()), ..Default::default() }).await?;
            println!("run_id = {run_id}，{} 个数据集，{} 个任务", summary.len(), summary.iter().map(|(_, n)| n).sum::<usize>());
        }
        Cmd::Check => match niulai::sync::check::run(&engine, "cli").await? {
            Some(id) => {
                let s = niulai::sync::check::status(&engine.pool).await?;
                println!("check_id = {id}\n{}", serde_json::to_string_pretty(&s.last.and_then(|l| l.result))?);
            }
            None => println!("正在核检"),
        },
        Cmd::Recompute => {
            postprocess::recompute_adjustments(&engine.pool, 0).await?;
            println!("已重算");
        }
        Cmd::User { cmd: UserCmd::List } => {
            for u in niulai::auth::list(&engine.pool).await? {
                let state = if u.disabled { "已停用" } else { "" };
                println!("{:>4} {:<7} {:<24} {state}", u.user_id, u.role.key(), u.username);
            }
        }
        Cmd::User { cmd: UserCmd::ResetPassword { username } } => {
            let user = niulai::auth::find_by_name(&engine.pool, &username).await?.ok_or_else(|| anyhow::anyhow!("没有用户「{username}」"))?;
            let password = niulai::auth::generate_password()?;
            niulai::auth::set_password(&engine.pool, user.user_id, &password, false).await?;
            if user.disabled {
                niulai::auth::update(&engine.pool, user.user_id, user.role, false).await?;
            }
            println!("已重置「{}」的密码：{password}\n登录后请立即修改。", user.username);
        }
        Cmd::Status { limit } => {
            type RunRow = (i64, String, String, i32, i32, i32, chrono::DateTime<chrono::Utc>, serde_json::Value);
            let rows: Vec<RunRow> = sqlx::query_as(
                "SELECT run_id, kind, status, tasks_total, tasks_done, tasks_failed, started_at, params FROM ops.run ORDER BY run_id DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&engine.pool)
            .await?;
            for (id, kind, status, total, done, failed, started, params) in rows {
                let ds = params.get("datasets").and_then(|v| v.as_array()).map_or(0, Vec::len);
                println!("{id:>5} {kind:<11} {status:<9} {done:>6}/{total:<6} failed={failed:<4} datasets={ds:<3} {}", started.format("%m-%d %H:%M:%S"));
            }
        }
    }
    Ok(())
}

async fn work(engine: Arc<Engine>, until_idle: bool, concurrency: Option<usize>) -> anyhow::Result<()> {
    let _lock = engine.lock_executor().await?;
    let (tx, rx) = tokio::sync::watch::channel(false);
    let n = concurrency.unwrap_or(engine.settings.worker_concurrency);
    let mut handles = vec![];
    for slot in 0..n {
        let ex = engine.executor(format!("cli-{}-{slot}", std::process::id()));
        handles.push(tokio::spawn(ex.run_loop(slot, rx.clone(), until_idle)));
    }
    let stopper = tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("收到停止信号，等待正在执行的任务结束");
        tx.send(true).ok();
    });
    for h in handles {
        h.await??;
    }
    stopper.abort();
    Ok(())
}
