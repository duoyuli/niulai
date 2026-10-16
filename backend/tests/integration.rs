//! 集成测试：在临时数据库上验证迁移、写入、复权、任务语义和 API。不调用供应商接口。
//!
//! 需要一个有建库权限的 PostgreSQL 连接串：
//!   NIULAI_TEST_DATABASE_URL=postgres://user:pass@127.0.0.1:5432/postgres cargo test --test integration
//! 未设置时测试直接跳过。每个测试创建并在结束时删除自己的数据库。

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use niulai::config::Settings;
use niulai::sync::Engine;
use niulai::sync::records::build_rows;
use niulai::sync::sinks::SinkContext;
use niulai::sync::spec::Registry;
use niulai::sync::{postprocess, sinks};
use niulai::vendor::{HithinkClient, RateLimiter};
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection, PgPool};
use tower::ServiceExt;

struct TestDb {
    admin_url: String,
    name: String,
    pool: PgPool,
}

impl TestDb {
    async fn create() -> Option<Self> {
        let admin_url = std::env::var("NIULAI_TEST_DATABASE_URL").ok()?;
        let name = format!("niulai_test_{}_{}", std::process::id(), rand_suffix());
        let mut admin = PgConnection::connect(&admin_url).await.expect("连接测试库");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&mut admin).await.unwrap();
        let url = replace_db(&admin_url, &name);
        let pool = PgPool::connect(&url).await.unwrap();
        niulai::db::migrate(&pool).await.expect("迁移应在空库上成功");
        Some(TestDb { admin_url, name, pool })
    }

    async fn drop(self) {
        self.pool.close().await;
        let mut admin = PgConnection::connect(&self.admin_url).await.unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name))).execute(&mut admin).await.unwrap();
    }
}

/// 临时库名的后缀：进程内递增的序号，保证并行运行的测试不会撞名
fn rand_suffix() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn replace_db(url: &str, db: &str) -> String {
    let (base, _) = url.rsplit_once('/').expect("连接串应包含库名");
    format!("{base}/{db}")
}

fn registry() -> Registry {
    Registry::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml")).unwrap()
}

fn sink_ctx<'a>(run_id: i64, dataset_id: &'a str, vars: &'a serde_json::Map<String, Value>) -> SinkContext<'a> {
    SinkContext { dataset_id, run_id, trade_date: Some("2026-06-29".parse().unwrap()), vars }
}

/// 贵州茅台 2026-06-24 至 06-29 的原始日线，06-26 除息 28.02423 元
fn moutai_bars() -> Vec<(String, Value)> {
    [(1_782_230_400_000_i64, 1207.68), (1_782_316_800_000, 1212.10), (1_782_403_200_000, 1168.63), (1_782_662_400_000, 1194.96)]
        .iter()
        .map(|(ms, c)| ("item".to_string(), json!({"date_ms": ms, "open_price": c, "high_price": c, "low_price": c, "close_price": c, "volume": 1000.0, "turnover": 1.0e6})))
        .collect()
}

async fn load_fixture(pool: &PgPool, reg: &Registry) -> i64 {
    niulai::sync::registry::sync_to_db(pool, reg).await.unwrap();
    let run_id: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind, trade_date) VALUES ('backfill', '2026-06-29') RETURNING run_id")
        .fetch_one(pool)
        .await
        .unwrap();
    // 证券
    let tickers = reg.get("meta.tickers").unwrap();
    let vars = json!({"asset_type": "a-share"}).as_object().unwrap().clone();
    let recs = vec![("item".to_string(), json!({"thscode": "600519.SH", "ticker": "600519", "name": "贵州茅台", "exchange": "SH", "asset_type": "a-share", "list_date": "2001-08-27"}))];
    let rows = build_rows(tickers, &vars, &recs, Some("2026-06-29")).unwrap();
    sinks::apply(pool, "ref.security", &sink_ctx(run_id, "meta.tickers", &vars), &rows).await.unwrap();
    // 日线
    let bar = reg.get("a_share.bar_1d").unwrap();
    let vars = json!({"thscode": "600519.SH"}).as_object().unwrap().clone();
    let rows = build_rows(bar, &vars, &moutai_bars(), None).unwrap();
    let ctx = sink_ctx(run_id, "a_share.bar_1d", &vars);
    assert_eq!(sinks::apply(pool, "mkt.stock_bar_1d", &ctx, &rows).await.unwrap(), 4);
    // 重复写入同样的数据：没有变化
    assert_eq!(sinks::apply(pool, "mkt.stock_bar_1d", &ctx, &rows).await.unwrap(), 0, "UPSERT 应幂等");
    // 公司行动（导出文件格式）
    let ca = reg.get("a_share.corp_action_dump").unwrap();
    let empty = serde_json::Map::new();
    let recs = vec![("parquet".to_string(), json!({"thscode": "600519.SH", "ex_date_ms": 1_782_403_200_000_i64, "dividend_per_share": 28.02423, "per_share_bonus": 0.0, "allotment_ratio": 0.0, "allotment_price": 0.0}))];
    let rows = build_rows(ca, &empty, &recs, None).unwrap();
    assert_eq!(rows[0].rkey, "600519.SH|2026-06-26|1");
    sinks::apply(pool, "mkt.corp_action", &sink_ctx(run_id, "a_share.corp_action_dump", &empty), &rows).await.unwrap();
    postprocess::recompute_adjustments(pool, run_id).await.unwrap();
    run_id
}

#[tokio::test]
async fn adjustment_and_api_end_to_end() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let reg = registry();
    load_fixture(&db.pool, &reg).await;

    // 等比前复权：除息日之前乘以 (1212.10 − 28.02423) / 1212.10
    let qfq: Vec<(String, f64)> = sqlx::query_as("SELECT trade_date::text, close FROM mkt.v_stock_bar_qfq WHERE thscode = '600519.SH' ORDER BY 1")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    let ratio = (1212.10 - 28.02423) / 1212.10;
    assert!((qfq[1].1 - 1212.10 * ratio).abs() < 1e-9);
    assert!((qfq[2].1 - 1168.63).abs() < 1e-9, "除息日及之后保持原价");
    // 同花顺口径（减法）：除息日之前减去分红
    let ths: f64 = sqlx::query_scalar("SELECT close FROM mkt.v_stock_bar_qfq_ths WHERE thscode = '600519.SH' AND trade_date = '2026-06-25'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!((ths - (1212.10 - 28.02423)).abs() < 1e-9);

    // API，同时提供一个最小的前端目录
    let web = std::env::temp_dir().join(format!("niulai_web_{}_{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>niulai</title>").unwrap();
    std::fs::write(web.join("assets/app-abc123.js"), "console.log(1)").unwrap();
    let settings = Settings {
        database_url: String::new(),
        hithink_finance_api_key: Some("test".into()),
        vendor_base_url: "http://127.0.0.1:1".into(),
        datasets_path: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml")),
        api_bind: String::new(),
        db_max_connections: 2,
        worker_concurrency: 1,
        rate_limit_rps: 1,
        http_timeout_s: 1,
        schedule_after_close: "18:00".into(),
        scheduler_enabled: false,
        web_dir: Some(web.clone()),
        admin_user: "admin".into(),
        admin_password: "secret-pass".into(),
    };
    let limiter = RateLimiter::new(1);
    let client = HithinkClient::new("http://127.0.0.1:1", Some("test"), Duration::from_secs(1), limiter).unwrap();
    let engine = Arc::new(Engine { settings, pool: db.pool.clone(), client: Arc::new(client), registry: Arc::new(reg) });
    let app = logged_in(niulai::api::router(engine.clone())).await;

    // 同一时刻只有一个进程能执行同步任务；持有者释放后其他进程才能取得
    let lock = engine.lock_executor().await.unwrap();
    assert!(engine.lock_executor().await.is_err(), "第二个执行者应被拒绝");
    drop(lock);
    let mut relocked = false;
    for _ in 0..50 {
        // 连接关闭后服务端才释放锁，稍等片刻
        if engine.lock_executor().await.is_ok() {
            relocked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(relocked, "锁应随连接关闭释放");
    let get = |uri: &str| {
        let app = app.clone();
        let req = Request::get(uri).body(Body::empty()).unwrap();
        async move {
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            (status, serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null))
        }
    };
    let (st, body) = get("/api/v1/securities/600519.SH/bars?adjust=qfq").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["bars"].as_array().unwrap().len(), 4);
    assert_eq!(body["bars"][2]["close"], "1168.630000");
    let (st, body) = get("/api/v1/securities/600519.SH").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["name"], "贵州茅台");
    assert_eq!(body["board"], "main");
    let (st, _) = get("/api/v1/securities/600519.SH/bars?adjust=bad").await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = get("/api/v1/securities/NOPE.SH").await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // 自选股：收藏、重复收藏、诊断、取消收藏
    let send = |method: &str, uri: &str| {
        let app = app.clone();
        let req = Request::builder().method(method).uri(uri).body(Body::empty()).unwrap();
        async move { app.oneshot(req).await.unwrap().status() }
    };
    assert_eq!(send("PUT", "/api/v1/watchlist/600519.SH").await, StatusCode::NO_CONTENT);
    assert_eq!(send("PUT", "/api/v1/watchlist/600519.SH").await, StatusCode::NO_CONTENT);
    assert_eq!(send("PUT", "/api/v1/watchlist/NOPE.SH").await, StatusCode::NOT_FOUND);
    let (st, body) = get("/api/v1/watchlist?fast=2&slow=3").await;
    assert_eq!(st, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "贵州茅台");
    assert!(items[0]["close"].is_string(), "未复权收盘价是十进制字符串");
    let diag = &items[0]["diagnosis"];
    assert!(diag["change_1d"].is_number());
    assert!(diag["ma"]["ma_slow"].is_number(), "4 根日线够算 3 日均线");
    assert_eq!(send("DELETE", "/api/v1/watchlist/600519.SH").await, StatusCode::NO_CONTENT);
    let (_, body) = get("/api/v1/watchlist").await;
    assert_eq!(body["items"].as_array().unwrap().len(), 0);

    // 全市场扫描：默认日期是本地最新交易日；第二次命中缓存，结果相同
    let (st, scan1) = get("/api/v1/signals/ma-cross?fast=2&slow=3").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(scan1["trade_date"], "2026-06-29");
    assert_eq!(scan1["scanned"], 1);
    let (_, scan2) = get("/api/v1/signals/ma-cross?fast=2&slow=3").await;
    assert_eq!(scan1, scan2);

    // 回测：每个交易日一个账户总值；参数不合法时 400
    let (st, bt) = get("/api/v1/backtest/ma-cross?thscode=600519.SH&fast=2&slow=3").await;
    assert_eq!(st, StatusCode::OK, "{bt}");
    assert_eq!(bt["equity"].as_array().unwrap().len(), 4);
    assert_eq!(bt["summary"]["capital"], 100000.0);
    let (st, _) = get("/api/v1/backtest/ma-cross?thscode=600519.SH&commission_rate=0.5").await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = get("/api/v1/backtest/ma-cross?thscode=NOPE.SH").await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, body) = get("/api/v1/openapi.json").await;
    assert_eq!(st, StatusCode::OK);
    assert!(body["paths"]["/securities/{thscode}/ma"].is_object());

    // 前端：页面路径回退到 index.html，静态资源长期缓存，未知接口仍是 JSON 404
    let page = |uri: &str| {
        let app = app.clone();
        let req = Request::get(uri).body(Body::empty()).unwrap();
        async move { app.oneshot(req).await.unwrap() }
    };
    let resp = page("/securities/600519.SH").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["cache-control"], "no-cache");
    assert!(String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes()).contains("<title>niulai</title>"));
    let resp = page("/assets/app-abc123.js").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()["cache-control"].to_str().unwrap().contains("immutable"));
    let (st, body) = get("/api/v1/no-such-endpoint").await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
    std::fs::remove_dir_all(&web).ok();

    db.drop().await;
}

#[tokio::test]
async fn task_queue_claim_dedup_and_stale_requeue() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let reg = registry();
    niulai::sync::registry::sync_to_db(&db.pool, &reg).await.unwrap();
    let run_id: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id").fetch_one(&db.pool).await.unwrap();
    let insert = "INSERT INTO ops.task (run_id, dataset_id, part_key, params) VALUES ($1, 'a_share.calendar', '*', '{}')
                  ON CONFLICT (dataset_id, part_key) WHERE status IN ('pending', 'running', 'failed') DO NOTHING";
    assert_eq!(sqlx::query(insert).bind(run_id).execute(&db.pool).await.unwrap().rows_affected(), 1);
    // 同一分区已有未完成任务：重复规划不会产生第二个任务
    assert_eq!(sqlx::query(insert).bind(run_id).execute(&db.pool).await.unwrap().rows_affected(), 0);

    // 认领后模拟执行者崩溃：超时回收，放回队列
    sqlx::query("UPDATE ops.task SET status = 'running', locked_by = 'w1', locked_at = now() - interval '1 hour', attempts = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let n: i32 = sqlx::query_scalar("SELECT ops.requeue_stale('10 minutes')").fetch_one(&db.pool).await.unwrap();
    assert_eq!(n, 1);
    let status: String = sqlx::query_scalar("SELECT status FROM ops.task").fetch_one(&db.pool).await.unwrap();
    assert_eq!(status, "failed", "回收后应能被重新认领");

    // 通用记录库：首次写入、重复写入、修订
    let recs = json!([{"rkey": "k1", "obs_date": "2026-10-14", "thscode": "600519.SH", "payload": {"v": 1}}]);
    let first: (i32, i32) = sqlx::query_as("SELECT * FROM raw.upsert_records('a_share.calendar', 1, $1)").bind(&recs).fetch_one(&db.pool).await.unwrap();
    let again: (i32, i32) = sqlx::query_as("SELECT * FROM raw.upsert_records('a_share.calendar', 1, $1)").bind(&recs).fetch_one(&db.pool).await.unwrap();
    let revised = json!([{"rkey": "k1", "obs_date": "2026-10-14", "thscode": "600519.SH", "payload": {"v": 2}}]);
    let rev: (i32, i32) = sqlx::query_as("SELECT * FROM raw.upsert_records('a_share.calendar', 1, $1)").bind(&revised).fetch_one(&db.pool).await.unwrap();
    assert_eq!((first, again, rev), ((1, 0), (0, 0), (0, 1)));
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM raw.record_version").fetch_one(&db.pool).await.unwrap();
    assert_eq!(versions, 1, "旧值应写入修订历史");

    db.drop().await;
}

#[tokio::test]
async fn run_completes_only_after_its_last_task() {
    use niulai::sync::queue::{Finished, TaskQueue, TaskStats};
    use niulai::sync::run::{self, NewRun, RunKind};
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    niulai::sync::registry::sync_to_db(&db.pool, &registry()).await.unwrap();
    let datasets = vec!["a_share.calendar".to_string()];
    let mut tx = db.pool.begin().await.unwrap();
    let new_run = NewRun { kind: RunKind::Backfill, trade_date: "2026-10-15".parse().unwrap(), datasets: &datasets, pending_waves: &[], root_run: None, only_thscodes: &[] };
    let run_id = run::create(&mut tx, &new_run).await.unwrap();
    for part in ["a", "b", "c"] {
        sqlx::query("INSERT INTO ops.task (run_id, dataset_id, part_key, params) VALUES ($1, 'a_share.calendar', $2, '{}')")
            .bind(run_id)
            .bind(part)
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    run::set_total(&mut tx, run_id, 3).await.unwrap();
    tx.commit().await.unwrap();

    let q = TaskQueue::new(db.pool.clone(), "w1".into());
    // 推迟：放回队列，不计入尝试次数
    let t = q.claim().await.unwrap().unwrap();
    q.defer(&t, "未就绪").await.unwrap();
    let (status, attempts): (String, i16) = sqlx::query_as("SELECT status, attempts FROM ops.task WHERE task_id = $1").bind(t.task_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!((status.as_str(), attempts), ("pending", 0));

    let t = q.claim().await.unwrap().unwrap();
    assert!(q.heartbeat(t.task_id).await.unwrap(), "本执行者持有的任务应能续租");
    assert!(!TaskQueue::new(db.pool.clone(), "w2".into()).heartbeat(t.task_id).await.unwrap(), "其他执行者不能续租");
    q.finish(&t, Finished::Done, TaskStats { pages: 1, records: 5, changed: 5 }, None).await.unwrap();
    assert_eq!(run::try_complete(&db.pool, run_id).await.unwrap(), None, "还有未完成的任务");

    let t = q.claim().await.unwrap().unwrap();
    q.fail(&t, Some(1002), "参数错误", true).await.unwrap();
    let progress: (i32, i32) = sqlx::query_as("SELECT tasks_done, tasks_failed FROM ops.run WHERE run_id = $1").bind(run_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(progress, (1, 1), "完成和失败的计数随任务结束累加");

    // 最后一个任务被超时回收后直接标为 dead：没有执行者结束运行，由维护流程发现
    sqlx::query("UPDATE ops.task SET status = 'running', locked_by = 'gone', locked_at = now() - interval '1 hour', attempts = max_attempts WHERE status = 'pending'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(q.requeue_stale().await.unwrap(), 1);
    assert_eq!(run::stalled(&db.pool).await.unwrap(), vec![run_id]);
    assert_eq!(run::try_complete(&db.pool, run_id).await.unwrap().as_deref(), Some("partial"));
    assert_eq!(run::try_complete(&db.pool, run_id).await.unwrap(), None, "运行只能被结束一次");
    let counts: (i32, i32) = sqlx::query_as("SELECT tasks_done, tasks_failed FROM ops.run WHERE run_id = $1").bind(run_id).fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (1, 2), "结束时按任务表重新统计");

    db.drop().await;
}

/// 模拟同花顺接口：Key 是 bad 时返回 2001（认证失败），否则返回空结果
async fn mock_vendor() -> String {
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

fn bare_settings(vendor: &str, api_key: Option<&str>) -> Settings {
    Settings {
        database_url: String::new(),
        hithink_finance_api_key: api_key.map(str::to_string),
        vendor_base_url: vendor.into(),
        datasets_path: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml")),
        api_bind: String::new(),
        db_max_connections: 2,
        worker_concurrency: 1,
        rate_limit_rps: 20,
        http_timeout_s: 5,
        schedule_after_close: "18:00".into(),
        scheduler_enabled: false,
        web_dir: None,
        admin_user: "admin".into(),
        admin_password: "secret-pass".into(),
    }
}

/// 登录后返回带登录 Cookie 的路由，之后的请求都自动带上它
async fn logged_in(app: axum::Router) -> axum::Router {
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(json!({"username": "admin", "password": "secret-pass"}).to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "测试账号登录失败");
    let set = resp.headers()["set-cookie"].to_str().unwrap();
    let cookie = axum::http::HeaderValue::from_str(set.split(';').next().unwrap()).unwrap();
    app.layer(tower::util::MapRequestLayer::new(move |mut req: Request<Body>| {
        req.headers_mut().insert("cookie", cookie.clone());
        req
    }))
}

async fn call(app: &axum::Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let req = req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string()))).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn login_is_required_and_lasts_30_days() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let engine = Engine::new(bare_settings("http://127.0.0.1:1", Some("test")), db.pool.clone()).await.unwrap();
    let app = niulai::api::router(engine.clone());

    // 健康检查不用登录，其余接口要登录
    let (st, _) = call(&app, "GET", "/api/v1/health", None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, body) = call(&app, "GET", "/api/v1/strategies", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
    let (st, _) = call(&app, "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // 密码不对
    let (st, _) = call(&app, "POST", "/api/v1/auth/login", Some(json!({"username": "admin", "password": "wrong"}))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // 登录成功：Cookie 保存 30 天
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(json!({"username": "admin", "password": "secret-pass"}).to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let set = resp.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(set.contains("Max-Age=2592000") && set.contains("HttpOnly"), "{set}");
    let cookie = set.split(';').next().unwrap().to_string();
    let with = |uri: &str, cookie: &str| Request::builder().uri(uri).header("cookie", cookie).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(with("/api/v1/auth/me", &cookie)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = app.clone().oneshot(with("/api/v1/strategies", &cookie)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 篡改过的 Cookie 无效
    let forged = format!("{}0", &cookie[..cookie.len() - 1]);
    let resp = app.clone().oneshot(with("/api/v1/strategies", &forged)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 重启后（新的路由，同一个数据库）登录状态仍然有效；改了密码后失效
    let app2 = niulai::api::router(engine.clone());
    assert_eq!(app2.oneshot(with("/api/v1/auth/me", &cookie)).await.unwrap().status(), StatusCode::OK);
    let mut changed = bare_settings("http://127.0.0.1:1", Some("test"));
    changed.admin_password = "new-pass".into();
    let app3 = niulai::api::router(Engine::new(changed, db.pool.clone()).await.unwrap());
    assert_eq!(app3.oneshot(with("/api/v1/auth/me", &cookie)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    db.drop().await;
}

#[tokio::test]
async fn api_key_setup_flow() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let vendor = mock_vendor().await;

    // 没有 Key：引导页显示未配置，执行循环暂停
    let engine = Engine::new(bare_settings(&vendor, None), db.pool.clone()).await.unwrap();
    assert!(!engine.client.has_api_key());
    let app = logged_in(niulai::api::router(engine.clone())).await;
    let (st, body) = call(&app, "GET", "/api/v1/setup", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["api_key_configured"], false);
    assert_eq!(body["api_key_url"], "https://fuyao.aicubes.cn/admin/");
    assert_eq!(body["backfill_planned"], false);

    // 空 Key、被拒绝的 Key：400，不保存
    let (st, _) = call(&app, "PUT", "/api/v1/setup/api-key", Some(json!({"api_key": "  "}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, body) = call(&app, "PUT", "/api/v1/setup/api-key", Some(json!({"api_key": "bad"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
    assert!(!engine.client.has_api_key());

    // 有效的 Key：保存、立即生效，第一次配置时规划全量回补
    let (st, body) = call(&app, "PUT", "/api/v1/setup/api-key", Some(json!({"api_key": " good-key-1234 "}))).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body["backfill_run_id"].is_i64());
    assert!(engine.client.has_api_key());
    let (_, body) = call(&app, "GET", "/api/v1/setup", None).await;
    assert_eq!(body["api_key_configured"], true);
    assert_eq!(body["api_key_source"], "database");
    assert_eq!(body["api_key_hint"], "••••1234");
    assert_eq!(body["backfill_planned"], true);

    // 更换 Key 不会再次规划回补
    let (st, body) = call(&app, "PUT", "/api/v1/setup/api-key", Some(json!({"api_key": "good-key-5678"}))).await;
    assert_eq!(st, StatusCode::OK);
    assert!(body["backfill_run_id"].is_null());

    // 重启后从数据库读回 Key
    let restarted = Engine::new(bare_settings(&vendor, None), db.pool.clone()).await.unwrap();
    assert!(restarted.client.has_api_key());

    // 环境变量提供 Key 时，网页上不能修改
    let from_env = Engine::new(bare_settings(&vendor, Some("env-key-0000")), db.pool.clone()).await.unwrap();
    let app = logged_in(niulai::api::router(from_env)).await;
    let (_, body) = call(&app, "GET", "/api/v1/setup", None).await;
    assert_eq!(body["api_key_source"], "env");
    assert_eq!(body["api_key_hint"], "••••0000");
    let (st, _) = call(&app, "PUT", "/api/v1/setup/api-key", Some(json!({"api_key": "good-key-9999"}))).await;
    assert_eq!(st, StatusCode::CONFLICT);

    db.drop().await;
}

#[tokio::test]
async fn strategies_screening_diagnosis_and_backtests() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    // 一只合成股票：120 个交易日，先跌后涨，最后一天放量站上 20 日均线
    let run_id: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id").fetch_one(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO ref.security (thscode, ticker, name, exchange, board, list_date, first_seen, last_seen) VALUES ('600000.SH', '600000', '测试银行', 'SH', 'main', '2020-01-02', current_date, current_date)")
        .execute(&db.pool)
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
    .execute(&db.pool)
    .await
    .unwrap();

    let settings = bare_settings("http://127.0.0.1:1", Some("test"));
    let engine = Engine::new(settings, db.pool.clone()).await.unwrap();
    let app = logged_in(niulai::api::router(engine)).await;

    // 首次启动时复制了 10 个模板
    let (st, list) = call(&app, "GET", "/api/v1/strategies", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 10);
    let (_, cat) = call(&app, "GET", "/api/v1/strategies/catalog", None).await;
    assert!(cat["signals"].as_array().unwrap().len() >= 20);

    // 试运行：最后一天放量（9 倍）且站在 20 日均线上方
    let def = json!({
        "universe": {"exclude_st": true, "min_price": 1, "min_amount": 1000},
        "all": [
            {"kind": "signal", "signal": "volume_surge", "params": {"ratio": 3}},
            {"kind": "compare", "left": {"id": "close"}, "op": ">", "right": {"id": "ma", "n": 20}}
        ],
        "exit": {"stop_loss_pct": 5, "max_hold_days": 10}
    });
    let (st, body) = call(&app, "POST", "/api/v1/strategies/preview", Some(json!({"definition": def}))).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["matched"], 1);
    assert_eq!(body["hits"][0]["thscode"], "600000.SH");
    assert!(body["hits"][0]["reasons"][0].as_str().unwrap().contains("放量"));

    // 保存、巡检、查结果
    let (st, created) = call(&app, "POST", "/api/v1/strategies", Some(json!({"name": "放量站上均线", "definition": def}))).await;
    assert_eq!(st, StatusCode::CREATED, "{created}");
    let id = created["strategy_id"].as_i64().unwrap();
    let (st, run) = call(&app, "POST", &format!("/api/v1/screening/run?strategy_id={id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{run}");
    assert_eq!(run["scanned"], 1);
    let (_, res) = call(&app, "GET", &format!("/api/v1/strategies/{id}/results"), None).await;
    assert_eq!(res["hits"].as_array().unwrap().len(), 1);
    assert_eq!(res["hits"][0]["is_new"], false, "第一次巡检没有可比较的上一次");
    // 补一次前一天（第 119 天，不满足）的巡检，再巡检最后一天：与前一天比是新入选
    let (st, _) = call(&app, "POST", &format!("/api/v1/screening/run?strategy_id={id}&date=2026-06-18"), None).await;
    assert_eq!(st, StatusCode::OK);
    call(&app, "POST", &format!("/api/v1/screening/run?strategy_id={id}"), None).await;
    let (_, res) = call(&app, "GET", &format!("/api/v1/strategies/{id}/results"), None).await;
    assert_eq!(res["trade_date"], "2026-06-19");
    assert_eq!(res["hits"][0]["is_new"], true);
    assert_eq!(res["dates"].as_array().unwrap().len(), 2);
    let (st, _) = call(&app, "PUT", &format!("/api/v1/strategies/{id}"), Some(json!({"name": "", "definition": def}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "名称不能为空");

    // 诊断、市场概况、回测
    let (st, diag) = call(&app, "GET", "/api/v1/securities/600000.SH/diagnosis", None).await;
    assert_eq!(st, StatusCode::OK, "{diag}");
    assert_eq!(diag["dimensions"].as_array().unwrap().len(), 4);
    assert!(diag["strategies"].as_array().unwrap().iter().any(|s| s["strategy_id"] == id), "当天满足刚保存的策略");
    let (st, ov) = call(&app, "GET", "/api/v1/market/overview", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ov["stocks"], 1);
    let (st, bt) = call(&app, "GET", &format!("/api/v1/backtest/strategy?strategy_id={id}&thscode=600000.SH"), None).await;
    assert_eq!(st, StatusCode::OK, "{bt}");
    assert_eq!(bt["equity"].as_array().unwrap().len(), 120);
    let (st, pf) = call(&app, "GET", &format!("/api/v1/backtest/portfolio?strategy_id={id}&start=2026-01-01&max_positions=3"), None).await;
    assert_eq!(st, StatusCode::OK, "{pf}");
    assert_eq!(pf["summary"]["signals"], 0, "信号在最后一天，第二天没有日线，不能买入");
    let (st, _) = call(&app, "GET", "/api/v1/backtest/portfolio?strategy_id=99999", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, _) = call(&app, "DELETE", &format!("/api/v1/strategies/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    db.drop().await;
}
