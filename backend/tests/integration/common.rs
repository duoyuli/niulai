//! 各领域集成测试共用的临时数据库、引擎配置、登录和请求辅助函数，以及跨领域使用的测试数据。

pub use axum::body::Body;
pub use axum::http::{Request, StatusCode};
pub use http_body_util::BodyExt;
pub use niulai::config::Settings;
pub use niulai::sync::Engine;
pub use niulai::sync::records::build_rows;
pub use niulai::sync::sinks::SinkContext;
pub use niulai::sync::spec::Registry;
pub use niulai::sync::{postprocess, sinks};
pub use niulai::vendor::{HithinkClient, RateLimiter};
pub use serde_json::{Value, json};
pub use sqlx::{Connection, PgConnection, PgPool};
pub use std::sync::Arc;
pub use std::time::Duration;
pub use tower::ServiceExt;

pub struct TestDb {
    pub admin_url: String,
    pub name: String,
    pub pool: PgPool,
}

impl TestDb {
    pub async fn create() -> Option<Self> {
        let admin_url = std::env::var("NIULAI_TEST_DATABASE_URL").ok()?;
        let name = format!("niulai_test_{}_{}", std::process::id(), rand_suffix());
        let mut admin = PgConnection::connect(&admin_url).await.expect("连接测试库");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&mut admin)
            .await
            .unwrap();
        let url = replace_db(&admin_url, &name);
        // 和程序一样，会话时区固定为上海
        let pool = niulai::db::pool(&url, 5).await.unwrap();
        let tz: String = sqlx::query_scalar("SHOW timezone")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tz, niulai::db::TIME_ZONE);
        niulai::db::migrate(&pool)
            .await
            .expect("迁移应在空库上成功");
        let settings = bare_settings("http://127.0.0.1:1", Some("test"));
        niulai::bootstrap::bootstrap(&pool, &settings)
            .await
            .expect("应用启动初始化应成功");
        Some(TestDb {
            admin_url,
            name,
            pool,
        })
    }

    pub async fn drop(self) {
        self.pool.close().await;
        let mut admin = PgConnection::connect(&self.admin_url).await.unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            self.name
        )))
        .execute(&mut admin)
        .await
        .unwrap();
    }
}

/// 临时库名的后缀：进程内递增的序号，保证并行运行的测试不会撞名
pub fn rand_suffix() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn replace_db(url: &str, db: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("连接串应包含库名");
    format!("{base}/{db}")
}

pub fn registry() -> Registry {
    Registry::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml"))
        .unwrap()
}

pub fn sink_ctx<'a>(
    run_id: i64,
    dataset_id: &'a str,
    vars: &'a serde_json::Map<String, Value>,
) -> SinkContext<'a> {
    SinkContext {
        dataset_id,
        run_id,
        trade_date: Some("2026-06-29".parse().unwrap()),
        vars,
        sink_mode: match dataset_id {
            "a_share.snapshot" | "fund.market_snapshot" | "index.snapshot" => Some("snapshot"),
            "a_share.auction_final" => Some("float_cap"),
            _ => Some("history"),
        },
    }
}

/// 贵州茅台 2026-06-24 至 06-29 的原始日线，06-26 除息 28.02423 元
pub fn moutai_bars() -> Vec<(String, Value)> {
    [(1_782_230_400_000_i64, 1207.68), (1_782_316_800_000, 1212.10), (1_782_403_200_000, 1168.63), (1_782_662_400_000, 1194.96)]
        .iter()
        .map(|(ms, c)| ("item".to_string(), json!({"date_ms": ms, "open_price": c, "high_price": c, "low_price": c, "close_price": c, "volume": 1000.0, "turnover": 1.0e6})))
        .collect()
}

pub async fn load_fixture(pool: &PgPool, reg: &Registry) -> i64 {
    niulai::sync::registry::sync_to_db(pool, reg).await.unwrap();
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, trade_date) VALUES ('backfill', '2026-06-29') RETURNING run_id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    // 证券
    let tickers = reg.get("meta.tickers").unwrap();
    let vars = json!({"asset_type": "a-share"})
        .as_object()
        .unwrap()
        .clone();
    let recs = vec![(
        "item".to_string(),
        json!({"thscode": "600519.SH", "ticker": "600519", "name": "贵州茅台", "exchange": "SH", "asset_type": "a-share", "list_date": "2001-08-27"}),
    )];
    let rows = build_rows(tickers, &vars, &recs, Some("2026-06-29")).unwrap();
    sinks::apply(
        pool,
        "ref.security",
        &sink_ctx(run_id, "meta.tickers", &vars),
        &rows,
    )
    .await
    .unwrap();
    // 日线
    let bar = reg.get("a_share.bar_1d").unwrap();
    let vars = json!({"thscode": "600519.SH"}).as_object().unwrap().clone();
    let rows = build_rows(bar, &vars, &moutai_bars(), None).unwrap();
    let ctx = sink_ctx(run_id, "a_share.bar_1d", &vars);
    assert_eq!(
        sinks::apply(pool, "mkt.stock_bar_1d", &ctx, &rows)
            .await
            .unwrap(),
        4
    );
    // 重复写入同样的数据：没有变化
    assert_eq!(
        sinks::apply(pool, "mkt.stock_bar_1d", &ctx, &rows)
            .await
            .unwrap(),
        0,
        "UPSERT 应幂等"
    );
    // 公司行动（导出文件格式）
    let ca = reg.get("a_share.corp_action_dump").unwrap();
    let empty = serde_json::Map::new();
    let recs = vec![(
        "parquet".to_string(),
        json!({"thscode": "600519.SH", "ex_date_ms": 1_782_403_200_000_i64, "dividend_per_share": 28.02423, "per_share_bonus": 0.0, "allotment_ratio": 0.0, "allotment_price": 0.0}),
    )];
    let rows = build_rows(ca, &empty, &recs, None).unwrap();
    assert_eq!(rows[0].rkey, "600519.SH|2026-06-26|1");
    sinks::apply(
        pool,
        "mkt.corp_action",
        &sink_ctx(run_id, "a_share.corp_action_dump", &empty),
        &rows,
    )
    .await
    .unwrap();
    postprocess::recompute_adjustments(pool, run_id)
        .await
        .unwrap();
    run_id
}

/// 模拟同花顺接口：Key 是 bad 时返回 2001（认证失败），否则返回空结果
pub async fn mock_vendor() -> String {
    use axum::http::HeaderMap;
    let app = axum::Router::new().fallback(|headers: HeaderMap| async move {
        let ok = headers.get("x-api-key").is_some_and(|k| k != "bad");
        let body = if ok {
            json!({"code": 0, "message": "ok", "request_id": "t", "data": {"item": []}})
        } else {
            json!({"code": 2001, "message": "invalid api key", "request_id": "t", "data": null})
        };
        axum::Json(body)
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.ok() });
    format!("http://{addr}")
}

pub fn bare_settings(vendor: &str, api_key: Option<&str>) -> Settings {
    Settings {
        database_url: String::new(),
        redis_url: None,
        hithink_finance_api_key: api_key.map(str::to_string),
        vendor_base_url: vendor.into(),
        datasets_path: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml"),
        ),
        api_bind: String::new(),
        api_timeout_s: 300,
        db_max_connections: 2,
        worker_concurrency: 1,
        rate_limit_rps: 20,
        http_timeout_s: 5,
        scheduler_enabled: false,
        web_dir: None,
        admin_user: "admin".into(),
        admin_password: "secret-pass".into(),
        upgrade_dir: None,
        update_repo: String::new(),
    }
}

/// 登录后返回带登录 Cookie 的路由，之后的请求都自动带上它
pub async fn logged_in(app: axum::Router) -> axum::Router {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.7")
        .header("user-agent", "NiulaiTestDevice/1.0")
        .body(Body::from(
            json!({"username": "admin", "password": "secret-pass"}).to_string(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "测试账号登录失败");
    let set = resp.headers()["set-cookie"].to_str().unwrap();
    let cookie = axum::http::HeaderValue::from_str(set.split(';').next().unwrap()).unwrap();
    app.layer(tower::util::MapRequestLayer::new(
        move |mut req: Request<Body>| {
            req.headers_mut().insert("cookie", cookie.clone());
            req
        },
    ))
}

pub async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    let req = req
        .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// 登录，成功时返回 Cookie（`niulai_session=...`）
pub async fn login(app: &axum::Router, username: &str, password: &str) -> Option<String> {
    let (resp, _) = call_as(
        app,
        None,
        "POST",
        "/api/v1/auth/login",
        Some(json!({"username": username, "password": password})),
    )
    .await;
    (resp.status() == StatusCode::OK).then(|| {
        resp.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    })
}

/// 用指定的 Cookie 调用接口，返回响应头和 JSON
pub async fn call_as(
    app: &axum::Router,
    cookie: Option<&str>,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (axum::http::Response<()>, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    let resp = app
        .clone()
        .oneshot(
            req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        axum::http::Response::from_parts(parts, ()),
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// 一只合成股票：120 个交易日，先跌后涨，最后一天（2026-06-19）放量站上 20 日均线
pub async fn synthetic_stock(pool: &PgPool) {
    let run_id: i64 =
        sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id")
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO ref.security (thscode, ticker, name, exchange, board, list_date, first_seen, last_seen) VALUES ('600000.SH', '600000', '测试银行', 'SH', 'main', '2020-01-02', current_date, current_date)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id)
         SELECT '600000.SH', d, c, c * 1.01, c * 0.99, c, v, v * c, $1
         FROM (SELECT d::date AS d, row_number() OVER (ORDER BY d) AS i FROM generate_series('2026-01-05'::date, '2026-10-30'::date, '1 day') d
               WHERE extract(isodow FROM d) < 6 LIMIT 120) t,
         LATERAL (SELECT round((CASE WHEN i < 90 THEN 20 - i * 0.05 ELSE 15.5 + (i - 90) * 0.25 END)::numeric, 2) AS c,
                         CASE WHEN i = 120 THEN 9000000 ELSE 1000000 END AS v) x",
    )
    .bind(run_id)
    .execute(pool)
    .await
    .unwrap();
}
