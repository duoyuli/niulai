//! 账户、持仓、流水、调仓建议、撤销流水、配股与每日净值。

use crate::common::*;

#[tokio::test]
async fn accounts_positions_and_daily_advice() {
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
    let admin = login(&app, "admin", "secret-pass").await.unwrap();
    let (_, v) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "mama", "password": "mama-pass-1"})),
    )
    .await;
    assert_eq!(v["user"]["role"], "member");
    let mama = login(&app, "mama", "mama-pass-1").await.unwrap();
    let st = |r: &axum::http::Response<()>| r.status();

    // 普通用户建账户：规则是组合 E
    let (r, v) = call_as(
        &app,
        Some(&mama),
        "POST",
        "/api/v1/accounts",
        Some(json!({"name": "华泰主账户", "cash": 100000})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::CREATED, "{v}");
    let acc = v["summary"]["account"]["account_id"].as_i64().unwrap();
    let default_rules = v["summary"]["account"]["rules"].clone();
    assert_eq!(default_rules["top_n"], 30);
    assert_eq!(default_rules["factors"].as_array().unwrap().len(), 5);
    let base = format!("/api/v1/accounts/{acc}");
    let rules = |cash: f64| {
        let mut r = default_rules.clone();
        r["min_cash_pct"] = json!(cash);
        json!({"name": "华泰主账户", "fees": {"commission_rate": 0.00025, "min_commission": 5}, "rules": r})
    };
    let (r, _) = call_as(&app, Some(&mama), "PUT", &base, Some(rules(95.0))).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "保留现金比例超出范围");
    let mut too_many = rules(10.0);
    too_many["rules"]["top_n"] = json!(60);
    let (r, v) = call_as(&app, Some(&mama), "PUT", &base, Some(too_many)).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "持有只数超出范围：{v}");
    let (r, v) = call_as(&app, Some(&mama), "PUT", &base, Some(rules(10.0))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let (r, _) = call_as(
        &app,
        Some(&mama),
        "POST",
        "/api/v1/accounts",
        Some(json!({"name": "华泰主账户"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::CONFLICT, "同一个人不能重名");
    let (r, _) = call_as(&app, Some(&admin), "GET", &base, None).await;
    assert_eq!(st(&r), StatusCode::NOT_FOUND, "管理员也看不到别人的账户");
    let (_, v) = call_as(&app, Some(&admin), "GET", "/api/v1/accounts", None).await;
    assert!(v.as_array().unwrap().is_empty());

    // 粘贴识别和校准
    let (_, rows) = call_as(
        &app,
        Some(&mama),
        "POST",
        &format!("{base}/positions/parse"),
        Some(json!({"text": "证券代码 名称 数量 成本\n600000 测试银行 1,000 30.000\n"})),
    )
    .await;
    assert_eq!(rows[0]["thscode"], "600000.SH");
    assert_eq!(
        (rows[0]["shares"].as_i64(), rows[0]["cost_price"].as_f64()),
        (Some(1000), Some(30.0))
    );
    let (r, v) = call_as(&app, Some(&mama), "PUT", &format!("{base}/positions"), Some(json!({"cash": 100000, "positions": [{"thscode": "600000.SH", "shares": 1000, "cost_price": 30.0}]}))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["positions"][0]["position"]["shares"], 1000);
    assert!(
        v["positions"][0]["pnl_pct"].as_f64().unwrap() < -20.0,
        "收盘 23 元，成本 30 元"
    );
    let (r, _) = call_as(
        &app,
        Some(&mama),
        "PUT",
        &format!("{base}/positions"),
        Some(json!({"cash": 1, "positions": [{"thscode": "999999.SH", "shares": 100}]})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "代码不存在");

    // 同步完成后生成建议：本地只有一只股票，因子排名算不出来时继续拿，不调仓
    niulai::advice::refresh_all(&db.pool, niulai::advice::Refresh::ReplaceUntouched)
        .await
        .unwrap();
    let (_, v) = call_as(&app, Some(&mama), "GET", &format!("{base}/advice"), None).await;
    let advice = &v["advice"];
    assert!(v["waiting"].is_null(), "{v}");
    let advice_id = advice["advice_id"].as_i64().unwrap();
    let item = advice["advice"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["thscode"] == "600000.SH")
        .unwrap()
        .clone();
    assert_eq!(item["action"], "hold", "{item}");
    assert!(
        item["headline"].as_str().unwrap().contains("排名没算出来"),
        "{item}"
    );
    assert!(
        !item["rule"].as_str().unwrap().is_empty()
            && !item["evidence"].as_array().unwrap().is_empty()
    );
    // 手动按实际成交价卖出，持仓清空、现金增加
    let (r, v) = call_as(
        &app,
        Some(&mama),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind":"sell","thscode":"600000.SH","shares":1000,"price":22.5})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let (_, d) = call_as(&app, Some(&mama), "GET", &base, None).await;
    assert!(d["positions"].as_array().unwrap().is_empty());
    let cash = d["summary"]["account"]["cash"].as_f64().unwrap();
    assert!(
        (cash - (100_000.0 + 22_500.0 - 5.63 - 11.25)).abs() < 0.02,
        "卖出 22,500 元，扣佣金 5.63 元和印花税 11.25 元：{cash}"
    );
    let realized = d["summary"]["realized_pnl"].as_f64().unwrap();
    assert!(
        (realized - (22_500.0 - 5.63 - 11.25 - 30_000.0)).abs() < 0.02,
        "按建议执行的卖出也算已实现盈亏：{realized}"
    );

    // 手动买入；现金不够时拒绝
    let (r, v) = call_as(
        &app,
        Some(&mama),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind": "buy", "thscode": "600000.SH", "shares": 1000, "price": 23})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let (r, _) = call_as(
        &app,
        Some(&mama),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind": "buy", "thscode": "600000.SH", "shares": 100000, "price": 23})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST);

    // 分红送转：除权日在持仓之后，自动记账（每 10 股送 3 股派 2 元），成本摊薄
    sqlx::query("UPDATE app.position SET since = '2026-01-01' WHERE account_id = $1")
        .bind(acc)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mkt.corp_action (thscode, ex_date, cash_div, bonus_ratio, source, run_id) VALUES ('600000.SH', '2026-03-02', 0.2, 0.3, 'dump', 1)")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        niulai::account::corp::apply(
            &db.pool,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        niulai::account::corp::apply(
            &db.pool,
            chrono::NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()
        )
        .await
        .unwrap(),
        0,
        "不重复记账"
    );
    let (_, d) = call_as(&app, Some(&mama), "GET", &base, None).await;
    let p = &d["positions"][0]["position"];
    assert_eq!(p["shares"], 1300);
    let cost = p["cost_price"].as_f64().unwrap();
    assert!(
        (cost - (23_005.75 - 200.0) / 1300.0).abs() < 0.001,
        "{cost}"
    );
    let (_, trades) = call_as(&app, Some(&mama), "GET", &format!("{base}/trades"), None).await;
    let kinds: Vec<&str> = trades
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"dividend")
            && kinds.contains(&"bonus")
            && kinds.contains(&"sell")
            && kinds.contains(&"adjust")
            && kinds.contains(&"cash_in"),
        "{kinds:?}"
    );

    // 历史、回看、不做与撤销
    let (_, h) = call_as(
        &app,
        Some(&mama),
        "GET",
        &format!("{base}/advice/history"),
        None,
    )
    .await;
    assert!(!h.as_array().unwrap().is_empty());
    let (r, _) = call_as(&app, Some(&mama), "GET", &format!("{base}/review"), None).await;
    assert_eq!(st(&r), StatusCode::OK);
    let (_, v) = call_as(
        &app,
        Some(&mama),
        "POST",
        &format!("{base}/advice/refresh"),
        None,
    )
    .await;
    let a2 = v["advice"]["advice_id"].as_i64().unwrap();
    assert_ne!(a2, advice_id, "重新生成后是新的一份");
    let key = v["advice"]["advice"]["items"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, v) = call_as(
        &app,
        Some(&mama),
        "POST",
        &format!("{base}/advice/{a2}/items/{key}/skip"),
        None,
    )
    .await;
    assert_eq!(v["advice"]["states"][&key], "skipped");
    let (_, v) = call_as(
        &app,
        Some(&mama),
        "DELETE",
        &format!("{base}/advice/{a2}/items/{key}/skip"),
        None,
    )
    .await;
    assert!(v["advice"]["states"].get(&key).is_none());

    // 删除账户
    let (r, _) = call_as(&app, Some(&mama), "DELETE", &base, None).await;
    assert_eq!(st(&r), StatusCode::NO_CONTENT);
    db.drop().await;
}

/// 取一个账户的流水和详情，撤销测试里反复用
async fn account_trades(app: &axum::Router, cookie: &str, base: &str) -> Vec<Value> {
    let (_, v) = call_as(app, Some(cookie), "GET", &format!("{base}/trades"), None).await;
    v.as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn undo_manual_trades_recomputes_cash_position_cost_and_realized_pnl() {
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
    let admin = login(&app, "admin", "secret-pass")
        .await
        .expect("管理员登录");
    let (_, v) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "undoer", "password": "undo-pass-1"})),
    )
    .await;
    assert_eq!(v["user"]["role"], "member");
    let member = login(&app, "undoer", "undo-pass-1")
        .await
        .expect("普通用户登录");
    let st = |r: &axum::http::Response<()>| r.status();

    // 普通用户也能撤销自己的流水（新接口在 MEMBER_WRITES 里）
    let (r, v) = call_as(
        &app,
        Some(&member),
        "POST",
        "/api/v1/accounts",
        Some(json!({"name": "记账测试", "cash": 100000})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::CREATED, "{v}");
    let acc = v["summary"]["account"]["account_id"].as_i64().unwrap();
    let base = format!("/api/v1/accounts/{acc}");
    let (r, v) = call_as(
        &app,
        Some(&member),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind": "buy", "thscode": "600000.SH", "shares": 1000, "price": 23})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(
        v["summary"]["realized_pnl"], 0.0,
        "还没卖出时已实现盈亏是 0"
    );
    let cost1 = 23_005.75 / 1000.0; // 成交额 23,000 元加佣金 5.75 元

    // 撤销建账户时的转入会让现金变成负数：拒绝
    let cash_in = account_trades(&app, &member, &base)
        .await
        .into_iter()
        .find(|t| t["kind"] == "cash_in")
        .unwrap();
    let (r, v) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{}", cash_in["trade_id"]),
        None,
    )
    .await;
    assert_eq!(
        st(&r),
        StatusCode::BAD_REQUEST,
        "现金不够扣回时不能撤销：{v}"
    );
    assert!(v["message"].as_str().unwrap().contains("不够"), "{v}");

    // 再买 500 股，然后卖 400 股：卖出时的平均成本是两笔买入合起来的
    let (r, v) = call_as(
        &app,
        Some(&member),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind": "buy", "thscode": "600000.SH", "shares": 500, "price": 25})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let (r, v) = call_as(
        &app,
        Some(&member),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind": "sell", "thscode": "600000.SH", "shares": 400, "price": 30})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let sell_id = account_trades(&app, &member, &base)
        .await
        .into_iter()
        .find(|t| t["kind"] == "sell")
        .unwrap()["trade_id"]
        .as_i64()
        .unwrap();
    let cost_both = 35_510.75 / 1500.0;
    let realized_both = 11_989.0 - 400.0 * cost_both; // 卖出 12,000 元，佣金 5 元加印花税 6 元
    let (_, d) = call_as(&app, Some(&member), "GET", &base, None).await;
    assert_eq!(d["positions"][0]["position"]["shares"], 1100);
    let got = d["summary"]["realized_pnl"].as_f64().unwrap();
    assert!(
        (got - realized_both).abs() < 0.05,
        "已实现盈亏按卖出时的平均成本算：{got} vs {realized_both}"
    );
    let sell = account_trades(&app, &member, &base)
        .await
        .into_iter()
        .find(|t| t["trade_id"].as_i64() == Some(sell_id))
        .unwrap();
    assert!(
        (sell["realized_pnl"].as_f64().unwrap() - realized_both).abs() < 0.05,
        "{sell}"
    );

    // 撤销第二笔买入：股数、成本、已实现盈亏全部按剩下的流水重算
    let buy2_id = account_trades(&app, &member, &base)
        .await
        .into_iter()
        .find(|t| t["kind"] == "buy" && t["shares"] == 500)
        .unwrap()["trade_id"]
        .as_i64()
        .unwrap();
    let (r, v) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{buy2_id}"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["positions"][0]["position"]["shares"], 600);
    let cost = v["positions"][0]["position"]["cost_price"]
        .as_f64()
        .unwrap();
    assert!(
        (cost - cost1).abs() < 0.001,
        "撤销后成本回到第一笔买入：{cost} vs {cost1}"
    );
    let cash = v["summary"]["account"]["cash"].as_f64().unwrap();
    assert!(
        (cash - 88_983.25).abs() < 0.02,
        "撤销买入要把付出的钱加回现金：{cash}"
    );
    let realized_after_undo = 11_989.0 - 400.0 * cost1;
    let got = v["summary"]["realized_pnl"].as_f64().unwrap();
    assert!(
        (got - realized_after_undo).abs() < 0.05,
        "后面的卖出按新的平均成本重算：{got} vs {realized_after_undo}"
    );
    assert!(
        account_trades(&app, &member, &base)
            .await
            .iter()
            .any(|t| t["trade_id"].as_i64() == Some(buy2_id) && t["reversed_at"].is_string()),
        "撤销的记录保留下来"
    );

    // 撤销第一笔买入会让后面那笔卖出的持仓不够：拒绝并提示去校准
    let buy1_id = account_trades(&app, &member, &base)
        .await
        .into_iter()
        .find(|t| t["kind"] == "buy" && t["reversed_at"].is_null())
        .unwrap()["trade_id"]
        .as_i64()
        .unwrap();
    let (r, v) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{buy1_id}"),
        None,
    )
    .await;
    assert_eq!(
        st(&r),
        StatusCode::BAD_REQUEST,
        "后面已经卖出过，不能撤销支撑它的买入：{v}"
    );
    assert!(v["message"].as_str().unwrap().contains("校准"), "{v}");

    // 撤销卖出：持仓和成本恢复，已实现盈亏回到 0
    let (r, v) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{sell_id}"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["positions"][0]["position"]["shares"], 1000);
    assert_eq!(v["summary"]["realized_pnl"], 0.0);
    let cash = v["summary"]["account"]["cash"].as_f64().unwrap();
    assert!(
        (cash - 76_994.25).abs() < 0.02,
        "撤销卖出要把回款扣回去：{cash}"
    );

    // 同一笔不能撤销两次；撤销最后一笔买入后持仓清空、现金回到建账户时
    let (r, _) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{sell_id}"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "已经撤销过");
    let (r, v) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{buy1_id}"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert!(
        v["positions"].as_array().unwrap().is_empty(),
        "撤销后持仓清空"
    );
    assert_eq!(v["summary"]["account"]["cash"], 100_000.0);
    let (r, _) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{buy1_id}"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "已经撤销过");
    let (r, _) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/999999"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::NOT_FOUND, "没有这笔流水");

    // 按建议执行、分红送转、校准的流水不能单独撤销
    for (kind, source, want) in [
        ("buy", "advice", "按建议"),
        ("dividend", "corp_action", "分红送转"),
    ] {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO app.account_trade (account_id, kind, trade_date, thscode, shares, amount, source, note)
             VALUES ($1, $2, current_date, '600000.SH', $3, 0, $4, '测试') RETURNING trade_id",
        )
        .bind(acc)
        .bind(kind)
        .bind(if kind == "buy" { 100 } else { 0 })
        .bind(source)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let (r, v) = call_as(
            &app,
            Some(&member),
            "DELETE",
            &format!("{base}/trades/{id}"),
            None,
        )
        .await;
        assert_eq!(
            st(&r),
            StatusCode::BAD_REQUEST,
            "{kind}/{source} 不能撤销：{v}"
        );
        assert!(v["message"].as_str().unwrap().contains(want), "{v}");
        sqlx::query("DELETE FROM app.account_trade WHERE trade_id = $1")
            .bind(id)
            .execute(&db.pool)
            .await
            .unwrap();
    }

    // 校准流水不能撤销；没填成本价的持仓卖出后，已实现盈亏算不出来
    let (r, _) = call_as(
        &app,
        Some(&member),
        "PUT",
        &format!("{base}/positions"),
        Some(json!({"cash": 50000, "positions": [{"thscode": "600000.SH", "shares": 1000}]})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK);
    let adjust = account_trades(&app, &member, &base)
        .await
        .into_iter()
        .find(|t| t["kind"] == "adjust")
        .unwrap();
    let (r, v) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("{base}/trades/{}", adjust["trade_id"]),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "{v}");
    assert!(v["message"].as_str().unwrap().contains("校准"), "{v}");
    let (r, _) = call_as(
        &app,
        Some(&member),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind": "sell", "thscode": "600000.SH", "shares": 500, "price": 24})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK);
    let (_, d) = call_as(&app, Some(&member), "GET", &base, None).await;
    assert!(
        d["summary"]["realized_pnl"].is_null(),
        "有卖出没填成本价时显示算不出，而不是错的数字：{d}"
    );

    // 别人的账户和自己的流水一样：看不到、也撤销不了
    let (_, v) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/accounts",
        Some(json!({"name": "管理员账户", "cash": 1000})),
    )
    .await;
    let admin_acc = v["summary"]["account"]["account_id"].as_i64().unwrap();
    let (_, admin_trades) = call_as(
        &app,
        Some(&admin),
        "GET",
        &format!("/api/v1/accounts/{admin_acc}/trades"),
        None,
    )
    .await;
    let admin_trade = admin_trades[0]["trade_id"].as_i64().unwrap();
    let (r, _) = call_as(
        &app,
        Some(&member),
        "DELETE",
        &format!("/api/v1/accounts/{admin_acc}/trades/{admin_trade}"),
        None,
    )
    .await;
    assert_eq!(
        st(&r),
        StatusCode::NOT_FOUND,
        "管理员看不到普通用户，普通用户也看不到管理员的账户"
    );

    db.drop().await;
}

#[tokio::test]
async fn rights_reminder_and_daily_equity() {
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
    let admin = login(&app, "admin", "secret-pass")
        .await
        .expect("管理员登录");
    let (_, v) = call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "investor", "password": "invest-pass-1"})),
    )
    .await;
    assert_eq!(v["user"]["role"], "member");
    let member = login(&app, "investor", "invest-pass-1")
        .await
        .expect("普通用户登录");
    let st = |r: &axum::http::Response<()>| r.status();

    // 建账户时现金为 0，用一笔转入和一笔买入构成历史（成交日落在日线范围内，便于核对净值）
    let (r, v) = call_as(
        &app,
        Some(&member),
        "POST",
        "/api/v1/accounts",
        Some(json!({"name": "净值测试", "cash": 0})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::CREATED, "{v}");
    let acc = v["summary"]["account"]["account_id"].as_i64().unwrap();
    let base = format!("/api/v1/accounts/{acc}");
    let (r, v) = call_as(
        &app,
        Some(&member),
        "POST",
        &format!("{base}/trades"),
        Some(json!({"kind": "cash_in", "amount": 100000, "trade_date": "2026-02-27"})),
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let (r, v) = call_as(&app, Some(&member), "POST", &format!("{base}/trades"), Some(json!({"kind": "buy", "thscode": "600000.SH", "shares": 1000, "price": 20, "trade_date": "2026-03-02"}))).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(
        v["summary"]["account"]["cash"], 79_995.0,
        "100,000 元减 20,000 元成交额和 5 元佣金"
    );
    assert_eq!(v["positions"][0]["position"]["shares"], 1000);

    let end: chrono::NaiveDate = sqlx::query_scalar("SELECT max(trade_date) FROM mkt.stock_bar_1d")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    // 配股不自动记账：自动记账只处理分红送转
    assert_eq!(
        niulai::account::corp::apply(&db.pool, end).await.unwrap(),
        0
    );

    // 配股提醒：除权日已过、按除权日持有的股数可以认购；4 月 1 日每 10 股配 3 股，配股价 10 元
    sqlx::query("INSERT INTO mkt.corp_action (thscode, ex_date, seq, rights_ratio, rights_price, source, run_id) VALUES ('600000.SH', '2026-04-01', 1, 0.3, 10, 'dump', 1)")
        .execute(&db.pool)
        .await
        .unwrap();
    let (r, v) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/corp-actions"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    let rights = &v[0];
    assert_eq!(rights["thscode"], "600000.SH");
    assert_eq!(rights["name"], "测试银行");
    assert_eq!(rights["rights_per_10"], 3.0);
    assert_eq!(rights["rights_price"], 10.0);
    assert_eq!(rights["held_shares"], 1000);
    assert_eq!(rights["rights_shares"], 300);
    assert_eq!(
        (
            rights["amount"].as_f64().unwrap(),
            rights["fee"].as_f64().unwrap()
        ),
        (3_000.0, 5.0)
    );

    // 认购：扣配股款和费用，持仓按平均成本更新，记一条 rights 流水，提醒消失
    let sub = format!("{base}/corp-actions/rights/600000.SH/2026-04-01/subscribe");
    let (r, v) = call_as(&app, Some(&member), "POST", &sub, None).await;
    assert_eq!(st(&r), StatusCode::OK, "{v}");
    assert_eq!(v["positions"][0]["position"]["shares"], 1300);
    let cost = v["positions"][0]["position"]["cost_price"]
        .as_f64()
        .unwrap();
    assert!(
        (cost - (20.005 * 1000.0 + 3_005.0) / 1300.0).abs() < 0.001,
        "{cost}"
    );
    assert_eq!(
        v["summary"]["account"]["cash"], 76_990.0,
        "扣 3,000 元配股款和 5 元费用"
    );
    let (_, trades) = call_as(&app, Some(&member), "GET", &format!("{base}/trades"), None).await;
    let right_trade = trades
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "rights")
        .unwrap();
    assert_eq!(right_trade["shares"], 300);
    assert_eq!(right_trade["source"], "corp_action");
    assert!(
        (right_trade["amount"].as_f64().unwrap() + 3_005.0).abs() < 0.01,
        "{right_trade}"
    );
    let (_, v) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/corp-actions"),
        None,
    )
    .await;
    assert!(v.as_array().unwrap().is_empty(), "处理过的不再提醒：{v}");
    let (r, v) = call_as(&app, Some(&member), "POST", &sub, None).await;
    assert_eq!(
        st(&r),
        StatusCode::BAD_REQUEST,
        "同一笔配股不能认购两次：{v}"
    );

    // 暂不处理：只记状态，不买、不扣钱，提醒消失
    sqlx::query("INSERT INTO mkt.corp_action (thscode, ex_date, seq, rights_ratio, rights_price, source, run_id) VALUES ('600000.SH', '2026-05-04', 1, 0.2, 8, 'dump', 1)")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, before) = call_as(&app, Some(&member), "GET", &base, None).await;
    let cash_before = before["summary"]["account"]["cash"].as_f64().unwrap();
    let ignore = format!("{base}/corp-actions/rights/600000.SH/2026-05-04/ignore");
    let (r, _) = call_as(&app, Some(&member), "POST", &ignore, None).await;
    assert_eq!(st(&r), StatusCode::NO_CONTENT);
    let (_, d) = call_as(&app, Some(&member), "GET", &base, None).await;
    assert_eq!(d["summary"]["account"]["cash"], cash_before);
    assert_eq!(
        d["positions"][0]["position"]["shares"], 1300,
        "暂不处理不加股数"
    );
    let (_, v) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/corp-actions"),
        None,
    )
    .await;
    assert!(v.as_array().unwrap().is_empty(), "{v}");

    // 配股款超过现金：拒绝并提示先转入现金
    sqlx::query("INSERT INTO mkt.corp_action (thscode, ex_date, seq, rights_ratio, rights_price, source, run_id) VALUES ('600000.SH', '2026-06-01', 1, 3.0, 100, 'dump', 1)")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, v) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/corp-actions"),
        None,
    )
    .await;
    assert_eq!(v[0]["rights_shares"], 3000);
    let expensive = format!("{base}/corp-actions/rights/600000.SH/2026-06-01/subscribe");
    let (r, v) = call_as(&app, Some(&member), "POST", &expensive, None).await;
    assert_eq!(st(&r), StatusCode::BAD_REQUEST, "{v}");
    assert!(v["message"].as_str().unwrap().contains("不够"), "{v}");
    // 除权日在本笔买入之前的配股不算：当时还没有持仓
    sqlx::query("INSERT INTO mkt.corp_action (thscode, ex_date, seq, rights_ratio, rights_price, source, run_id) VALUES ('600000.SH', '2026-02-01', 1, 0.5, 9, 'dump', 1)")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, v) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/corp-actions"),
        None,
    )
    .await;
    assert_eq!(
        v.as_array().unwrap().len(),
        1,
        "买入之前的配股不算，仍只有 6 月 1 日那笔：{v}"
    );
    let (r, _) = call_as(
        &app,
        Some(&member),
        "POST",
        &format!("{base}/corp-actions/rights/600000.SH/2026-06-01/ignore"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::NO_CONTENT);

    // 净值测试用完整的交易日历：停牌那天日线里没有这一天，但净值也要有点
    sqlx::query("INSERT INTO ref.trading_day (cal_date, source) SELECT DISTINCT trade_date, 'derived' FROM mkt.stock_bar_1d ON CONFLICT DO NOTHING")
        .execute(&db.pool)
        .await
        .unwrap();

    // 每日净值：最后一个点是日线最新一天；现金扣掉买入的费用，持仓按当天收盘价
    let (r, curve) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/equity?days=200"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::OK, "{curve}");
    let points = curve["points"].as_array().unwrap();
    assert!(!points.is_empty(), "{curve}");
    let last = points.last().unwrap();
    assert_eq!(last["trade_date"], end.to_string());
    let close: f64 = sqlx::query_scalar("SELECT close::float8 FROM mkt.stock_bar_1d WHERE thscode = '600000.SH' AND trade_date = $1")
        .bind(end)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        last["cash"], 79_995.0,
        "配股认购记在今天，6 月的净值里还没有它"
    );
    assert!(
        (last["market_value"].as_f64().unwrap() - 1000.0 * close).abs() < 0.01,
        "{last}"
    );
    assert!(
        (last["total_assets"].as_f64().unwrap() - (79_995.0 + 1000.0 * close)).abs() < 0.01,
        "{last}"
    );
    let return_pct = curve["return_pct"].as_f64().unwrap();
    assert!(
        (return_pct - (1000.0 * close - 20_005.0) / 100_000.0 * 100.0).abs() < 0.01,
        "区间收益率：{return_pct}"
    );
    assert_eq!(curve["net_transfer"], 0.0, "区间内没有转入转出");
    assert_eq!(
        points[0]["trade_date"], "2026-02-27",
        "从第一笔流水（转入）那天开始"
    );

    // 停牌：没有当天日线时沿用最近收盘价，当天盈亏按没有变化算
    sqlx::query(
        "DELETE FROM mkt.stock_bar_1d WHERE thscode = '600000.SH' AND trade_date = '2026-04-15'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (_, curve) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/equity?days=200"),
        None,
    )
    .await;
    let suspended = curve["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["trade_date"] == "2026-04-15")
        .expect("停牌那天也要有点");
    let prev: f64 = sqlx::query_scalar("SELECT close::float8 FROM mkt.stock_bar_1d WHERE thscode = '600000.SH' AND trade_date < '2026-04-15' ORDER BY trade_date DESC LIMIT 1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert!(
        (suspended["market_value"].as_f64().unwrap() - 1000.0 * prev).abs() < 0.01,
        "{suspended}"
    );
    assert_eq!(suspended["day_pnl"], 0.0, "停牌当天没有涨跌：{suspended}");

    // 快照落库；days 限制返回的天数
    let snapshots: i64 =
        sqlx::query_scalar("SELECT count(*) FROM app.account_equity WHERE account_id = $1")
            .bind(acc)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let total_points = curve["points"].as_array().unwrap().len() as i64;
    assert!(
        snapshots >= total_points,
        "快照按交易日 upsert：{snapshots} vs {total_points}"
    );
    let (_, short) = call_as(
        &app,
        Some(&member),
        "GET",
        &format!("{base}/equity?days=10"),
        None,
    )
    .await;
    assert_eq!(short["points"].as_array().unwrap().len(), 10);
    let (r, _) = call_as(&app, Some(&admin), "GET", &format!("{base}/equity"), None).await;
    assert_eq!(st(&r), StatusCode::NOT_FOUND, "别人看不到这个账户的净值");
    let (r, _) = call_as(
        &app,
        Some(&admin),
        "GET",
        &format!("{base}/corp-actions"),
        None,
    )
    .await;
    assert_eq!(st(&r), StatusCode::NOT_FOUND);

    db.drop().await;
}
