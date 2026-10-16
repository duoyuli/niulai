//! 策略回测接口。回测不保存结果，每次按参数重新计算（单只股票全历史约几十毫秒）。

use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::market::windows;
use crate::analysis::backtest::{self, Costs, EquityPoint, Summary, Trade};
use crate::analysis::prices;
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(ma_cross_backtest))
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

/// 均线交叉策略回测：金叉后下一个交易日开盘满仓买入，死叉后下一个交易日开盘全部卖出
#[utoipa::path(get, path = "/backtest/ma-cross", tag = "backtest", params(BacktestQuery),
    responses((status = 200, description = "成功", body = BacktestResult),
              (status = 400, description = "参数不合法，或区间内没有日线", body = crate::error::ErrorBody),
              (status = 404, description = "证券不存在", body = crate::error::ErrorBody)))]
async fn ma_cross_backtest(State(s): State<AppState>, Query(q): Query<BacktestQuery>) -> ApiResult<Json<BacktestResult>> {
    let (fast, slow) = windows(q.fast, q.slow)?;
    let d = Costs::default();
    let costs = Costs {
        commission_rate: rate(q.commission_rate, d.commission_rate, "commission_rate")?,
        stamp_tax_rate: rate(q.stamp_tax_rate, d.stamp_tax_rate, "stamp_tax_rate")?,
        min_commission: q.min_commission.unwrap_or(d.min_commission),
    };
    if !(0.0..=100.0).contains(&costs.min_commission) {
        return Err(ApiError::BadRequest("min_commission 应在 0 到 100 之间".into()));
    }
    let capital = q.capital.unwrap_or(100_000.0);
    if !(1_000.0..=1e10).contains(&capital) {
        return Err(ApiError::BadRequest("capital 应在 1000 到 100 亿之间".into()));
    }
    if let (Some(a), Some(b)) = (q.start, q.end)
        && a > b
    {
        return Err(ApiError::BadRequest("start 不能晚于 end".into()));
    }
    let name: String = sqlx::query_scalar("SELECT name FROM ref.security WHERE thscode = $1")
        .bind(&q.thscode)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{}", q.thscode)))?;
    let bars = prices::stock_bars(&s.pool, &q.thscode, q.end).await?;
    let events = prices::corp_events(&s.pool, &q.thscode, q.end).await?;
    let params = backtest::Params { fast, slow, start: q.start, capital, costs };
    let out = backtest::run(&bars, &events, &params).ok_or_else(|| ApiError::BadRequest("所选区间内没有前复权日线".into()))?;
    Ok(Json(BacktestResult {
        thscode: q.thscode,
        name,
        fast: fast as i32,
        slow: slow as i32,
        costs: BacktestCosts { commission_rate: costs.commission_rate, min_commission: costs.min_commission, stamp_tax_rate: costs.stamp_tax_rate },
        summary: out.summary,
        trades: out.trades,
        equity: out.equity,
    }))
}
