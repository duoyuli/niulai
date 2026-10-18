use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{FixedOffset, Utc};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

/// 进程开始初始化日志的时间，用来算运行时长（`GET /api/v1/metrics`）
static STARTED: OnceLock<Instant> = OnceLock::new();

/// 进程已经运行了多久
pub fn uptime() -> Duration {
    STARTED.get_or_init(Instant::now).elapsed()
}

/// 日志时间用上海时间（带 +08:00），与服务器所在的时区无关
struct ShanghaiTime;

impl FormatTime for ShanghaiTime {
    fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
        let cn = FixedOffset::east_opt(8 * 3600).expect("+08:00");
        write!(w, "{}", Utc::now().with_timezone(&cn).format("%Y-%m-%dT%H:%M:%S%.3f%:z"))
    }
}

/// 初始化结构化日志。级别由 `RUST_LOG` 控制（默认 info）；`NIULAI_LOG_FORMAT=json` 时输出 JSON。
pub fn init() {
    STARTED.get_or_init(Instant::now);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_target(false).with_timer(ShanghaiTime);
    if std::env::var("NIULAI_LOG_FORMAT").as_deref() == Ok("json") {
        builder.json().init();
    } else {
        builder.init();
    }
}
