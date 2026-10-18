//! 个股诊断与市场环境接口。计算在 `crate::analysis`（diagnosis、market），规则见 docs/domains/strategies.md。

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::analysis::diagnosis::{self, Diagnosis};
use crate::analysis::market::{self, Overview, SectorStrength};
use crate::analysis::prices;
use crate::analysis::strategy::Definition;
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(security_diagnosis)).routes(routes!(market_overview)).routes(routes!(sectors))
}

/// 个股诊断：趋势、动量、量能、位置各一句结论，最近 20 个交易日的信号，关键价位，当天满足的策略
#[utoipa::path(get, path = "/securities/{thscode}/diagnosis", tag = "market", params(("thscode" = String, Path, description = "例：600519.SH")),
    responses((status = 200, description = "成功", body = Diagnosis), (status = 404, description = "证券不存在或没有日线", body = crate::error::ErrorBody)))]
async fn security_diagnosis(State(s): State<AppState>, Path(code): Path<String>) -> ApiResult<Json<Diagnosis>> {
    let sec = market::securities(&s.pool).await?;
    let meta = sec.meta.get(&code).ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))?;
    let bars = prices::recent_bars(&s.pool, &code, diagnosis::BARS_NEEDED).await?;
    let rows: Vec<(i64, String, serde_json::Value)> =
        sqlx::query_as("SELECT strategy_id, name, definition FROM app.strategy ORDER BY strategy_id").fetch_all(&s.pool).await?;
    let lib = crate::analysis::signal_lib::load(&s.pool).await?;
    let strategies: Vec<(i64, String, Definition)> = rows
        .into_iter()
        .filter_map(|(id, name, v)| serde_json::from_value::<Definition>(v).ok().map(|d| (id, name, d.with_lib(lib.clone()))))
        .collect();
    diagnosis::diagnose(&bars, meta, &strategies).map(Json).ok_or_else(|| ApiError::NotFound(format!("本地还没有 {code} 的日线")))
}

#[derive(Deserialize, IntoParams)]
pub struct OverviewQuery {
    /// 交易日，默认最新
    pub date: Option<NaiveDate>,
}

/// 市场概况：涨跌家数、涨跌停数、站上 20 日均线的比例
#[utoipa::path(get, path = "/market/overview", tag = "market", params(OverviewQuery),
    responses((status = 200, description = "成功", body = Overview), (status = 404, description = "本地还没有日线", body = crate::error::ErrorBody)))]
async fn market_overview(State(s): State<AppState>, Query(q): Query<OverviewQuery>) -> ApiResult<Json<Overview>> {
    s.overview.get(&s.pool, q.date).await?.map(Json).ok_or_else(|| ApiError::NotFound("本地还没有日线".into()))
}

#[derive(Deserialize, IntoParams)]
pub struct SectorQuery {
    /// industry 同花顺行业（默认）、cn_concept 同花顺概念
    pub category: Option<String>,
    /// 按最近多少个交易日的涨跌幅排名，默认 5，范围 1 到 60
    pub days: Option<u32>,
    pub date: Option<NaiveDate>,
}

/// 板块强弱：行业或概念按最近 N 个交易日涨跌幅排名。板块指数日线从 2022 年起
#[utoipa::path(get, path = "/market/sectors", tag = "market", params(SectorQuery),
    responses((status = 200, description = "成功", body = [SectorStrength]), (status = 400, description = "参数不合法", body = crate::error::ErrorBody)))]
async fn sectors(State(s): State<AppState>, Query(q): Query<SectorQuery>) -> ApiResult<Json<Vec<SectorStrength>>> {
    let category = q.category.unwrap_or_else(|| "industry".into());
    if !["industry", "cn_concept"].contains(&category.as_str()) {
        return Err(ApiError::BadRequest("category 只能是 industry 或 cn_concept".into()));
    }
    let days = q.days.unwrap_or(5);
    if !(1..=60).contains(&days) {
        return Err(ApiError::BadRequest("days 应在 1 到 60 之间".into()));
    }
    let Some(date) = market::resolve_date(&s.pool, q.date).await? else { return Ok(Json(vec![])) };
    Ok(Json(market::sector_strength(&s.pool, &category, days, date).await?))
}
