//! 监控指标：`GET /api/v1/metrics`，Prometheus 文本格式。
//!
//! 接口不经过登录中间件，改为校验 `NIULAI_METRICS_TOKEN`：没有设置这个环境变量时返回 404（默认关闭），
//! 设置了就要求请求头 `Authorization: Bearer <token>`（或查询参数 `?token=`）。这样外部监控服务不用登录。
//! 指标只包含进程和队列的聚合数字，没有证券、账户等业务数据。规则见 docs/operations/runbook.md#metrics。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::AppState;
use crate::telemetry::uptime;
use crate::time::now_cn;

/// 请求计数和累计耗时，由中间件在每次请求结束时更新
#[derive(Default)]
pub struct Metrics {
    requests: AtomicU64,
    errors: AtomicU64,
    /// 累计耗时（微秒），避免浮点累加
    micros: AtomicU64,
}

impl Metrics {
    pub fn record(&self, status: u16, elapsed: Duration) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        if status >= 500 {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        self.micros.fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
    }

    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    pub fn micros(&self) -> u64 {
        self.micros.load(Ordering::Relaxed)
    }
}

/// 记录每个 API 请求的次数、错误数和耗时
pub async fn track(State(metrics): State<Arc<Metrics>>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let resp = next.run(req).await;
    metrics.record(resp.status().as_u16(), started.elapsed());
    resp
}

#[derive(Deserialize)]
pub struct MetricsQuery {
    /// 令牌；也可以用 `Authorization: Bearer <token>` 请求头
    pub token: Option<String>,
}

/// 读一次环境变量就记住：部署期间不会变
fn expected_token() -> Option<&'static str> {
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    TOKEN
        .get_or_init(|| std::env::var("NIULAI_METRICS_TOKEN").ok().filter(|t| !t.trim().is_empty()))
        .as_deref()
}

/// 请求带的和预期的令牌是否一致。逐字节比较，不因为前面几个字节相同就提前返回
fn token_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 请求带的令牌：优先 `Authorization: Bearer`，其次 `?token=`
fn given_token(headers: &HeaderMap, query: &MetricsQuery) -> Option<String> {
    if let Some(v) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok())
        && let Some(t) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))
    {
        return Some(t.trim().to_string());
    }
    query.token.as_ref().map(|t| t.trim().to_string())
}

pub async fn handler(State(s): State<AppState>, headers: HeaderMap, Query(q): Query<MetricsQuery>) -> Response {
    let Some(expected) = expected_token() else {
        return (StatusCode::NOT_FOUND, "没有启用监控指标：在环境变量里设置 NIULAI_METRICS_TOKEN 后可用\n").into_response();
    };
    let ok = given_token(&headers, &q).is_some_and(|t| token_matches(expected, &t));
    if !ok {
        return (StatusCode::UNAUTHORIZED, "监控指标的令牌不对\n").into_response();
    }
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM ops.task WHERE status IN ('pending', 'failed')")
        .fetch_one(&s.pool)
        .await
        .unwrap_or(-1);
    let body = format(&s.metrics, super::system::current_version(), uptime(), pending, s.pool.size(), s.pool.num_idle());
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], body).into_response()
}

/// Prometheus 文本格式。测试直接检查每一行，避免指标名拼错
pub fn format(metrics: &Metrics, version: &str, uptime: Duration, pending: i64, db_size: u32, db_idle: usize) -> String {
    let requests = metrics.requests();
    let mut out = String::with_capacity(1024);
    out.push_str("# HELP niulai_up 1 表示进程正在运行\n# TYPE niulai_up gauge\nniulai_up 1\n");
    out.push_str("# HELP niulai_info 版本信息\n# TYPE niulai_info gauge\n");
    out.push_str(&format!("niulai_info{{version=\"{}\"}} 1\n", escape_label(version)));
    out.push_str("# HELP niulai_uptime_seconds 进程已运行的秒数\n# TYPE niulai_uptime_seconds gauge\n");
    out.push_str(&format!("niulai_uptime_seconds {}\n", uptime.as_secs_f64()));
    out.push_str("# HELP niulai_http_requests_total 处理过的 API 请求数\n# TYPE niulai_http_requests_total counter\n");
    out.push_str(&format!("niulai_http_requests_total {requests}\n"));
    out.push_str("# HELP niulai_http_errors_total 返回 5xx 的请求数\n# TYPE niulai_http_errors_total counter\n");
    out.push_str(&format!("niulai_http_errors_total {}\n", metrics.errors()));
    out.push_str("# HELP niulai_http_request_duration_seconds API 请求累计耗时\n# TYPE niulai_http_request_duration_seconds summary\n");
    out.push_str(&format!("niulai_http_request_duration_seconds_sum {:.6}\n", metrics.micros() as f64 / 1e6));
    out.push_str(&format!("niulai_http_request_duration_seconds_count {requests}\n"));
    out.push_str("# HELP niulai_sync_pending_tasks 等待执行的任务数（含等待重试的）\n# TYPE niulai_sync_pending_tasks gauge\n");
    out.push_str(&format!("niulai_sync_pending_tasks {pending}\n"));
    out.push_str("# HELP niulai_db_pool_connections 数据库连接池里的连接数\n# TYPE niulai_db_pool_connections gauge\n");
    out.push_str(&format!("niulai_db_pool_connections {db_size}\n"));
    out.push_str("# HELP niulai_db_pool_idle 其中空闲的连接数\n# TYPE niulai_db_pool_idle gauge\n");
    out.push_str(&format!("niulai_db_pool_idle {}\n", db_idle as u32));
    out.push_str("# HELP niulai_scrape_time_seconds 生成这些指标的 Unix 时间\n# TYPE niulai_scrape_time_seconds gauge\n");
    out.push_str(&format!("niulai_scrape_time_seconds {}\n", now_cn().and_utc().timestamp()));
    out
}

/// 标签值里的反斜杠、双引号和换行要转义，否则 Prometheus 解析失败
fn escape_label(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("abc", "ab"));
        assert!(!token_matches("", "x"));
    }

    #[test]
    fn format_contains_all_series() {
        let m = Metrics::default();
        m.record(200, Duration::from_millis(12));
        m.record(500, Duration::from_millis(8));
        let text = format(&m, "2026.10.18.9", Duration::from_secs(60), 7, 10, 3);
        for line in [
            "niulai_up 1",
            "niulai_info{version=\"2026.10.18.9\"} 1",
            "niulai_uptime_seconds 60",
            "niulai_http_requests_total 2",
            "niulai_http_errors_total 1",
            "niulai_http_request_duration_seconds_sum 0.020000",
            "niulai_http_request_duration_seconds_count 2",
            "niulai_sync_pending_tasks 7",
            "niulai_db_pool_connections 10",
            "niulai_db_pool_idle 3",
        ] {
            assert!(text.lines().any(|l| l == line), "缺少 {line}：\n{text}");
        }
    }

    #[test]
    fn labels_are_escaped() {
        assert_eq!(escape_label("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }
}
