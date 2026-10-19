//! 行情接口：证券、日线（含复权）、均线与交叉信号、指数。
//!
//! 复权口径（见数据方案 §3.1）：
//! - `qfq` / `hfq`：等比前复权 / 等比后复权，计算指标的标准口径
//! - `qfq_ths` / `hfq_ths`：同花顺减法复权，只用于和同花顺软件对数
//! - `none`：未复权原价
//!
//! 均线和交叉信号一律用等比前复权计算；因为等比复权的前、后复权只差一个常数倍，信号与基准日无关。
//! 计算本身在 `crate::analysis` 中，这里只做参数校验和响应格式。

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
        .routes(routes!(security_fundamentals))
        .routes(routes!(security_bars))
        .routes(routes!(list_indices))
        .routes(routes!(index_bars))
        .routes(routes!(index_members))
        .routes(routes!(market_hot))
}

// ───────────────────────── 证券 ─────────────────────────

#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct Security {
    pub thscode: String,
    pub ticker: String,
    pub name: String,
    pub exchange: String,
    /// stock / etf
    pub asset_type: String,
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
async fn list_securities(
    State(s): State<AppState>,
    Query(q): Query<SecurityQuery>,
) -> ApiResult<Json<Page<Security>>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let pattern = q.q.as_deref().map(|t| format!("%{}%", t.trim()));
    let filter = "($1::text IS NULL OR thscode ILIKE $1 OR name ILIKE $1) AND ($2::text IS NULL OR board = $2)";
    // 审计：filter 是固定的 SQL 片段，用户输入全部走绑定参数
    let total: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT count(*) FROM ref.instrument WHERE {filter}"
    )))
    .bind(&pattern)
    .bind(&q.board)
    .fetch_one(&s.pool)
    .await?;
    let items = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT thscode, ticker, name, exchange, asset_type, board, list_date, delisted_on, last_seen FROM ref.instrument WHERE {filter} ORDER BY thscode LIMIT $3 OFFSET $4"
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

#[derive(Deserialize, IntoParams)]
pub struct FundamentalsQuery {
    /// 返回最近多少个交易日的估值，默认 90，最大 365
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct ValuationPoint {
    pub trade_date: NaiveDate,
    /// 流通市值（元）
    pub float_market_cap: Option<String>,
    /// 市盈率（TTM）
    pub pe_ttm: Option<String>,
    /// 市盈率（最近报告期）
    pub pe_mrq: Option<String>,
    /// 市净率（最近报告期）
    pub pb_mrq: Option<String>,
    /// 市销率（TTM）
    pub ps_ttm: Option<String>,
    /// 市现率（TTM）
    pub pcf_ttm: Option<String>,
}

type ValuationRow = (
    NaiveDate,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Serialize, ToSchema)]
pub struct FundamentalMetric {
    /// 面向用户的指标名称
    pub name: String,
    pub value: f64,
}

#[derive(Serialize, ToSchema)]
pub struct FinancialStatement {
    pub period_end: Option<NaiveDate>,
    pub report_date: Option<NaiveDate>,
    pub metrics: Vec<FundamentalMetric>,
}

#[derive(Serialize, ToSchema)]
pub struct FinancialAbility {
    /// growth / profitability / solvency / operation / cash-flow
    pub name: String,
    pub metrics: Vec<FundamentalMetric>,
}

#[derive(Serialize, ToSchema)]
pub struct Fundamentals {
    pub thscode: String,
    /// stock / etf
    pub asset_type: String,
    /// 每个交易日一条，按日期倒序
    pub valuation: Vec<ValuationPoint>,
    pub income: Option<FinancialStatement>,
    pub balance: Option<FinancialStatement>,
    pub cashflow: Option<FinancialStatement>,
    pub indicators: Vec<FinancialAbility>,
    /// ETF 专用资料和行情概览；股票时为空
    pub etf: Option<EtfFundamentals>,
}

#[derive(Serialize, ToSchema)]
pub struct EtfFundamentals {
    pub latest_close: Option<String>,
    pub latest_date: Option<NaiveDate>,
    pub first_bar: Option<NaiveDate>,
    pub last_bar: Option<NaiveDate>,
    pub bars: i64,
    /// vendor_forward_adjusted / market_snapshot
    pub price_basis: Option<String>,
    pub fund_name: Option<String>,
    pub fund_scale: Option<String>,
    pub unit_nav: Option<String>,
    pub management_company: Option<String>,
    pub management_fee: Option<String>,
}

/// 单只证券详情
#[utoipa::path(get, path = "/securities/{thscode}", tag = "market", params(("thscode" = String, Path, description = "例：600519.SH")),
    responses((status = 200, description = "成功", body = SecurityDetail), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn get_security(
    State(s): State<AppState>,
    Path(code): Path<String>,
) -> ApiResult<Json<SecurityDetail>> {
    let security: Security = sqlx::query_as(
        "SELECT thscode, ticker, name, exchange, asset_type, board, list_date, delisted_on, last_seen FROM ref.instrument WHERE thscode = $1",
    )
    .bind(&code)
    .fetch_optional(&s.pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))?;
    let name_history = sqlx::query_as("SELECT name, valid_from, valid_to, is_st FROM ref.security_name_hist WHERE thscode = $1 ORDER BY valid_from")
        .bind(&code)
        .fetch_all(&s.pool)
        .await?;
    let (first_bar, last_bar, bars): (Option<NaiveDate>, Option<NaiveDate>, i64) = if security
        .asset_type
        == "etf"
    {
        sqlx::query_as("SELECT min(trade_date), max(trade_date), count(*) FROM mkt.etf_bar_1d WHERE thscode = $1")
                .bind(&code)
                .fetch_one(&s.pool)
                .await?
    } else {
        sqlx::query_as("SELECT min(trade_date), max(trade_date), count(*) FROM mkt.stock_bar_1d WHERE thscode = $1")
                .bind(&code)
                .fetch_one(&s.pool)
                .await?
    };
    Ok(Json(SecurityDetail {
        security,
        name_history,
        first_bar,
        last_bar,
        bars,
    }))
}

/// 个股估值和最近财务摘要。财务数据按披露日期标记，页面不会把报告期误当成数据可见日期。
#[utoipa::path(get, path = "/securities/{thscode}/fundamentals", tag = "market", params(("thscode" = String, Path, description = "例：600519.SH"), FundamentalsQuery),
    responses((status = 200, description = "成功", body = Fundamentals), (status = 404, description = "证券不存在", body = crate::error::ErrorBody)))]
async fn security_fundamentals(
    State(s): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<FundamentalsQuery>,
) -> ApiResult<Json<Fundamentals>> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ref.instrument WHERE thscode = $1)")
            .bind(&code)
            .fetch_one(&s.pool)
            .await?;
    if !exists {
        return Err(ApiError::NotFound(format!("证券不存在：{code}")));
    }

    let asset_type: String =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(&code)
            .fetch_one(&s.pool)
            .await?;
    if asset_type == "etf" {
        return Ok(Json(Fundamentals {
            thscode: code.clone(),
            asset_type,
            valuation: vec![],
            income: None,
            balance: None,
            cashflow: None,
            indicators: vec![],
            etf: Some(etf_fundamentals(&s.pool, &code).await?),
        }));
    }

    let limit = q.limit.unwrap_or(90).clamp(1, 365);
    let valuation_rows: Vec<ValuationRow> = sqlx::query_as(
        "SELECT trade_date, float_market_cap::text, pe_ttm::text, pe_mrq::text, pb_mrq::text, ps_ttm::text, pcf_ttm::text
         FROM fin.valuation_1d WHERE thscode = $1 ORDER BY trade_date DESC LIMIT $2",
    )
    .bind(&code)
    .bind(limit)
    .fetch_all(&s.pool)
    .await?;
    let valuation = valuation_rows
        .into_iter()
        .map(
            |(trade_date, float_market_cap, pe_ttm, pe_mrq, pb_mrq, ps_ttm, pcf_ttm)| {
                ValuationPoint {
                    trade_date,
                    float_market_cap,
                    pe_ttm,
                    pe_mrq,
                    pb_mrq,
                    ps_ttm,
                    pcf_ttm,
                }
            },
        )
        .collect();

    let income = latest_statement(&s.pool, "a_share.income_q", &code).await?;
    let balance = latest_statement(&s.pool, "a_share.balance_q", &code).await?;
    let cashflow = latest_statement(&s.pool, "a_share.cashflow_q", &code).await?;
    let indicator_rows: Vec<(Option<NaiveDate>, Value)> = sqlx::query_as(
        "SELECT obs_date, payload FROM raw.record
         WHERE dataset_id = 'a_share.fin_indicator' AND thscode = $1
         ORDER BY obs_date DESC NULLS LAST, last_seen_at DESC LIMIT 1",
    )
    .bind(&code)
    .fetch_all(&s.pool)
    .await?;
    let indicators = indicator_rows
        .into_iter()
        .flat_map(|(_, payload)| parse_abilities(&payload))
        .collect();

    Ok(Json(Fundamentals {
        thscode: code,
        asset_type,
        valuation,
        income,
        balance,
        cashflow,
        indicators,
        etf: None,
    }))
}

async fn etf_fundamentals(pool: &sqlx::PgPool, code: &str) -> sqlx::Result<EtfFundamentals> {
    let (first_bar, last_bar, bars): (Option<NaiveDate>, Option<NaiveDate>, i64) = sqlx::query_as(
        "SELECT min(trade_date), max(trade_date), count(*) FROM mkt.etf_bar_1d WHERE thscode = $1",
    )
    .bind(code)
    .fetch_one(pool)
    .await?;
    let latest: Option<(NaiveDate, String, String)> = sqlx::query_as(
        "SELECT trade_date, close::text, price_basis FROM mkt.etf_bar_1d WHERE thscode = $1 ORDER BY trade_date DESC LIMIT 1",
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;
    let profile: Option<Value> = sqlx::query_scalar(
        "SELECT payload FROM raw.record WHERE dataset_id = 'fund.profile' AND thscode = $1 ORDER BY last_seen_at DESC LIMIT 1",
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;
    let number_text = |key: &str| {
        profile.as_ref().and_then(|v| v.get(key)).and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_f64().map(|n| n.to_string()))
        })
    };
    let management_fee = profile.as_ref().and_then(|v| {
        v.get("rate_info")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("rate_type").and_then(Value::as_str) == Some("management")
                })
            })
            .and_then(|item| item.get("standard_rate").and_then(Value::as_str))
            .map(str::to_owned)
    });
    Ok(EtfFundamentals {
        latest_close: latest.as_ref().map(|(_, close, _)| close.clone()),
        latest_date: latest.as_ref().map(|(date, _, _)| *date),
        first_bar,
        last_bar,
        bars,
        price_basis: latest.map(|(_, _, basis)| basis),
        fund_name: profile
            .as_ref()
            .and_then(|v| v.get("fund_name").and_then(Value::as_str))
            .map(str::to_owned),
        fund_scale: number_text("fund_scale"),
        unit_nav: number_text("unit_nav"),
        management_company: profile
            .as_ref()
            .and_then(|v| v.get("mgmt_name").and_then(Value::as_str))
            .map(str::to_owned),
        management_fee,
    })
}

async fn latest_statement(
    pool: &sqlx::PgPool,
    dataset: &str,
    code: &str,
) -> sqlx::Result<Option<FinancialStatement>> {
    let row: Option<(Option<NaiveDate>, Value)> = sqlx::query_as(
        "SELECT obs_date, payload FROM raw.record
         WHERE dataset_id = $1 AND thscode = $2
         ORDER BY obs_date DESC NULLS LAST, last_seen_at DESC LIMIT 1",
    )
    .bind(dataset)
    .bind(code)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(period_end, payload)| parse_statement(&payload, period_end, dataset)))
}

fn parse_statement(
    payload: &Value,
    period_end: Option<NaiveDate>,
    dataset: &str,
) -> Option<FinancialStatement> {
    let keys: &[(&str, &str)] = match dataset {
        "a_share.income_q" => &[
            ("operating_income", "营业收入"),
            ("operating_profit", "营业利润"),
            ("parent_holder_net_profit", "归母净利润"),
            ("net_profit", "净利润"),
            ("basic_eps", "基本每股收益"),
        ],
        "a_share.balance_q" => &[
            ("assets_total", "资产总额"),
            ("total_debt", "负债总额"),
            ("holder_equity_total", "所有者权益"),
            ("cash", "货币资金"),
            ("accounts_receivable", "应收账款"),
        ],
        "a_share.cashflow_q" => &[
            ("act_cash_flow_net", "经营现金流净额"),
            ("invest_cash_flow_net", "投资现金流净额"),
            ("financing_cash_flow_net", "筹资现金流净额"),
            ("cash_equivalents_net_addition", "现金及等价物净增加"),
        ],
        _ => return None,
    };
    let metrics = keys
        .iter()
        .filter_map(|(key, name)| {
            json_number(payload, key).map(|value| FundamentalMetric {
                name: (*name).into(),
                value,
            })
        })
        .collect::<Vec<_>>();
    if metrics.is_empty() {
        return None;
    }
    Some(FinancialStatement {
        period_end,
        report_date: json_date_ms(payload, "report_date_ms"),
        metrics,
    })
}

fn parse_abilities(payload: &Value) -> Vec<FinancialAbility> {
    payload
        .get("abilities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| {
            let name = group.get("ability")?.as_str()?.to_string();
            let metrics = group
                .get("indicators")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| {
                    Some(FundamentalMetric {
                        name: item.get("index_id")?.as_str()?.to_string(),
                        value: json_number(item, "value")?,
                    })
                })
                .collect::<Vec<_>>();
            (!metrics.is_empty()).then_some(FinancialAbility { name, metrics })
        })
        .collect()
}

fn json_number(payload: &Value, key: &str) -> Option<f64> {
    let value = payload.get(key)?;
    let number = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())?;
    number.is_finite().then_some(number)
}

fn json_date_ms(payload: &Value, key: &str) -> Option<NaiveDate> {
    let ms = payload
        .get(key)?
        .as_i64()
        .or_else(|| payload.get(key)?.as_str()?.parse::<i64>().ok())?;
    chrono::DateTime::from_timestamp(ms / 1000, ((ms % 1000) * 1_000_000) as u32)
        .map(|d| d.date_naive())
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
fn bar_view(asset_type: &str, adjust: &str) -> ApiResult<(&'static str, &'static str)> {
    if asset_type == "etf" {
        return match adjust {
            "none" | "qfq" | "hfq" => Ok(("mkt.etf_bar_1d", "NULL::float8")),
            other => Err(ApiError::BadRequest(format!(
                "ETF 的 adjust 只能是 none / qfq / hfq：{other}"
            ))),
        };
    }
    Ok(match adjust {
        "none" => ("mkt.stock_bar_1d", "NULL::float8"),
        "qfq" => ("mkt.v_stock_bar_qfq", "factor"),
        "hfq" => ("mkt.v_stock_bar_hfq", "factor"),
        "qfq_ths" => ("mkt.v_stock_bar_qfq_ths", "NULL::float8"),
        "hfq_ths" => ("mkt.v_stock_bar_hfq_ths", "NULL::float8"),
        other => {
            return Err(ApiError::BadRequest(format!(
                "adjust 只能是 none / qfq / hfq / qfq_ths / hfq_ths：{other}"
            )));
        }
    })
}

/// 个股日线
#[utoipa::path(get, path = "/securities/{thscode}/bars", tag = "market", params(("thscode" = String, Path), BarQuery),
    responses((status = 200, description = "成功", body = BarSeries), (status = 400, description = "参数不合法", body = crate::error::ErrorBody)))]
async fn security_bars(
    State(s): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<BarQuery>,
) -> ApiResult<Json<BarSeries>> {
    let adjust = q.adjust.unwrap_or_else(|| "qfq".into());
    let asset_type: Option<String> =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(&code)
            .fetch_optional(&s.pool)
            .await?;
    let asset_type = asset_type.ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))?;
    let (view, factor) = bar_view(&asset_type, &adjust)?;
    let limit = q.limit.unwrap_or(500).clamp(1, 10_000);
    let sql = format!(
        "SELECT * FROM (
           SELECT trade_date, open::text, high::text, low::text, close::text, volume, amount, {factor}
           FROM {view} WHERE thscode = $1 AND ($2::date IS NULL OR trade_date >= $2) AND ($3::date IS NULL OR trade_date <= $3)
           ORDER BY trade_date DESC LIMIT $4) t ORDER BY trade_date"
    );
    /// (日期, 开, 高, 低, 收, 成交量, 成交额, 复权系数)
    type BarRow = (
        NaiveDate,
        String,
        String,
        String,
        String,
        i64,
        Decimal,
        Option<f64>,
    );
    // 审计：view 来自白名单，factor 是固定表达式
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(&code)
        .bind(q.start)
        .bind(q.end)
        .bind(limit)
        .fetch_all(&s.pool)
        .await?;
    let fmt = |v: String| {
        if adjust == "none" {
            v
        } else {
            v.parse::<f64>().map(|f| format!("{f:.6}")).unwrap_or(v)
        }
    };
    let bars = rows
        .into_iter()
        .map(|(trade_date, o, h, l, c, volume, amount, factor)| Bar {
            trade_date,
            open: fmt(o),
            high: fmt(h),
            low: fmt(l),
            close: fmt(c),
            volume,
            amount,
            factor,
        })
        .collect();
    Ok(Json(BarSeries {
        thscode: code,
        adjust,
        bars,
    }))
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
async fn list_indices(
    State(s): State<AppState>,
    Query(q): Query<IndexQuery>,
) -> ApiResult<Json<Vec<IndexInfo>>> {
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
async fn index_bars(
    State(s): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<BarQuery>,
) -> ApiResult<Json<BarSeries>> {
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
    Ok(Json(BarSeries {
        thscode: code,
        adjust: "none".into(),
        bars,
    }))
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
async fn index_members(
    State(s): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<MemberQuery>,
) -> ApiResult<Json<Vec<IndexMember>>> {
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

// ───────────────────────── 今日热点 ─────────────────────────

/// 涨跌停池的数据集：交易所口径（同花顺整理），这些数据集是每周同步的
const HOT_LIMIT_UP: &str = "special.limit_up_pool";
const HOT_LIMIT_DOWN: &str = "special.limit_down_pool";
const HOT_LIMIT_BREAK: &str = "special.limit_break_pool";
const HOT_LIMIT_STATS: &str = "special.limit_up_stats";

/// 每个列表最多列多少只股票；全部家数在 total 字段里另给
const HOT_STOCK_LIMIT: usize = 50;

#[derive(Deserialize, IntoParams)]
pub struct HotQuery {
    /// 交易日，默认最新；取不晚于它的、本地有日线的最近一天
    pub date: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema)]
pub struct HotMarket {
    /// 数据对应的交易日；本地还没有任何行情数据时为空
    pub trade_date: Option<NaiveDate>,
    /// 这一天的权威涨跌停数据（涨停、跌停、炸板池）是否已经同步
    pub available: bool,
    /// 没有数据时的一句白话说明，例如「今天的涨停数据还没同步，稍后再看」
    pub note: Option<String>,
    /// 涨停家数（不只是下面列出的那些）
    pub limit_up_total: i64,
    /// 按连板高度分组，从高到低；1 表示首板
    pub limit_up_groups: Vec<HotLimitUpGroup>,
    /// 最高连板数，没有涨停时为 0
    pub max_board: i32,
    /// 跌停家数
    pub limit_down_total: i64,
    /// 跌停股，最多列 50 只
    pub limit_down_stocks: Vec<HotStock>,
    /// 炸板家数（盘中涨停、收盘没封住）
    pub broken_total: i64,
    /// 炸板股，最多列 50 只
    pub broken_stocks: Vec<HotStock>,
    /// 当天的封板率等统计，没同步时为空
    pub stats: Option<HotStats>,
}

#[derive(Serialize, ToSchema)]
pub struct HotLimitUpGroup {
    /// 连板高度，1 表示首板
    pub board_num: i32,
    /// 「首板」「2 板」「3 板」……
    pub label: String,
    /// 这一档一共多少只（不只是下面列出的那些）
    pub count: i64,
    pub stocks: Vec<HotLimitUpStock>,
}

#[derive(Serialize, ToSchema)]
pub struct HotLimitUpStock {
    pub thscode: String,
    pub name: String,
    /// 供应商的连板说明，例如「首板」「2 连板」
    pub continue_day_text: Option<String>,
    /// 涨停原因，例如「低空经济+无人机物流+海上油气」
    pub limit_up_reason: Option<String>,
    /// 最新价（字符串，保留供应商给的精度）
    pub last_price: Option<String>,
    /// 收盘时的封单额（元，字符串）
    pub seal_money: Option<String>,
    /// 首次涨停时间，例如「10:55」
    pub limit_up_time: Option<String>,
    /// 是否 ST 股
    pub is_st: bool,
}

#[derive(Serialize, ToSchema)]
pub struct HotStock {
    pub thscode: String,
    pub name: String,
    /// 最新价（字符串，保留供应商给的精度）
    pub last_price: Option<String>,
    /// 当天涨跌幅（%）
    pub price_change_pct: Option<f64>,
    /// 成交额（元，字符串）
    pub turnover: Option<String>,
    /// 换手率（%）
    pub turnover_ratio_pct: Option<f64>,
    /// 炸板次数，只有炸板股有
    pub open_times: Option<i64>,
    /// 首次跌停时间，只有跌停股有
    pub first_limit_time: Option<String>,
    /// 最后一次跌停时间，只有跌停股有
    pub last_limit_time: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct HotStats {
    /// 封板率：收盘还封在涨停的占当天触及涨停的比例（0.55 表示 55%）
    pub seal_rate: Option<f64>,
    /// 收盘涨停家数
    pub limit_up_count: Option<i64>,
    /// 盘中触及过涨停的家数
    pub touched_limit_up_count: Option<i64>,
    /// 盘中涨停后打开、收盘没封住的家数
    pub open_count: Option<i64>,
}

/// 今日热点：权威涨停、跌停、炸板池和当天的封板率。
///
/// 涨跌停池由供应商整理，按周同步，最近几天可能还没更新；`available` 为 false 时
/// `note` 用白话说明原因，页面据此显示空态。
// @project-doc docs/domains/hotspots.md#rules
#[utoipa::path(get, path = "/market/hot", tag = "market", params(HotQuery),
    responses((status = 200, description = "成功；这一天的涨跌停数据还没同步时 available 为 false，note 说明原因", body = HotMarket)))]
async fn market_hot(
    State(s): State<AppState>,
    Query(q): Query<HotQuery>,
) -> ApiResult<Json<HotMarket>> {
    // 交易日和 /market/overview 一样取本地日线里不晚于请求日期的最近一天；
    // 日线还没同步时退回涨跌停池自己的最新日期，先把已经同步的涨停池显示出来
    let bar_date: Option<NaiveDate> = sqlx::query_scalar(
        "SELECT max(trade_date) FROM mkt.stock_bar_1d WHERE ($1::date IS NULL OR trade_date <= $1)",
    )
    .bind(q.date)
    .fetch_one(&s.pool)
    .await?;
    let trade_date = match (bar_date, q.date) {
        (Some(d), _) => Some(d),
        (None, Some(d)) => Some(d),
        (None, None) => {
            sqlx::query_scalar("SELECT max(obs_date) FROM raw.record WHERE dataset_id = $1")
                .bind(HOT_LIMIT_UP)
                .fetch_one(&s.pool)
                .await?
        }
    };
    let Some(trade_date) = trade_date else {
        return Ok(empty_hot(
            None,
            "本地还没有行情数据，等第一次同步完成后再看".to_string(),
        ));
    };

    let ids: Vec<String> = [
        HOT_LIMIT_UP,
        HOT_LIMIT_DOWN,
        HOT_LIMIT_BREAK,
        HOT_LIMIT_STATS,
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    // 只取这 4 个数据集当天的记录；供应商偶尔有一行的 thscode 为空，用 ticker 到证券表里补
    let rows: Vec<(String, Value, Option<String>)> = sqlx::query_as(
        "SELECT r.dataset_id, r.payload,
                coalesce(nullif(r.payload->>'thscode', ''),
                         (SELECT sec.thscode FROM ref.security sec WHERE sec.ticker = r.payload->>'ticker' LIMIT 1))
         FROM raw.record r
         WHERE r.dataset_id = ANY($1) AND r.obs_date = $2",
    )
    .bind(&ids)
    .bind(trade_date)
    .fetch_all(&s.pool)
    .await?;

    let (mut up_rows, mut down_rows, mut break_rows, mut stats) =
        (Vec::new(), Vec::new(), Vec::new(), None);
    for (dataset, payload, thscode) in rows {
        match dataset.as_str() {
            HOT_LIMIT_UP => up_rows.push((payload, thscode)),
            HOT_LIMIT_DOWN => down_rows.push((payload, thscode)),
            HOT_LIMIT_BREAK => break_rows.push((payload, thscode)),
            HOT_LIMIT_STATS => stats = Some(hot_stats(&payload)),
            _ => {}
        }
    }
    let available =
        !up_rows.is_empty() || !down_rows.is_empty() || !break_rows.is_empty() || stats.is_some();
    if !available {
        // 涨停池从 2020-07-22 才有；比这更早的日期不是没同步，而是那时还没数据
        let earliest = NaiveDate::from_ymd_opt(2020, 7, 22).expect("固定日期");
        let note = if trade_date < earliest {
            format!(
                "{} 太早了：涨停数据从 2020 年 7 月 22 日才开始有",
                trade_date.format("%Y-%m-%d")
            )
        } else if trade_date == crate::time::today_cn() {
            "今天的涨停数据还没同步，稍后再看".to_string()
        } else {
            format!(
                "{} 的涨停、跌停、炸板数据还没同步，稍后再看",
                trade_date.format("%Y-%m-%d")
            )
        };
        return Ok(empty_hot(Some(trade_date), note));
    }

    // 涨停按连板高度分组，组内封单额大的排在前面（封单越大，说明想买的人越多）
    let mut by_board: BTreeMap<i32, Vec<(f64, HotLimitUpStock)>> = BTreeMap::new();
    for (payload, thscode) in up_rows {
        let seal = json_num(&payload, "seal_money").unwrap_or(0.0);
        by_board
            .entry(board_num_of(&payload))
            .or_default()
            .push((seal, limit_up_stock(&payload, thscode)));
    }
    let limit_up_total = by_board.values().map(|v| v.len()).sum::<usize>() as i64;
    let max_board = by_board.keys().next_back().copied().unwrap_or(0);
    let limit_up_groups = by_board
        .into_iter()
        .rev()
        .map(|(board_num, mut stocks)| {
            stocks.sort_by(|a, b| {
                b.0.total_cmp(&a.0)
                    .then_with(|| a.1.thscode.cmp(&b.1.thscode))
            });
            let count = stocks.len() as i64;
            let stocks = stocks
                .into_iter()
                .take(HOT_STOCK_LIMIT)
                .map(|(_, s)| s)
                .collect();
            HotLimitUpGroup {
                board_num,
                label: board_label(board_num),
                count,
                stocks,
            }
        })
        .collect();

    // 跌停按跌幅从大到小；炸板按炸板次数从多到少。都只列前 50 只，总数另给
    let limit_down_total = down_rows.len() as i64;
    let mut limit_down_stocks: Vec<HotStock> = down_rows
        .into_iter()
        .map(|(payload, thscode)| hot_stock(&payload, thscode))
        .collect();
    limit_down_stocks.sort_by(|a, b| {
        a.price_change_pct
            .unwrap_or(0.0)
            .total_cmp(&b.price_change_pct.unwrap_or(0.0))
            .then_with(|| a.thscode.cmp(&b.thscode))
    });
    limit_down_stocks.truncate(HOT_STOCK_LIMIT);

    let broken_total = break_rows.len() as i64;
    let mut broken_stocks: Vec<HotStock> = break_rows
        .into_iter()
        .map(|(payload, thscode)| hot_stock(&payload, thscode))
        .collect();
    broken_stocks.sort_by(|a, b| {
        b.open_times
            .unwrap_or(0)
            .cmp(&a.open_times.unwrap_or(0))
            .then_with(|| a.thscode.cmp(&b.thscode))
    });
    broken_stocks.truncate(HOT_STOCK_LIMIT);

    Ok(Json(HotMarket {
        trade_date: Some(trade_date),
        available: true,
        note: None,
        limit_up_total,
        limit_up_groups,
        max_board,
        limit_down_total,
        limit_down_stocks,
        broken_total,
        broken_stocks,
        stats,
    }))
}

// @project-doc docs/domains/hotspots.md#empty
fn empty_hot(trade_date: Option<NaiveDate>, note: String) -> Json<HotMarket> {
    Json(HotMarket {
        trade_date,
        available: false,
        note: Some(note),
        limit_up_total: 0,
        limit_up_groups: Vec::new(),
        max_board: 0,
        limit_down_total: 0,
        limit_down_stocks: Vec::new(),
        broken_total: 0,
        broken_stocks: Vec::new(),
        stats: None,
    })
}

/// 连板高度：`continue_day_cnt` 为 1 表示首板；缺字段时按首板算
fn board_num_of(payload: &Value) -> i32 {
    payload
        .get("continue_day_cnt")
        .and_then(Value::as_i64)
        .or_else(|| payload.get("board_num").and_then(Value::as_i64))
        .map(|n| n.clamp(1, 100) as i32)
        .unwrap_or(1)
}

fn board_label(board_num: i32) -> String {
    if board_num <= 1 {
        "首板".to_string()
    } else {
        format!("{board_num} 板")
    }
}

fn limit_up_stock(payload: &Value, thscode: Option<String>) -> HotLimitUpStock {
    HotLimitUpStock {
        thscode: stock_code(payload, thscode),
        name: json_str(payload, "name").unwrap_or_default(),
        continue_day_text: json_str(payload, "continue_day_text"),
        limit_up_reason: json_str(payload, "limit_up_reason"),
        last_price: json_str(payload, "last_price"),
        seal_money: json_str(payload, "seal_money"),
        limit_up_time: json_str(payload, "limit_up_time"),
        is_st: payload
            .get("is_st")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn hot_stock(payload: &Value, thscode: Option<String>) -> HotStock {
    HotStock {
        thscode: stock_code(payload, thscode),
        name: json_str(payload, "name").unwrap_or_default(),
        last_price: json_str(payload, "last_price"),
        price_change_pct: json_num(payload, "price_change_ratio_pct"),
        turnover: json_str(payload, "turnover"),
        turnover_ratio_pct: json_num(payload, "turnover_ratio_pct"),
        open_times: payload.get("open_times").and_then(Value::as_i64),
        first_limit_time: json_str(payload, "first_limit_time"),
        last_limit_time: json_str(payload, "last_limit_time"),
    }
}

fn hot_stats(payload: &Value) -> HotStats {
    HotStats {
        seal_rate: json_num(payload, "seal_rate"),
        limit_up_count: payload.get("limit_up_stock_count").and_then(Value::as_i64),
        touched_limit_up_count: payload
            .get("touched_limit_up_stock_count")
            .and_then(Value::as_i64),
        open_count: payload.get("open_stock_count").and_then(Value::as_i64),
    }
}

/// 记录的证券代码：优先用 payload 的 thscode（数据库里已经用 ticker 在证券表里补过一次），
/// 都没有时按 6 位代码段补交易所后缀，保证页面上的链接可用
fn stock_code(payload: &Value, resolved: Option<String>) -> String {
    resolved
        .or_else(|| {
            payload
                .get("thscode")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .get("ticker")
                .and_then(Value::as_str)
                .and_then(ticker_with_exchange)
        })
        .unwrap_or_default()
}

fn ticker_with_exchange(ticker: &str) -> Option<String> {
    let exchange = match ticker.chars().next()? {
        '6' => "SH",
        '0' | '3' => "SZ",
        '4' | '8' | '9' => "BJ",
        _ => return None,
    };
    Some(format!("{ticker}.{exchange}"))
}

fn json_num(payload: &Value, key: &str) -> Option<f64> {
    payload.get(key).and_then(Value::as_f64)
}

/// 取字符串字段；供应商的数值字段（价格、金额）也转成十进制字符串返回
fn json_str(payload: &Value, key: &str) -> Option<String> {
    match payload.get(key)? {
        Value::String(s) => (!s.is_empty()).then(|| s.clone()),
        Value::Number(n) => Some(decimal_text(n)),
        _ => None,
    }
}

/// JSON 数字转文本：整数不带小数点，小数保留原样（例如 172300270 → "172300270"、61.44 → "61.44"）
fn decimal_text(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(v) if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 => format!("{}", v as i64),
        _ => n.to_string(),
    }
}
