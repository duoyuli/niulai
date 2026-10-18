//! 选股策略与每日巡检接口。计算在 `crate::analysis`（strategy、screener），规则见 docs/domains/strategies.md。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::analysis::indicators::{self, IndicatorInfo};
use crate::analysis::screener::{self, Hit, ScreeningSummary};
use crate::analysis::signal_lib;
use crate::analysis::signals::{self, SignalInfo};
use crate::analysis::strategy::{Definition, Template, Universe, templates};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(catalog))
        .routes(routes!(list_strategies, create_strategy))
        .routes(routes!(reorder_strategies))
        .routes(routes!(get_strategy, update_strategy, delete_strategy))
        .routes(routes!(preview))
        .routes(routes!(results))
        .routes(routes!(run_screening))
}

#[derive(Serialize, ToSchema)]
pub struct Catalog {
    pub indicators: Vec<IndicatorInfo>,
    pub signals: Vec<SignalInfo>,
    pub templates: Vec<Template>,
    /// 新建策略时的默认股票池
    pub default_universe: Universe,
}

/// 可用的指标、信号和内置模板
#[utoipa::path(get, path = "/strategies/catalog", tag = "strategy", responses((status = 200, description = "成功", body = Catalog)))]
async fn catalog() -> Json<Catalog> {
    Json(Catalog {
        indicators: indicators::CATALOG.to_vec(),
        signals: signals::CATALOG.to_vec(),
        templates: templates(),
        default_universe: Universe::default(),
    })
}

/// 策略的白话说明：股票池、买入条件、卖出规则
#[derive(Serialize, ToSchema)]
pub struct Explained {
    pub universe: Vec<String>,
    /// 全部满足
    pub all: Vec<String>,
    /// 至少满足一个
    pub any: Vec<String>,
    pub exit: Vec<String>,
}

fn explain(d: &Definition) -> Explained {
    let mut exit: Vec<String> = d.exit.any.iter().map(|c| d.describe_condition(c)).collect();
    if let Some(v) = d.exit.stop_loss_pct {
        exit.push(format!("亏损 {v}% 止损"));
    }
    if let Some(v) = d.exit.take_profit_pct {
        exit.push(format!("盈利 {v}% 止盈"));
    }
    if let Some(v) = d.exit.max_hold_days {
        exit.push(format!("最多持有 {v} 个交易日"));
    }
    Explained {
        universe: d.describe_universe(),
        all: d.all.iter().map(|c| d.describe_condition(c)).collect(),
        any: d.any.iter().map(|c| d.describe_condition(c)).collect(),
        exit,
    }
}

#[derive(Serialize, ToSchema)]
pub struct LatestRun {
    pub trade_date: NaiveDate,
    /// 最新买点数（截断前）
    pub matched: i32,
    /// 保存的买点数
    pub hits: i64,
    /// 其中上一次巡检没有出现的
    pub new_hits: i64,
    /// 最新卖点数
    pub sells: i64,
    /// 持有中的股票数
    pub holdings: i64,
}

#[derive(Serialize, ToSchema)]
pub struct StrategyItem {
    pub strategy_id: i64,
    pub name: String,
    pub description: String,
    /// 是否参加每日巡检
    pub screening: bool,
    /// 是否在首页展示选出的股票
    pub show_on_today: bool,
    pub template: Option<String>,
    /// 内置策略：由模板生成，只读、不能删除、总是参加巡检，账户的调仓建议会参考它；想改就复制一份
    pub builtin: bool,
    pub definition: Definition,
    pub explained: Explained,
    /// 最近一次巡检；还没有巡检过时为空
    pub latest: Option<LatestRun>,
    pub updated_at: DateTime<Utc>,
}

type StrategyRow = (
    i64,
    String,
    String,
    bool,
    bool,
    Option<String>,
    bool,
    serde_json::Value,
    DateTime<Utc>,
    i32,
);

async fn load_items(s: &AppState, id: Option<i64>) -> ApiResult<Vec<StrategyItem>> {
    let rows: Vec<StrategyRow> = sqlx::query_as(
        "SELECT strategy_id, name, description, screening, show_on_today, template, builtin_key IS NOT NULL, definition, updated_at, sort_order FROM app.strategy
         WHERE ($1::bigint IS NULL OR strategy_id = $1)
         ORDER BY sort_order, strategy_id",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?;
    let latest: Vec<(i64, NaiveDate, i32, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT DISTINCT ON (r.strategy_id) r.strategy_id, r.trade_date, r.matched,
                (SELECT count(*) FROM app.screen_hit h WHERE h.run_id = r.run_id AND h.kind = 'buy'),
                (SELECT count(*) FROM app.screen_hit h WHERE h.run_id = r.run_id AND h.kind = 'buy' AND h.is_new),
                (SELECT count(*) FROM app.screen_hit h WHERE h.run_id = r.run_id AND h.kind = 'sell'),
                (SELECT count(*) FROM app.screen_hit h WHERE h.run_id = r.run_id AND h.kind = 'holding')
         FROM app.screen_run r WHERE ($1::bigint IS NULL OR r.strategy_id = $1) ORDER BY r.strategy_id, r.trade_date DESC",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?;
    let lib = signal_lib::load(&s.pool).await?;
    let mut out = vec![];
    for (
        strategy_id,
        name,
        description,
        screening,
        show_on_today,
        template,
        builtin,
        v,
        updated_at,
        _sort_order,
    ) in rows
    {
        let definition: Definition = serde_json::from_value::<Definition>(v)
            .map_err(|e| ApiError::Internal(e.into()))?
            .with_lib(lib.clone());
        let latest = latest.iter().find(|l| l.0 == strategy_id).map(
            |&(_, trade_date, matched, hits, new_hits, sells, holdings)| LatestRun {
                trade_date,
                matched,
                hits,
                new_hits,
                sells,
                holdings,
            },
        );
        out.push(StrategyItem {
            strategy_id,
            name,
            description,
            screening,
            show_on_today,
            template,
            builtin,
            explained: explain(&definition),
            definition,
            latest,
            updated_at,
        });
    }
    Ok(out)
}

/// 全部策略和各自最近一次巡检的概况
#[utoipa::path(get, path = "/strategies", tag = "strategy", responses((status = 200, description = "成功", body = [StrategyItem])))]
async fn list_strategies(State(s): State<AppState>) -> ApiResult<Json<Vec<StrategyItem>>> {
    Ok(Json(load_items(&s, None).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct StrategyOrderInput {
    /// 当前策略页的完整顺序，从上到下排列
    pub strategy_ids: Vec<i64>,
}

/// 保存策略页顺序（管理员）
#[utoipa::path(put, path = "/strategies/order", tag = "strategy", request_body = StrategyOrderInput,
    responses((status = 204, description = "顺序已保存"),
              (status = 400, description = "必须提供全部策略且不能重复", body = crate::error::ErrorBody)))]
async fn reorder_strategies(
    State(s): State<AppState>,
    Json(input): Json<StrategyOrderInput>,
) -> ApiResult<StatusCode> {
    let all: Vec<i64> = sqlx::query_scalar("SELECT strategy_id FROM app.strategy")
        .fetch_all(&s.pool)
        .await?;
    let want: HashSet<i64> = input.strategy_ids.iter().copied().collect();
    let have: HashSet<i64> = all.iter().copied().collect();
    if want.len() != input.strategy_ids.len() || want != have {
        return Err(ApiError::BadRequest(
            "策略顺序必须包含全部策略，且不能重复".into(),
        ));
    }
    let mut tx = s.pool.begin().await?;
    for (order, strategy_id) in input.strategy_ids.iter().enumerate() {
        sqlx::query("UPDATE app.strategy SET sort_order = $2 WHERE strategy_id = $1")
            .bind(strategy_id)
            .bind(order as i32)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct StrategyInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub definition: Definition,
    /// 是否参加每日巡检，默认 true
    #[serde(default = "yes")]
    pub screening: bool,
    /// 是否在首页展示，默认 true。只为测试建的策略可以关掉
    #[serde(default = "yes")]
    pub show_on_today: bool,
    /// 由哪个模板创建（只作记录）
    #[serde(default)]
    pub template: Option<String>,
}

fn yes() -> bool {
    true
}

async fn check(s: &AppState, input: &StrategyInput) -> ApiResult<()> {
    let n = input.name.trim().chars().count();
    if n == 0 || n > 40 {
        return Err(ApiError::BadRequest("策略名称应为 1 到 40 个字".into()));
    }
    // 条件里可能引用自定义信号：带上信号库再校验
    let lib = signal_lib::load(&s.pool).await?;
    input
        .definition
        .clone()
        .with_lib(lib)
        .validate()
        .map_err(ApiError::BadRequest)
}

/// 新建策略
#[utoipa::path(post, path = "/strategies", tag = "strategy", request_body = StrategyInput,
    responses((status = 201, description = "已创建", body = StrategyItem), (status = 400, description = "定义不合法", body = crate::error::ErrorBody)))]
async fn create_strategy(
    State(s): State<AppState>,
    Json(input): Json<StrategyInput>,
) -> ApiResult<(StatusCode, Json<StrategyItem>)> {
    check(&s, &input).await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO app.strategy (name, description, definition, screening, template, show_on_today, sort_order)
         VALUES ($1, $2, $3, $4, $5, $6, COALESCE((SELECT max(sort_order) + 1 FROM app.strategy), 0))
         RETURNING strategy_id",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(serde_json::to_value(&input.definition).map_err(|e| ApiError::Internal(e.into()))?)
    .bind(input.screening)
    .bind(&input.template)
    .bind(input.show_on_today)
    .fetch_one(&s.pool)
    .await?;
    let item = load_items(&s, Some(id))
        .await?
        .pop()
        .ok_or_else(|| ApiError::NotFound("策略不存在".into()))?;
    Ok((StatusCode::CREATED, Json(item)))
}

/// 单个策略
#[utoipa::path(get, path = "/strategies/{strategy_id}", tag = "strategy", params(("strategy_id" = i64, Path)),
    responses((status = 200, description = "成功", body = StrategyItem), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn get_strategy(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<StrategyItem>> {
    load_items(&s, Some(id))
        .await?
        .pop()
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("策略不存在：{id}")))
}

/// 修改策略。修改后之前的巡检结果保留，下一次巡检起按新定义。内置策略只能改「是否在首页展示」
#[utoipa::path(put, path = "/strategies/{strategy_id}", tag = "strategy", params(("strategy_id" = i64, Path)), request_body = StrategyInput,
    responses((status = 200, description = "已保存", body = StrategyItem),
              (status = 400, description = "定义不合法", body = crate::error::ErrorBody),
              (status = 404, description = "不存在", body = crate::error::ErrorBody),
              (status = 409, description = "内置策略只能改是否在首页展示", body = crate::error::ErrorBody)))]
async fn update_strategy(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<StrategyInput>,
) -> ApiResult<Json<StrategyItem>> {
    check(&s, &input).await?;
    let current = load_items(&s, Some(id))
        .await?
        .pop()
        .ok_or_else(|| ApiError::NotFound(format!("策略不存在：{id}")))?;
    if current.builtin {
        let same = input.name.trim() == current.name
            && input.description.trim() == current.description
            && input.screening
            && serde_json::to_value(&input.definition).ok()
                == serde_json::to_value(&current.definition).ok();
        if !same {
            return Err(ApiError::Conflict(
                "内置策略不能修改，可以复制一份再改".into(),
            ));
        }
    }
    let n = sqlx::query("UPDATE app.strategy SET name = $2, description = $3, definition = $4, screening = $5, show_on_today = $6, updated_at = now() WHERE strategy_id = $1")
        .bind(id)
        .bind(input.name.trim())
        .bind(input.description.trim())
        .bind(serde_json::to_value(&input.definition).map_err(|e| ApiError::Internal(e.into()))?)
        .bind(input.screening)
        .bind(input.show_on_today)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(format!("策略不存在：{id}")));
    }
    get_strategy(State(s), Path(id)).await
}

/// 删除策略和它的巡检结果。内置策略不能删除
#[utoipa::path(delete, path = "/strategies/{strategy_id}", tag = "strategy", params(("strategy_id" = i64, Path)),
    responses((status = 204, description = "已删除"), (status = 409, description = "内置策略不能删除", body = crate::error::ErrorBody)))]
async fn delete_strategy(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let n = sqlx::query("DELETE FROM app.strategy WHERE strategy_id = $1 AND builtin_key IS NULL")
        .bind(id)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 {
        let builtin: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM app.strategy WHERE strategy_id = $1)")
                .bind(id)
                .fetch_one(&s.pool)
                .await?;
        if builtin {
            return Err(ApiError::Conflict(
                "内置策略不能删除，可以关掉「在首页展示」".into(),
            ));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct PreviewInput {
    pub definition: Definition,
    /// 交易日，默认最新
    #[serde(default)]
    pub date: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema)]
pub struct PreviewResult {
    pub trade_date: NaiveDate,
    pub scanned: usize,
    pub matched: usize,
    pub hits: Vec<Hit>,
    pub explained: Explained,
}

/// 试运行：用这份定义在某个交易日选股，不保存。全市场计算一次约几秒
#[utoipa::path(post, path = "/strategies/preview", tag = "strategy", request_body = PreviewInput,
    responses((status = 200, description = "成功", body = PreviewResult), (status = 400, description = "定义不合法或没有日线", body = crate::error::ErrorBody)))]
async fn preview(
    State(s): State<AppState>,
    Json(input): Json<PreviewInput>,
) -> ApiResult<Json<PreviewResult>> {
    let definition = input.definition.with_lib(signal_lib::load(&s.pool).await?);
    definition.validate().map_err(ApiError::BadRequest)?;
    let out = screener::screen(&s.pool, &[&definition], input.date, screener::Mode::Preview)
        .await?
        .ok_or_else(|| ApiError::BadRequest("本地还没有日线".into()))?;
    let r = out.results.into_iter().next().expect("一个策略一个结果");
    Ok(Json(PreviewResult {
        trade_date: out.trade_date,
        scanned: out.scanned,
        matched: r.matched,
        hits: r.hits,
        explained: explain(&definition),
    }))
}

#[derive(Deserialize, IntoParams)]
pub struct ResultsQuery {
    /// 交易日，默认最近一次巡检
    pub date: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct StoredHit {
    pub thscode: String,
    pub name: String,
    pub rank: i32,
    /// 上一次巡检时不在结果中
    pub is_new: bool,
    #[schema(value_type = Vec<String>)]
    pub reasons: sqlx::types::Json<Vec<String>>,
    pub close: Option<f64>,
    pub change_pct: Option<f64>,
    pub amount: Option<f64>,
    pub sort_value: Option<f64>,
    /// 买入日、买入价（未复权开盘价）、买入以来的涨跌（%）、已持有的交易日数：卖点和持有中才有
    pub entry_date: Option<NaiveDate>,
    pub entry_price: Option<f64>,
    pub return_pct: Option<f64>,
    pub bars_held: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct ScreenResults {
    /// 没有巡检过时为空
    pub trade_date: Option<NaiveDate>,
    pub scanned: Option<i32>,
    pub matched: Option<i32>,
    pub ran_at: Option<DateTime<Utc>>,
    /// 最新买点：今天满足买入条件、之前空仓，明天开盘买入
    pub hits: Vec<StoredHit>,
    /// 最新卖点：持有中触发了卖出规则，明天开盘卖出；reasons 是原因
    pub sells: Vec<StoredHit>,
    /// 持有中：按最近一年的买卖点推算
    pub holdings: Vec<StoredHit>,
    /// 有巡检结果的交易日（最近 60 个，新的在前）
    pub dates: Vec<NaiveDate>,
}

/// 某个策略某个交易日的巡检结果
#[utoipa::path(get, path = "/strategies/{strategy_id}/results", tag = "strategy", params(("strategy_id" = i64, Path), ResultsQuery),
    responses((status = 200, description = "成功", body = ScreenResults), (status = 404, description = "策略不存在", body = crate::error::ErrorBody)))]
async fn results(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<ResultsQuery>,
) -> ApiResult<Json<ScreenResults>> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM app.strategy WHERE strategy_id = $1)")
            .bind(id)
            .fetch_one(&s.pool)
            .await?;
    if !exists {
        return Err(ApiError::NotFound(format!("策略不存在：{id}")));
    }
    let dates: Vec<NaiveDate> =
        sqlx::query_scalar("SELECT trade_date FROM app.screen_run WHERE strategy_id = $1 ORDER BY trade_date DESC LIMIT 60").bind(id).fetch_all(&s.pool).await?;
    let run: Option<(i64, NaiveDate, i32, i32, DateTime<Utc>)> = sqlx::query_as(
        "SELECT run_id, trade_date, scanned, matched, ran_at FROM app.screen_run
         WHERE strategy_id = $1 AND ($2::date IS NULL OR trade_date = $2) ORDER BY trade_date DESC LIMIT 1",
    )
    .bind(id)
    .bind(q.date)
    .fetch_optional(&s.pool)
    .await?;
    let Some((run_id, trade_date, scanned, matched, ran_at)) = run else {
        return Ok(Json(ScreenResults {
            trade_date: None,
            scanned: None,
            matched: None,
            ran_at: None,
            hits: vec![],
            sells: vec![],
            holdings: vec![],
            dates,
        }));
    };
    let rows = |kind: &'static str| {
        sqlx::query_as::<_, StoredHit>(
            "SELECT thscode, coalesce(metrics->>'name', '') AS name, rank, is_new, reasons,
                    (metrics->>'close')::float8 AS close, (metrics->>'change_pct')::float8 AS change_pct,
                    (metrics->>'amount')::float8 AS amount, (metrics->>'sort_value')::float8 AS sort_value,
                    (metrics->>'entry_date')::date AS entry_date, (metrics->>'entry_price')::float8 AS entry_price,
                    (metrics->>'return_pct')::float8 AS return_pct, (metrics->>'bars_held')::bigint AS bars_held
             FROM app.screen_hit WHERE run_id = $1 AND kind = $2 ORDER BY rank",
        )
        .bind(run_id)
        .bind(kind)
        .fetch_all(&s.pool)
    };
    let (hits, sells, holdings) = (
        rows("buy").await?,
        rows("sell").await?,
        rows("holding").await?,
    );
    Ok(Json(ScreenResults {
        trade_date: Some(trade_date),
        scanned: Some(scanned),
        matched: Some(matched),
        ran_at: Some(ran_at),
        hits,
        sells,
        holdings,
        dates,
    }))
}

#[derive(Deserialize, IntoParams)]
pub struct RunQuery {
    /// 交易日，默认最新
    pub date: Option<NaiveDate>,
    /// 只巡检这个策略；默认巡检全部开启巡检的策略
    pub strategy_id: Option<i64>,
}

/// 立即巡检并保存结果。每个交易日收盘同步完成后会自动执行，这里用于手动补跑或重跑某一天
#[utoipa::path(post, path = "/screening/run", tag = "strategy", params(RunQuery), responses((status = 200, description = "已完成", body = ScreeningSummary)))]
async fn run_screening(
    State(s): State<AppState>,
    Query(q): Query<RunQuery>,
) -> ApiResult<Json<ScreeningSummary>> {
    Ok(Json(
        screener::run_and_store(&s.pool, q.date, q.strategy_id).await?,
    ))
}
