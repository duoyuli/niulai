//! 回测接口：选股策略（单只股票）、选股策略（全市场组合）、滚动样本外验证和运行记录。
//! 每次按参数重新计算，不缓存结果；汇总和运行清单写入 `ops.backtest_run`。规则见 docs/domains/backtest.md。

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::auth::CurrentUser;
use crate::analysis::backtest::{self, Costs, EquityPoint, LotRule, Risk, Signals, Summary, Trade};
use crate::analysis::execution::DealPrice;
use crate::analysis::indicators::Frame;
use crate::analysis::limit_pool::LimitPool;
use crate::analysis::portfolio::{self, PortfolioOutcome};
use crate::analysis::research::{self, BacktestManifest, Coverage};
use crate::analysis::strategy::Definition;
use crate::analysis::{market, prices};
use crate::auth::Role;
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(strategy_backtest))
        .routes(routes!(portfolio_backtest))
        .routes(routes!(backtest_runs))
        .routes(routes!(validation_windows))
        .routes(routes!(strategy_validation))
}

#[derive(Deserialize, IntoParams)]
pub struct ValidationWindowQuery {
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub train_days: i64,
    pub test_days: i64,
    pub embargo_days: Option<i64>,
}

/// 生成滚动样本外验证窗口；窗口只描述日期，不自动挑选最优参数。
#[utoipa::path(get, path = "/backtest/validation-windows", tag = "backtest", params(ValidationWindowQuery),
    responses((status = 200, description = "成功", body = [crate::analysis::research::ValidationWindow]),
              (status = 400, description = "日期或窗口参数不合法", body = crate::error::ErrorBody)))]
async fn validation_windows(
    Query(q): Query<ValidationWindowQuery>,
) -> ApiResult<Json<Vec<crate::analysis::research::ValidationWindow>>> {
    let windows = crate::analysis::research::walk_forward_windows(
        q.start,
        q.end,
        q.train_days,
        q.test_days,
        q.embargo_days.unwrap_or(5),
    )
    .map_err(ApiError::BadRequest)?;
    Ok(Json(windows))
}

#[derive(Deserialize, IntoParams)]
pub struct StrategyValidationQuery {
    pub strategy_id: i64,
    pub thscode: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub train_days: i64,
    pub test_days: i64,
    pub embargo_days: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct ValidationWindowResult {
    pub window: crate::analysis::research::ValidationWindow,
    pub train_return: f64,
    pub test_return: f64,
    pub train_risk: crate::analysis::research::RiskMetrics,
    pub test_risk: crate::analysis::research::RiskMetrics,
}

#[derive(Serialize, ToSchema)]
pub struct StrategyValidationResult {
    pub thscode: String,
    pub strategy_id: i64,
    pub manifest: BacktestManifest,
    pub windows: Vec<ValidationWindowResult>,
    /// 多窗口的选择偏差近似；不是完整的 DSR/PBO 统计检验。
    pub robustness: ValidationRobustness,
}

#[derive(Serialize, ToSchema)]
pub struct ValidationRobustness {
    pub deflated_sharpe_proxy: Option<f64>,
    pub pbo_proxy: Option<f64>,
}

#[allow(clippy::too_many_arguments)]
async fn run_single_segment(
    bars: &[crate::analysis::prices::DailyBar],
    events: &[crate::analysis::prices::CorpEvent],
    limits: &LimitPool,
    meta: &crate::analysis::indicators::StockMeta,
    history: &[crate::analysis::indicators::StPeriod],
    code: &str,
    strategy: &Definition,
    costs: Costs,
    start: NaiveDate,
    end: NaiveDate,
) -> ApiResult<Summary> {
    let n = bars.partition_point(|b| b.date <= end);
    let bars = &bars[..n];
    let events = &events[..events.partition_point(|e| e.ex_date <= end)];
    let f = Frame::with_authority_and_st_history(bars, Some(meta), limits, code, history);
    let entry: Vec<bool> = (0..bars.len())
        .map(|i| strategy.in_universe(&f, meta, i) && strategy.matches(&f, i))
        .collect();
    let exit: Vec<bool> = (0..bars.len())
        .map(|i| strategy.exit_signal(&f, i))
        .collect();
    let (min, step) = crate::instrument::extension(meta.asset_type).lot_rule(&meta.board);
    backtest::run(
        bars,
        events,
        &Signals {
            entry: &entry,
            exit: &exit,
        },
        &backtest::Params {
            start: Some(start),
            capital: 100_000.0,
            costs,
            risk: Risk {
                stop_loss_pct: strategy.exit.stop_loss_pct,
                take_profit_pct: strategy.exit.take_profit_pct,
                trailing_stop_atr: strategy.exit.trailing_stop_atr,
                max_hold_days: strategy.exit.max_hold_days,
            },
            lot: LotRule { min, step },
        },
    )
    .map(|o| o.summary)
    .ok_or_else(|| ApiError::BadRequest("验证窗口没有足够日线".into()))
}

/// 对单只证券执行滚动训练/样本外验证；测试窗口只使用测试日以前的数据。
#[utoipa::path(get, path = "/backtest/strategy-validation", tag = "backtest", params(StrategyValidationQuery),
    responses((status = 200, description = "成功", body = StrategyValidationResult),
              (status = 400, description = "参数不合法或历史不足", body = crate::error::ErrorBody),
              (status = 404, description = "证券或策略不存在", body = crate::error::ErrorBody)))]
async fn strategy_validation(
    State(s): State<AppState>,
    Query(q): Query<StrategyValidationQuery>,
) -> ApiResult<Json<StrategyValidationResult>> {
    let costs = Costs::default();
    let (_, strategy) = strategy(&s, q.strategy_id).await?;
    let meta = market::securities(&s.pool)
        .await?
        .meta
        .remove(&q.thscode)
        .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{}", q.thscode)))?;
    let windows = crate::analysis::research::walk_forward_windows(
        q.start,
        q.end,
        q.train_days,
        q.test_days,
        q.embargo_days.unwrap_or(5),
    )
    .map_err(ApiError::BadRequest)?;
    if windows.is_empty() {
        return Err(ApiError::BadRequest("没有可用的训练/测试窗口".into()));
    }
    let bars = prices::instrument_bars(&s.pool, &q.thscode, Some(q.end)).await?;
    let events = prices::corp_events(&s.pool, &q.thscode, Some(q.end)).await?;
    let limits =
        LimitPool::load_for(&s.pool, std::slice::from_ref(&q.thscode), q.start, q.end).await?;
    let sec = market::securities(&s.pool).await?;
    let history = sec
        .st_history
        .get(&q.thscode)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut results = Vec::with_capacity(windows.len());
    for window in windows {
        let train = run_single_segment(
            &bars,
            &events,
            &limits,
            &meta,
            history,
            &q.thscode,
            &strategy,
            costs,
            window.train_start,
            window.train_end,
        )
        .await?;
        let test = run_single_segment(
            &bars,
            &events,
            &limits,
            &meta,
            history,
            &q.thscode,
            &strategy,
            costs,
            window.test_start,
            window.test_end,
        )
        .await?;
        results.push(ValidationWindowResult {
            window,
            train_return: train.total_return,
            test_return: test.total_return,
            train_risk: train.risk,
            test_risk: test.risk,
        });
    }
    let mut run_manifest = manifest(&s.pool, &strategy, q.start, q.end, &costs).await?;
    run_manifest.validation_kind = "walk_forward_embargoed".into();
    run_manifest.trial_count = 1;
    run_manifest.coverage.instruments = 1;
    run_manifest.coverage.observed_bars = bars.len() as u64;
    let pbo_proxy = (results.len() >= 2).then(|| {
        results
            .iter()
            .filter(|r| r.test_return < r.train_return)
            .count() as f64
            / results.len() as f64
    });
    let deflated_sharpe_proxy = if results.len() >= 2 {
        let sharpes: Vec<f64> = results.iter().filter_map(|r| r.test_risk.sharpe).collect();
        if sharpes.is_empty() {
            None
        } else {
            let mean = sharpes.iter().sum::<f64>() / sharpes.len() as f64;
            Some(mean - (2.0 * (results.len() as f64).ln()).sqrt())
        }
    } else {
        None
    };
    let result = StrategyValidationResult {
        thscode: q.thscode,
        strategy_id: q.strategy_id,
        manifest: run_manifest,
        windows: results,
        robustness: ValidationRobustness {
            deflated_sharpe_proxy,
            pbo_proxy,
        },
    };
    record_run(
        &s.pool,
        q.strategy_id,
        "single",
        q.start,
        q.end,
        &result.manifest,
        &result.windows,
    )
    .await?;
    Ok(Json(result))
}

#[derive(Serialize, ToSchema)]
pub struct BacktestRunBrief {
    pub run_id: i64,
    pub strategy_id: Option<i64>,
    pub account_id: Option<i64>,
    pub mode: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub definition_hash: String,
    pub manifest: BacktestManifest,
    pub summary: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

type BacktestRunRow = (
    i64,
    Option<i64>,
    Option<i64>,
    String,
    NaiveDate,
    NaiveDate,
    String,
    serde_json::Value,
    serde_json::Value,
    chrono::DateTime<chrono::Utc>,
);

#[derive(Deserialize, IntoParams)]
pub struct BacktestRunsQuery {
    pub strategy_id: Option<i64>,
    pub limit: Option<i64>,
}

/// 已保存的回测运行清单，供复现实验和比较不同执行模型。
#[utoipa::path(get, path = "/backtest/runs", tag = "backtest", params(BacktestRunsQuery),
    responses((status = 200, description = "成功", body = [BacktestRunBrief])))]
async fn backtest_runs(
    State(s): State<AppState>,
    me: CurrentUser,
    Query(q): Query<BacktestRunsQuery>,
) -> ApiResult<Json<Vec<BacktestRunBrief>>> {
    let rows: Vec<BacktestRunRow> = sqlx::query_as(
        "SELECT r.run_id, r.strategy_id, r.account_id, r.mode, r.start_date, r.end_date,
                r.definition_hash, r.manifest, r.summary, r.created_at
         FROM ops.backtest_run r
         WHERE ($1::bigint IS NULL OR r.strategy_id = $1)
           AND (
             r.account_id IS NULL
             OR $2
             OR EXISTS (
               SELECT 1 FROM app.account a
               WHERE a.account_id = r.account_id AND a.user_id = $3
             )
           )
         ORDER BY r.created_at DESC LIMIT $4",
    )
    .bind(q.strategy_id)
    .bind(me.role == Role::Admin)
    .bind(me.user_id)
    .bind(q.limit.unwrap_or(50).clamp(1, 500))
    .fetch_all(&s.pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(BacktestRunBrief {
                run_id: r.0,
                strategy_id: r.1,
                account_id: r.2,
                mode: r.3,
                start_date: r.4,
                end_date: r.5,
                definition_hash: r.6,
                manifest: serde_json::from_value(r.7).map_err(|e| ApiError::Internal(e.into()))?,
                summary: r.8,
                created_at: r.9,
            })
        })
        .collect::<ApiResult<Vec<_>>>()
        .map(Json)
}

#[derive(Serialize, ToSchema)]
pub struct BacktestCosts {
    pub commission_rate: f64,
    pub min_commission: f64,
    pub stamp_tax_rate: f64,
    pub transfer_fee_rate: f64,
    pub dividend_tax_rate: f64,
    pub cash_interest_rate: f64,
    pub deal_price: DealPrice,
    pub slippage_bps: f64,
    pub participation_rate: Option<f64>,
}

fn rate(v: Option<f64>, default: f64, name: &str) -> ApiResult<f64> {
    let v = v.unwrap_or(default);
    if !(0.0..=0.01).contains(&v) {
        return Err(ApiError::BadRequest(format!("{name} 应在 0 到 0.01 之间")));
    }
    Ok(v)
}

pub(super) struct CostOptions {
    pub commission_rate: Option<f64>,
    pub min_commission: Option<f64>,
    pub stamp_tax_rate: Option<f64>,
    pub transfer_fee_rate: Option<f64>,
    pub dividend_tax_rate: Option<f64>,
    pub cash_interest_rate: Option<f64>,
    pub deal_price: Option<DealPrice>,
    pub slippage_bps: Option<f64>,
    pub participation_rate: Option<f64>,
}

pub(super) fn costs(o: CostOptions) -> ApiResult<Costs> {
    let d = Costs::default();
    let c = Costs {
        commission_rate: rate(o.commission_rate, d.commission_rate, "commission_rate")?,
        stamp_tax_rate: rate(o.stamp_tax_rate, d.stamp_tax_rate, "stamp_tax_rate")?,
        min_commission: o.min_commission.unwrap_or(d.min_commission),
        transfer_fee_rate: rate(
            o.transfer_fee_rate,
            d.transfer_fee_rate,
            "transfer_fee_rate",
        )?,
        dividend_tax_rate: rate(
            o.dividend_tax_rate,
            d.dividend_tax_rate,
            "dividend_tax_rate",
        )?,
        cash_interest_rate: rate(
            o.cash_interest_rate,
            d.cash_interest_rate,
            "cash_interest_rate",
        )?,
        deal_price: o.deal_price.unwrap_or(d.deal_price),
        slippage_bps: o.slippage_bps.unwrap_or(d.slippage_bps),
        participation_rate: o.participation_rate,
    };
    if !(0.0..=100.0).contains(&c.min_commission) {
        return Err(ApiError::BadRequest(
            "min_commission 应在 0 到 100 之间".into(),
        ));
    }
    c.execution().validate().map_err(ApiError::BadRequest)?;
    Ok(c)
}

pub(super) fn capital(v: Option<f64>) -> ApiResult<f64> {
    let c = v.unwrap_or(100_000.0);
    if !(1_000.0..=1e10).contains(&c) {
        return Err(ApiError::BadRequest(
            "capital 应在 1000 到 100 亿之间".into(),
        ));
    }
    Ok(c)
}

pub(super) fn costs_echo(c: &Costs) -> BacktestCosts {
    BacktestCosts {
        commission_rate: c.commission_rate,
        min_commission: c.min_commission,
        stamp_tax_rate: c.stamp_tax_rate,
        transfer_fee_rate: c.transfer_fee_rate,
        dividend_tax_rate: c.dividend_tax_rate,
        cash_interest_rate: c.cash_interest_rate,
        deal_price: c.deal_price,
        slippage_bps: c.slippage_bps,
        participation_rate: c.participation_rate,
    }
}

pub(super) async fn manifest(
    pool: &sqlx::PgPool,
    strategy: &impl Serialize,
    start: NaiveDate,
    end: NaiveDate,
    costs: &Costs,
) -> ApiResult<BacktestManifest> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, updated_at::text FROM ops.derived_version ORDER BY name")
            .fetch_all(pool)
            .await?;
    Ok(BacktestManifest {
        strategy_definition_hash: research::definition_hash(strategy),
        data_start: start,
        data_end: end,
        price_basis: "signal_forward_adjusted_execution_raw".into(),
        execution_model: format!(
            "{:?};slippage_bps={};participation_rate={:?}",
            costs.deal_price, costs.slippage_bps, costs.participation_rate
        ),
        point_in_time_universe: true,
        derived_versions: rows.into_iter().collect::<BTreeMap<_, _>>(),
        trial_count: 1,
        validation_kind: "single_run_unvalidated".into(),
        coverage: Coverage::default(),
        delisted_policy: "exclude_after_point_in_time_delisted_on;unknown_st_fallback_current"
            .into(),
    })
}

pub(super) async fn coverage(
    pool: &sqlx::PgPool,
    start: NaiveDate,
    end: NaiveDate,
    include_stock: bool,
    include_etf: bool,
) -> ApiResult<Coverage> {
    let (instruments, observed_bars): (i64, i64) = match (include_stock, include_etf) {
        (true, true) => sqlx::query_as(
            "SELECT count(DISTINCT thscode), count(*) FROM (
                SELECT thscode, trade_date FROM mkt.stock_bar_1d WHERE trade_date BETWEEN $1 AND $2
                UNION ALL
                SELECT thscode, trade_date FROM mkt.etf_bar_1d WHERE trade_date BETWEEN $1 AND $2
             ) x",
        )
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await?,
        (true, false) => sqlx::query_as(
            "SELECT count(DISTINCT thscode), count(*) FROM mkt.stock_bar_1d WHERE trade_date BETWEEN $1 AND $2",
        )
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await?,
        (false, true) => sqlx::query_as(
            "SELECT count(DISTINCT thscode), count(*) FROM mkt.etf_bar_1d WHERE trade_date BETWEEN $1 AND $2",
        )
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await?,
        (false, false) => (0, 0),
    };
    let mut asset_types = Vec::new();
    if include_stock {
        asset_types.push("stock".to_string());
    }
    if include_etf {
        asset_types.push("etf".to_string());
    }
    let expected: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ref.instrument WHERE asset_type = ANY($1)")
            .bind(&asset_types)
            .fetch_one(pool)
            .await?;
    let observed_instruments = instruments.max(0) as u64;
    Ok(Coverage {
        instruments: expected.max(0) as u64,
        observed_bars: observed_bars.max(0) as u64,
        missing_instruments: expected.max(0) as u64
            - observed_instruments.min(expected.max(0) as u64),
        unknown_state_instruments: 0,
    })
}

async fn record_run(
    pool: &sqlx::PgPool,
    strategy_id: i64,
    mode: &str,
    start: NaiveDate,
    end: NaiveDate,
    manifest: &BacktestManifest,
    summary: &impl serde::Serialize,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO ops.backtest_run
         (strategy_id, mode, start_date, end_date, definition_hash, manifest, summary)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(strategy_id)
    .bind(mode)
    .bind(start)
    .bind(end)
    .bind(&manifest.strategy_definition_hash)
    .bind(serde_json::to_value(manifest).map_err(|e| ApiError::Internal(e.into()))?)
    .bind(serde_json::to_value(summary).map_err(|e| ApiError::Internal(e.into()))?)
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) fn check_range(start: Option<NaiveDate>, end: Option<NaiveDate>) -> ApiResult<()> {
    if let (Some(a), Some(b)) = (start, end)
        && a > b
    {
        return Err(ApiError::BadRequest("start 不能晚于 end".into()));
    }
    Ok(())
}

async fn security_name(s: &AppState, code: &str) -> ApiResult<String> {
    sqlx::query_scalar("SELECT name FROM ref.instrument WHERE thscode = $1")
        .bind(code)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))
}

/// 读取一个已保存的策略：(名称, 定义)
async fn strategy(s: &AppState, id: i64) -> ApiResult<(String, Definition)> {
    let row: Option<(String, serde_json::Value)> =
        sqlx::query_as("SELECT name, definition FROM app.strategy WHERE strategy_id = $1")
            .bind(id)
            .fetch_optional(&s.pool)
            .await?;
    let (name, v) = row.ok_or_else(|| ApiError::NotFound(format!("策略不存在：{id}")))?;
    let d: Definition = serde_json::from_value(v).map_err(|e| ApiError::Internal(e.into()))?;
    Ok((
        name,
        d.with_lib(crate::analysis::signal_lib::load(&s.pool).await?),
    ))
}

#[derive(Deserialize, IntoParams)]
pub struct StrategyBacktestQuery {
    pub strategy_id: i64,
    /// 例：600519.SH
    pub thscode: String,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub capital: Option<f64>,
    pub commission_rate: Option<f64>,
    pub min_commission: Option<f64>,
    pub stamp_tax_rate: Option<f64>,
    pub transfer_fee_rate: Option<f64>,
    pub dividend_tax_rate: Option<f64>,
    pub cash_interest_rate: Option<f64>,
    pub deal_price: Option<DealPrice>,
    pub slippage_bps: Option<f64>,
    pub participation_rate: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct StrategyBacktestResult {
    pub thscode: String,
    pub name: String,
    pub strategy_id: i64,
    pub strategy_name: String,
    pub costs: BacktestCosts,
    pub manifest: BacktestManifest,
    pub summary: Summary,
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
}

/// 用选股策略回测一只股票：满足策略的买入条件（含股票池）后下一个交易日开盘买入，满足卖出规则后卖出
#[utoipa::path(get, path = "/backtest/strategy", tag = "backtest", params(StrategyBacktestQuery),
    responses((status = 200, description = "成功", body = StrategyBacktestResult),
              (status = 400, description = "参数不合法，或区间内没有日线", body = crate::error::ErrorBody),
              (status = 404, description = "证券或策略不存在", body = crate::error::ErrorBody)))]
async fn strategy_backtest(
    State(s): State<AppState>,
    Query(q): Query<StrategyBacktestQuery>,
) -> ApiResult<Json<StrategyBacktestResult>> {
    let costs = costs(CostOptions {
        commission_rate: q.commission_rate,
        min_commission: q.min_commission,
        stamp_tax_rate: q.stamp_tax_rate,
        transfer_fee_rate: q.transfer_fee_rate,
        dividend_tax_rate: q.dividend_tax_rate,
        cash_interest_rate: q.cash_interest_rate,
        deal_price: q.deal_price,
        slippage_bps: q.slippage_bps,
        participation_rate: q.participation_rate,
    })?;
    let capital = capital(q.capital)?;
    check_range(q.start, q.end)?;
    let name = security_name(&s, &q.thscode).await?;
    let asset_type: String =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(&q.thscode)
            .fetch_one(&s.pool)
            .await?;
    let costs = if asset_type == "etf" {
        Costs {
            stamp_tax_rate: 0.0,
            ..costs
        }
    } else {
        costs
    };
    let (strategy_name, d) = strategy(&s, q.strategy_id).await?;
    let costs = if d.universe.asset_types.len() == 1 && d.universe.asset_types[0] == "etf" {
        Costs {
            stamp_tax_rate: 0.0,
            ..costs
        }
    } else {
        costs
    };
    let sec = market::securities(&s.pool).await?;
    let meta = sec
        .meta
        .get(&q.thscode)
        .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{}", q.thscode)))?;
    let bars = prices::instrument_bars(&s.pool, &q.thscode, q.end).await?;
    let events = prices::corp_events(&s.pool, &q.thscode, q.end).await?;
    // 权威涨跌停池：策略里的涨跌停、炸板、连板信号以供应商的名单为准；一字板能否成交的判断不变
    let codes = [q.thscode.clone()];
    let limits = match (bars.first(), bars.last()) {
        (Some(first), Some(last)) => {
            LimitPool::load_for(&s.pool, &codes, first.date, last.date).await?
        }
        _ => LimitPool::empty(),
    };
    let (entry, exit) = {
        let history = sec
            .st_history
            .get(&q.thscode)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let f =
            Frame::with_authority_and_st_history(&bars, Some(meta), &limits, &q.thscode, history);
        let entry: Vec<bool> = (0..bars.len())
            .map(|i| d.in_universe(&f, meta, i) && d.matches(&f, i))
            .collect();
        let exit: Vec<bool> = (0..bars.len()).map(|i| d.exit_signal(&f, i)).collect();
        (entry, exit)
    };
    let risk = Risk {
        stop_loss_pct: d.exit.stop_loss_pct,
        take_profit_pct: d.exit.take_profit_pct,
        trailing_stop_atr: d.exit.trailing_stop_atr,
        max_hold_days: d.exit.max_hold_days,
    };
    let params = backtest::Params {
        start: q.start,
        capital,
        costs,
        risk,
        lot: {
            let (min, step) = crate::instrument::extension(meta.asset_type).lot_rule(&meta.board);
            LotRule { min, step }
        },
    };
    let out = backtest::run(
        &bars,
        &events,
        &Signals {
            entry: &entry,
            exit: &exit,
        },
        &params,
    )
    .ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    let start = out.summary.start;
    let end = out.summary.end;
    let mut run_manifest = manifest(&s.pool, &d, start, end, &costs).await?;
    run_manifest.coverage.instruments = 1;
    run_manifest.coverage.observed_bars = bars.len() as u64;
    record_run(
        &s.pool,
        q.strategy_id,
        "single",
        start,
        end,
        &run_manifest,
        &out.summary,
    )
    .await?;
    Ok(Json(StrategyBacktestResult {
        thscode: q.thscode,
        name,
        strategy_id: q.strategy_id,
        strategy_name,
        costs: costs_echo(&costs),
        manifest: run_manifest,
        summary: out.summary,
        trades: out.trades,
        equity: out.equity,
    }))
}

#[derive(Deserialize, IntoParams)]
pub struct PortfolioQuery {
    pub strategy_id: i64,
    /// 开始日，默认结束日之前 3 年
    pub start: Option<NaiveDate>,
    /// 结束日，默认本地最新交易日
    pub end: Option<NaiveDate>,
    /// 最多同时持有几只，默认 5，范围 1 到 20
    pub max_positions: Option<usize>,
    pub capital: Option<f64>,
    pub commission_rate: Option<f64>,
    pub min_commission: Option<f64>,
    pub stamp_tax_rate: Option<f64>,
    pub transfer_fee_rate: Option<f64>,
    pub dividend_tax_rate: Option<f64>,
    pub cash_interest_rate: Option<f64>,
    pub deal_price: Option<DealPrice>,
    pub slippage_bps: Option<f64>,
    pub participation_rate: Option<f64>,
    pub max_weight_pct: Option<f64>,
    pub target_vol_pct: Option<f64>,
    pub max_turnover_pct: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct PortfolioResult {
    pub strategy_id: i64,
    pub strategy_name: String,
    pub max_positions: usize,
    pub costs: BacktestCosts,
    pub manifest: BacktestManifest,
    #[serde(flatten)]
    pub outcome: PortfolioOutcome,
}

/// 全市场组合回测：用选股策略每天选股，最多同时持有 max_positions 只，空余仓位平分现金。区间最长 5 年
#[utoipa::path(get, path = "/backtest/portfolio", tag = "backtest", params(PortfolioQuery),
    responses((status = 200, description = "成功", body = PortfolioResult),
              (status = 400, description = "参数不合法，或区间内没有日线", body = crate::error::ErrorBody),
              (status = 404, description = "策略不存在", body = crate::error::ErrorBody)))]
async fn portfolio_backtest(
    State(s): State<AppState>,
    Query(q): Query<PortfolioQuery>,
) -> ApiResult<Json<PortfolioResult>> {
    let costs = costs(CostOptions {
        commission_rate: q.commission_rate,
        min_commission: q.min_commission,
        stamp_tax_rate: q.stamp_tax_rate,
        transfer_fee_rate: q.transfer_fee_rate,
        dividend_tax_rate: q.dividend_tax_rate,
        cash_interest_rate: q.cash_interest_rate,
        deal_price: q.deal_price,
        slippage_bps: q.slippage_bps,
        participation_rate: q.participation_rate,
    })?;
    let capital = capital(q.capital)?;
    let max_positions = q.max_positions.unwrap_or(5);
    if !(1..=20).contains(&max_positions) {
        return Err(ApiError::BadRequest(
            "max_positions 应在 1 到 20 之间".into(),
        ));
    }
    for (name, value) in [
        ("max_weight_pct", q.max_weight_pct),
        ("target_vol_pct", q.target_vol_pct),
        ("max_turnover_pct", q.max_turnover_pct),
    ] {
        if let Some(v) = value
            && (!v.is_finite() || !(0.0..=100.0).contains(&v))
        {
            return Err(ApiError::BadRequest(format!("{name} 应在 0 到 100 之间")));
        }
    }
    let (strategy_name, d) = strategy(&s, q.strategy_id).await?;
    let end = market::resolve_date(&s.pool, q.end)
        .await?
        .ok_or_else(|| ApiError::BadRequest("本地还没有日线".into()))?;
    let start = q.start.unwrap_or(end - chrono::Duration::days(365 * 3));
    check_range(Some(start), Some(end))?;
    if (end - start).num_days() > 366 * portfolio::MAX_YEARS {
        return Err(ApiError::BadRequest(format!(
            "组合回测的区间最长 {} 年",
            portfolio::MAX_YEARS
        )));
    }
    let p = portfolio::Params {
        start,
        end,
        capital,
        costs,
        max_positions,
        risk: portfolio::PortfolioRisk {
            max_weight_pct: q.max_weight_pct,
            target_vol_pct: q.target_vol_pct,
            max_turnover_pct: q.max_turnover_pct,
        },
    };
    let outcome = portfolio::run(&s.pool, &d, &p)
        .await?
        .ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    let mut run_manifest = manifest(&s.pool, &d, start, end, &costs).await?;
    run_manifest.coverage = coverage(
        &s.pool,
        outcome.summary.start,
        outcome.summary.end,
        d.universe.asset_types.iter().any(|a| a == "stock"),
        d.universe.asset_types.iter().any(|a| a == "etf"),
    )
    .await?;
    record_run(
        &s.pool,
        q.strategy_id,
        "portfolio",
        outcome.summary.start,
        outcome.summary.end,
        &run_manifest,
        &outcome.summary,
    )
    .await?;
    Ok(Json(PortfolioResult {
        strategy_id: q.strategy_id,
        strategy_name,
        max_positions,
        costs: costs_echo(&costs),
        manifest: run_manifest,
        outcome,
    }))
}
