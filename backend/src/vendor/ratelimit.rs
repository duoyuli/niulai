//! 供应商接口限速：1 秒滑动窗口，在进程内计数。
//!
//! 免费 API Key 全局每秒只能请求约 2 次，多开进程也不会更快，所以同一时刻只让一个进程执行同步任务
//! （`Engine::lock_executor`），限速只需在进程内做，不需要 Redis。理由见 docs/decisions/in_process_rate_limit.md。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

// @project-doc docs/architecture/sync_pipeline.md#rate_limit
pub struct RateLimiter {
    rps: u32,
    window: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn new(rps: u32) -> Self {
        RateLimiter { rps, window: Mutex::new(VecDeque::new()) }
    }

    /// 等待直到允许发出一次请求
    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut q = self.window.lock().await;
                let now = Instant::now();
                while q.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) {
                    q.pop_front();
                }
                if q.len() < self.rps as usize {
                    q.push_back(now);
                    return;
                }
                Duration::from_secs(1) - now.duration_since(*q.front().unwrap())
            };
            tokio::time::sleep(wait).await;
        }
    }
}
