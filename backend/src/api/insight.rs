//! 市场环境接口：市场概况和板块强弱。计算在 `crate::analysis::market`，口径见 docs/domains/market_data_rules.md#market。

use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::analysis::limit_pool::CacheVersions;
use crate::analysis::market::{self, Overview, SectorStrength};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(market_overview))
        .routes(routes!(sectors))
}

#[derive(Deserialize, IntoParams)]
pub struct OverviewQuery {
    /// 交易日，默认最新
    pub date: Option<NaiveDate>,
}

/// 市场概况：涨跌家数、涨跌停数、站上 20 日均线的比例
#[utoipa::path(get, path = "/market/overview", tag = "market", params(OverviewQuery),
    responses((status = 200, description = "成功", body = Overview), (status = 404, description = "本地还没有日线", body = crate::error::ErrorBody)))]
async fn market_overview(
    State(s): State<AppState>,
    Query(q): Query<OverviewQuery>,
) -> ApiResult<Json<Overview>> {
    let Some(date) = market::resolve_date(&s.pool, q.date).await? else {
        return Err(ApiError::NotFound("本地还没有日线".into()));
    };
    let version = CacheVersions::load(&s.pool).await?;
    let key = super::derived_cache_key("market:overview", &version, &date.to_string());
    match s.cache.get_json(&key).await {
        Ok(Some(hit)) => {
            tracing::debug!(
                cache = "redis",
                endpoint = "market_overview",
                "命中共享缓存"
            );
            return Ok(Json(hit));
        }
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(error = %e, endpoint = "market_overview", "读取共享缓存失败，继续计算")
        }
    }
    let value = s
        .overview
        .get(&s.pool, Some(date))
        .await?
        .ok_or_else(|| ApiError::NotFound("本地还没有日线".into()))?;
    if let Err(e) = s.cache.set_json(&key, &value).await {
        tracing::debug!(error = %e, endpoint = "market_overview", "写入共享缓存失败，继续返回计算结果");
    }
    Ok(Json(value))
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
async fn sectors(
    State(s): State<AppState>,
    Query(q): Query<SectorQuery>,
) -> ApiResult<Json<Vec<SectorStrength>>> {
    let category = q.category.unwrap_or_else(|| "industry".into());
    if !["industry", "cn_concept"].contains(&category.as_str()) {
        return Err(ApiError::BadRequest(
            "category 只能是 industry 或 cn_concept".into(),
        ));
    }
    let days = q.days.unwrap_or(5);
    if !(1..=60).contains(&days) {
        return Err(ApiError::BadRequest("days 应在 1 到 60 之间".into()));
    }
    let Some(date) = market::resolve_date(&s.pool, q.date).await? else {
        return Ok(Json(vec![]));
    };
    let version = CacheVersions::load(&s.pool).await?;
    let key = super::derived_cache_key(
        "market:sectors",
        &version,
        &format!("{category}:{days}:{date}"),
    );
    match s.cache.get_json(&key).await {
        Ok(Some(hit)) => {
            tracing::debug!(cache = "redis", endpoint = "market_sectors", "命中共享缓存");
            return Ok(Json(hit));
        }
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(error = %e, endpoint = "market_sectors", "读取共享缓存失败，继续计算")
        }
    }
    let value = market::sector_strength(&s.pool, &category, days, date).await?;
    if let Err(e) = s.cache.set_json(&key, &value).await {
        tracing::debug!(error = %e, endpoint = "market_sectors", "写入共享缓存失败，继续返回计算结果");
    }
    Ok(Json(value))
}
