//! 全市场计算的资源预算：同时最多几个、前台和后台各占哪些槽位，以及耗 CPU 的计算放到哪里执行。
//!
//! 预算按用例持有：一次因子研究、一次市场概况、一次账户排名，从读入输入到算完结果都占着同一个槽位，
//! 而不只是在扫描日线时占着，「最多两个」才等于「最多两个完整任务」。入口用 [`run`] 显式声明预算；
//! 同一个任务里嵌套的 [`run`]（例如后台建议里读市场概况、算排名）直接复用已经拿到的槽位，不会再排一次队。
//! 规则见 docs/architecture/sync_pipeline.md#rate_limit。

use std::future::Future;

/// 前台（页面上发起的）全市场计算同时最多几个。每个都要读几百万行日线、算几秒到十几秒，
/// 不限制的话几个页面同时打开就会占满数据库连接和 CPU，拖慢接口和同步任务；多出来的排队等待
pub const FOREGROUND_SLOTS: usize = 2;
static FOREGROUND: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(FOREGROUND_SLOTS);
/// 后台计算（调仓建议）另有一个槽位：页面上同时开几个因子研究，也不会让它排不上
static BACKGROUND: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// 一个用例用哪份预算
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Budget {
    /// 页面请求：因子研究、市场概况等
    Foreground,
    /// 后台任务：同步完成后的建议刷新、改账户后的重算、维护
    Background,
}

tokio::task_local! {
    /// 当前任务已经占着的预算
    static HELD: Budget;
}

/// 当前任务是否已经占着一个全市场计算的槽位
pub fn held() -> Option<Budget> {
    HELD.try_with(|b| *b).ok()
}

/// 在 budget 的一个槽位里执行整个用例 f；当前任务已经占着槽位时直接执行，不重复排队
pub async fn run<F: Future>(budget: Budget, f: F) -> F::Output {
    if held().is_some() {
        return f.await;
    }
    let slots = match budget {
        Budget::Foreground => &FOREGROUND,
        Budget::Background => &BACKGROUND,
    };
    let _permit = slots.acquire().await.expect("信号量不会被关闭");
    HELD.scope(budget, f).await
}

/// 执行一段耗 CPU 的计算：多线程运行时（服务）里用 block_in_place，让这个线程上的其他异步任务（接口、同步任务的续租）
/// 挪到别的线程继续；单线程运行时（测试）里直接执行
pub fn run_cpu<R>(f: impl FnOnce() -> R) -> R {
    use tokio::runtime::{Handle, RuntimeFlavor};
    if Handle::try_current().is_ok_and(|h| h.runtime_flavor() == RuntimeFlavor::MultiThread) {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nested_runs_reuse_the_held_slot() {
        // 后台只有一个槽位：嵌套的 run 如果再排队就会死锁
        let got = run(Budget::Background, async {
            assert_eq!(held(), Some(Budget::Background));
            run(Budget::Foreground, async { held() }).await
        })
        .await;
        assert_eq!(got, Some(Budget::Background));
        assert_eq!(held(), None);
    }
}
