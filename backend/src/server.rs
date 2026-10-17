//! 常驻服务：在一个进程中同时运行 REST API、任务执行循环和定时调度（`niulai serve`）。
//!
//! 三部分共用同一个同步引擎和数据库连接池。收到 Ctrl+C / SIGTERM 后：API 停止接受新请求，
//! 执行循环不再认领新任务、等正在执行的任务结束，调度器停止，然后进程退出。
//! 同一时刻只有一个进程执行同步任务（`Engine::lock_executor`）；`--no-worker` 的进程可以另外启动，只提供 API。

use std::sync::Arc;

use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};

use crate::api;
use crate::sync::{Engine, scheduler};

#[derive(Debug, Clone, Copy)]
pub struct ServeOptions {
    pub api: bool,
    pub worker: bool,
    pub scheduler: bool,
}

/// 一个后台组件的运行结果：名字和返回值
type Component = (&'static str, anyhow::Result<()>);

// @project-doc docs/architecture/sync_pipeline.md#pipeline_flow
/// 任何一个组件在收到停止信号之前退出（出错或 panic），都按故障处理：停止其他组件，进程以错误退出，
/// 由 Docker 的 `restart: unless-stopped` 或 systemd 重新拉起。否则执行循环或调度器悄悄停了，API 却还显示正常
pub async fn serve(engine: Arc<Engine>, opts: ServeOptions) -> anyhow::Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut set: JoinSet<Component> = JoinSet::new();

    // 持有到函数返回：进程退出前一直独占任务执行
    let _executor_lock = if opts.worker { Some(engine.lock_executor().await?) } else { None };
    if opts.worker {
        let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".into());
        for slot in 0..engine.settings.worker_concurrency {
            let ex = engine.executor(format!("{host}-{}-{slot}", std::process::id()));
            let stop = stop_rx.clone();
            set.spawn(async move { ("执行循环", ex.run_loop(slot, stop, false).await) });
        }
    }
    if opts.scheduler {
        let (e, stop) = (engine.clone(), stop_rx.clone());
        set.spawn(async move { ("调度器", scheduler::run(e, stop).await) });
    }
    if opts.api {
        let bind = engine.settings.api_bind.clone();
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        let app = api::router(engine.clone());
        let mut rx = stop_rx.clone();
        tracing::info!(%bind, "API 已启动，接口规范：/api/v1/openapi.json");
        set.spawn(async move {
            let r = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = rx.wait_for(|stop| *stop).await;
            });
            ("API", r.await.map_err(anyhow::Error::from))
        });
    }
    if set.is_empty() {
        anyhow::bail!("--no-api、--no-worker、--no-scheduler 不能同时使用");
    }
    tracing::info!(
        version = crate::api::system::current_version(),
        api = opts.api,
        worker = opts.worker,
        concurrency = engine.settings.worker_concurrency,
        scheduler = opts.scheduler,
        "niulai 服务已启动"
    );

    let failure = tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("收到停止信号，等待正在执行的任务结束");
            None
        }
        Some(done) = set.join_next() => Some(describe(done)),
    };
    if let Some(why) = &failure {
        tracing::error!(error = %why, "服务组件意外退出，停止整个服务等待重启");
    }
    stop_tx.send(true).ok();
    while let Some(done) = set.join_next().await {
        if let Err(e) = describe_result(done) {
            tracing::error!(error = %e, "服务组件异常退出");
        }
    }
    match failure {
        Some(why) => anyhow::bail!("服务组件意外退出：{why}"),
        None => Ok(()),
    }
}

/// 收到停止信号之前退出的组件：不论有没有出错，都说明它不该停
fn describe(done: Result<Component, JoinError>) -> String {
    match describe_result(done) {
        Ok(name) => format!("{name}提前结束"),
        Err(e) => e,
    }
}

fn describe_result(done: Result<Component, JoinError>) -> Result<&'static str, String> {
    match done {
        Ok((name, Ok(()))) => Ok(name),
        Ok((name, Err(e))) => Err(format!("{name}：{e:#}")),
        Err(e) if e.is_panic() => Err(format!("组件 panic：{e}")),
        Err(e) => Err(format!("组件被取消：{e}")),
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("注册 SIGTERM");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn early_exit_error_and_panic_are_all_failures() {
        let mut set: JoinSet<Component> = JoinSet::new();
        set.spawn(async { ("调度器", Ok(())) });
        assert_eq!(describe(set.join_next().await.unwrap()), "调度器提前结束");
        set.spawn(async { ("执行循环", Err(anyhow::anyhow!("连不上数据库"))) });
        assert_eq!(describe(set.join_next().await.unwrap()), "执行循环：连不上数据库");
        set.spawn(async {
            if std::hint::black_box(true) {
                panic!("出错了");
            }
            ("API", Ok(()))
        });
        assert!(describe(set.join_next().await.unwrap()).starts_with("组件 panic"));
    }
}
