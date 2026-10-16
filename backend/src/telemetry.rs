use tracing_subscriber::EnvFilter;

/// 初始化结构化日志。级别由 `RUST_LOG` 控制（默认 info）；`NIULAI_LOG_FORMAT=json` 时输出 JSON。
pub fn init() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_target(false);
    if std::env::var("NIULAI_LOG_FORMAT").as_deref() == Ok("json") {
        builder.json().init();
    } else {
        builder.init();
    }
}
