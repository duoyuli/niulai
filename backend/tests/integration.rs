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
        // 和程序一样，会话时区固定为上海
        let pool = niulai::db::pool(&url, 5).await.unwrap();
        let tz: String = sqlx::query_scalar("SHOW timezone").fetch_one(&pool).await.unwrap();
        assert_eq!(tz, niulai::db::TIME_ZONE);
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
        scheduler_enabled: false,
        web_dir: Some(web.clone()),
        admin_user: "admin".into(),
        admin_password: "secret-pass".into(),
        upgrade_dir: None,
        update_repo: String::new(),
    };
    let limiter = RateLimiter::new(1);
    let client = HithinkClient::new("http://127.0.0.1:1", Some("test"), Duration::from_secs(1), limiter).unwrap();
    // 直接组装引擎（不经过 Engine::new），要自己设置第一个管理员
    niulai::auth::init_first_admin(&db.pool, &settings).await.unwrap();
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
    assert!(body["paths"]["/securities/{thscode}/bars"].is_object());

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
    let new_run = NewRun { kind: RunKind::Backfill, trade_date: "2026-10-15".parse().unwrap(), datasets: &datasets, pending_waves: &[], root_run: None, only_thscodes: &[], trigger: None };
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
        scheduler_enabled: false,
        web_dir: None,
        admin_user: "admin".into(),
        admin_password: "secret-pass".into(),
        upgrade_dir: None,
        update_repo: String::new(),
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
    // 换掉签名的最后一个字符（换成和原来不同的字符，否则偶尔等于原签名）
    let last = if cookie.ends_with('0') { '1' } else { '0' };
    let forged = format!("{}{last}", &cookie[..cookie.len() - 1]);
    let resp = app.clone().oneshot(with("/api/v1/strategies", &forged)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 重启后（新的路由，同一个数据库）登录状态仍然有效
    let app2 = niulai::api::router(engine.clone());
    assert_eq!(app2.oneshot(with("/api/v1/auth/me", &cookie)).await.unwrap().status(), StatusCode::OK);
    // 配置里的账号密码只用来设置第一个管理员一次：之后改配置不影响登录，密码在网页上改
    let mut changed = bare_settings("http://127.0.0.1:1", Some("test"));
    changed.admin_password = "new-pass".into();
    let app3 = niulai::api::router(Engine::new(changed, db.pool.clone()).await.unwrap());
    assert_eq!(app3.clone().oneshot(with("/api/v1/auth/me", &cookie)).await.unwrap().status(), StatusCode::OK);
    assert!(login(&app3, "admin", "secret-pass").await.is_some() && login(&app3, "admin", "new-pass").await.is_none());
    db.drop().await;
}

/// 登录，成功时返回 Cookie（`niulai_session=...`）
async fn login(app: &axum::Router, username: &str, password: &str) -> Option<String> {
    let (resp, _) = call_as(app, None, "POST", "/api/v1/auth/login", Some(json!({"username": username, "password": password}))).await;
    (resp.status() == StatusCode::OK).then(|| resp.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string())
}

/// 用指定的 Cookie 调用接口，返回响应头和 JSON
async fn call_as(app: &axum::Router, cookie: Option<&str>, method: &str, uri: &str, body: Option<Value>) -> (axum::http::Response<()>, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    let resp = app.clone().oneshot(req.body(body.map_or(Body::empty(), |b| Body::from(b.to_string()))).unwrap()).await.unwrap();
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (axum::http::Response::from_parts(parts, ()), serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn users_are_added_by_admins_and_members_only_touch_their_own_data() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    synthetic_stock(&db.pool).await;
    let app = niulai::api::router(Engine::new(bare_settings("http://127.0.0.1:1", Some("test")), db.pool.clone()).await.unwrap());
    let admin = login(&app, "Admin", "secret-pass").await.expect("第一个管理员来自配置，用户名不区分大小写");
    let st = |r: &axum::http::Response<()>| r.status();

    // 没有注册接口；添加用户只能由管理员做，不填角色时是普通用户，不填密码时生成一个
    let (r, v) = call_as(&app, Some(&admin), "POST", "/api/v1/users", Some(json!({"username": "mama"}))).await;
    assert_eq!(st(&r), StatusCode::CREATED, "{v}");
    assert_eq!(v["user"]["role"], "member");
    assert!(v["user"].get("password_hash").is_none(), "不返回密码哈希");
    let generated = v["generated_password"].as_str().unwrap().to_string();
    let mama_id = v["user"]["user_id"].as_i64().unwrap();
    let (r, _) = call_as(&app, Some(&admin), "POST", "/api/v1/users", Some(json!({"username": "MAMA", "password": "12345678"}))).await;
    assert_eq!(st(&r), StatusCode::CONFLICT);
    let (r, _) = call_as(&app, Some(&admin), "POST", "/api/v1/users", Some(json!({"username": "kid", "password": "short"}))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);

    // 普通用户：能看，不能改策略、不能进管理功能；第一次登录提醒改密码
    let mama = login(&app, "mama", &generated).await.unwrap();
    let (_, me) = call_as(&app, Some(&mama), "GET", "/api/v1/auth/me", None).await;
    assert_eq!((me["role"].as_str(), me["must_change_password"].as_bool()), (Some("member"), Some(true)));
    for (method, uri, want) in [
        ("GET", "/api/v1/strategies", StatusCode::OK),
        ("POST", "/api/v1/strategies", StatusCode::FORBIDDEN),
        ("GET", "/api/v1/sync/runs", StatusCode::FORBIDDEN),
        ("GET", "/api/v1/setup", StatusCode::FORBIDDEN),
        ("GET", "/api/v1/users", StatusCode::FORBIDDEN),
        ("POST", "/api/v1/users", StatusCode::FORBIDDEN),
        ("POST", "/api/v1/system/upgrade", StatusCode::FORBIDDEN),
    ] {
        let (r, v) = call_as(&app, Some(&mama), method, uri, Some(json!({}))).await;
        assert_eq!(st(&r), want, "{method} {uri} {v}");
        if want == StatusCode::FORBIDDEN {
            assert_eq!(v["code"], "forbidden");
        }
    }

    // 自选股各自一份
    let (r, _) = call_as(&app, Some(&mama), "PUT", "/api/v1/watchlist/600000.SH", None).await;
    assert_eq!(st(&r), StatusCode::NO_CONTENT);
    let codes = |v: &Value| v["items"].as_array().unwrap().iter().map(|i| i["thscode"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let (_, v) = call_as(&app, Some(&mama), "GET", "/api/v1/watchlist", None).await;
    assert_eq!(codes(&v), ["600000.SH"]);
    let (_, v) = call_as(&app, Some(&admin), "GET", "/api/v1/watchlist", None).await;
    assert!(codes(&v).is_empty(), "管理员看不到别人的自选股");

    // 本人改密码：要先输对现在的密码；改完后旧的登录状态失效，响应里是新的
    let (r, _) = call_as(&app, Some(&mama), "PUT", "/api/v1/auth/password", Some(json!({"current_password": "wrong", "new_password": "mama-new-pass"}))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);
    let (r, v) = call_as(&app, Some(&mama), "PUT", "/api/v1/auth/password", Some(json!({"current_password": generated, "new_password": "mama-new-pass"}))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["must_change_password"], false);
    let fresh = r.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string();
    let (r, _) = call_as(&app, Some(&mama), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::UNAUTHORIZED);
    let (r, _) = call_as(&app, Some(&fresh), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::OK);

    // 管理员重置别人的密码：旧的登录失效，本人登录后又会被提醒修改；不能用这个接口重置自己
    let (r, v) = call_as(&app, Some(&admin), "POST", &format!("/api/v1/users/{mama_id}/password"), Some(json!({}))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let reset = v["generated_password"].as_str().unwrap().to_string();
    let (r, _) = call_as(&app, Some(&fresh), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::UNAUTHORIZED);
    let mama = login(&app, "mama", &reset).await.unwrap();
    let (_, me) = call_as(&app, Some(&admin), "GET", "/api/v1/auth/me", None).await;
    let admin_id = me["user_id"].as_i64().unwrap();
    let (r, _) = call_as(&app, Some(&admin), "POST", &format!("/api/v1/users/{admin_id}/password"), Some(json!({}))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);

    // 设为管理员后可以管理用户；不能修改自己；停用后已登录的立即失效，也不能再登录
    let (r, _) = call_as(&app, Some(&admin), "PUT", &format!("/api/v1/users/{mama_id}"), Some(json!({"role": "admin", "disabled": false}))).await;
    assert_eq!(st(&r), StatusCode::OK);
    let (r, _) = call_as(&app, Some(&mama), "GET", "/api/v1/users", None).await;
    assert_eq!(st(&r), StatusCode::OK);
    let (r, _) = call_as(&app, Some(&mama), "PUT", &format!("/api/v1/users/{mama_id}"), Some(json!({"role": "member", "disabled": false}))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);
    let (r, _) = call_as(&app, Some(&admin), "PUT", &format!("/api/v1/users/{mama_id}"), Some(json!({"role": "member", "disabled": true}))).await;
    assert_eq!(st(&r), StatusCode::OK);
    let (r, _) = call_as(&app, Some(&mama), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::UNAUTHORIZED);
    assert!(login(&app, "mama", &reset).await.is_none());

    // 至少留一个可以登录的管理员
    let err = niulai::auth::update(&db.pool, admin_id, niulai::auth::Role::Member, false).await.unwrap_err();
    assert!(err.to_string().contains("至少要留一个"), "{err}");

    // 连续输错 5 次后暂停登录，正确的密码也要等
    for _ in 0..5 {
        assert!(login(&app, "admin", "wrong-pass").await.is_none());
    }
    let (r, v) = call_as(&app, None, "POST", "/api/v1/auth/login", Some(json!({"username": "admin", "password": "secret-pass"}))).await;
    assert_eq!(st(&r), StatusCode::TOO_MANY_REQUESTS, "{v}");
    db.drop().await;
}

#[tokio::test]
async fn one_click_upgrade_hands_request_to_upgrader() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let dir = std::env::temp_dir().join(format!("niulai_upgrade_it_{}_{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut settings = bare_settings("http://127.0.0.1:1", Some("test"));
    settings.upgrade_dir = Some(dir.clone());
    let app = logged_in(niulai::api::router(Engine::new(settings, db.pool.clone()).await.unwrap())).await;

    // 没有心跳：不能一键升级
    let (st, v) = call(&app, "GET", "/api/v1/system/version", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["current"], "dev");
    assert_eq!(v["upgrade"]["available"], false);
    let (st, _) = call(&app, "POST", "/api/v1/system/upgrade", Some(json!({"version": "2026.10.18"}))).await;
    assert_eq!(st, StatusCode::CONFLICT);

    // upgrader 在运行：提交请求，写入版本号
    std::fs::write(dir.join("heartbeat"), chrono::Utc::now().timestamp().to_string()).unwrap();
    let (st, _) = call(&app, "POST", "/api/v1/system/upgrade", Some(json!({"version": "2026.10.18; rm -rf /"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "只接受正式版本号");
    let (st, body) = call(&app, "POST", "/api/v1/system/upgrade", Some(json!({"version": "v2026.10.18"}))).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "requested");
    assert_eq!(std::fs::read_to_string(dir.join("request")).unwrap(), "2026.10.18");
    let (st, _) = call(&app, "POST", "/api/v1/system/upgrade", Some(json!({"version": "2026.10.18"}))).await;
    assert_eq!(st, StatusCode::CONFLICT, "正在升级时不能再提交");

    // upgrader 接手并报告进度
    std::fs::remove_file(dir.join("request")).unwrap();
    std::fs::write(dir.join("status"), r#"{"state":"running","target":"2026.10.18","message":"拉取镜像","at":1}"#).unwrap();
    let (_, v) = call(&app, "GET", "/api/v1/system/version", None).await;
    assert_eq!(v["upgrade"]["state"], "running");
    assert_eq!(v["upgrade"]["message"], "拉取镜像");
    std::fs::remove_dir_all(&dir).unwrap();
    db.drop().await;
}

#[tokio::test]
async fn sync_schedule_can_be_viewed_and_changed() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let app = logged_in(niulai::api::router(Engine::new(bare_settings("http://127.0.0.1:1", Some("test")), db.pool.clone()).await.unwrap())).await;

    // 默认：每个交易日 18:00、每周六 08:00
    let (st, v) = call(&app, "GET", "/api/v1/sync/schedule", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["times"], json!({"daily_time": "18:00", "weekly_weekday": 6, "weekly_time": "08:00"}));
    let plan = |v: &Value, key: &str| v["plans"].as_array().unwrap().iter().find(|p| p["key"] == key).cloned().unwrap();
    let has = |p: &Value, id: &str| p["datasets"].as_array().unwrap().iter().any(|d| d == id);
    assert!(has(&plan(&v, "daily"), "a_share.snapshot"));
    assert!(has(&plan(&v, "weekly"), "meta.tickers") && has(&plan(&v, "weekly"), "a_share.calendar"));
    assert!(!has(&plan(&v, "daily"), "fund.nav"), "只列已开启的数据集（P2、P3 默认关闭）");
    assert!(plan(&v, "manual")["next_at"].is_null());
    assert!(plan(&v, "weekly")["next_at"].is_string());

    // 从配置删掉的数据集留在 ops.dataset 中（已关闭），数据集列表不再展示
    sqlx::query("INSERT INTO ops.dataset (dataset_id, domain, endpoint, priority, enabled, spec, spec_hash) VALUES ('a_share.removed', 'a_share', '/x', 'P1', false, '{}', '')")
        .execute(&db.pool)
        .await
        .unwrap();
    let (st, v) = call(&app, "GET", "/api/v1/datasets", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let ids: Vec<&str> = v.as_array().unwrap().iter().map(|d| d["dataset_id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"a_share.snapshot") && !ids.contains(&"a_share.removed"));

    let (st, _) = call(&app, "PUT", "/api/v1/sync/schedule", Some(json!({"daily_time": "25:00", "weekly_weekday": 6, "weekly_time": "08:00"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(&app, "PUT", "/api/v1/sync/schedule", Some(json!({"daily_time": "18:00", "weekly_weekday": 0, "weekly_time": "08:00"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(&app, "PUT", "/api/v1/sync/schedule", Some(json!({"daily_time": "19:30", "weekly_weekday": 7, "weekly_time": "9:05"}))).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["times"], json!({"daily_time": "19:30", "weekly_weekday": 7, "weekly_time": "09:05"}));
    assert_eq!(plan(&v, "weekly")["rule"], "每周日 09:05");
    let (_, v) = call(&app, "GET", "/api/v1/sync/schedule", None).await;
    assert_eq!(v["times"]["daily_time"], "19:30", "保存在数据库里");

    // 手动发起的运行记下来源
    let (st, body) = call(&app, "POST", "/api/v1/sync/runs", Some(json!({"kind": "incremental", "datasets": ["a_share.calendar"]}))).await;
    assert_eq!(st, StatusCode::ACCEPTED, "{body}");
    let (_, runs) = call(&app, "GET", "/api/v1/sync/runs", None).await;
    assert_eq!(runs[0]["params"]["trigger"], "manual");

    // 详情是整次同步；同一部分已经在排队的，再发起时不新建任务，并说明在哪次同步里
    let first = body["run_id"].as_i64().unwrap();
    let (st, d) = call(&app, "GET", &format!("/api/v1/sync/runs/{first}"), None).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert_eq!(d["waves"].as_array().unwrap().len(), 1);
    assert_eq!(d["progress"][0]["dataset_id"], "a_share.calendar");
    assert_eq!(d["progress"][0]["pending"], 1);
    let (_, again) = call(&app, "POST", "/api/v1/sync/runs", Some(json!({"kind": "incremental", "datasets": ["a_share.calendar"]}))).await;
    let (_, d) = call(&app, "GET", &format!("/api/v1/sync/runs/{}", again["run_id"]), None).await;
    assert_eq!(d["progress"].as_array().unwrap().len(), 0);
    assert_eq!(d["without_tasks"], json!([{"dataset_id": "a_share.calendar", "queued_in": first}]));
    db.drop().await;
}

#[tokio::test]
async fn data_check_finds_gaps_refetches_and_confirms_suspensions() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = db.pool.clone();
    let engine = Engine::new(bare_settings("http://127.0.0.1:1", Some("test")), pool.clone()).await.unwrap();
    // 两只股票，10 个交易日；B 缺 01-07、01-08。还有一只没有上市日、也没有日线的（还没上市），不查
    for (code, list_date) in [("000001.SZ", Some("2026-01-05")), ("000002.SZ", Some("2026-01-05")), ("301999.SZ", None)] {
        sqlx::query(
            "INSERT INTO raw.record (dataset_id, rkey, thscode, payload, payload_hash)
             VALUES ('meta.tickers', $1, $1, jsonb_build_object('thscode', $1, 'asset_type', 'a-share', 'list_date', $2::text), 'x')",
        )
        .bind(code)
        .bind(list_date)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO ref.trading_day SELECT d::date, 'api' FROM generate_series('2026-01-05'::date, '2026-01-16'::date, '1 day') d WHERE extract(isodow FROM d) < 6")
        .execute(&pool)
        .await
        .unwrap();
    let run_id: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id").fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id)
         SELECT c, d.cal_date, 10, 10, 10, 10, 100, 1000, $1 FROM ref.trading_day d, unnest(ARRAY['000001.SZ', '000002.SZ']) c
         WHERE NOT (c = '000002.SZ' AND d.cal_date IN ('2026-01-07', '2026-01-08'))",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();

    let result = |v: &Value| v["last"]["result"].as_array().unwrap().iter().find(|r| r["dataset_id"] == "a_share.bar_1d").cloned().unwrap();
    let app = logged_in(niulai::api::router(Engine::new(bare_settings("http://127.0.0.1:1", Some("test")), pool.clone()).await.unwrap())).await;

    // 第一次：发现 2 个缺口，合并成一个窗口重取，排在补偿那一层
    niulai::sync::check::run(&engine, "manual").await.unwrap().unwrap();
    let (st, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let r = result(&v);
    assert_eq!((r["securities"].as_i64(), r["security_days"].as_i64(), r["new_gaps"].as_i64(), r["open"].as_i64()), (Some(2), Some(20), Some(2), Some(2)), "{r}");
    assert_eq!(r["refetch_tasks"], 1);
    let refetch = r["refetch_run"].as_i64().unwrap();
    let (part, priority): (String, i16) = sqlx::query_as("SELECT part_key, priority FROM ops.task WHERE run_id = $1").bind(refetch).fetch_one(&pool).await.unwrap();
    assert_eq!(part, "thscode=000002.SZ|2026-01-07..2026-01-08");
    assert!((50..100).contains(&priority));
    assert_eq!(v["open"][0]["thscode"], "000002.SZ");
    assert_eq!(v["open"][0]["days"], 2);

    // 重取期间这只股票有未完成任务：先不查，也不重复安排
    niulai::sync::check::run(&engine, "manual").await.unwrap().unwrap();
    let (_, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    let r = result(&v);
    assert_eq!((r["securities"].as_i64(), r["busy"].as_i64(), r["refetch_tasks"].as_i64(), r["open"].as_i64()), (Some(1), Some(1), Some(0), Some(2)), "{r}");

    // 重取成功：01-07 取回来了，01-08 接口仍然没有，确认是停牌
    sqlx::query("INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id) VALUES ('000002.SZ', '2026-01-07', 10, 10, 10, 10, 100, 1000, $1)")
        .bind(refetch)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE ops.task SET status = 'done', finished_at = now() WHERE run_id = $1").bind(refetch).execute(&pool).await.unwrap();
    sqlx::query("UPDATE ops.run SET status = 'succeeded', finished_at = now() WHERE run_id = $1").bind(refetch).execute(&pool).await.unwrap();
    niulai::sync::check::run(&engine, "followup").await.unwrap().unwrap();
    let (_, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    let r = result(&v);
    assert_eq!((r["filled"].as_i64(), r["confirmed_now"].as_i64(), r["suspended"].as_i64(), r["open"].as_i64()), (Some(1), Some(1), Some(1), Some(0)), "{r}");
    assert!(r["refetch_run"].is_null());
    assert_eq!(v["open"].as_array().unwrap().len(), 0);

    // 再查一次：停牌日不再报，也不再重取
    niulai::sync::check::run(&engine, "weekly").await.unwrap().unwrap();
    let (_, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    let r = result(&v);
    assert_eq!((r["new_gaps"].as_i64(), r["suspended"].as_i64(), r["refetch_tasks"].as_i64()), (Some(0), Some(1), Some(0)), "{r}");

    let (st, _) = call(&app, "POST", "/api/v1/sync/check", None).await;
    assert_eq!(st, StatusCode::ACCEPTED);
    db.drop().await;
}

#[tokio::test]
async fn old_tasks_and_call_logs_are_pruned() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = db.pool.clone();
    niulai::sync::registry::sync_to_db(&pool, &registry()).await.unwrap();
    let old = "now() - interval '90 days'";
    let run: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind, status) VALUES ('incremental', 'succeeded') RETURNING run_id").fetch_one(&pool).await.unwrap();
    let open_run: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id").fetch_one(&pool).await.unwrap();
    let task = |run: i64, ds: &str, part: &str, status: &str, finished: &str| {
        let sql = format!("INSERT INTO ops.task (run_id, dataset_id, part_key, params, status, finished_at) VALUES ($1, $2, $3, '{{}}', $4, {finished}) RETURNING task_id");
        let pool = pool.clone();
        let (ds, part, status) = (ds.to_string(), part.to_string(), status.to_string());
        async move { sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql)).bind(run).bind(ds).bind(part).bind(status).fetch_one(&pool).await.unwrap() }
    };
    let resolved_dead = task(run, "special.dragon_tiger", "a", "dead", old).await;
    let old_done = task(run, "special.dragon_tiger", "a", "done", old).await;
    let unresolved_dead = task(run, "special.dragon_tiger", "b", "dead", old).await;
    let newest_done = task(run, "special.dragon_tiger", "c", "done", old).await;
    let in_open_run = task(open_run, "a_share.calendar", "*", "done", old).await;
    let recent = task(run, "a_share.calendar", "*", "done", "now()").await;
    sqlx::query("INSERT INTO ops.api_call (dataset_id, endpoint, params, called_at) VALUES ('a_share.calendar', '/x', '{}', now() - interval '90 days'), ('a_share.calendar', '/x', '{}', now())")
        .execute(&pool)
        .await
        .unwrap();

    let p = niulai::sync::retention::prune(&pool).await.unwrap();
    assert_eq!((p.tasks, p.api_calls), (2, 1), "{p:?}");
    let left: Vec<i64> = sqlx::query_scalar("SELECT task_id FROM ops.task ORDER BY task_id").fetch_all(&pool).await.unwrap();
    assert!(!left.contains(&resolved_dead) && !left.contains(&old_done), "已经解决的失败和过期的成功删掉");
    for (id, why) in [(unresolved_dead, "还没解决的失败"), (newest_done, "数据集最近一个成功的任务"), (in_open_run, "进行中的运行"), (recent, "保留期内的")] {
        assert!(left.contains(&id), "{why}要保留");
    }
    assert_eq!(niulai::sync::retention::prune(&pool).await.unwrap(), niulai::sync::retention::Pruned::default(), "再清理一次没有可删的");
    db.drop().await;
}

#[tokio::test]
async fn audit_finds_gaps_and_plans_repairs() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = &db.pool;
    let engine = Engine::new(bare_settings("http://127.0.0.1:1", Some("test")), pool.clone()).await.unwrap();
    // 两个过去的交易日：d1 同步过（估值错过了，一个任务因供应商不可用重试用尽），d2 完全没有同步；一只在市的股票没有日线
    let d1: chrono::NaiveDate = sqlx::query_scalar("SELECT current_date - 3").fetch_one(pool).await.unwrap();
    let d2 = d1 + chrono::Duration::days(1);
    sqlx::query("INSERT INTO ref.trading_day (cal_date, source) VALUES ($1, 'api'), ($2, 'api')").bind(d1).bind(d2).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO ref.security (thscode, ticker, name, exchange, board, list_date, first_seen, last_seen) VALUES ('600000.SH', '600000', '测试银行', 'SH', 'main', '2020-01-02', current_date, current_date)")
        .execute(pool)
        .await
        .unwrap();
    let run: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind, trade_date, status) VALUES ('incremental', $1, 'partial') RETURNING run_id").bind(d1).fetch_one(pool).await.unwrap();
    for (ds, status, err) in [
        ("a_share.snapshot", "done", ""),
        ("index.snapshot", "done", ""),
        ("a_share.corp_action_dump", "done", ""),
        ("a_share.auction_final", "done", ""),
        ("special.anomaly_reason", "done", ""),
        ("a_share.valuation", "dead", "已错过：供应商最新交易日已是更晚的一天"),
        ("special.hot_stock_daily", "dead", "可重试错误（code=Some(5003)，http=Some(200)）：Data source unavailable"),
        // 同一个分区失败了两次：只能重新排队一个（每个分区同时只能有一个未完成的任务）
        ("special.hot_stock_daily", "dead", "可重试错误（code=Some(5003)，http=Some(200)）：Data source unavailable"),
    ] {
        sqlx::query("INSERT INTO ops.task (run_id, dataset_id, part_key, params, status, last_error, finished_at) VALUES ($1, $2, '*', '{}', $3, nullif($4, ''), now())")
            .bind(run)
            .bind(ds)
            .bind(status)
            .bind(err)
            .execute(pool)
            .await
            .unwrap();
    }

    // 首次部署时日历还没取到、被当成交易日的节假日，以及排队期间滑出一年范围的日期：接口返回 1002、1003
    sqlx::query("INSERT INTO ref.trading_day (cal_date, source) VALUES ($1, 'api')").bind(d1 - chrono::Duration::days(10)).execute(pool).await.unwrap();
    let holiday = (1..10).map(|n| d1 - chrono::Duration::days(n)).find(|d| chrono::Datelike::weekday(d).number_from_monday() <= 5).unwrap();
    let expired = d1 - chrono::Duration::days(400);
    let backfill: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind, trade_date, status) VALUES ('backfill', $1, 'running') RETURNING run_id").bind(d1).fetch_one(pool).await.unwrap();
    for (date, err) in [
        (holiday, "不可重试错误（code=Some(1002)，http=Some(200)）：date must be an A-share trading day"),
        (expired, "不可重试错误（code=Some(1003)，http=Some(200)）：date must be within one year"),
    ] {
        sqlx::query("INSERT INTO ops.task (run_id, dataset_id, part_key, params, status, last_error, finished_at) VALUES ($1, 'special.dragon_tiger', $2, $3, 'dead', $4, now())")
            .bind(backfill)
            .bind(format!("date={date}"))
            .bind(json!({"vars": {"date": date.to_string()}}))
            .bind(err)
            .execute(pool)
            .await
            .unwrap();
    }

    let h = niulai::sync::audit::health(&engine).await.unwrap();
    let dragon: Vec<&str> = h.failures.iter().filter(|f| f.dataset_id == "special.dragon_tiger").map(|f| f.reason.as_str()).collect();
    assert!(dragon.iter().any(|r| r.contains("不是交易日")) && dragon.iter().any(|r| r.contains("最近一年")), "{dragon:?}");
    let kinds: Vec<(chrono::NaiveDate, &str, niulai::sync::audit::GapKind, bool)> =
        h.gaps.iter().map(|g| (g.trade_date, g.dataset_id.as_str(), g.kind, g.recoverable)).collect();
    use niulai::sync::audit::GapKind::*;
    assert!(kinds.contains(&(d1, "a_share.valuation", Missed, false)), "{kinds:?}");
    assert!(kinds.contains(&(d1, "a_share.bar_1d", Incomplete, true)), "{kinds:?}");
    assert!(kinds.contains(&(d2, "a_share.snapshot", NotRun, true)), "快照没同步，可由个股日线补：{kinds:?}");
    assert!(kinds.contains(&(d2, "a_share.valuation", NotRun, false)), "{kinds:?}");
    assert!(!kinds.iter().any(|k| k.0 == d1 && k.1 == "a_share.snapshot"), "d1 的快照成功了");
    assert_eq!(h.latest_trade_date, Some(d2));
    assert_eq!(h.daily_state, niulai::sync::audit::DailyState::Incomplete);
    assert!(h.failures.iter().any(|f| f.dataset_id == "a_share.valuation" && f.reason.contains("无法补回")));

    // 补偿：能补的规划补偿运行，供应商不可用的任务重新排队，错过的估值补不回来
    let s = niulai::sync::audit::repair(&engine).await.unwrap();
    assert!(!s.runs.is_empty());
    assert_eq!(s.retried, 1);
    assert_eq!(s.cleared, 2, "节假日和超出一年的任务本来就没有数据，改为跳过");
    let h = niulai::sync::audit::health(&engine).await.unwrap();
    assert!(!h.failures.iter().any(|f| f.dataset_id == "special.dragon_tiger"), "{:?}", h.failures);
    assert!(s.unrecoverable >= 2);
    let repair_runs: Vec<String> = sqlx::query_scalar("SELECT dataset_id FROM ops.run WHERE kind = 'repair' AND dataset_id IS NOT NULL ORDER BY 1").fetch_all(pool).await.unwrap();
    assert!(repair_runs.contains(&"a_share.bar_1d".to_string()), "{repair_runs:?}");
    assert!(repair_runs.contains(&"a_share.corp_action_dump".to_string()), "{repair_runs:?}");
    let reopened: String = sqlx::query_scalar("SELECT status FROM ops.run WHERE run_id = $1").bind(run).fetch_one(pool).await.unwrap();
    assert_eq!(reopened, "running", "重新排队的任务所在的运行重新打开");
    let pr: i16 = sqlx::query_scalar("SELECT priority FROM ops.task WHERE dataset_id = 'special.hot_stock_daily' AND status = 'pending'").fetch_one(pool).await.unwrap();
    assert!((50..100).contains(&pr), "重新排队的任务在补偿层：{pr}");
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

/// 一只合成股票：120 个交易日，先跌后涨，最后一天（2026-06-19）放量站上 20 日均线
async fn synthetic_stock(pool: &PgPool) {
    let run_id: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id").fetch_one(pool).await.unwrap();
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

#[tokio::test]
async fn strategies_screening_diagnosis_and_backtests() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    synthetic_stock(&db.pool).await;

    let settings = bare_settings("http://127.0.0.1:1", Some("test"));
    let engine = Engine::new(settings, db.pool.clone()).await.unwrap();
    let app = logged_in(niulai::api::router(engine)).await;

    // 首次启动时复制了全部模板
    let (st, list) = call(&app, "GET", "/api/v1/strategies", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), niulai::analysis::strategy::templates().len());
    let (_, cat) = call(&app, "GET", "/api/v1/strategies/catalog", None).await;
    assert_eq!(cat["signals"].as_array().unwrap().len(), niulai::analysis::signals::CATALOG.len());

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
    assert_eq!(created["show_on_today"], true, "默认在今日展示");
    let (st, upd) = call(&app, "PUT", &format!("/api/v1/strategies/{id}"), Some(json!({"name": "放量站上均线", "definition": def, "show_on_today": false}))).await;
    assert_eq!(st, StatusCode::OK, "{upd}");
    assert_eq!(upd["show_on_today"], false);
    assert_eq!(upd["screening"], true, "不在今日展示，照常巡检");
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

#[tokio::test]
async fn signals_indicators_and_strategy_points() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    synthetic_stock(&db.pool).await;
    let settings = bare_settings("http://127.0.0.1:1", Some("test"));
    let engine = Engine::new(settings, db.pool.clone()).await.unwrap();
    let app = logged_in(niulai::api::router(engine)).await;

    // 内置信号：能写成条件的给出定义和白话，特殊信号给出规则
    let (st, list) = call(&app, "GET", "/api/v1/signals", None).await;
    assert_eq!(st, StatusCode::OK, "{list}");
    assert_eq!(list.as_array().unwrap().len(), niulai::analysis::signals::CATALOG.len());
    // 不说明方向的信号（十字星等）是 neutral；连续上涨用「连续 N 天」写成条件
    let (_, doji) = call(&app, "GET", "/api/v1/signals/doji", None).await;
    assert_eq!((doji["tone"].as_str(), doji["special"].as_bool()), (Some("neutral"), Some(true)), "{doji}");
    let (_, streak) = call(&app, "GET", "/api/v1/signals/up_streak", None).await;
    assert!(streak["explained"][0].as_str().unwrap().contains("连续 3 个交易日"), "{streak}");
    let (_, cross) = call(&app, "GET", "/api/v1/signals/ma_cross_up", None).await;
    assert_eq!(cross["special"], false);
    assert!(cross["explained"][0].as_str().unwrap().contains("上穿"), "{cross}");
    assert!(cross["indicators"].as_array().unwrap().len() >= 2);
    let (_, gap) = call(&app, "GET", "/api/v1/signals/gap_up", None).await;
    assert_eq!(gap["special"], true);
    assert!(gap["definition"].is_null());
    let (st, _) = call(&app, "GET", "/api/v1/signals/nope", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // 自定义信号：放量并站上 20 日均线
    let def = json!({"match_all": true, "conditions": [
        {"kind": "signal", "signal": "volume_surge", "params": {"ratio": 3}},
        {"kind": "compare", "left": {"id": "close"}, "op": ">", "right": {"id": "ma", "n": 20}}
    ]});
    let (st, _) = call(&app, "POST", "/api/v1/signals", Some(json!({"name": " ", "definition": def}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "名称不能为空");
    let nested = json!({"conditions": [{"kind": "my_signal", "signal_id": 1}]});
    let (st, _) = call(&app, "POST", "/api/v1/signals", Some(json!({"name": "套娃", "definition": nested}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "自定义信号不能引用自定义信号");
    let (st, sig) = call(&app, "POST", "/api/v1/signals", Some(json!({"name": "放量站上", "definition": def, "based_on": "volume_surge"}))).await;
    assert_eq!(st, StatusCode::CREATED, "{sig}");
    let sid = sig["signal_id"].as_i64().unwrap();
    assert_eq!(sig["key"], format!("my-{sid}"));
    assert_eq!(sig["group"], "我的信号");
    assert_eq!(sig["explained"].as_array().unwrap().len(), 2);

    let mine = json!({"condition": {"kind": "my_signal", "signal_id": sid}});
    let (st, today) = call(&app, "POST", "/api/v1/signals/today", Some(json!({"target": mine}))).await;
    assert_eq!(st, StatusCode::OK, "{today}");
    assert_eq!(today["matched"], 1);
    assert_eq!(today["hits"][0]["thscode"], "600000.SH");
    let (st, days) = call(&app, "POST", "/api/v1/securities/600000.SH/signal-days", Some(json!({"target": mine}))).await;
    assert_eq!(st, StatusCode::OK, "{days}");
    assert_eq!(days["dates"], json!(["2026-06-19"]));
    // 草稿不用保存也能看
    let (st, stats) = call(&app, "POST", "/api/v1/signals/stats", Some(json!({"target": {"draft": def}, "years": 1}))).await;
    assert_eq!(st, StatusCode::OK, "{stats}");
    assert_eq!(stats["occurrences"], 1);
    assert_eq!(stats["horizons"].as_array().unwrap().len(), 3);
    assert_eq!(stats["horizons"][0]["samples"], 0, "最后一天出现，没有后续数据");
    assert_eq!(stats["recent"][0]["trade_date"], "2026-06-19");

    // 策略用了它就不能删
    let sdef = json!({"all": [{"kind": "my_signal", "signal_id": sid}], "exit": {"max_hold_days": 5}});
    let (st, strat) = call(&app, "POST", "/api/v1/strategies", Some(json!({"name": "用我的信号", "definition": sdef}))).await;
    assert_eq!(st, StatusCode::CREATED, "{strat}");
    let id = strat["strategy_id"].as_i64().unwrap();
    let (_, sig) = call(&app, "GET", &format!("/api/v1/signals/my-{sid}"), None).await;
    assert_eq!(sig["used_by"][0]["strategy_id"], id);
    let (st, _) = call(&app, "DELETE", &format!("/api/v1/signals/my/{sid}"), None).await;
    assert_eq!(st, StatusCode::CONFLICT);

    // 巡检：今日买点；股票页上的买卖点
    let (st, run) = call(&app, "POST", &format!("/api/v1/screening/run?strategy_id={id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{run}");
    let (_, res) = call(&app, "GET", &format!("/api/v1/strategies/{id}/results"), None).await;
    assert_eq!(res["hits"].as_array().unwrap().len(), 1, "{res}");
    assert_eq!(res["sells"].as_array().unwrap().len(), 0);
    assert_eq!(res["holdings"].as_array().unwrap().len(), 0);
    let (st, pts) = call(&app, "GET", &format!("/api/v1/securities/600000.SH/strategy-points?strategy_id={id}"), None).await;
    assert_eq!(st, StatusCode::OK, "{pts}");
    assert_eq!(pts["today"], "buy");
    assert_eq!(pts["trades"].as_array().unwrap().len(), 0, "买点在最后一天，还没有成交");

    // 改定义：收盘价低于 20 日均线，今天就不满足了
    let strict = json!({"conditions": [{"kind": "compare", "left": {"id": "close"}, "op": "<", "right": {"id": "ma", "n": 20}}]});
    let (st, _) = call(&app, "PUT", &format!("/api/v1/signals/my/{sid}"), Some(json!({"name": "放量站上", "definition": strict}))).await;
    assert_eq!(st, StatusCode::OK);
    let (_, today) = call(&app, "POST", "/api/v1/signals/today", Some(json!({"target": mine}))).await;
    assert_eq!(today["matched"], 0);

    // 指标：说明、公式、常用写法、画图用的数值
    let (st, inds) = call(&app, "GET", "/api/v1/indicators", None).await;
    assert_eq!(st, StatusCode::OK);
    let rsi = inds.as_array().unwrap().iter().find(|i| i["id"] == "rsi").unwrap();
    assert!(!rsi["formula"].as_str().unwrap().is_empty());
    assert!(!rsi["examples"].as_array().unwrap().is_empty());
    let ma = inds.as_array().unwrap().iter().find(|i| i["id"] == "ma").unwrap();
    assert!(ma["signals"].as_array().unwrap().iter().any(|s| s["key"] == "ma_cross_up"));
    let (st, series) = call(&app, "GET", "/api/v1/securities/600000.SH/indicator?id=ma&n=20", None).await;
    assert_eq!(st, StatusCode::OK, "{series}");
    assert_eq!(series["points"].as_array().unwrap().len(), 101);
    let (st, _) = call(&app, "GET", "/api/v1/securities/600000.SH/indicator?id=ma&n=0", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    call(&app, "DELETE", &format!("/api/v1/strategies/{id}"), None).await;
    let (st, _) = call(&app, "DELETE", &format!("/api/v1/signals/my/{sid}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    db.drop().await;
}

#[tokio::test]
async fn accounts_positions_and_daily_advice() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    synthetic_stock(&db.pool).await;
    let app = niulai::api::router(Engine::new(bare_settings("http://127.0.0.1:1", Some("test")), db.pool.clone()).await.unwrap());
    let admin = login(&app, "admin", "secret-pass").await.unwrap();
    let (_, v) = call_as(&app, Some(&admin), "POST", "/api/v1/users", Some(json!({"username": "mama", "password": "mama-pass-1"}))).await;
    assert_eq!(v["user"]["role"], "member");
    let mama = login(&app, "mama", "mama-pass-1").await.unwrap();
    let st = |r: &axum::http::Response<()>| r.status();

    // 内置策略：每个模板一行，只读、不能删
    let (_, list) = call_as(&app, Some(&admin), "GET", "/api/v1/strategies", None).await;
    let builtin: Vec<&Value> = list.as_array().unwrap().iter().filter(|s| s["builtin"] == true).collect();
    assert_eq!(builtin.len(), niulai::analysis::strategy::templates().len());
    let b = builtin[0];
    let id = b["strategy_id"].as_i64().unwrap();
    let (r, _) = call_as(&app, Some(&admin), "DELETE", &format!("/api/v1/strategies/{id}"), None).await;
    assert_eq!(st(&r), StatusCode::CONFLICT);
    let mut changed = json!({"name": "改个名", "description": b["description"], "definition": b["definition"], "screening": true, "show_on_today": false});
    let (r, _) = call_as(&app, Some(&admin), "PUT", &format!("/api/v1/strategies/{id}"), Some(changed.clone())).await;
    assert_eq!(st(&r), StatusCode::CONFLICT, "内置策略不能改名");
    changed["name"] = b["name"].clone();
    let (r, v) = call_as(&app, Some(&admin), "PUT", &format!("/api/v1/strategies/{id}"), Some(changed)).await;
    assert_eq!(st(&r), StatusCode::OK, "只改是否在今日展示可以：{v}");

    // 规则的辅助：三组参考值和全部参加巡检的策略。存下来的参考值回测只有汇总，没有交易记录
    sqlx::query(
        "INSERT INTO app.preset_stat (preset, rules_hash, end_date, result) VALUES ('balanced', 'h', '2026-06-30', $1)",
    )
    .bind(json!({"summary": {"start": "2023-06-30", "end": "2026-06-30", "total_return": 0.1, "annual_return": 0.03, "max_drawdown": 0.2,
        "benchmark_name": null, "benchmark_return": null, "trades": 10, "win_rate": 0.5, "avg_positions": 3.0, "avg_cash_ratio": 0.2,
        "strategies": 14, "avg_effective": 5.0}, "equity": []}))
    .execute(&db.pool)
    .await
    .unwrap();
    let (r, guide) = call_as(&app, Some(&mama), "GET", "/api/v1/rules/guide", None).await;
    assert_eq!(st(&r), StatusCode::OK, "{guide}");
    assert!(guide["presets"].as_array().unwrap().iter().any(|p| p["backtest"]["summary"]["trades"] == 10));
    assert_eq!(guide["presets"].as_array().unwrap().len(), 3);
    assert!(!guide["presets"][0]["design"].as_array().unwrap().is_empty());
    assert!(guide["strategies"].as_array().unwrap().len() >= builtin.len());

    // 普通用户建账户；还没有巡检时建议在等待
    let (r, v) = call_as(&app, Some(&mama), "POST", "/api/v1/accounts", Some(json!({"name": "华泰主账户", "cash": 100000}))).await;
    assert_eq!(st(&r), StatusCode::CREATED, "{v}");
    let acc = v["summary"]["account"]["account_id"].as_i64().unwrap();
    assert_eq!(v["summary"]["account"]["rules"]["stop_loss_pct"], 8.0, "新账户用稳健型的参考值");
    let base = format!("/api/v1/accounts/{acc}");
    let rules = |stop: f64| json!({"name": "华泰主账户", "fees": {"commission_rate": 0.00025, "min_commission": 5}, "rules": {"stop_loss_pct": stop}});
    let (r, _) = call_as(&app, Some(&mama), "PUT", &base, Some(rules(50.0))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "止损线超出范围");
    let mut none = rules(8.0);
    none["rules"]["strategies"] = json!([]);
    let (r, v) = call_as(&app, Some(&mama), "PUT", &base, Some(none)).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "一个策略都不勾不会被当成全部：{v}");
    let (r, v) = call_as(&app, Some(&mama), "PUT", &base, Some(rules(8.0))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let (r, _) = call_as(&app, Some(&mama), "POST", "/api/v1/accounts", Some(json!({"name": "华泰主账户"}))).await;
    assert_eq!(st(&r), StatusCode::CONFLICT, "同一个人不能重名");
    let (r, _) = call_as(&app, Some(&admin), "GET", &base, None).await;
    assert_eq!(st(&r), StatusCode::NOT_FOUND, "管理员也看不到别人的账户");
    let (_, v) = call_as(&app, Some(&admin), "GET", "/api/v1/accounts", None).await;
    assert!(v.as_array().unwrap().is_empty());

    // 粘贴识别和校准
    let (_, rows) = call_as(&app, Some(&mama), "POST", &format!("{base}/positions/parse"), Some(json!({"text": "证券代码 名称 数量 成本\n600000 测试银行 1,000 30.000\n"}))).await;
    assert_eq!(rows[0]["thscode"], "600000.SH");
    assert_eq!((rows[0]["shares"].as_i64(), rows[0]["cost_price"].as_f64()), (Some(1000), Some(30.0)));
    let (r, v) = call_as(&app, Some(&mama), "PUT", &format!("{base}/positions"), Some(json!({"cash": 100000, "positions": [{"thscode": "600000.SH", "shares": 1000, "cost_price": 30.0}]}))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["positions"][0]["position"]["shares"], 1000);
    assert!(v["positions"][0]["pnl_pct"].as_f64().unwrap() < -20.0, "收盘 23 元，成本 30 元");
    let (r, _) = call_as(&app, Some(&mama), "PUT", &format!("{base}/positions"), Some(json!({"cash": 1, "positions": [{"thscode": "999999.SH", "shares": 100}]}))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "代码不存在");

    // 巡检后生成建议：亏 23%，超过稳健风格的止损线 8%
    niulai::analysis::screener::run_and_store(&db.pool, None, None).await.unwrap();
    niulai::advice::refresh_all(&db.pool, true).await.unwrap();
    let (_, v) = call_as(&app, Some(&mama), "GET", &format!("{base}/advice"), None).await;
    let advice = &v["advice"];
    assert!(v["waiting"].is_null(), "{v}");
    let item = advice["advice"]["items"].as_array().unwrap().iter().find(|i| i["thscode"] == "600000.SH").unwrap().clone();
    assert_eq!(item["action"], "sell", "{item}");
    assert!(item["headline"].as_str().unwrap().contains("超过止损线 8%"), "{item}");
    assert!(!item["rule"].as_str().unwrap().is_empty() && !item["evidence"].as_array().unwrap().is_empty() && !item["plan"].as_array().unwrap().is_empty());
    let advice_id = advice["advice_id"].as_i64().unwrap();

    // 已执行：按实际成交价卖出，持仓清空、现金增加；同一条不能记两次
    let exec = format!("{base}/advice/{advice_id}/items/{}/execute", item["key"].as_str().unwrap());
    let (r, v) = call_as(&app, Some(&mama), "POST", &exec, Some(json!({"price": 22.5}))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["advice"]["states"][item["key"].as_str().unwrap()], "executed");
    let (r, _) = call_as(&app, Some(&mama), "POST", &exec, Some(json!({}))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);
    let (_, d) = call_as(&app, Some(&mama), "GET", &base, None).await;
    assert!(d["positions"].as_array().unwrap().is_empty());
    let cash = d["summary"]["account"]["cash"].as_f64().unwrap();
    assert!((cash - (100_000.0 + 22_500.0 - 5.63 - 11.25)).abs() < 0.02, "卖出 22,500 元，扣佣金 5.63 元和印花税 11.25 元：{cash}");

    // 手动买入；现金不够时拒绝
    let (r, v) = call_as(&app, Some(&mama), "POST", &format!("{base}/trades"), Some(json!({"kind": "buy", "thscode": "600000.SH", "shares": 1000, "price": 23}))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let (r, _) = call_as(&app, Some(&mama), "POST", &format!("{base}/trades"), Some(json!({"kind": "buy", "thscode": "600000.SH", "shares": 100000, "price": 23}))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);

    // 分红送转：除权日在持仓之后，自动记账（每 10 股送 3 股派 2 元），成本摊薄
    sqlx::query("UPDATE app.position SET since = '2026-01-01' WHERE account_id = $1").bind(acc).execute(&db.pool).await.unwrap();
    sqlx::query("INSERT INTO mkt.corp_action (thscode, ex_date, cash_div, bonus_ratio, source, run_id) VALUES ('600000.SH', '2026-03-02', 0.2, 0.3, 'dump', 1)")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(niulai::account::corp::apply(&db.pool, chrono::NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()).await.unwrap(), 1);
    assert_eq!(niulai::account::corp::apply(&db.pool, chrono::NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()).await.unwrap(), 0, "不重复记账");
    let (_, d) = call_as(&app, Some(&mama), "GET", &base, None).await;
    let p = &d["positions"][0]["position"];
    assert_eq!(p["shares"], 1300);
    let cost = p["cost_price"].as_f64().unwrap();
    assert!((cost - (23_005.75 - 200.0) / 1300.0).abs() < 0.001, "{cost}");
    let (_, trades) = call_as(&app, Some(&mama), "GET", &format!("{base}/trades"), None).await;
    let kinds: Vec<&str> = trades.as_array().unwrap().iter().map(|t| t["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"dividend") && kinds.contains(&"bonus") && kinds.contains(&"sell") && kinds.contains(&"adjust") && kinds.contains(&"cash_in"), "{kinds:?}");

    // 历史、回看、不做与撤销
    let (_, h) = call_as(&app, Some(&mama), "GET", &format!("{base}/advice/history"), None).await;
    assert!(!h.as_array().unwrap().is_empty());
    let (r, _) = call_as(&app, Some(&mama), "GET", &format!("{base}/review"), None).await;
    assert_eq!(st(&r), StatusCode::OK);
    let (_, v) = call_as(&app, Some(&mama), "POST", &format!("{base}/advice/refresh"), None).await;
    let a2 = v["advice"]["advice_id"].as_i64().unwrap();
    assert_ne!(a2, advice_id, "重新生成后是新的一份");
    let key = v["advice"]["advice"]["items"][0]["key"].as_str().unwrap().to_string();
    let (_, v) = call_as(&app, Some(&mama), "POST", &format!("{base}/advice/{a2}/items/{key}/skip"), None).await;
    assert_eq!(v["advice"]["states"][&key], "skipped");
    let (_, v) = call_as(&app, Some(&mama), "DELETE", &format!("{base}/advice/{a2}/items/{key}/skip"), None).await;
    assert!(v["advice"]["states"].get(&key).is_none());

    // 旧版本保存的模板复制品（省略了默认值，字面上和模板不同、含义相同）：再次同步时合并进内置策略
    let (key, def): (String, Value) = sqlx::query_as("SELECT builtin_key, definition FROM app.strategy WHERE builtin_key IS NOT NULL ORDER BY strategy_id LIMIT 1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let mut old = def.clone();
    for c in old["all"].as_array_mut().unwrap() {
        c.as_object_mut().unwrap().remove("within_days");
        c.as_object_mut().unwrap().remove("streak_days");
    }
    assert_ne!(old, def);
    let copy: i64 = sqlx::query_scalar("INSERT INTO app.strategy (name, definition, template) VALUES ('旧复制品', $1, $2) RETURNING strategy_id")
        .bind(&old)
        .bind(&key)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE app.position SET strategy_id = $2 WHERE account_id = $1").bind(acc).bind(copy).execute(&db.pool).await.unwrap();
    niulai::analysis::screener::sync_builtin_strategies(&db.pool).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM app.strategy WHERE builtin_key = $1 OR template = $1").bind(&key).fetch_one(&db.pool).await.unwrap();
    assert_eq!(n, 1, "重复的复制品被合并");
    let pointed: Option<String> = sqlx::query_scalar("SELECT s.builtin_key FROM app.position p JOIN app.strategy s USING (strategy_id) WHERE p.account_id = $1")
        .bind(acc)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(pointed.as_deref(), Some(key.as_str()), "持仓的买入依据改指向保留的内置策略");

    // 删除账户
    let (r, _) = call_as(&app, Some(&mama), "DELETE", &base, None).await;
    assert_eq!(st(&r), StatusCode::NO_CONTENT);
    db.drop().await;
}
