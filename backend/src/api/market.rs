//! 行情接口：证券、日线（含复权）、均线与交叉信号、指数。
//!
//! 复权口径（见数据方案 §3.1）：
//! - `qfq` / `hfq`：等比前复权 / 等比后复权，计算指标的标准口径
//! - `qfq_ths` / `hfq_ths`：同花顺减法复权，只用于和同花顺软件对数
//! - `none`：未复权原价
//!
//! 均线和交叉信号一律用等比前复权计算；因为等比复权的前、后复权只差一个常数倍，信号与基准日无关。
//! 计算本身在 `crate::analysis` 中，这里只做参数校验和响应格式。

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sqlx::AssertSqlSafe;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::analysis::ma;
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_securities))
        .routes(routes!(get_security))
        .routes(routes!(security_bars))
        .routes(routes!(list_indices))
        .routes(routes!(index_bars))
        .routes(routes!(index_members))
}

// ───────────────────────── 证券 ─────────────────────────

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct Security {
    pub thscode: String,
    pub ticker: String,
    pub name: String,
    pub exchange: String,
    /// main / chinext / star / bse
    pub board: String,
    pub list_date: Option<NaiveDate>,
    pub delisted_on: Option<NaiveDate>,
    pub last_seen: NaiveDate,
}

#[derive(Deserialize, IntoParams)]
pub struct SecurityQuery {
    /// 代码或名称片段
    pub q: Option<String>,
    /// main / chinext / star / bse
    pub board: Option<String>,
    /// 默认 50，最大 500
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct Page<T> {
    pub total: i64,
    pub items: Vec<T>,
}

/// 证券列表
#[utoipa::path(get, path = "/securities", tag = "market", params(SecurityQuery), responses((status = 200, description = "成功", body = Page<Security>)))]
async fn list_securities(State(s): State<AppState>, Query(q): Query<SecurityQuery>) -> ApiResult<Json<Page<Security>>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let pattern = q.q.as_deref().map(|t| format!("%{}%", t.trim()));
    let filter = "($1::text IS NULL OR thscode ILIKE $1 OR name ILIKE $1) AND ($2::text IS NULL OR board = $2)";
    // 审计：filter 是固定的 SQL 片段，用户输入全部走绑定参数
    let total: i64 = sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM ref.security WHERE {filter}")))
        .bind(&pattern)
        .bind(&q.board)
        .fetch_one(&s.pool)
        .await?;
    let items = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT thscode, ticker, name, exchange, board, list_date, delisted_on, last_seen FROM ref.security WHERE {filter} ORDER BY thscode LIMIT $3 OFFSET $4"
    )))
    .bind(&pattern)
    .bind(&q.board)
    .bind(limit)
    .bind(q.offset.unwrap_or(0).max(0))
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(Page { total, items }))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct NameChange {
    pub name: String,
    pub valid_from: NaiveDate,
    pub valid_to: Option<NaiveDate>,
    pub is_st: bool,
}

#[derive(Serialize, ToSchema)]
pub struct SecurityDetail {
    #[serde(flatten)]
    pub security: Security,
    pub name_history: Vec<NameChange>,
    /// 本地已有日线的范围
    pub first_bar: Option<NaiveDate>,
    pub last_bar: Option<NaiveDate>,
    pub bars: i64,
}

/// 单只证券详情
#[utoipa::path(get, path = "/securities/{thscode}", tag = "market", params(("thscode" = String, Path, description = "例：600519.SH")),
    responses((status = 200, description = "成功", body = SecurityDetail), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn get_security(State(s): State<AppState>, Path(code): Path<String>) -> ApiResult<Json<SecurityDetail>> {
    let security: Security = sqlx::query_as(
        "SELECT thscode, ticker, name, exchange, board, list_date, delisted_on, last_seen FROM ref.security WHERE thscode = $1",
    )
    .bind(&code)
    .fetch_optional(&s.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))?;
    let name_history = sqlx::query_as("SELECT name, valid_from, valid_to, is_st FROM ref.security_name_hist WHERE thscode = $1 ORDER BY valid_from")
        .bind(&code)
        .fetch_all(&s.pool)
        .await?;
    let (first_bar, last_bar, bars): (Option<NaiveDate>, Option<NaiveDate>, i64) =
        sqlx::query_as("SELECT min(trade_date), max(trade_date), count(*) FROM mkt.stock_bar_1d WHERE thscode = $1")
            .bind(&code)
            .fetch_one(&s.pool)
            .await?;
    Ok(Json(SecurityDetail { security, name_history, first_bar, last_bar, bars }))
}

// ───────────────────────── 日线 ─────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct BarQuery {
    /// none / qfq / hfq / qfq_ths / hfq_ths，默认 qfq（等比前复权）
    pub adjust: Option<String>,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    /// 最多返回最近的 N 根，默认 500，最大 10000
    pub limit: Option<i64>,
}

/// 日线。价格以字符串返回：未复权为原始十进制；复权价保留 6 位小数
#[derive(Serialize, ToSchema)]
pub struct Bar {
    pub trade_date: NaiveDate,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    /// 成交量（股）
    pub volume: i64,
    /// 成交额（元）
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// 复权系数（none 时为空）
    pub factor: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct BarSeries {
    pub thscode: String,
    pub adjust: String,
    pub bars: Vec<Bar>,
}

// @project-doc docs/domains/price_adjustment.md#adjust_views
fn bar_view(adjust: &str) -> ApiResult<&'static str> {
    Ok(match adjust {
        "none" => "mkt.stock_bar_1d",
        "qfq" => "mkt.v_stock_bar_qfq",
        "hfq" => "mkt.v_stock_bar_hfq",
        "qfq_ths" => "mkt.v_stock_bar_qfq_ths",
        "hfq_ths" => "mkt.v_stock_bar_hfq_ths",
        other => return Err(ApiError::BadRequest(format!("adjust 只能是 none / qfq / hfq / qfq_ths / hfq_ths：{other}"))),
    })
}

/// 个股日线
#[utoipa::path(get, path = "/securities/{thscode}/bars", tag = "market", params(("thscode" = String, Path), BarQuery),
    responses((status = 200, description = "成功", body = BarSeries), (status = 400, description = "参数不合法", body = crate::error::ErrorBody)))]
async fn security_bars(State(s): State<AppState>, Path(code): Path<String>, Query(q): Query<BarQuery>) -> ApiResult<Json<BarSeries>> {
    let adjust = q.adjust.unwrap_or_else(|| "qfq".into());
    let view = bar_view(&adjust)?;
    let limit = q.limit.unwrap_or(500).clamp(1, 10_000);
    let factor = match adjust.as_str() {
        "qfq" | "hfq" => "factor",
        _ => "NULL::float8",
    };
    let sql = format!(
        "SELECT * FROM (
           SELECT trade_date, open::text, high::text, low::text, close::text, volume, amount, {factor} AS factor
           FROM {view} WHERE thscode = $1 AND ($2::date IS NULL OR trade_date >= $2) AND ($3::date IS NULL OR trade_date <= $3)
           ORDER BY trade_date DESC LIMIT $4) t ORDER BY trade_date"
    );
    /// (日期, 开, 高, 低, 收, 成交量, 成交额, 复权系数)
    type BarRow = (NaiveDate, String, String, String, String, i64, Decimal, Option<f64>);
    // 审计：view 来自白名单，factor 是固定表达式
    let rows: Vec<BarRow> =
        sqlx::query_as(AssertSqlSafe(sql)).bind(&code).bind(q.start).bind(q.end).bind(limit).fetch_all(&s.pool).await?;
    let fmt = |v: String| if adjust == "none" { v } else { v.parse::<f64>().map(|f| format!("{f:.6}")).unwrap_or(v) };
    let bars = rows
        .into_iter()
        .map(|(trade_date, o, h, l, c, volume, amount, factor)| Bar { trade_date, open: fmt(o), high: fmt(h), low: fmt(l), close: fmt(c), volume, amount, factor })
        .collect();
    Ok(Json(BarSeries { thscode: code, adjust, bars }))
}

/// 校验快慢线周期，返回 (fast, slow)
pub(super) fn windows(fast: Option<i32>, slow: Option<i32>) -> ApiResult<(usize, usize)> {
    ma::validate_windows(fast.unwrap_or(15), slow.unwrap_or(30)).map_err(ApiError::BadRequest)
}


// ───────────────────────── 指数 ─────────────────────────

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct IndexInfo {
    pub thscode: String,
    pub name: String,
    /// industry / cn_concept / region / tszs / exchange / ths
    pub category: String,
    pub members: i64,
}

#[derive(Deserialize, IntoParams)]
pub struct IndexQuery {
    pub category: Option<String>,
    /// 名称片段
    pub q: Option<String>,
}

/// 指数与板块列表
#[utoipa::path(get, path = "/indices", tag = "market", params(IndexQuery), responses((status = 200, description = "成功", body = [IndexInfo])))]
async fn list_indices(State(s): State<AppState>, Query(q): Query<IndexQuery>) -> ApiResult<Json<Vec<IndexInfo>>> {
    let rows = sqlx::query_as(
        "SELECT i.thscode, i.name, i.category,
                (SELECT count(*) FROM ref.index_member m WHERE m.index_code = i.thscode AND m.valid_to IS NULL) AS members
         FROM ref.index_info i
         WHERE ($1::text IS NULL OR i.category = $1) AND ($2::text IS NULL OR i.name ILIKE '%' || $2 || '%')
         ORDER BY i.category, i.thscode LIMIT 2000",
    )
    .bind(&q.category)
    .bind(&q.q)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}

/// 指数日线（指数没有复权）
#[utoipa::path(get, path = "/indices/{thscode}/bars", tag = "market", params(("thscode" = String, Path), BarQuery), responses((status = 200, description = "成功", body = BarSeries)))]
async fn index_bars(State(s): State<AppState>, Path(code): Path<String>, Query(q): Query<BarQuery>) -> ApiResult<Json<BarSeries>> {
    let limit = q.limit.unwrap_or(500).clamp(1, 10_000);
    let rows: Vec<(NaiveDate, String, String, String, String, Decimal, Decimal)> = sqlx::query_as(
        "SELECT * FROM (SELECT trade_date, open::text, high::text, low::text, close::text, volume, amount FROM mkt.index_bar_1d
           WHERE thscode = $1 AND ($2::date IS NULL OR trade_date >= $2) AND ($3::date IS NULL OR trade_date <= $3)
           ORDER BY trade_date DESC LIMIT $4) t ORDER BY trade_date",
    )
    .bind(&code)
    .bind(q.start)
    .bind(q.end)
    .bind(limit)
    .fetch_all(&s.pool)
    .await?;
    let bars = rows
        .into_iter()
        .map(|(trade_date, open, high, low, close, volume, amount)| Bar {
            trade_date,
            open,
            high,
            low,
            close,
            volume: volume.trunc().to_i64().unwrap_or(i64::MAX),
            amount,
            factor: None,
        })
        .collect();
    Ok(Json(BarSeries { thscode: code, adjust: "none".into(), bars }))
}

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct IndexMember {
    pub thscode: String,
    pub name: Option<String>,
    pub valid_from: NaiveDate,
    pub valid_to: Option<NaiveDate>,
}

#[derive(Deserialize, IntoParams)]
pub struct MemberQuery {
    /// 查询某一天的成分；默认今天（当前成分）
    pub date: Option<NaiveDate>,
}

/// 指数成分（从开始采集那天起有历史）
#[utoipa::path(get, path = "/indices/{thscode}/members", tag = "market", params(("thscode" = String, Path), MemberQuery), responses((status = 200, description = "成功", body = [IndexMember])))]
async fn index_members(State(s): State<AppState>, Path(code): Path<String>, Query(q): Query<MemberQuery>) -> ApiResult<Json<Vec<IndexMember>>> {
    let rows = sqlx::query_as(
        "SELECT m.thscode, s.name, m.valid_from, m.valid_to FROM ref.index_member m LEFT JOIN ref.security s USING (thscode)
         WHERE m.index_code = $1 AND m.valid_from <= coalesce($2, current_date) AND (m.valid_to IS NULL OR m.valid_to > coalesce($2, current_date))
         ORDER BY m.thscode",
    )
    .bind(&code)
    .bind(q.date)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(rows))
}
