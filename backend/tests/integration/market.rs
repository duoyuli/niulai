//! 行情、复权、ETF、个股基本面、权威涨跌停池、市场概况和今日热点。

use crate::common::*;

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
    let web = std::env::temp_dir().join(format!(
        "niulai_web_{}_{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(
        web.join("index.html"),
        "<!doctype html><title>niulai</title>",
    )
    .unwrap();
    std::fs::write(web.join("assets/app-abc123.js"), "console.log(1)").unwrap();
    let settings = Settings {
        database_url: String::new(),
        redis_url: None,
        hithink_finance_api_key: Some("test".into()),
        vendor_base_url: "http://127.0.0.1:1".into(),
        datasets_path: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config/datasets.yaml"),
        ),
        api_bind: String::new(),
        api_timeout_s: 300,
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
    let client = HithinkClient::new(
        "http://127.0.0.1:1",
        Some("test"),
        Duration::from_secs(1),
        limiter,
    )
    .unwrap();
    // 直接组装引擎（不经过应用启动入口），要显式执行启动初始化
    niulai::bootstrap::bootstrap(&db.pool, &settings)
        .await
        .unwrap();
    let engine = Arc::new(Engine {
        settings,
        pool: db.pool.clone(),
        client: Arc::new(client),
        registry: Arc::new(reg),
        events: niulai::sync::events::EventBus::new(),
    });
    let app = logged_in(niulai::api::router(engine.clone())).await;

    // 同一时刻只有一个进程能执行同步任务；持有者释放后其他进程才能取得
    let lock = engine.lock_executor().await.unwrap();
    assert!(
        engine.lock_executor().await.is_err(),
        "第二个执行者应被拒绝"
    );
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
            (
                status,
                serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
            )
        }
    };
    let (st, body) = get("/api/v1/securities/600519.SH/bars?adjust=qfq").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["bars"].as_array().unwrap().len(), 4);
    assert_eq!(body["bars"][2]["close"], "1168.630000");
    // K 线接口（mkt.v_stock_bar_qfq）和因子、建议读的分析价格（mkt.v_analysis_bar）是两份 SQL 定义：
    // 逐根比对，改其中一份复权表达式时这里会失败
    let analysis = niulai::analysis::prices::instrument_bars(&db.pool, "600519.SH", None)
        .await
        .unwrap();
    let api_bars = body["bars"].as_array().unwrap();
    assert_eq!(analysis.len(), api_bars.len());
    for (a, b) in analysis.iter().zip(api_bars) {
        assert_eq!(a.date.to_string(), b["trade_date"].as_str().unwrap());
        for (mine, theirs) in [
            (a.q_open, &b["open"]),
            (a.q_high, &b["high"]),
            (a.q_low, &b["low"]),
            (a.q_close, &b["close"]),
        ] {
            let theirs: f64 = theirs.as_str().unwrap().parse().unwrap();
            assert!(
                (mine - theirs).abs() < 1e-6,
                "{} {mine} != {theirs}",
                a.date
            );
        }
    }
    let (st, body) = get("/api/v1/securities/600519.SH").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["name"], "贵州茅台");
    assert_eq!(body["board"], "main");
    let (st, _) = get("/api/v1/securities/600519.SH/bars?adjust=bad").await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = get("/api/v1/securities/NOPE.SH").await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // 自选股：收藏、重复收藏、涨跌、取消收藏
    let send = |method: &str, uri: &str| {
        let app = app.clone();
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        async move { app.oneshot(req).await.unwrap().status() }
    };
    assert_eq!(
        send("PUT", "/api/v1/watchlist/600519.SH").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send("PUT", "/api/v1/watchlist/600519.SH").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send("PUT", "/api/v1/watchlist/NOPE.SH").await,
        StatusCode::NOT_FOUND
    );
    let (st, body) = get("/api/v1/watchlist").await;
    assert_eq!(st, StatusCode::OK);
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "贵州茅台");
    assert!(items[0]["close"].is_string(), "未复权收盘价是十进制字符串");
    assert!(items[0]["change_1d"].is_number());
    assert!(items[0]["change_20d"].is_null(), "4 根日线算不出 20 日涨跌");
    assert_eq!(
        send("DELETE", "/api/v1/watchlist/600519.SH").await,
        StatusCode::NO_CONTENT
    );
    let (_, body) = get("/api/v1/watchlist").await;
    assert_eq!(body.as_array().unwrap().len(), 0);

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
    assert!(
        String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
            .contains("<title>niulai</title>")
    );
    let resp = page("/assets/app-abc123.js").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    let (st, body) = get("/api/v1/no-such-endpoint").await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
    std::fs::remove_dir_all(&web).ok();

    db.drop().await;
}

#[tokio::test]
async fn etf_domain_sync_and_analysis_path() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let reg = registry();
    niulai::sync::registry::sync_to_db(&db.pool, &reg)
        .await
        .unwrap();
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, trade_date) VALUES ('backfill', '2026-06-29') RETURNING run_id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let tickers = reg.get("meta.tickers").unwrap();
    let vars = json!({"asset_type": "fund-etf"})
        .as_object()
        .unwrap()
        .clone();
    let recs = vec![(
        "item".to_string(),
        json!({"thscode":"510300.SH","ticker":"510300","name":"沪深300ETF","exchange":"SH","asset_type":"fund-etf","list_date":"2012-05-28"}),
    )];
    let rows = build_rows(tickers, &vars, &recs, Some("2026-06-29")).unwrap();
    sinks::apply(
        &db.pool,
        "ref.etf_security",
        &sink_ctx(run_id, "meta.tickers", &vars),
        &rows,
    )
    .await
    .unwrap();

    let etf = reg.get("fund.etf_bar_1d").unwrap();
    let vars = json!({"thscode":"510300.SH"}).as_object().unwrap().clone();
    let recs = vec![
        (
            "item".to_string(),
            json!({"date_ms": 1782230400000_i64, "open_price": 4.10, "high_price": 4.20, "low_price": 4.00, "close_price": 4.15, "volume": 1000000.0, "turnover": 4150000.0}),
        ),
        (
            "item".to_string(),
            json!({"date_ms": 1782316800000_i64, "open_price": 4.15, "high_price": 4.30, "low_price": 4.10, "close_price": 4.25, "volume": 1200000.0, "turnover": 5100000.0}),
        ),
        (
            "item".to_string(),
            json!({"date_ms": 1782403200000_i64, "open_price": 4.25, "high_price": 4.40, "low_price": 4.20, "close_price": 4.35, "volume": 1300000.0, "turnover": 5655000.0}),
        ),
    ];
    let rows = build_rows(etf, &vars, &recs, None).unwrap();
    assert_eq!(
        sinks::apply(
            &db.pool,
            "mkt.etf_bar_1d",
            &sink_ctx(run_id, "fund.etf_bar_1d", &vars),
            &rows,
        )
        .await
        .unwrap(),
        3
    );
    let instrument: (String, String) =
        sqlx::query_as("SELECT asset_type, name FROM ref.instrument WHERE thscode = '510300.SH'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(instrument, ("etf".into(), "沪深300ETF".into()));
    let bars = niulai::analysis::prices::instrument_bars(&db.pool, "510300.SH", None)
        .await
        .unwrap();
    assert_eq!(bars.len(), 3);
    assert_eq!(bars[2].q_close, 4.35);

    // ETF 快照与历史日线共用同一张事实表；快照应只补当天市场字段，不覆盖已有历史价格。
    let snapshot = vec![(
        "item".to_string(),
        json!({"open_price": 4.36, "high_price": 4.40, "low_price": 4.30, "last_price": 4.38,
            "prev_price": 4.35, "volume": 1400000.0, "turnover": 6132000.0}),
    )];
    let rows = build_rows(
        reg.get("fund.market_snapshot").unwrap(),
        &vars,
        &snapshot,
        Some("2026-06-29"),
    )
    .unwrap();
    assert_eq!(
        sinks::apply(
            &db.pool,
            "mkt.etf_bar_1d",
            &sink_ctx(run_id, "fund.market_snapshot", &vars),
            &rows,
        )
        .await
        .unwrap(),
        1
    );

    let sec = niulai::analysis::securities::load(&db.pool).await.unwrap();
    assert_eq!(
        sec.meta["510300.SH"].asset_type,
        niulai::instrument::AssetType::Etf
    );

    let engine = Engine::new(
        bare_settings("http://127.0.0.1:1", Some("test")),
        db.pool.clone(),
    )
    .await
    .unwrap();
    let app = logged_in(niulai::api::router(engine)).await;
    let (status, body) = call(
        &app,
        "GET",
        "/api/v1/securities/510300.SH/bars?adjust=qfq",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["bars"].as_array().unwrap().len(), 4);
    assert_eq!(body["adjust"], "qfq");
    let (status, fundamentals) = call(
        &app,
        "GET",
        "/api/v1/securities/510300.SH/fundamentals",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fundamentals}");
    assert_eq!(fundamentals["asset_type"], "etf");
    assert!(fundamentals["etf"]["latest_close"].is_string());
    let (status, created) = call(
        &app,
        "POST",
        "/api/v1/accounts",
        Some(json!({"name": "ETF 测试账户", "cash": 10000})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let account_id = created["summary"]["account"]["account_id"]
        .as_i64()
        .unwrap();
    let (status, trade) = call(
        &app,
        "POST",
        &format!("/api/v1/accounts/{account_id}/trades"),
        Some(json!({"kind":"buy","thscode":"510300.SH","shares":1000,"price":4.35})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{trade}");
    let (status, detail) = call(&app, "GET", &format!("/api/v1/accounts/{account_id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["positions"][0]["asset_type"], "etf");
    db.drop().await;
}

#[tokio::test]
async fn security_fundamentals_exposes_valuation_and_report_dates() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    synthetic_stock(&db.pool).await;
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, trade_date) VALUES ('backfill', '2026-06-29') RETURNING run_id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO fin.valuation_1d (thscode, trade_date, pe_ttm, pb_mrq, float_market_cap, run_id)
         VALUES ('600000.SH', '2026-06-29', 12.3, 2.1, 123456789.0, $1)",
    )
    .bind(run_id)
    .execute(&db.pool)
    .await
    .unwrap();
    for (dataset, key, payload) in [
        (
            "a_share.income_q",
            "600000.SH|2026-03-31",
            json!({"period_end_ms": 1774915200000_i64, "report_date_ms": 1782864000000_i64, "operating_income": 100.0, "parent_holder_net_profit": 20.0, "basic_eps": 0.5}),
        ),
        (
            "a_share.balance_q",
            "600000.SH|2026-03-31",
            json!({"period_end_ms": 1774915200000_i64, "report_date_ms": 1782864000000_i64, "assets_total": 500.0, "total_debt": 200.0, "holder_equity_total": 300.0}),
        ),
        (
            "a_share.cashflow_q",
            "600000.SH|2026-03-31",
            json!({"period_end_ms": 1774915200000_i64, "report_date_ms": 1782864000000_i64, "act_cash_flow_net": 30.0, "invest_cash_flow_net": -10.0}),
        ),
        (
            "a_share.fin_indicator",
            "600000.SH|2026-1",
            json!({"report": "2026-1", "abilities": [{"ability": "profitability", "indicators": [{"index_id": "sale_gross_margin", "value": "18.5"}]}]}),
        ),
    ] {
        sqlx::query(
            "INSERT INTO raw.record (dataset_id, rkey, obs_date, thscode, payload, payload_hash)
             VALUES ($1, $2, '2026-03-31', '600000.SH', $3, 'test')",
        )
        .bind(dataset)
        .bind(key)
        .bind(payload)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let app = niulai::api::router(
        Engine::new(
            bare_settings("http://127.0.0.1:1", Some("test")),
            db.pool.clone(),
        )
        .await
        .unwrap(),
    );
    let admin = login(&app, "admin", "secret-pass").await.unwrap();
    let (r, body) = call_as(
        &app,
        Some(&admin),
        "GET",
        "/api/v1/securities/600000.SH/fundamentals?limit=1",
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK, "{body}");
    assert_eq!(body["valuation"][0]["pe_ttm"], "12.300000");
    assert_eq!(body["income"]["period_end"], "2026-03-31");
    assert_eq!(body["income"]["report_date"], "2026-07-01");
    assert_eq!(
        body["indicators"][0]["metrics"][0]["name"],
        "sale_gross_margin"
    );
    db.drop().await;
}

/// 写一条涨跌停池的 raw.record；payload 是供应商原样的 JSON
async fn put_hot_record(pool: &PgPool, dataset: &str, date: &str, key: &str, payload: Value) {
    sqlx::query(
        "INSERT INTO raw.record (dataset_id, rkey, obs_date, thscode, payload, payload_hash)
         VALUES ($1, $2, $3::date, $4, $5, 'test')",
    )
    .bind(dataset)
    .bind(format!("{date}|{key}"))
    .bind(date)
    .bind(
        payload
            .get("thscode")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty()),
    )
    .bind(&payload)
    .execute(pool)
    .await
    .unwrap();
}

/// 写 2026-10-12 到 2026-10-16 的 5 根日线；highs 与 closes 一一对应
async fn five_bars(pool: &PgPool, run_id: i64, code: &str, closes: &[f64], highs: &[f64]) {
    let dates = [
        "2026-10-12",
        "2026-10-13",
        "2026-10-14",
        "2026-10-15",
        "2026-10-16",
    ];
    for (i, date) in dates.iter().enumerate() {
        let close = closes[i];
        let high = highs[i];
        let low = close.min(high) * 0.99;
        sqlx::query(
            "INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id)
             VALUES ($1, $2::date, $3::float8::numeric, $4::float8::numeric, $5::float8::numeric, $6::float8::numeric, 1000000, 1000000, $7)",
        )
        .bind(code)
        .bind(date)
        .bind(close)
        .bind(high)
        .bind(low)
        .bind(close)
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// 权威涨跌停池是市场概况涨停跌停数的依据：池覆盖的日期以池为准，没进池的股票不算涨跌停
/// （这里覆盖了 2026-10-16 这一天）。另外验证 limit_pool 版本更新后市场概况的缓存不再命中
#[tokio::test]
async fn authoritative_limit_pool_drives_market_overview() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let run_id: i64 =
        sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    for (code, name, board) in [
        ("600001.SH", "实涨停", "main"),
        ("600002.SH", "ST旧规则", "main"),
        ("600003.SH", "炸板旧算法", "main"),
        ("600004.SH", "ST跌停旧规则", "main"),
    ] {
        sqlx::query(
            "INSERT INTO ref.security (thscode, ticker, name, exchange, board, list_date, first_seen, last_seen)
             VALUES ($1, $2, $3, 'SH', $4, '2020-01-02', current_date, current_date)",
        )
        .bind(code)
        .bind(&code[..6])
        .bind(name)
        .bind(board)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    // 最后一天：600001 涨 10%（在涨停池里，5 连板）；600002 是主板 ST 涨 7%，自算按 5% 会误判成涨停；
    // 600003 盘中冲到涨停价、收盘回落（自算会判成炸板）；600004 是 ST 跌 7%（自算按 5% 会误判成跌停）
    five_bars(
        &db.pool,
        run_id,
        "600001.SH",
        &[10.0, 10.1, 10.2, 10.3, 11.33],
        &[10.0, 10.1, 10.2, 10.3, 11.33],
    )
    .await;
    five_bars(
        &db.pool,
        run_id,
        "600002.SH",
        &[10.0, 10.1, 10.2, 10.3, 11.02],
        &[10.0, 10.1, 10.2, 10.3, 11.02],
    )
    .await;
    five_bars(
        &db.pool,
        run_id,
        "600003.SH",
        &[10.0, 10.1, 10.2, 10.3, 10.6],
        &[10.0, 10.1, 10.2, 10.3, 11.33],
    )
    .await;
    five_bars(
        &db.pool,
        run_id,
        "600004.SH",
        &[10.0, 9.9, 9.8, 9.7, 9.02],
        &[10.0, 9.9, 9.8, 9.7, 9.02],
    )
    .await;
    // 2026-10-16 的权威名单：涨停只有 600001（5 连板）；另一只不存在的股票 600099.SH 进了跌停、炸板池，
    // 证明这两个池覆盖了这一天，而 600003、600004 不在名单里
    put_hot_record(
        &db.pool,
        "special.limit_up_pool",
        "2026-10-16",
        "600001.SH",
        json!({"thscode": "600001.SH", "continue_day_cnt": 5, "is_st": false}),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_up_pool",
        "2026-10-16",
        "600099.SH",
        json!({"thscode": "600099.SH", "continue_day_cnt": 1, "is_st": false}),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_down_pool",
        "2026-10-16",
        "600099.SH",
        json!({"thscode": "600099.SH"}),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_break_pool",
        "2026-10-16",
        "600099.SH",
        json!({"thscode": "600099.SH"}),
    )
    .await;
    // 迁移建好了 limit_pool 的版本行
    let has_version: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ops.derived_version WHERE name = 'limit_pool')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(has_version, "迁移应插入 limit_pool 版本行");

    let settings = bare_settings("http://127.0.0.1:1", Some("test"));
    let engine = Engine::new(settings, db.pool.clone()).await.unwrap();
    let app = logged_in(niulai::api::router(engine)).await;

    // 市场概况：涨停 1 只（自算会是 2 只），跌停 0 只（自算会是 1 只）
    let (st, ov) = call(&app, "GET", "/api/v1/market/overview?date=2026-10-16", None).await;
    assert_eq!(st, StatusCode::OK, "{ov}");
    assert_eq!(ov["stocks"], 4);
    assert_eq!(ov["limit_up"], 1, "以涨停池为准：600002 涨 7% 不是涨停");
    assert_eq!(ov["limit_down"], 0, "以跌停池为准：600004 跌 7% 不是跌停");

    // 缓存：第一次市场概况已缓存；补上 600002 的涨停记录并更新 limit_pool 版本后，不再命中旧结果
    assert_eq!(ov["limit_up"], 1);
    put_hot_record(
        &db.pool,
        "special.limit_up_pool",
        "2026-10-16",
        "600002.SH",
        json!({"thscode": "600002.SH", "continue_day_cnt": 1, "is_st": true}),
    )
    .await;
    sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = 'limit_pool'")
        .execute(&db.pool)
        .await
        .unwrap();
    let (st, ov2) = call(&app, "GET", "/api/v1/market/overview?date=2026-10-16", None).await;
    assert_eq!(st, StatusCode::OK, "{ov2}");
    assert_eq!(ov2["limit_up"], 2, "版本更新后重新计算，600002 现在在池里");

    db.drop().await;
}

#[tokio::test]
async fn market_hot_groups_truncates_and_explains_missing_data() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let app = niulai::api::router(
        Engine::new(
            bare_settings("http://127.0.0.1:1", Some("test")),
            db.pool.clone(),
        )
        .await
        .unwrap(),
    );
    let admin = login(&app, "admin", "secret-pass")
        .await
        .expect("管理员登录");
    let get = |date: &str| format!("/api/v1/market/hot?date={date}");

    // 还没有任何日线和涨跌停池：空态用白话说明，交易日为空
    let (r, v) = call_as(&app, Some(&admin), "GET", "/api/v1/market/hot", None).await;
    assert_eq!(r.status(), StatusCode::OK, "{v}");
    assert_eq!(v["available"], false);
    assert!(v["trade_date"].is_null(), "{v}");
    assert!(
        v["note"].as_str().unwrap().contains("还没有行情数据"),
        "{v}"
    );
    assert_eq!(v["limit_up_total"], 0);
    assert_eq!(v["limit_up_groups"].as_array().unwrap().len(), 0);

    // 2026-10-15：首板 2 只（其中一只的 thscode 为空，按 ticker 补）、3 连板 1 只；跌停 2 只、炸板 1 只；另有统计
    let up = |name: &str, ticker: &str, thscode: &str, seal: f64, cnt: i64| {
        json!({
            "name": name, "ticker": ticker, "thscode": thscode, "is_st": false,
            "last_price": 61.44, "seal_money": seal, "limit_up_time": "10:55",
            "limit_up_reason": "半导体存储+重组审核中止", "continue_day_cnt": cnt,
            "continue_day_text": if cnt == 1 { "首板" } else { "3连板" }, "price_change_ratio_pct": 10.009
        })
    };
    put_hot_record(
        &db.pool,
        "special.limit_up_pool",
        "2026-10-15",
        "600002.SH",
        up("大封单", "600002", "600002.SH", 9_000_000.0, 1),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_up_pool",
        "2026-10-15",
        "605178.SH",
        up("时空科技", "605178", "", 1_000_000.0, 1),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_up_pool",
        "2026-10-15",
        "600001.SH",
        up("三连板", "600001", "600001.SH", 2_000_000.0, 3),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_down_pool",
        "2026-10-15",
        "002134.SZ",
        json!({"name": "天津普林", "ticker": "002134", "thscode": "002134.SZ", "last_price": 21.91,
               "last_limit_time": "14:03", "first_limit_time": "13:42", "price_change_ratio_pct": -9.98}),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_down_pool",
        "2026-10-15",
        "002552.SZ",
        json!({"name": "宝鼎科技", "ticker": "002552", "thscode": "002552.SZ", "last_price": 42.06,
               "last_limit_time": "14:18", "first_limit_time": "14:15", "price_change_ratio_pct": -9.99}),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_break_pool",
        "2026-10-15",
        "000011.SZ",
        json!({"name": "深物业A", "ticker": "000011", "thscode": "000011.SZ", "turnover": 1050535140.0,
               "last_price": 11.4, "open_times": 2, "turnover_ratio_pct": 17.3043, "price_change_ratio_pct": 5.3604}),
    )
    .await;
    put_hot_record(
        &db.pool,
        "special.limit_up_stats",
        "2026-10-15",
        "trade_date",
        json!({"seal_rate": 0.5, "trade_date": "2026-10-15", "limit_up_stock_count": 3,
               "touched_limit_up_stock_count": 6, "open_stock_count": 3}),
    )
    .await;

    // 没传日期：没有日线时退回涨跌停池里最新的那一天
    let (r, v) = call_as(&app, Some(&admin), "GET", "/api/v1/market/hot", None).await;
    assert_eq!(r.status(), StatusCode::OK, "{v}");
    assert_eq!(v["trade_date"], "2026-10-15");
    assert_eq!(v["available"], true);
    assert!(v["note"].is_null(), "{v}");
    assert_eq!(
        (v["limit_up_total"].as_i64(), v["max_board"].as_i64()),
        (Some(3), Some(3))
    );
    let groups = v["limit_up_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "{v}");
    assert_eq!(
        (
            groups[0]["board_num"].as_i64(),
            groups[0]["label"].as_str(),
            groups[0]["count"].as_i64()
        ),
        (Some(3), Some("3 板"), Some(1))
    );
    assert_eq!(
        (groups[1]["label"].as_str(), groups[1]["count"].as_i64()),
        (Some("首板"), Some(2))
    );
    // 组内按封单额从大到小；thscode 为空的那只按 ticker 补交易所后缀
    assert_eq!(groups[1]["stocks"][0]["name"], "大封单");
    assert_eq!(groups[1]["stocks"][1]["thscode"], "605178.SH", "{v}");
    assert_eq!(groups[1]["stocks"][1]["continue_day_text"], "首板");
    assert_eq!(groups[1]["stocks"][1]["seal_money"], "1000000");
    assert_eq!(v["limit_down_total"], 2);
    assert_eq!(
        v["limit_down_stocks"][0]["name"], "宝鼎科技",
        "跌得多的在前"
    );
    assert_eq!(v["broken_total"], 1);
    assert_eq!(v["broken_stocks"][0]["open_times"], 2);
    assert_eq!(v["broken_stocks"][0]["turnover"], "1050535140");
    assert_eq!(v["stats"]["seal_rate"], 0.5);
    assert_eq!(v["stats"]["touched_limit_up_count"], 6);

    // 日期是截止日期：这天还没有数据时给出空态和白话说明
    let (r, v) = call_as(&app, Some(&admin), "GET", &get("2026-10-14"), None).await;
    assert_eq!(r.status(), StatusCode::OK, "{v}");
    assert_eq!(v["trade_date"], "2026-10-14");
    assert_eq!(v["available"], false);
    assert!(
        v["note"].as_str().unwrap().contains("2026-10-14"),
        "{}",
        v["note"]
    );

    // 2026-10-14：55 只首板、55 只炸板；每组只列 50 只，总数另给，没有统计时 stats 为空
    for i in 0..55 {
        let code = format!("60{:04}.SH", i);
        put_hot_record(
            &db.pool,
            "special.limit_up_pool",
            "2026-10-14",
            &code,
            up(
                &format!("涨停{i}"),
                &format!("60{i:04}"),
                &code,
                f64::from(i),
                1,
            ),
        )
        .await;
        put_hot_record(
            &db.pool,
            "special.limit_break_pool",
            "2026-10-14",
            &code,
            json!({"name": format!("炸板{i}"), "ticker": format!("60{i:04}"), "thscode": code, "open_times": i % 5}),
        )
        .await;
    }
    let (r, v) = call_as(&app, Some(&admin), "GET", &get("2026-10-14"), None).await;
    assert_eq!(r.status(), StatusCode::OK, "{v}");
    assert_eq!(v["available"], true);
    assert_eq!(v["limit_up_total"], 55);
    let first = &v["limit_up_groups"][0];
    assert_eq!(
        (
            first["count"].as_i64(),
            first["stocks"].as_array().unwrap().len() as i64
        ),
        (Some(55), 50)
    );
    assert_eq!(
        (
            v["broken_total"].as_i64(),
            v["broken_stocks"].as_array().unwrap().len() as i64
        ),
        (Some(55), 50)
    );
    assert_eq!(v["limit_down_total"], 0);
    assert!(v["stats"].is_null());
    assert_eq!(v["broken_stocks"][0]["open_times"], 4, "炸板次数多的在前");

    // 日期格式不对时返回 400
    let (r, _) = call_as(
        &app,
        Some(&admin),
        "GET",
        "/api/v1/market/hot?date=2026-13-01",
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    // 比涨停池开始采集更早的日期：说明那时还没有数据，不是「没同步」
    let (r, v) = call_as(&app, Some(&admin), "GET", &get("2019-01-04"), None).await;
    assert_eq!(r.status(), StatusCode::OK, "{v}");
    assert_eq!(v["available"], false);
    assert!(
        v["note"].as_str().unwrap().contains("2020 年 7 月"),
        "{}",
        v["note"]
    );

    // 普通用户也能看
    let (_, u) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "hotfan"})),
    )
    .await;
    let generated = u["generated_password"]
        .as_str()
        .expect("生成初始密码")
        .to_string();
    let member = login(&app, "hotfan", &generated)
        .await
        .expect("普通用户登录");
    let (r, _) = call_as(&app, Some(&member), "GET", &get("2026-10-15"), None).await;
    assert_eq!(r.status(), StatusCode::OK, "今日热点对普通用户开放");

    db.drop().await;
}
