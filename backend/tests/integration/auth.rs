//! 登录、多用户权限、一键升级和首次登录引导。

use crate::common::*;

#[tokio::test]
async fn login_is_required_and_lasts_30_days() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let engine = Engine::new(
        bare_settings("http://127.0.0.1:1", Some("test")),
        db.pool.clone(),
    )
    .await
    .unwrap();
    let app = niulai::api::router(engine.clone());

    // 健康检查不用登录，其余接口要登录
    let (st, _) = call(&app, "GET", "/api/v1/health", None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, body) = call(&app, "GET", "/api/v1/accounts", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
    let (st, _) = call(&app, "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // 密码不对
    let (st, _) = call(
        &app,
        "POST",
        "/api/v1/auth/login",
        Some(json!({"username": "admin", "password": "wrong"})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // 登录成功：Cookie 保存 30 天
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
    assert_eq!(resp.status(), StatusCode::OK);
    let set = resp.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(
        set.contains("Max-Age=2592000") && set.contains("HttpOnly"),
        "{set}"
    );
    let log: (i64, String, String, String, String) = sqlx::query_as(
        "SELECT user_id, username, action, ip_address, device
         FROM app.operation_log
         ORDER BY log_id DESC
         LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(log.0, 1);
    assert_eq!(log.1, "admin");
    assert_eq!(log.2, "login");
    assert_eq!(log.3, "203.0.113.7");
    assert_eq!(log.4, "NiulaiTestDevice/1.0");
    let cookie = set.split(';').next().unwrap().to_string();
    let with = |uri: &str, cookie: &str| {
        Request::builder()
            .uri(uri)
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap()
    };
    let resp = app
        .clone()
        .oneshot(with("/api/v1/auth/me", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = app
        .clone()
        .oneshot(with("/api/v1/accounts", &cookie))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 篡改过的 Cookie 无效
    // 换掉签名的最后一个字符（换成和原来不同的字符，否则偶尔等于原签名）
    let last = if cookie.ends_with('0') { '1' } else { '0' };
    let forged = format!("{}{last}", &cookie[..cookie.len() - 1]);
    let resp = app
        .clone()
        .oneshot(with("/api/v1/accounts", &forged))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 重启后（新的路由，同一个数据库）登录状态仍然有效
    let app2 = niulai::api::router(engine.clone());
    assert_eq!(
        app2.oneshot(with("/api/v1/auth/me", &cookie))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let logs = app
        .clone()
        .oneshot(with("/api/v1/operation-logs?limit=1", &cookie))
        .await
        .unwrap();
    assert_eq!(logs.status(), StatusCode::OK);
    let logs = logs.into_body().collect().await.unwrap().to_bytes();
    let logs: Value = serde_json::from_slice(&logs).unwrap();
    assert_eq!(logs.as_array().unwrap().len(), 1);
    assert_eq!(logs[0]["action"], "login");
    // 配置里的账号密码只用来设置第一个管理员一次：之后改配置不影响登录，密码在网页上改
    let mut changed = bare_settings("http://127.0.0.1:1", Some("test"));
    changed.admin_password = "new-pass".into();
    let app3 = niulai::api::router(Engine::new(changed, db.pool.clone()).await.unwrap());
    assert_eq!(
        app3.clone()
            .oneshot(with("/api/v1/auth/me", &cookie))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(
        login(&app3, "admin", "secret-pass").await.is_some()
            && login(&app3, "admin", "new-pass").await.is_none()
    );
    db.drop().await;
}

#[tokio::test]
async fn users_are_added_by_admins_and_members_only_touch_their_own_data() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    synthetic_stock(&db.pool).await;
    let app = niulai::api::router(
        Engine::new(
            bare_settings("http://127.0.0.1:1", Some("test")),
            db.pool.clone(),
        )
        .await
        .unwrap(),
    );
    let admin = login(&app, "Admin", "secret-pass")
        .await
        .expect("第一个管理员来自配置，用户名不区分大小写");
    let st = |r: &axum::http::Response<()>| r.status();

    // 没有注册接口；添加用户只能由管理员做，不填角色时是普通用户，不填密码时生成一个
    let (r, v) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "mama"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::CREATED, "{v}");
    assert_eq!(v["user"]["role"], "member");
    assert!(v["user"].get("password_hash").is_none(), "不返回密码哈希");
    let generated = v["generated_password"].as_str().unwrap().to_string();
    let mama_id = v["user"]["user_id"].as_i64().unwrap();
    let (r, _) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "MAMA", "password": "12345678"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::CONFLICT);
    let (r, _) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "kid", "password": "short"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);

    // 普通用户：能看，不能进管理功能；第一次登录提醒改密码
    let mama = login(&app, "mama", &generated).await.unwrap();
    let (_, me) = call_as(&app, Some(&mama), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(
        (me["role"].as_str(), me["must_change_password"].as_bool()),
        (Some("member"), Some(true))
    );
    for (method, uri, want) in [
        ("GET", "/api/v1/research/factors", StatusCode::OK),
        ("POST", "/api/v1/sync/runs", StatusCode::FORBIDDEN),
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
    let (r, _) = call_as(
        &app,
        Some(&mama),
        "PUT",
        "/api/v1/watchlist/600000.SH",
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::NO_CONTENT);
    let codes = |v: &Value| {
        v.as_array()
            .unwrap()
            .iter()
            .map(|i| i["thscode"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let (_, v) = call_as(&app, Some(&mama), "GET", "/api/v1/watchlist", None).await;
    assert_eq!(codes(&v), ["600000.SH"]);
    let (_, v) = call_as(&app, Some(&admin), "GET", "/api/v1/watchlist", None).await;
    assert!(codes(&v).is_empty(), "管理员看不到别人的自选股");

    // 本人改密码：要先输对现在的密码；改完后旧的登录状态失效，响应里是新的
    let (r, _) = call_as(
        &app,
        Some(&mama),
        "PUT",
        "/api/v1/auth/password",
        Some(json!({"current_password": "wrong", "new_password": "mama-new-pass"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);
    let (r, v) = call_as(
        &app,
        Some(&mama),
        "PUT",
        "/api/v1/auth/password",
        Some(json!({"current_password": generated, "new_password": "mama-new-pass"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["must_change_password"], false);
    let fresh = r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let (r, _) = call_as(&app, Some(&mama), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::UNAUTHORIZED);
    let (r, _) = call_as(&app, Some(&fresh), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::OK);

    // 管理员重置别人的密码：旧的登录失效，本人登录后又会被提醒修改；不能用这个接口重置自己
    let (r, v) = call_as(
        &app,
        Some(&admin),
        "POST",
        &format!("/api/v1/users/{mama_id}/password"),
        Some(json!({})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let reset = v["generated_password"].as_str().unwrap().to_string();
    let (r, _) = call_as(&app, Some(&fresh), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::UNAUTHORIZED);
    let mama = login(&app, "mama", &reset).await.unwrap();
    let (_, me) = call_as(&app, Some(&admin), "GET", "/api/v1/auth/me", None).await;
    let admin_id = me["user_id"].as_i64().unwrap();
    let (r, _) = call_as(
        &app,
        Some(&admin),
        "POST",
        &format!("/api/v1/users/{admin_id}/password"),
        Some(json!({})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);

    // 设为管理员后可以管理用户；不能修改自己；停用后已登录的立即失效，也不能再登录
    let (r, _) = call_as(
        &app,
        Some(&admin),
        "PUT",
        &format!("/api/v1/users/{mama_id}"),
        Some(json!({"role": "admin", "disabled": false})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK);
    let (r, _) = call_as(&app, Some(&mama), "GET", "/api/v1/users", None).await;
    assert_eq!(st(&r), StatusCode::OK);
    let (r, _) = call_as(
        &app,
        Some(&mama),
        "PUT",
        &format!("/api/v1/users/{mama_id}"),
        Some(json!({"role": "member", "disabled": false})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);
    let (r, _) = call_as(
        &app,
        Some(&admin),
        "PUT",
        &format!("/api/v1/users/{mama_id}"),
        Some(json!({"role": "member", "disabled": true})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK);
    let (r, _) = call_as(&app, Some(&mama), "GET", "/api/v1/auth/me", None).await;
    assert_eq!(st(&r), StatusCode::UNAUTHORIZED);
    assert!(login(&app, "mama", &reset).await.is_none());

    // 至少留一个可以登录的管理员
    let err = niulai::auth::update(&db.pool, admin_id, niulai::auth::Role::Member, false)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("至少要留一个"), "{err}");

    // 连续输错 5 次后暂停登录，正确的密码也要等
    for _ in 0..5 {
        assert!(login(&app, "admin", "wrong-pass").await.is_none());
    }
    let (r, v) = call_as(
        &app,
        None,
        "POST",
        "/api/v1/auth/login",
        Some(json!({"username": "admin", "password": "secret-pass"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::TOO_MANY_REQUESTS, "{v}");
    db.drop().await;
}

#[tokio::test]
async fn one_click_upgrade_hands_request_to_upgrader() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let dir = std::env::temp_dir().join(format!(
        "niulai_upgrade_it_{}_{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut settings = bare_settings("http://127.0.0.1:1", Some("test"));
    settings.upgrade_dir = Some(dir.clone());
    let app = logged_in(niulai::api::router(
        Engine::new(settings, db.pool.clone()).await.unwrap(),
    ))
    .await;

    // 没有心跳：不能一键升级
    let (st, v) = call(&app, "GET", "/api/v1/system/version", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["current"], "dev");
    assert_eq!(v["upgrade"]["available"], false);
    let (st, _) = call(
        &app,
        "POST",
        "/api/v1/system/upgrade",
        Some(json!({"version": "2026.10.18"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    // upgrader 在运行：提交请求，写入版本号
    std::fs::write(
        dir.join("heartbeat"),
        chrono::Utc::now().timestamp().to_string(),
    )
    .unwrap();
    let (st, _) = call(
        &app,
        "POST",
        "/api/v1/system/upgrade",
        Some(json!({"version": "2026.10.18; rm -rf /"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "只接受正式版本号");
    let (st, body) = call(
        &app,
        "POST",
        "/api/v1/system/upgrade",
        Some(json!({"version": "v2026.10.18"})),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["state"], "requested");
    assert_eq!(
        std::fs::read_to_string(dir.join("request")).unwrap(),
        "2026.10.18"
    );
    let (st, _) = call(
        &app,
        "POST",
        "/api/v1/system/upgrade",
        Some(json!({"version": "2026.10.18"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "正在升级时不能再提交");

    // upgrader 接手并报告进度
    std::fs::remove_file(dir.join("request")).unwrap();
    std::fs::write(
        dir.join("status"),
        r#"{"state":"running","target":"2026.10.18","message":"拉取镜像","at":1}"#,
    )
    .unwrap();
    let (_, v) = call(&app, "GET", "/api/v1/system/version", None).await;
    assert_eq!(v["upgrade"]["state"], "running");
    assert_eq!(v["upgrade"]["message"], "拉取镜像");
    std::fs::remove_dir_all(&dir).unwrap();
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
    let engine = Engine::new(bare_settings(&vendor, None), db.pool.clone())
        .await
        .unwrap();
    assert!(!engine.client.has_api_key());
    let app = logged_in(niulai::api::router(engine.clone())).await;
    let (st, body) = call(&app, "GET", "/api/v1/setup", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["api_key_configured"], false);
    assert_eq!(body["api_key_url"], "https://fuyao.aicubes.cn/admin/");
    assert_eq!(body["backfill_planned"], false);

    // 空 Key、被拒绝的 Key：400，不保存
    let (st, _) = call(
        &app,
        "PUT",
        "/api/v1/setup/api-key",
        Some(json!({"api_key": "  "})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, body) = call(
        &app,
        "PUT",
        "/api/v1/setup/api-key",
        Some(json!({"api_key": "bad"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
    assert!(!engine.client.has_api_key());

    // 有效的 Key：保存、立即生效，第一次配置时规划全量回补
    let (st, body) = call(
        &app,
        "PUT",
        "/api/v1/setup/api-key",
        Some(json!({"api_key": " good-key-1234 "})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert!(body["backfill_run_id"].is_i64());
    assert!(engine.client.has_api_key());
    let (_, body) = call(&app, "GET", "/api/v1/setup", None).await;
    assert_eq!(body["api_key_configured"], true);
    assert_eq!(body["api_key_source"], "database");
    assert_eq!(body["api_key_hint"], "••••1234");
    assert_eq!(body["backfill_planned"], true);

    // 更换 Key 不会再次规划回补
    let (st, body) = call(
        &app,
        "PUT",
        "/api/v1/setup/api-key",
        Some(json!({"api_key": "good-key-5678"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(body["backfill_run_id"].is_null());

    // 重启后从数据库读回 Key
    let restarted = Engine::new(bare_settings(&vendor, None), db.pool.clone())
        .await
        .unwrap();
    assert!(restarted.client.has_api_key());

    // 环境变量提供 Key 时，网页上不能修改
    let from_env = Engine::new(
        bare_settings(&vendor, Some("env-key-0000")),
        db.pool.clone(),
    )
    .await
    .unwrap();
    let app = logged_in(niulai::api::router(from_env)).await;
    let (_, body) = call(&app, "GET", "/api/v1/setup", None).await;
    assert_eq!(body["api_key_source"], "env");
    assert_eq!(body["api_key_hint"], "••••0000");
    let (st, _) = call(
        &app,
        "PUT",
        "/api/v1/setup/api-key",
        Some(json!({"api_key": "good-key-9999"})),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    db.drop().await;
}
