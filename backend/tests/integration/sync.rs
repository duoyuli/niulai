//! 同步框架：任务队列、运行结束与波次接续、规划中断、同步计划、数据核检、定期清理、同步完成事件和完整性核对。

use crate::common::*;

#[tokio::test]
async fn task_queue_claim_dedup_and_stale_requeue() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let reg = registry();
    niulai::sync::registry::sync_to_db(&db.pool, &reg)
        .await
        .unwrap();
    let run_id: i64 =
        sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let insert = "INSERT INTO ops.task (run_id, dataset_id, part_key, params) VALUES ($1, 'a_share.calendar', '*', '{}')
                  ON CONFLICT (dataset_id, part_key) WHERE status IN ('pending', 'running', 'failed') DO NOTHING";
    assert_eq!(
        sqlx::query(insert)
            .bind(run_id)
            .execute(&db.pool)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    // 同一分区已有未完成任务：重复规划不会产生第二个任务
    assert_eq!(
        sqlx::query(insert)
            .bind(run_id)
            .execute(&db.pool)
            .await
            .unwrap()
            .rows_affected(),
        0
    );

    // 认领后模拟执行者崩溃：超时回收，放回队列
    sqlx::query("UPDATE ops.task SET status = 'running', locked_by = 'w1', locked_at = now() - interval '1 hour', attempts = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    let n: i32 = sqlx::query_scalar("SELECT ops.requeue_stale('10 minutes')")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let status: String = sqlx::query_scalar("SELECT status FROM ops.task")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(status, "failed", "回收后应能被重新认领");

    // 通用记录库：首次写入、重复写入、修订
    let recs = json!([{"rkey": "k1", "obs_date": "2026-10-14", "thscode": "600519.SH", "payload": {"v": 1}}]);
    let first: (i32, i32) =
        sqlx::query_as("SELECT * FROM raw.upsert_records('a_share.calendar', 1, $1)")
            .bind(&recs)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let again: (i32, i32) =
        sqlx::query_as("SELECT * FROM raw.upsert_records('a_share.calendar', 1, $1)")
            .bind(&recs)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let revised = json!([{"rkey": "k1", "obs_date": "2026-10-14", "thscode": "600519.SH", "payload": {"v": 2}}]);
    let rev: (i32, i32) =
        sqlx::query_as("SELECT * FROM raw.upsert_records('a_share.calendar', 1, $1)")
            .bind(&revised)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((first, again, rev), ((1, 0), (0, 0), (0, 1)));
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM raw.record_version")
        .fetch_one(&db.pool)
        .await
        .unwrap();
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
    niulai::sync::registry::sync_to_db(&db.pool, &registry())
        .await
        .unwrap();
    let datasets = vec!["a_share.calendar".to_string()];
    let mut tx = db.pool.begin().await.unwrap();
    let new_run = NewRun {
        kind: RunKind::Backfill,
        trade_date: "2026-10-15".parse().unwrap(),
        datasets: &datasets,
        pending_waves: &[],
        root_run: None,
        only_thscodes: &[],
        trigger: None,
        planning: false,
    };
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
    let (status, attempts): (String, i16) =
        sqlx::query_as("SELECT status, attempts FROM ops.task WHERE task_id = $1")
            .bind(t.task_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), attempts), ("pending", 0));

    let t = q.claim().await.unwrap().unwrap();
    assert!(
        q.heartbeat(t.task_id).await.unwrap(),
        "本执行者持有的任务应能续租"
    );
    assert!(
        !TaskQueue::new(db.pool.clone(), "w2".into())
            .heartbeat(t.task_id)
            .await
            .unwrap(),
        "其他执行者不能续租"
    );
    q.finish(
        &t,
        Finished::Done,
        TaskStats {
            pages: 1,
            records: 5,
            changed: 5,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        run::try_complete(&db.pool, run_id).await.unwrap(),
        None,
        "还有未完成的任务"
    );

    let t = q.claim().await.unwrap().unwrap();
    q.fail(&t, Some(1002), "参数错误", true).await.unwrap();
    let progress: (i32, i32) =
        sqlx::query_as("SELECT tasks_done, tasks_failed FROM ops.run WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(progress, (1, 1), "完成和失败的计数随任务结束累加");

    // 最后一个任务被超时回收后直接标为 dead：没有执行者结束运行，由维护流程发现
    sqlx::query("UPDATE ops.task SET status = 'running', locked_by = 'gone', locked_at = now() - interval '1 hour', attempts = max_attempts WHERE status = 'pending'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(q.requeue_stale().await.unwrap(), 1);
    assert_eq!(run::stalled(&db.pool).await.unwrap(), vec![run_id]);
    assert_eq!(
        run::try_complete(&db.pool, run_id)
            .await
            .unwrap()
            .as_deref(),
        Some("partial")
    );
    assert_eq!(
        run::try_complete(&db.pool, run_id).await.unwrap(),
        None,
        "运行只能被结束一次"
    );
    let counts: (i32, i32) =
        sqlx::query_as("SELECT tasks_done, tasks_failed FROM ops.run WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(counts, (1, 2), "结束时按任务表重新统计");

    db.drop().await;
}

#[tokio::test]
async fn sync_schedule_can_be_viewed_and_changed() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let app = logged_in(niulai::api::router(
        Engine::new(
            bare_settings("http://127.0.0.1:1", Some("test")),
            db.pool.clone(),
        )
        .await
        .unwrap(),
    ))
    .await;

    // 默认：每个交易日 18:00、每周六 08:00
    let (st, v) = call(&app, "GET", "/api/v1/sync/schedule", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["times"],
        json!({"daily_time": "18:00", "weekly_weekday": 6, "weekly_time": "08:00"})
    );
    let plan = |v: &Value, key: &str| {
        v["plans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["key"] == key)
            .cloned()
            .unwrap()
    };
    let has = |p: &Value, id: &str| p["datasets"].as_array().unwrap().iter().any(|d| d == id);
    assert!(has(&plan(&v, "daily"), "a_share.snapshot"));
    assert!(
        has(&plan(&v, "weekly"), "meta.tickers") && has(&plan(&v, "weekly"), "a_share.calendar")
    );
    assert!(
        !has(&plan(&v, "daily"), "fund.nav"),
        "只列已开启的数据集（P2、P3 默认关闭）"
    );
    assert!(plan(&v, "manual")["next_at"].is_null());
    assert!(plan(&v, "weekly")["next_at"].is_string());

    // 从配置删掉的数据集留在 ops.dataset 中（已关闭），数据集列表不再展示
    sqlx::query("INSERT INTO ops.dataset (dataset_id, domain, endpoint, priority, enabled, spec, spec_hash) VALUES ('a_share.removed', 'a_share', '/x', 'P1', false, '{}', '')")
        .execute(&db.pool)
        .await
        .unwrap();
    let (st, v) = call(&app, "GET", "/api/v1/datasets", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["dataset_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"a_share.snapshot") && !ids.contains(&"a_share.removed"));

    let (st, _) = call(
        &app,
        "PUT",
        "/api/v1/sync/schedule",
        Some(json!({"daily_time": "25:00", "weekly_weekday": 6, "weekly_time": "08:00"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        &app,
        "PUT",
        "/api/v1/sync/schedule",
        Some(json!({"daily_time": "18:00", "weekly_weekday": 0, "weekly_time": "08:00"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        &app,
        "PUT",
        "/api/v1/sync/schedule",
        Some(json!({"daily_time": "19:30", "weekly_weekday": 7, "weekly_time": "9:05"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        v["times"],
        json!({"daily_time": "19:30", "weekly_weekday": 7, "weekly_time": "09:05"})
    );
    assert_eq!(plan(&v, "weekly")["rule"], "每周日 09:05");
    let (_, v) = call(&app, "GET", "/api/v1/sync/schedule", None).await;
    assert_eq!(v["times"]["daily_time"], "19:30", "保存在数据库里");

    // 手动发起的运行记下来源
    let (st, body) = call(
        &app,
        "POST",
        "/api/v1/sync/runs",
        Some(json!({"kind": "incremental", "datasets": ["a_share.calendar"]})),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "{body}");
    let (_, runs) = call(&app, "GET", "/api/v1/sync/runs", None).await;
    assert_eq!(runs[0]["params"]["trigger"], "manual");

    // 详情是整次同步；同一部分已经在排队的，再发起时不新建任务，并说明在哪次同步里
    let first = body["run_id"].as_i64().unwrap();
    let (st, d) = call(&app, "GET", &format!("/api/v1/sync/runs/{first}"), None).await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert_eq!(d["waves"].as_array().unwrap().len(), 1);
    let mut d = d;
    for _ in 0..50 {
        if d["progress"].as_array().is_some_and(|p| !p.is_empty()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        let (_, next) = call(&app, "GET", &format!("/api/v1/sync/runs/{first}"), None).await;
        d = next;
    }
    assert_eq!(d["progress"][0]["dataset_id"], "a_share.calendar");
    assert_eq!(d["progress"][0]["pending"], 1);
    let (_, again) = call(
        &app,
        "POST",
        "/api/v1/sync/runs",
        Some(json!({"kind": "incremental", "datasets": ["a_share.calendar"]})),
    )
    .await;
    let (_, d) = call(
        &app,
        "GET",
        &format!("/api/v1/sync/runs/{}", again["run_id"]),
        None,
    )
    .await;
    assert_eq!(d["progress"].as_array().unwrap().len(), 0);
    assert_eq!(
        d["without_tasks"],
        json!([{"dataset_id": "a_share.calendar", "queued_in": first}])
    );
    db.drop().await;
}

#[tokio::test]
async fn interrupted_planning_run_is_failed_by_maintenance_rule() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, status, params, started_at) VALUES ('backfill', 'running', '{\"planning\": true}'::jsonb, now() - interval '11 minutes') RETURNING run_id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let stale = niulai::sync::run::stale_planning(&db.pool).await.unwrap();
    assert_eq!(stale, vec![run_id]);
    assert!(
        niulai::sync::run::fail_planning(&db.pool, run_id)
            .await
            .unwrap()
    );
    let state: (String, Option<String>, serde_json::Value) =
        sqlx::query_as("SELECT status, error, params FROM ops.run WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(state.0, "failed");
    assert_eq!(state.1.as_deref(), Some("规划中断（服务重启），请重新发起"));
    assert_eq!(state.2.get("planning"), None);
    let live_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, status, params, started_at) VALUES ('backfill', 'running', '{\"planning\": true}'::jsonb, now() - interval '11 minutes') RETURNING run_id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(
        niulai::sync::run::touch_planning(&db.pool, live_id)
            .await
            .unwrap()
    );
    assert!(
        !niulai::sync::run::stale_planning(&db.pool)
            .await
            .unwrap()
            .contains(&live_id)
    );
    sqlx::query("DELETE FROM ops.run WHERE run_id = $1")
        .bind(live_id)
        .execute(&db.pool)
        .await
        .unwrap();
    db.drop().await;
}

/// 执行一次数据核检。核检在进程内同时只做一次，并行的其他测试正在核检时稍等再试
async fn check_now(engine: &Engine, trigger: &str) -> i64 {
    for _ in 0..600 {
        if let Some(id) = niulai::sync::check::run(engine, trigger).await.unwrap() {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("等了 60 秒，其他测试的核检还没结束");
}

#[tokio::test]
async fn data_check_finds_gaps_refetches_and_confirms_suspensions() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = db.pool.clone();
    let engine = Engine::new(
        bare_settings("http://127.0.0.1:1", Some("test")),
        pool.clone(),
    )
    .await
    .unwrap();
    // 两只股票，10 个交易日；B 缺 01-07、01-08。还有一只没有上市日、也没有日线的（还没上市），不查
    for (code, list_date) in [
        ("000001.SZ", Some("2026-01-05")),
        ("000002.SZ", Some("2026-01-05")),
        ("301999.SZ", None),
    ] {
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
    let run_id: i64 =
        sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id)
         SELECT c, d.cal_date, 10, 10, 10, 10, 100, 1000, $1 FROM ref.trading_day d, unnest(ARRAY['000001.SZ', '000002.SZ']) c
         WHERE NOT (c = '000002.SZ' AND d.cal_date IN ('2026-01-07', '2026-01-08'))",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();

    let result = |v: &Value| {
        v["last"]["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["dataset_id"] == "a_share.bar_1d")
            .cloned()
            .unwrap()
    };
    let app = logged_in(niulai::api::router(
        Engine::new(
            bare_settings("http://127.0.0.1:1", Some("test")),
            pool.clone(),
        )
        .await
        .unwrap(),
    ))
    .await;

    // 第一次：发现 2 个缺口，合并成一个窗口重取，排在补偿那一层
    check_now(&engine, "manual").await;
    let (st, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let r = result(&v);
    assert_eq!(
        (
            r["securities"].as_i64(),
            r["security_days"].as_i64(),
            r["new_gaps"].as_i64(),
            r["open"].as_i64()
        ),
        (Some(2), Some(20), Some(2), Some(2)),
        "{r}"
    );
    assert_eq!(r["refetch_tasks"], 1);
    let refetch = r["refetch_run"].as_i64().unwrap();
    let (part, priority): (String, i16) =
        sqlx::query_as("SELECT part_key, priority FROM ops.task WHERE run_id = $1")
            .bind(refetch)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(part, "thscode=000002.SZ|2026-01-07..2026-01-08");
    assert!((50..100).contains(&priority));
    assert_eq!(v["open"][0]["thscode"], "000002.SZ");
    assert_eq!(v["open"][0]["days"], 2);

    // 重取期间这只股票有未完成任务：先不查，也不重复安排
    check_now(&engine, "manual").await;
    let (_, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    let r = result(&v);
    assert_eq!(
        (
            r["securities"].as_i64(),
            r["busy"].as_i64(),
            r["refetch_tasks"].as_i64(),
            r["open"].as_i64()
        ),
        (Some(1), Some(1), Some(0), Some(2)),
        "{r}"
    );

    // 重取成功：01-07 取回来了，01-08 接口仍然没有，确认是停牌
    sqlx::query("INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id) VALUES ('000002.SZ', '2026-01-07', 10, 10, 10, 10, 100, 1000, $1)")
        .bind(refetch)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE ops.task SET status = 'done', finished_at = now() WHERE run_id = $1")
        .bind(refetch)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE ops.run SET status = 'succeeded', finished_at = now() WHERE run_id = $1")
        .bind(refetch)
        .execute(&pool)
        .await
        .unwrap();
    check_now(&engine, "followup").await;
    let (_, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    let r = result(&v);
    assert_eq!(
        (
            r["filled"].as_i64(),
            r["confirmed_now"].as_i64(),
            r["suspended"].as_i64(),
            r["open"].as_i64()
        ),
        (Some(1), Some(1), Some(1), Some(0)),
        "{r}"
    );
    assert!(r["refetch_run"].is_null());
    assert_eq!(v["open"].as_array().unwrap().len(), 0);

    // 再查一次：停牌日不再报，也不再重取
    check_now(&engine, "weekly").await;
    let (_, v) = call(&app, "GET", "/api/v1/sync/check", None).await;
    let r = result(&v);
    assert_eq!(
        (
            r["new_gaps"].as_i64(),
            r["suspended"].as_i64(),
            r["refetch_tasks"].as_i64()
        ),
        (Some(0), Some(1), Some(0)),
        "{r}"
    );

    let (st, _) = call(&app, "POST", "/api/v1/sync/check", None).await;
    assert_eq!(st, StatusCode::ACCEPTED);
    db.drop().await;
}

#[tokio::test]
async fn data_check_stops_at_delisting_date() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = db.pool.clone();
    let engine = Engine::new(
        bare_settings("http://127.0.0.1:1", Some("test")),
        pool.clone(),
    )
    .await
    .unwrap();
    // 一只股票 01-12 起推断为退市（被吸收合并），供应商之后对它返回 1002：退市日以后不能算缺口，
    // 否则每次核检都会安排一次注定失败的重取
    sqlx::query(
        "INSERT INTO raw.record (dataset_id, rkey, thscode, payload, payload_hash)
         VALUES ('meta.tickers', '601059.SH', '601059.SH', jsonb_build_object('thscode', '601059.SH', 'asset_type', 'a-share', 'list_date', '2026-01-05'), 'x')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO ref.security (thscode, ticker, name, exchange, board, list_date, first_seen, last_seen, delisted_on)
         VALUES ('601059.SH', '601059', '信达证券', 'SH', 'main', '2026-01-05', '2026-01-05', '2026-01-11', '2026-01-12')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO ref.trading_day SELECT d::date, 'api' FROM generate_series('2026-01-05'::date, '2026-01-16'::date, '1 day') d WHERE extract(isodow FROM d) < 6")
        .execute(&pool)
        .await
        .unwrap();
    let run_id: i64 =
        sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO mkt.stock_bar_1d (thscode, trade_date, open, high, low, close, volume, amount, run_id)
         SELECT '601059.SH', cal_date, 10, 10, 10, 10, 100, 1000, $1 FROM ref.trading_day WHERE cal_date < '2026-01-12'",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();
    check_now(&engine, "manual").await;
    let gaps: i64 = sqlx::query_scalar("SELECT count(*) FROM ops.bar_gap")
        .fetch_one(&pool)
        .await
        .unwrap();
    let refetch: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ops.task WHERE dataset_id = 'a_share.bar_1d'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((gaps, refetch), (0, 0), "退市日以后的交易日不应算成缺口");
    db.drop().await;
}

#[tokio::test]
async fn old_tasks_and_call_logs_are_pruned() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = db.pool.clone();
    niulai::sync::registry::sync_to_db(&pool, &registry())
        .await
        .unwrap();
    let old = "now() - interval '90 days'";
    let run: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, status) VALUES ('incremental', 'succeeded') RETURNING run_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let open_run: i64 =
        sqlx::query_scalar("INSERT INTO ops.run (kind) VALUES ('backfill') RETURNING run_id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let task = |run: i64, ds: &str, part: &str, status: &str, finished: &str| {
        let sql = format!(
            "INSERT INTO ops.task (run_id, dataset_id, part_key, params, status, finished_at) VALUES ($1, $2, $3, '{{}}', $4, {finished}) RETURNING task_id"
        );
        let pool = pool.clone();
        let (ds, part, status) = (ds.to_string(), part.to_string(), status.to_string());
        async move {
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql))
                .bind(run)
                .bind(ds)
                .bind(part)
                .bind(status)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
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
    let left: Vec<i64> = sqlx::query_scalar("SELECT task_id FROM ops.task ORDER BY task_id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        !left.contains(&resolved_dead) && !left.contains(&old_done),
        "已经解决的失败和过期的成功删掉"
    );
    for (id, why) in [
        (unresolved_dead, "还没解决的失败"),
        (newest_done, "数据集最近一个成功的任务"),
        (in_open_run, "进行中的运行"),
        (recent, "保留期内的"),
    ] {
        assert!(left.contains(&id), "{why}要保留");
    }
    assert_eq!(
        niulai::sync::retention::prune(&pool).await.unwrap(),
        niulai::sync::retention::Pruned::default(),
        "再清理一次没有可删的"
    );
    db.drop().await;
}

#[tokio::test]
async fn sync_events_wait_for_postprocess_and_ack_only_what_they_handled() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = db.pool.clone();
    niulai::sync::registry::sync_to_db(&pool, &registry())
        .await
        .unwrap();
    // 两个运行：一个只写了财报，一个只写了涨停池；都是最后一个任务刚结束
    let mut runs = vec![];
    for dataset in ["a_share.income_q", "special.limit_up_pool"] {
        let run: i64 = sqlx::query_scalar(
            "INSERT INTO ops.run (kind) VALUES ('incremental') RETURNING run_id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ops.task (run_id, dataset_id, part_key, params, status, changed, finished_at) VALUES ($1, $2, 'x', '{}', 'done', 3, now())")
            .bind(run)
            .bind(dataset)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            niulai::sync::run::try_complete(&pool, run)
                .await
                .unwrap()
                .is_some()
        );
        runs.push(run);
    }
    let versions = || {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (String, chrono::DateTime<chrono::Utc>)>(
                "SELECT name, updated_at FROM ops.derived_version ORDER BY name",
            )
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    let before = versions().await;

    // 运行到了终态，但复权等后处理还没做完：应用层不消费
    assert!(
        niulai::sync::run::ready_sync_events(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    niulai::bootstrap::replay_ready_events(&pool).await;
    assert_eq!(versions().await, before);
    // 后处理的租约还在，维护不接手；结束运行的执行者中途退出、租约过期后才接手
    assert!(
        niulai::sync::run::claim_stale_postprocess(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("UPDATE ops.run SET params = jsonb_set(params, '{sync_event_pending,claimed_at}', to_jsonb(now() - interval '1 hour')) WHERE run_id = ANY($1)")
        .bind(&runs)
        .execute(&pool)
        .await
        .unwrap();
    let mut claimed = niulai::sync::run::claim_stale_postprocess(&pool)
        .await
        .unwrap();
    claimed.sort();
    assert_eq!(claimed, runs);
    assert!(
        niulai::sync::run::claim_stale_postprocess(&pool)
            .await
            .unwrap()
            .is_empty(),
        "同一事件只有一个调用者能接手"
    );
    for run in &runs {
        niulai::sync::run::mark_sync_event_ready(&pool, *run, false)
            .await
            .unwrap();
    }
    assert_eq!(
        niulai::sync::run::ready_sync_events(&pool)
            .await
            .unwrap()
            .len(),
        2
    );

    // 两个事件合在一起处理：两个运行的影响都算上，然后只确认这两个
    let late: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind, status, params) VALUES ('incremental', 'succeeded', '{\"sync_event_pending\": {\"stage\": \"postprocess\"}}') RETURNING run_id")
        .fetch_one(&pool)
        .await
        .unwrap();
    niulai::bootstrap::replay_ready_events(&pool).await;
    let after = versions().await;
    let changed: Vec<&str> = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| b.1 != a.1)
        .map(|(b, _)| b.0.as_str())
        .collect();
    assert_eq!(changed, ["factor_input", "limit_pool"], "复权价没有变化");
    assert!(
        niulai::sync::run::ready_sync_events(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    let late_pending: bool =
        sqlx::query_scalar("SELECT params ? 'sync_event_pending' FROM ops.run WHERE run_id = $1")
            .bind(late)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(late_pending, "还在后处理的事件不被确认");
    db.drop().await;
}

#[tokio::test]
async fn audit_finds_gaps_and_plans_repairs() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let pool = &db.pool;
    let engine = Engine::new(
        bare_settings("http://127.0.0.1:1", Some("test")),
        pool.clone(),
    )
    .await
    .unwrap();
    // 两个过去的交易日：d1 同步过（估值错过了，一个任务因供应商不可用重试用尽），d2 完全没有同步；一只在市的股票没有日线
    let d1: chrono::NaiveDate = sqlx::query_scalar("SELECT current_date - 3")
        .fetch_one(pool)
        .await
        .unwrap();
    let d2 = d1 + chrono::Duration::days(1);
    sqlx::query("INSERT INTO ref.trading_day (cal_date, source) VALUES ($1, 'api'), ($2, 'api')")
        .bind(d1)
        .bind(d2)
        .execute(pool)
        .await
        .unwrap();
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
        (
            "a_share.valuation",
            "dead",
            "已错过：供应商最新交易日已是更晚的一天",
        ),
        (
            "special.hot_stock_daily",
            "dead",
            "可重试错误（code=Some(5003)，http=Some(200)）：Data source unavailable",
        ),
        // 同一个分区失败了两次：只能重新排队一个（每个分区同时只能有一个未完成的任务）
        (
            "special.hot_stock_daily",
            "dead",
            "可重试错误（code=Some(5003)，http=Some(200)）：Data source unavailable",
        ),
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
    sqlx::query("INSERT INTO ref.trading_day (cal_date, source) VALUES ($1, 'api')")
        .bind(d1 - chrono::Duration::days(10))
        .execute(pool)
        .await
        .unwrap();
    let holiday = (1..10)
        .map(|n| d1 - chrono::Duration::days(n))
        .find(|d| chrono::Datelike::weekday(d).number_from_monday() <= 5)
        .unwrap();
    let expired = d1 - chrono::Duration::days(400);
    let backfill: i64 = sqlx::query_scalar("INSERT INTO ops.run (kind, trade_date, status) VALUES ('backfill', $1, 'running') RETURNING run_id").bind(d1).fetch_one(pool).await.unwrap();
    for (date, err) in [
        (
            holiday,
            "不可重试错误（code=Some(1002)，http=Some(200)）：date must be an A-share trading day",
        ),
        (
            expired,
            "不可重试错误（code=Some(1003)，http=Some(200)）：date must be within one year",
        ),
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
    // 已推断退市的股票：退市前排队的日线任务返回 1002，补不回来
    sqlx::query("INSERT INTO ref.instrument (thscode, ticker, name, asset_type, exchange, first_seen, last_seen, delisted_on) VALUES ('601059.SH', '601059', '退市股', 'stock', 'SH', $1, $1, $2)")
        .bind(d1 - chrono::Duration::days(30))
        .bind(d1)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO ops.task (run_id, dataset_id, part_key, params, status, last_api_code, last_error, finished_at) VALUES ($1, 'a_share.bar_1d', $2, $3, 'dead', 1002, '不可重试错误（code=Some(1002)，http=Some(200)）：Unknown thscode: 601059.SH', now())")
        .bind(backfill)
        .bind(format!("thscode=601059.SH|{d1}..{d1}"))
        .bind(json!({"vars": {"thscode": "601059.SH"}}))
        .execute(pool)
        .await
        .unwrap();

    let h = niulai::sync::audit::health(&engine).await.unwrap();
    let dragon: Vec<&str> = h
        .failures
        .iter()
        .filter(|f| f.dataset_id == "special.dragon_tiger")
        .map(|f| f.reason.as_str())
        .collect();
    assert!(
        dragon.iter().any(|r| r.contains("不是交易日"))
            && dragon.iter().any(|r| r.contains("最近一年")),
        "{dragon:?}"
    );
    let kinds: Vec<(chrono::NaiveDate, &str, niulai::sync::audit::GapKind, bool)> = h
        .gaps
        .iter()
        .map(|g| (g.trade_date, g.dataset_id.as_str(), g.kind, g.recoverable))
        .collect();
    use niulai::sync::audit::GapKind::*;
    assert!(
        kinds.contains(&(d1, "a_share.valuation", Missed, false)),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(d1, "a_share.bar_1d", Incomplete, true)),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&(d2, "a_share.snapshot", NotRun, true)),
        "快照没同步，可由个股日线补：{kinds:?}"
    );
    assert!(
        kinds.contains(&(d2, "a_share.valuation", NotRun, false)),
        "{kinds:?}"
    );
    assert!(
        !kinds.iter().any(|k| k.0 == d1 && k.1 == "a_share.snapshot"),
        "d1 的快照成功了"
    );
    assert_eq!(h.latest_trade_date, Some(d2));
    assert_eq!(h.daily_state, niulai::sync::audit::DailyState::Incomplete);
    assert!(
        h.failures
            .iter()
            .any(|f| f.dataset_id == "a_share.valuation" && f.reason.contains("无法补回"))
    );

    // 补偿：能补的规划补偿运行，供应商不可用的任务重新排队，错过的估值补不回来
    let s = niulai::sync::audit::repair(&engine).await.unwrap();
    assert!(!s.runs.is_empty());
    assert_eq!(s.retried, 1);
    assert_eq!(
        s.cleared, 3,
        "节假日、超出一年和已退市证券的任务本来就没有数据，改为跳过"
    );
    let h = niulai::sync::audit::health(&engine).await.unwrap();
    assert!(
        !h.failures
            .iter()
            .any(|f| f.dataset_id == "special.dragon_tiger"),
        "{:?}",
        h.failures
    );
    assert!(s.unrecoverable >= 2);
    let repair_runs: Vec<String> = sqlx::query_scalar("SELECT dataset_id FROM ops.run WHERE kind = 'repair' AND dataset_id IS NOT NULL ORDER BY 1").fetch_all(pool).await.unwrap();
    assert!(
        repair_runs.contains(&"a_share.bar_1d".to_string()),
        "{repair_runs:?}"
    );
    assert!(
        repair_runs.contains(&"a_share.corp_action_dump".to_string()),
        "{repair_runs:?}"
    );
    let reopened: String = sqlx::query_scalar("SELECT status FROM ops.run WHERE run_id = $1")
        .bind(run)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(reopened, "partial", "补偿不会重开非 repair 终态运行");
    let pr: i16 = sqlx::query_scalar("SELECT priority FROM ops.task WHERE dataset_id = 'special.hot_stock_daily' AND status = 'pending'").fetch_one(pool).await.unwrap();
    assert!((50..100).contains(&pr), "重新排队的任务在补偿层：{pr}");
    db.drop().await;
}

#[tokio::test]
async fn finished_run_continues_its_chain_once() {
    let Some(db) = TestDb::create().await else {
        eprintln!("未设置 NIULAI_TEST_DATABASE_URL，跳过");
        return;
    };
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO ops.run (kind, status, finished_at, params) VALUES ('incremental', 'succeeded', now() - interval '1 hour', '{\"pending_waves\": [[\"a_share.calendar\"]]}')
         RETURNING run_id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // 结束了却没接上下一个波次：维护能找出来
    assert_eq!(
        niulai::sync::run::unchained(&db.pool).await.unwrap(),
        vec![run_id]
    );
    // 只有第一个接续者能清空 pending_waves
    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        niulai::sync::run::take_pending_waves(&mut tx, run_id)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    assert!(
        !niulai::sync::run::take_pending_waves(&mut tx, run_id)
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
    assert!(
        niulai::sync::run::unchained(&db.pool)
            .await
            .unwrap()
            .is_empty()
    );
}
