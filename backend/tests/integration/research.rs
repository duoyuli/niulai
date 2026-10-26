//! 因子研究、研究运行记录，以及账户的因子排名、建议重算和建议处理。

use crate::common::*;

/// 因子研究：合成的全市场日线、行业和三张财报上走一遍目录、因子检验、多因子组合和试验次数统计。
/// 第 k 只股票每天漂移 (k − 40) × 0.0002，所以「短期反转」在这里方向相反，「成交额」越大的股票涨得越多
#[tokio::test]
async fn factor_research_end_to_end() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let reg = registry();
    niulai::sync::registry::sync_to_db(&db.pool, &reg)
        .await
        .unwrap();
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, trade_date) VALUES ('backfill', '2025-08-29') RETURNING run_id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    for sql in [
        "INSERT INTO ref.trading_day (cal_date, source)
         SELECT d::date, 'api' FROM generate_series('2024-01-01'::date, '2025-08-29', '1 day') d
         WHERE extract(isodow FROM d) < 6",
        "INSERT INTO ref.security (thscode, ticker, name, exchange, board, list_date, first_seen, last_seen)
         SELECT format('%s.SZ', lpad(k::text, 6, '0')), lpad(k::text, 6, '0'), format('测试%s', k), 'SZ', 'main',
                '2010-01-04', '2024-01-01', '2025-08-29'
         FROM generate_series(1, 80) k",
        "INSERT INTO ref.instrument (thscode, ticker, name, asset_type, exchange, board, list_date, first_seen, last_seen)
         SELECT thscode, ticker, name, 'stock', exchange, board, list_date, first_seen, last_seen FROM ref.security",
        "INSERT INTO ref.index_info (thscode, name, category, first_seen, last_seen) VALUES
           ('881001.TI', '行业甲', 'industry', '2024-01-01', '2025-08-29'),
           ('881002.TI', '行业乙', 'industry', '2024-01-01', '2025-08-29')",
        "INSERT INTO ref.index_member (index_code, thscode, valid_from)
         SELECT CASE WHEN k % 2 = 0 THEN '881001.TI' ELSE '881002.TI' END, format('%s.SZ', lpad(k::text, 6, '0')), '2024-01-01'
         FROM generate_series(1, 80) k",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    sqlx::query(
        "WITH d AS (SELECT cal_date AS d, row_number() OVER (ORDER BY cal_date) AS i FROM ref.trading_day),
              s AS (SELECT k, format('%s.SZ', lpad(k::text, 6, '0')) AS code FROM generate_series(1, 80) k)
         INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id)
         SELECT code, d, round(p * 0.998, 4), round(p * 1.01, 4), round(p * 0.99, 4), p, 1000000, 6e7 + k * 1e6, $1
         FROM d, s, LATERAL (SELECT round((10 * exp(i * (k - 40) * 0.0002 + 0.02 * sin(i * 0.7 + k)))::numeric, 4) AS p) x",
    )
    .bind(run_id)
    .execute(&db.pool)
    .await
    .unwrap();
    // 三张财报：2023 年到 2025 年一季度的累计值，报告日期按截止日前一周
    for (dataset, fields) in [
        (
            "a_share.income_q",
            "'parent_holder_net_profit', k * 1e6 * q",
        ),
        (
            "a_share.balance_q",
            "'holder_equity_total', 1e9, 'assets_total', 2e9",
        ),
        ("a_share.cashflow_q", "'act_cash_flow_net', k * 5e5 * q"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO raw.record (dataset_id, rkey, thscode, payload, payload_hash)
             SELECT '{dataset}', format('%s|%s|Q%s', code, fy, q), code,
                    jsonb_build_object('thscode', code, 'fiscal_year', fy, 'fiscal_period', 'Q' || q,
                      'report_date_ms', extract(epoch FROM (make_date(fy + (q = 4)::int, (ARRAY[4, 8, 10, 4])[q], 20))) * 1000,
                      {fields}),
                    md5(code || fy || q)
             FROM (SELECT k, format('%s.SZ', lpad(k::text, 6, '0')) AS code FROM generate_series(1, 80) k) s,
                  generate_series(2023, 2025) fy, generate_series(1, 4) q
             WHERE fy < 2025 OR q = 1"
        )))
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let app = logged_in(niulai::api::router(
        Engine::new(
            bare_settings("http://127.0.0.1:1", Some("test")),
            db.pool.clone(),
        )
        .await
        .unwrap(),
    ))
    .await;

    let (s, v) = call(&app, "GET", "/api/v1/research/factors", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 12);

    let (s, v) = call(
        &app,
        "GET",
        "/api/v1/research/factor-test?factors=reversal,liquidity,idio_vol,roe&start=2025-01-02&end=2025-08-29",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let factors = v["factors"].as_array().unwrap();
    assert_eq!(factors.len(), 5, "四个因子加等权合成");
    let by_name = |n: &str| factors.iter().find(|f| f["name"] == n).unwrap();
    assert!(by_name("短期反转")["periods"].as_u64().unwrap() > 20);
    // 成交额越大漂移越大：与「越小越好」的预期相反，RankIC 应显著为负
    let liquidity = by_name("成交额（规模代理）");
    assert!(
        liquidity["rank_ic_mean"].as_f64().unwrap() < -0.3,
        "{liquidity}"
    );
    assert_eq!(liquidity["consistent"], false);
    // 财务因子按截止日推断可用日后有值
    assert!(by_name("净资产收益率")["periods"].as_u64().unwrap() > 0);
    assert!(v["overfit"]["pbo"].is_number(), "{}", v["overfit"]);
    assert_eq!(v["manifest"]["trial_count"], 1);

    let uri = "/api/v1/research/factor-portfolio?factors=liquidity:-1,idio_vol&start=2025-01-02&end=2025-08-29&top_n=10&capital=1000000";
    let (s, v) = call(&app, "GET", uri, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let summary = &v["summary"];
    assert!(
        summary["avg_positions"].as_f64().unwrap() > 8.0,
        "{summary}"
    );
    // 反过来用成交额：持有漂移最大的股票，应当跑赢等权基准
    assert!(
        summary["excess_annual"].as_f64().unwrap() > 0.0,
        "{summary}"
    );
    assert_eq!(v["picks"].as_array().unwrap().len(), 10);
    // 合成得分里还有特质波动率和行业中性化，排第一的应在成交额最大的那一批里
    let first: u32 = v["picks"][0]["thscode"].as_str().unwrap()[..6]
        .parse()
        .unwrap();
    assert!(first > 60, "{}", v["picks"][0]);
    assert!(v["deflated_sharpe"].is_number());
    assert_eq!(v["manifest"]["trial_count"], 1);
    // 同一组配置再算一次不增加试验次数，换一组配置才增加
    let (_, again) = call(&app, "GET", uri, None).await;
    assert_eq!(again["manifest"]["trial_count"], 1);
    let (_, other) = call(&app, "GET", &format!("{uri}&rebalance=10"), None).await;
    assert_eq!(other["manifest"]["trial_count"], 2);
    // 成本和本金也属于方法定义：换了就是另一组配置
    let (_, other) = call(&app, "GET", &format!("{uri}&slippage_bps=30"), None).await;
    assert_eq!(other["manifest"]["trial_count"], 3);
    // 检验的分组数同理；完整配置随运行记录保存
    let test_uri = "/api/v1/research/factor-test?factors=reversal,liquidity,idio_vol,roe&start=2025-01-02&end=2025-08-29";
    let (_, other) = call(&app, "GET", &format!("{test_uri}&quantiles=10"), None).await;
    assert_eq!(other["manifest"]["trial_count"], 2, "{}", other["config"]);
    assert_eq!(other["config"]["quantiles"], 10);
    let configs: Vec<Value> =
        sqlx::query_scalar("SELECT config FROM ops.backtest_run ORDER BY run_id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(configs[0]["quantiles"], 5);
    assert_eq!(configs[1]["capital"], 1_000_000.0);
    assert_eq!(configs[1]["costs"]["slippage_bps"], 10.0);
    assert_eq!(configs[1]["universe"]["min_listed_days"], 60);
    let modes: Vec<String> =
        sqlx::query_scalar("SELECT mode FROM ops.backtest_run ORDER BY run_id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        modes,
        [
            "factor_test",
            "factor_portfolio",
            "factor_portfolio",
            "factor_portfolio",
            "factor_portfolio",
            "factor_test"
        ]
    );

    let (s, _) = call(
        &app,
        "GET",
        "/api/v1/research/factor-test?factors=nope",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(
        &app,
        "GET",
        "/api/v1/research/factor-portfolio?factors=reversal&top_n=500",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // 账户的因子排名和因子研究同一套计算；新账户第一份建议就按排名建仓
    let date = chrono::NaiveDate::from_ymd_opt(2025, 8, 29).unwrap();
    let rules = niulai::account::rules::Rules::default();
    let ranked = niulai::advice::ranking::ranking(&db.pool, date, &rules)
        .await
        .unwrap();
    assert!(ranked.len() >= 50, "{}", ranked.len());
    assert!(ranked.windows(2).all(|w| w[0].1 >= w[1].1));
    let (s, acc) = call(
        &app,
        "POST",
        "/api/v1/accounts",
        Some(serde_json::json!({"name": "组合 E", "cash": 1000000})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{acc}");
    let id = acc["summary"]["account"]["account_id"].as_i64().unwrap();
    let advice_uri = format!("/api/v1/accounts/{id}/advice");
    // 建账户在同一个事务里记下重算待办；后台消费者生成成功后确认
    let (_, v) = call(&app, "GET", &advice_uri, None).await;
    assert_eq!(v["regenerating"], true, "{v}");
    assert!(v["advice"].is_null());
    assert_eq!(niulai::advice::regen::drain(&db.pool).await.unwrap(), 1);
    let (_, v) = call(&app, "GET", &advice_uri, None).await;
    assert_eq!(v["regenerating"], false, "{v}");
    assert_eq!(v["advice"]["advice"]["headline"], "买入 30 只", "{v}");
    assert_eq!(
        niulai::advice::regen::drain(&db.pool).await.unwrap(),
        0,
        "确认后不再生成"
    );
    niulai::advice::refresh_all(&db.pool, niulai::advice::Refresh::ReplaceUntouched)
        .await
        .unwrap();
    let (_, v) = call(&app, "GET", &advice_uri, None).await;
    let advice = &v["advice"]["advice"];
    assert_eq!(advice["headline"], "买入 30 只", "{v}");
    let first = &advice["items"][0];
    assert_eq!(first["thscode"].as_str(), Some(ranked[0].0.as_str()));
    // 2025-08-29 是 8 月最后一个交易日：整体调仓
    assert_eq!(first["headline"], "月度调仓：因子排名第 1，买入", "{first}");

    // 处理建议：流水、处理状态和影子订单一起提交
    let advice_id = v["advice"]["advice_id"].as_i64().unwrap();
    let key0 = first["key"].as_str().unwrap().to_string();
    let key1 = advice["items"][1]["key"].as_str().unwrap().to_string();
    let item_uri = |key: &str, what: &str| format!("{advice_uri}/{advice_id}/items/{key}/{what}");
    let shadow = |key: String| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_as::<_, (String, Option<i64>)>(
                "SELECT status, actual_shares FROM app.shadow_order WHERE advice_id = $1 AND item_key = $2",
            )
            .bind(advice_id)
            .bind(key)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let shares = first["shares"].as_i64().unwrap();
    let price = first["ref_price"].as_f64().unwrap();
    let (s, v) = call(
        &app,
        "POST",
        &item_uri(&key0, "execute"),
        Some(json!({"price": price})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["advice"]["states"][&key0], "executed");
    assert_eq!(
        shadow(key0.clone()).await,
        ("filled".to_string(), Some(shares))
    );
    let (s, _) = call(
        &app,
        "POST",
        &item_uri(&key0, "execute"),
        Some(json!({"price": price})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "同一条只能记一次");
    let (s, _) = call(&app, "POST", &item_uri(&key0, "skip"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "已执行的不能改成不做");
    assert_eq!(
        shadow(key0.clone()).await.0,
        "filled",
        "影子订单不被改成取消"
    );
    let (s, v) = call(&app, "POST", &item_uri(&key1, "skip"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(shadow(key1.clone()).await.0, "cancelled");
    let (s, v) = call(&app, "DELETE", &item_uri(&key1, "skip"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["advice"]["states"].get(&key1).is_none());
    assert_eq!(
        shadow(key1.clone()).await.0,
        "planned",
        "撤销不做后影子订单回到计划中"
    );
    let (s, _) = call(&app, "POST", &item_uri("buy-NOPE.SZ", "skip"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = call(&app, "GET", &advice_uri, None).await;
    assert_eq!(v["regenerating"], false, "点「已执行」不重新生成");

    // 重新生成失败时保留待办、把原因给页面，修好后重试成功
    sqlx::query(
        "UPDATE app.account SET rules = '{}', advice_dirty_at = now() WHERE account_id = $1",
    )
    .bind(id)
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(niulai::advice::regen::drain(&db.pool).await.unwrap(), 1);
    let (failed, error): (bool, Option<String>) =
        niulai::account::stale::status(&db.pool, id).await.unwrap();
    assert!(failed && error.is_some(), "{error:?}");
    assert_eq!(
        niulai::advice::regen::drain(&db.pool).await.unwrap(),
        0,
        "失败后推迟重试"
    );
    sqlx::query("UPDATE app.account SET rules = $2, advice_retry_at = now() WHERE account_id = $1")
        .bind(id)
        .bind(serde_json::to_value(&rules).unwrap())
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(niulai::advice::regen::drain(&db.pool).await.unwrap(), 1);
    assert_eq!(
        niulai::account::stale::status(&db.pool, id).await.unwrap(),
        (false, None)
    );

    // 因子排名按数据版本缓存：财报变了但版本没变时命中旧结果，因子输入的版本更新后重算
    sqlx::query("DELETE FROM raw.record WHERE dataset_id = 'a_share.income_q'")
        .execute(&db.pool)
        .await
        .unwrap();
    let cached = niulai::advice::ranking::ranking(&db.pool, date, &rules)
        .await
        .unwrap();
    assert_eq!(cached, ranked, "版本没变，命中缓存");
    sqlx::query("UPDATE ops.derived_version SET updated_at = now() WHERE name = 'factor_input'")
        .execute(&db.pool)
        .await
        .unwrap();
    let fresh = niulai::advice::ranking::ranking(&db.pool, date, &rules)
        .await
        .unwrap();
    assert_ne!(fresh, ranked, "利润增速等财务因子没有数据了，排名应当变化");
    db.drop().await;
}

/// 因子预设：同名覆盖、按用户隔离、参数校验和删除
#[tokio::test]
async fn factor_presets_per_user() {
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
    let admin = login(&app, "admin", "secret-pass").await.unwrap();
    call_as(
        &app,
        Some(&admin),
        "POST",
        "/api/v1/users",
        Some(json!({"username": "mama", "password": "mama-pass-1"})),
    )
    .await;
    let mama = login(&app, "mama", "mama-pass-1").await.unwrap();
    let save = |cookie: String, body: Value| {
        let app = app.clone();
        async move {
            call_as(
                &app,
                Some(&cookie),
                "PUT",
                "/api/v1/research/presets",
                Some(body),
            )
            .await
        }
    };
    let preset = |name: &str, top_n: u32| {
        json!({"name": name, "factors": [{"factor": "volatility", "weight": 1.0}, {"factor": "roe", "weight": 0.5}],
               "top_n": top_n, "buffer": 1.0})
    };

    let (r, v) = save(admin.clone(), preset("稳健", 30)).await;
    assert_eq!(r.status(), StatusCode::OK, "{v}");
    assert_eq!(v["factors"][1]["factor"], "roe");
    // 同名（不区分大小写）覆盖，不新增
    let (r, v) = save(admin.clone(), preset(" 稳健 ", 20)).await;
    assert_eq!(r.status(), StatusCode::OK, "{v}");
    let (_, list) = call_as(&app, Some(&admin), "GET", "/api/v1/research/presets", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["top_n"], 20);
    // 持有只数和账户规则同样限制在 5 到 50；没有因子、名字为空都不行
    for bad in [
        preset("太多", 60),
        json!({"name": "空", "factors": [], "top_n": 30, "buffer": 1.0}),
        preset("", 30),
    ] {
        let (r, _) = save(admin.clone(), bad).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }
    // 别人看不到，也删不掉
    let (_, list) = call_as(&app, Some(&mama), "GET", "/api/v1/research/presets", None).await;
    assert!(list.as_array().unwrap().is_empty());
    let id = v["preset_id"].as_i64().unwrap();
    let uri = format!("/api/v1/research/presets/{id}");
    call_as(&app, Some(&mama), "DELETE", &uri, None).await;
    let (_, list) = call_as(&app, Some(&admin), "GET", "/api/v1/research/presets", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (r, _) = call_as(&app, Some(&admin), "DELETE", &uri, None).await;
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let (_, list) = call_as(&app, Some(&admin), "GET", "/api/v1/research/presets", None).await;
    assert!(list.as_array().unwrap().is_empty());
    db.drop().await;
}
