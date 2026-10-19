//! 同步领域事件：同步层只发布事实，应用层决定是否触发分析和建议。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

/// 同步层对外发布的事实。
#[derive(Clone, Debug)]
pub enum DomainEvent {
    /// 一个同步运行完成，复权后处理是否发现价格表有变化。
    SyncRunCompleted { run_id: i64, price_changed: bool },
    /// 执行循环完成一次周期维护。
    MaintenanceTick,
}

/// 进程内事件总线。没有订阅者时事件会被丢弃，避免 CLI 或测试场景积累无人消费的事件。
#[derive(Clone)]
pub struct EventBus {
    sender: mpsc::UnboundedSender<DomainEvent>,
    receiver: Arc<Mutex<Option<mpsc::UnboundedReceiver<DomainEvent>>>>,
    active: Arc<AtomicBool>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver: Arc::new(Mutex::new(Some(receiver))),
            active: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn take_receiver(&self) -> Option<mpsc::UnboundedReceiver<DomainEvent>> {
        let receiver = self.receiver.lock().ok()?.take();
        if receiver.is_some() {
            self.active.store(true, Ordering::Release);
        }
        receiver
    }

    pub fn publish(&self, event: DomainEvent) {
        if self.active.load(Ordering::Acquire) {
            let _ = self.sender.send(event);
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
