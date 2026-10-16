//! 回测接口：均线交叉（单只股票）、选股策略（单只股票）、选股策略（全市场组合）。
//! 回测不保存结果，每次按参数重新计算。规则见 docs/domains/backtest.md。

use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::market::windows;
use crate::analysis::backtest::{self, Costs, EquityPoint, Risk, Signals, Summary, Trade};
use crate::analysis::indicators::Frame;
use crate::analysis::portfolio::{self, PortfolioOutcome};
use crate::analysis::strategy::Definition;
use crate::analysis::{market, prices};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(ma_cross_backtest)).routes(routes!(strategy_backtest)).routes(routes!(portfolio_backtest))
}

#[derive(Deserialize, IntoParams)]
pub struct BacktestQuery {
    /// 例：600519.SH
    pub thscode: String,
    /// 快线周期，默认 15
    pub fast: Option<i32>,
    /// 慢线周期，默认 30
    pub slow: Option<i32>,
    /// 回测开始日，默认最早一根日线
    pub start: Option<NaiveDate>,
    /// 回测结束日，默认最新一根日线
    pub end: Option<NaiveDate>,
    /// 初始资金（元），默认 100000
    pub capital: Option<f64>,
    /// 佣金费率，默认 0.00025（万 2.5），最大 0.01
    pub commission_rate: Option<f64>,
    /// 单笔最低佣金（元），默认 5
    pub min_commission: Option<f64>,
    /// 卖出印花税率，默认 0.0005，最大 0.01
    pub stamp_tax_rate: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct BacktestCosts {
    pub commission_rate: f64,
    pub min_commission: f64,
    pub stamp_tax_rate: f64,
}

#[derive(Serialize, ToSchema)]
pub struct BacktestResult {
    pub thscode: String,
    pub name: String,
    pub fast: i32,
    pub slow: i32,
    pub costs: BacktestCosts,
    pub summary: Summary,
    /// 按买入时间排序；最后一笔可能期末仍持有（exit_date 为空）
    pub trades: Vec<Trade>,
    /// 每个交易日一个点
    pub equity: Vec<EquityPoint>,
}

fn rate(v: Option<f64>, default: f64, name: &str) -> ApiResult<f64> {
    let v = v.unwrap_or(default);
    if !(0.0..=0.01).contains(&v) {
        return Err(ApiError::BadRequest(format!("{name} 应在 0 到 0.01 之间")));
    }
    Ok(v)
}

fn costs(commission_rate: Option<f64>, min_commission: Option<f64>, stamp_tax_rate: Option<f64>) -> ApiResult<Costs> {
    let d = Costs::default();
    let c = Costs {
        commission_rate: rate(commission_rate, d.commission_rate, "commission_rate")?,
        stamp_tax_rate: rate(stamp_tax_rate, d.stamp_tax_rate, "stamp_tax_rate")?,
        min_commission: min_commission.unwrap_or(d.min_commission),
    };
    if !(0.0..=100.0).contains(&c.min_commission) {
        return Err(ApiError::BadRequest("min_commission 应在 0 到 100 之间".into()));
    }
    Ok(c)
}

fn capital(v: Option<f64>) -> ApiResult<f64> {
    let c = v.unwrap_or(100_000.0);
    if !(1_000.0..=1e10).contains(&c) {
        return Err(ApiError::BadRequest("capital 应在 1000 到 100 亿之间".into()));
    }
    Ok(c)
}

fn costs_echo(c: &Costs) -> BacktestCosts {
    BacktestCosts { commission_rate: c.commission_rate, min_commission: c.min_commission, stamp_tax_rate: c.stamp_tax_rate }
}

fn check_range(start: Option<NaiveDate>, end: Option<NaiveDate>) -> ApiResult<()> {
    if let (Some(a), Some(b)) = (start, end)
        && a > b
    {
        return Err(ApiError::BadRequest("start 不能晚于 end".into()));
    }
    Ok(())
}

async fn security_name(s: &AppState, code: &str) -> ApiResult<String> {
    sqlx::query_scalar("SELECT name FROM ref.security WHERE thscode = $1")
        .bind(code)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))
}

/// 读取一个已保存的策略：(名称, 定义)
async fn strategy(s: &AppState, id: i64) -> ApiResult<(String, Definition)> {
    let row: Option<(String, serde_json::Value)> =
        sqlx::query_as("SELECT name, definition FROM app.strategy WHERE strategy_id = $1").bind(id).fetch_optional(&s.pool).await?;
    let (name, v) = row.ok_or_else(|| ApiError::NotFound(format!("策略不存在：{id}")))?;
    let d = serde_json::from_value(v).map_err(|e| ApiError::Internal(e.into()))?;
    Ok((name, d))
}

/// 均线交叉策略回测：金叉后下一个交易日开盘满仓买入，死叉后下一个交易日开盘全部卖出
#[utoipa::path(get, path = "/backtest/ma-cross", tag = "backtest", params(BacktestQuery),
    responses((status = 200, description = "成功", body = BacktestResult),
              (status = 400, description = "参数不合法，或区间内没有日线", body = crate::error::ErrorBody),
              (status = 404, description = "证券不存在", body = crate::error::ErrorBody)))]
async fn ma_cross_backtest(State(s): State<AppState>, Query(q): Query<BacktestQuery>) -> ApiResult<Json<BacktestResult>> {
    let (fast, slow) = windows(q.fast, q.slow)?;
    let costs = costs(q.commission_rate, q.min_commission, q.stamp_tax_rate)?;
    let capital = capital(q.capital)?;
    check_range(q.start, q.end)?;
    let name = security_name(&s, &q.thscode).await?;
    let bars = prices::stock_bars(&s.pool, &q.thscode, q.end).await?;
    let events = prices::corp_events(&s.pool, &q.thscode, q.end).await?;
    let (entry, exit) = backtest::ma_cross_signals(&bars, fast, slow);
    let params = backtest::Params { start: q.start, capital, costs, risk: Risk::default() };
    let out = backtest::run(&bars, &events, &Signals { entry: &entry, exit: &exit }, &params)
        .ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    Ok(Json(BacktestResult {
        thscode: q.thscode,
        name,
        fast: fast as i32,
        slow: slow as i32,
        costs: costs_echo(&costs),
        summary: out.summary,
        trades: out.trades,
        equity: out.equity,
    }))
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
}

#[derive(Serialize, ToSchema)]
pub struct StrategyBacktestResult {
    pub thscode: String,
    pub name: String,
    pub strategy_id: i64,
    pub strategy_name: String,
    pub costs: BacktestCosts,
    pub summary: Summary,
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
}

/// 用选股策略回测一只股票：满足策略的买入条件（含股票池）后下一个交易日开盘买入，满足卖出规则后卖出
#[utoipa::path(get, path = "/backtest/strategy", tag = "backtest", params(StrategyBacktestQuery),
    responses((status = 200, description = "成功", body = StrategyBacktestResult),
              (status = 400, description = "参数不合法，或区间内没有日线", body = crate::error::ErrorBody),
              (status = 404, description = "证券或策略不存在", body = crate::error::ErrorBody)))]
async fn strategy_backtest(State(s): State<AppState>, Query(q): Query<StrategyBacktestQuery>) -> ApiResult<Json<StrategyBacktestResult>> {
    let costs = costs(q.commission_rate, q.min_commission, q.stamp_tax_rate)?;
    let capital = capital(q.capital)?;
    check_range(q.start, q.end)?;
    let name = security_name(&s, &q.thscode).await?;
    let (strategy_name, d) = strategy(&s, q.strategy_id).await?;
    let sec = market::securities(&s.pool).await?;
    let meta = sec.meta.get(&q.thscode).ok_or_else(|| ApiError::NotFound(format!("证券不存在：{}", q.thscode)))?;
    let bars = prices::stock_bars(&s.pool, &q.thscode, q.end).await?;
    let events = prices::corp_events(&s.pool, &q.thscode, q.end).await?;
    let (entry, exit) = {
        let f = Frame::new(&bars, Some(meta));
        let entry: Vec<bool> = (0..bars.len()).map(|i| d.in_universe(&f, meta, i) && d.matches(&f, i)).collect();
        let exit: Vec<bool> = (0..bars.len()).map(|i| d.exit_signal(&f, i)).collect();
        (entry, exit)
    };
    let risk = Risk { stop_loss_pct: d.exit.stop_loss_pct, take_profit_pct: d.exit.take_profit_pct, max_hold_days: d.exit.max_hold_days };
    let params = backtest::Params { start: q.start, capital, costs, risk };
    let out = backtest::run(&bars, &events, &Signals { entry: &entry, exit: &exit }, &params)
        .ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    Ok(Json(StrategyBacktestResult {
        thscode: q.thscode,
        name,
        strategy_id: q.strategy_id,
        strategy_name,
        costs: costs_echo(&costs),
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
}

#[derive(Serialize, ToSchema)]
pub struct PortfolioResult {
    pub strategy_id: i64,
    pub strategy_name: String,
    pub max_positions: usize,
    pub costs: BacktestCosts,
    #[serde(flatten)]
    pub outcome: PortfolioOutcome,
}

/// 全市场组合回测：用选股策略每天选股，最多同时持有 max_positions 只，空余仓位平分现金。区间最长 5 年
#[utoipa::path(get, path = "/backtest/portfolio", tag = "backtest", params(PortfolioQuery),
    responses((status = 200, description = "成功", body = PortfolioResult),
              (status = 400, description = "参数不合法，或区间内没有日线", body = crate::error::ErrorBody),
              (status = 404, description = "策略不存在", body = crate::error::ErrorBody)))]
async fn portfolio_backtest(State(s): State<AppState>, Query(q): Query<PortfolioQuery>) -> ApiResult<Json<PortfolioResult>> {
    let costs = costs(q.commission_rate, q.min_commission, q.stamp_tax_rate)?;
    let capital = capital(q.capital)?;
    let max_positions = q.max_positions.unwrap_or(5);
    if !(1..=20).contains(&max_positions) {
        return Err(ApiError::BadRequest("max_positions 应在 1 到 20 之间".into()));
    }
    let (strategy_name, d) = strategy(&s, q.strategy_id).await?;
    let end = market::resolve_date(&s.pool, q.end).await?.ok_or_else(|| ApiError::BadRequest("本地还没有日线".into()))?;
    let start = q.start.unwrap_or(end - chrono::Duration::days(365 * 3));
    check_range(Some(start), Some(end))?;
    if (end - start).num_days() > 366 * portfolio::MAX_YEARS {
        return Err(ApiError::BadRequest(format!("组合回测的区间最长 {} 年", portfolio::MAX_YEARS)));
    }
    let p = portfolio::Params { start, end, capital, costs, max_positions };
    let outcome = portfolio::run(&s.pool, &d, &p).await?.ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    Ok(Json(PortfolioResult { strategy_id: q.strategy_id, strategy_name, max_positions, costs: costs_echo(&costs), outcome }))
}

