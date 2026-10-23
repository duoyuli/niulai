//! 改了账户之后重新生成建议：`niulai serve` 中受监督的常驻组件，消费 `account::stale` 的持久化待办。
//!
//! 保存持仓、现金、规则或设置的请求在自己的事务里标记账户，不等生成；这里每隔几秒（或被保存请求唤醒时）
//! 取出待办，在后台计算槽位里逐个生成，成功后按标记时刻确认，失败保留待办、推迟重试并把原因给页面看。
//! 进程退出、生成失败都不会丢掉一次修改，多个进程同时消费也只会多生成一份（旧的标记为被取代）。

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::{Notify, watch};

use crate::account::{self, stale};
use crate::analysis::compute;

/// 没有被唤醒时多久查一次待办
const POLL: Duration = Duration::from_secs(5);
/// 一轮最多处理几个账户，处理完立即再查一轮
const BATCH: i64 = 20;

/// 常驻循环：被 `wake` 唤醒或每 [`POLL`] 处理一轮待办，直到收到停止信号
pub async fn run(
    pool: PgPool,
    wake: Arc<Notify>,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    loop {
        match drain(&pool).await {
            Ok(n) if n >= BATCH as usize => continue,
            Ok(_) => {}
            Err(e) => tracing::error!(error = %format!("{e:#}"), "读取建议重算的待办失败"),
        }
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
            _ = wake.notified() => {}
            _ = tokio::time::sleep(POLL) => {}
        }
    }
}

/// 处理一轮到期的待办，返回处理了几个账户
pub async fn drain(pool: &PgPool) -> anyhow::Result<usize> {
    let due = stale::pending(pool, BATCH).await?;
    for (account_id, at) in &due {
        let result = compute::run(compute::Budget::Background, async {
            match account::get_by_id(pool, *account_id).await? {
                Some(acc) => super::regenerate(pool, &acc).await.map(|_| ()),
                // 账户已经删了：待办随之消失
                None => Ok(()),
            }
        })
        .await;
        match result {
            Ok(()) => {
                if !stale::settle(pool, *account_id, *at).await? {
                    tracing::debug!(account_id, "生成期间账户又改过，按最新的再生成一次");
                }
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::error!(account_id, error = %msg, "重新生成建议失败，稍后重试");
                stale::defer(pool, *account_id, &msg).await?;
            }
        }
    }
    Ok(due.len())
}
