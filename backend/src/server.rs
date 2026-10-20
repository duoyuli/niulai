//! 常驻服务：在一个进程中同时运行 REST API、任务执行循环和定时调度（`niulai serve`）。
//!
//! 三部分共用同一个同步引擎和数据库连接池。收到 Ctrl+C / SIGTERM 后：API 停止接受新请求，
//! 执行循环不再认领新任务、等正在执行的任务结束，调度器停止，然后进程退出。
//! 同一时刻只有一个进程执行同步任务（`Engine::lock_executor`）：取不到时先只提供 API，定期再试；`--no-worker` 的进程可以另外启动，只提供 API。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};

use crate::api;
use crate::bootstrap;
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
    if !opts.api && !opts.worker && !opts.scheduler {
        anyhow::bail!("--no-api、--no-worker、--no-scheduler 不能同时使用");
    }
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut set: JoinSet<Component> = JoinSet::new();

    let events = engine
        .take_event_receiver()
        .ok_or_else(|| anyhow::anyhow!("应用事件消费者已经启动"))?;
    let event_pool = engine.pool.clone();
    let event_stop = stop_rx.clone();
    set.spawn(async move {
        (
            "应用事件",
            bootstrap::run_events(events, event_pool, event_stop).await,
        )
    });

    if engine.cache.enabled() {
        let (e, stop) = (engine.clone(), stop_rx.clone());
        set.spawn(async move { ("页面预热", api::precompute::run(e, stop).await) });
    }

    if opts.worker {
        let (e, stop) = (engine.clone(), stop_rx.clone());
        set.spawn(async move { ("执行循环", run_workers(e, stop).await) });
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
            let r = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = rx.wait_for(|stop| *stop).await;
            });
            ("API", r.await.map_err(anyhow::Error::from))
        });
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
    // 在 Docker 的 stop_grace_period（2 分钟）之前收尾：超过期限的组件直接取消，不等被 SIGKILL。
    // 被取消的任务停在 running，租约过期后由下一个进程回收
    let drain = async {
        while let Some(done) = set.join_next().await {
            if let Err(e) = describe_result(done) {
                tracing::error!(error = %e, "服务组件异常退出");
            }
        }
    };
    if tokio::time::timeout(SHUTDOWN_DEADLINE, drain)
        .await
        .is_err()
    {
        tracing::warn!(
            seconds = SHUTDOWN_DEADLINE.as_secs(),
            "停机等待超时，取消还没结束的组件"
        );
        set.abort_all();
    }
    match failure {
        Some(why) => anyhow::bail!("服务组件意外退出：{why}"),
        None => Ok(()),
    }
}

/// 收到停止信号后最多等多久
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(100);

/// 执行循环：先取得执行同步任务的独占权，取不到（另一个进程占着，或宿主机崩溃后数据库还没发现旧连接断开）时
/// API 照常提供，每 30 秒再试，不让整个进程反复重启。取到后启动各执行槽，并每 30 秒确认持锁的连接还活着；
/// 连接断了就停止认领，等在途任务结束后按故障退出，由 Docker 重启后重新取锁
async fn run_workers(engine: Arc<Engine>, mut stop: watch::Receiver<bool>) -> anyhow::Result<()> {
    let mut lock = loop {
        if *stop.borrow() {
            return Ok(());
        }
        match engine.try_lock_executor().await {
            Ok(Some(lock)) => break lock,
            Ok(None) => tracing::warn!(
                "另一个 niulai 进程正在执行同步任务，本进程先只提供 API，30 秒后再试"
            ),
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "取执行同步任务的独占权失败，30 秒后再试")
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            _ = stop.changed() => {}
        }
    };
    let (slots_stop, slots_rx) = watch::channel(false);
    let mut slots = JoinSet::new();
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".into());
    for slot in 0..engine.settings.worker_concurrency {
        let ex = engine.executor(format!("{host}-{}-{slot}", std::process::id()));
        let stop = slots_rx.clone();
        slots.spawn(async move { ex.run_loop(slot, stop, false).await });
    }
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    let result = loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break Ok(());
                }
            }
            _ = tick.tick() => {
                if let Err(e) = lock.alive().await {
                    break Err(e.context("持有执行独占权的数据库连接断了"));
                }
            }
            Some(done) = slots.join_next() => {
                break Err(match done {
                    Ok(Ok(())) => anyhow::anyhow!("一个执行槽提前结束"),
                    Ok(Err(e)) => e.context("一个执行槽出错退出"),
                    Err(e) => anyhow::anyhow!("一个执行槽 panic：{e}"),
                });
            }
        }
    };
    slots_stop.send(true).ok();
    while slots.join_next().await.is_some() {}
    result
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
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("注册 SIGTERM");
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
        assert_eq!(
            describe(set.join_next().await.unwrap()),
            "执行循环：连不上数据库"
        );
        set.spawn(async {
            if std::hint::black_box(true) {
                panic!("出错了");
            }
            ("API", Ok(()))
        });
        assert!(describe(set.join_next().await.unwrap()).starts_with("组件 panic"));
    }
}
