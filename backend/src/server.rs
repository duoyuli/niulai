//! 常驻服务：在一个进程中同时运行 REST API、任务执行循环和定时调度（`niulai serve`）。
//!
//! 三部分共用同一个同步引擎和数据库连接池。收到 Ctrl+C / SIGTERM 后：API 停止接受新请求，
//! 执行循环不再认领新任务、等正在执行的任务结束，调度器停止，然后进程退出。
//! 同一时刻只有一个进程执行同步任务（`Engine::lock_executor`）；`--no-worker` 的进程可以另外启动，只提供 API。

use std::sync::Arc;

use tokio::sync::watch;

use crate::api;
use crate::sync::{Engine, scheduler};

#[derive(Debug, Clone, Copy)]
pub struct ServeOptions {
    pub api: bool,
    pub worker: bool,
    pub scheduler: bool,
}

// @project-doc docs/architecture/sync_pipeline.md#pipeline_flow
pub async fn serve(engine: Arc<Engine>, opts: ServeOptions) -> anyhow::Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut handles: Vec<tokio::task::JoinHandle<anyhow::Result<()>>> = vec![];

    // 持有到函数返回：进程退出前一直独占任务执行
    let _executor_lock = if opts.worker { Some(engine.lock_executor().await?) } else { None };
    if opts.worker {
        let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".into());
        for slot in 0..engine.settings.worker_concurrency {
            let ex = engine.executor(format!("{host}-{}-{slot}", std::process::id()));
            handles.push(tokio::spawn(ex.run_loop(slot, stop_rx.clone(), false)));
        }
    }
    if opts.scheduler {
        handles.push(tokio::spawn(scheduler::run(engine.clone(), stop_rx.clone())));
    }
    if opts.api {
        let bind = engine.settings.api_bind.clone();
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        let app = api::router(engine.clone());
        let mut rx = stop_rx.clone();
        tracing::info!(%bind, "API 已启动，接口规范：/api/v1/openapi.json");
        handles.push(tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.wait_for(|stop| *stop).await;
                })
                .await?;
            Ok(())
        }));
    }
    if handles.is_empty() {
        anyhow::bail!("--no-api、--no-worker、--no-scheduler 不能同时使用");
    }
    tracing::info!(
        api = opts.api,
        worker = opts.worker,
        concurrency = engine.settings.worker_concurrency,
        scheduler = opts.scheduler,
        "niulai 服务已启动"
    );

    shutdown_signal().await;
    tracing::info!("收到停止信号，等待正在执行的任务结束");
    stop_tx.send(true).ok();
    for h in handles {
        if let Err(e) = h.await? {
            tracing::error!(error = %format!("{e:#}"), "服务组件异常退出");
        }
    }
    Ok(())
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
