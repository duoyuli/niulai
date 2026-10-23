//! 行情接口：证券、日线（含复权）、财务摘要、指数和今日热点（计算在 `crate::analysis::hotspots`）。
//!
//! 复权口径（见数据方案 §3.1）：
//! - `qfq` / `hfq`：等比前复权 / 等比后复权，计算因子和收益的标准口径
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
use serde_json::Value;
use sqlx::AssertSqlSafe;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::analysis::hotspots::{self, HotMarket};
use crate::analysis::limit_pool::CacheVersions;
use crate::error::{ApiError, ApiResult};
use crate::instrument::AssetType;

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
    pub asset_type: AssetType,
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
    // 审计：filter 是固定的 SQL 片段，用户输入全部走绑定参数
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
    pub asset_type: AssetType,
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
    // 审计：表名来自 AssetType::bar_table 的固定值
    let sql = AssertSqlSafe(format!(
        "SELECT min(trade_date), max(trade_date), count(*) FROM {} WHERE thscode = $1",
        security.asset_type.bar_table()
    ));
    let (first_bar, last_bar, bars): (Option<NaiveDate>, Option<NaiveDate>, i64) =
        sqlx::query_as(sql).bind(&code).fetch_one(&s.pool).await?;
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
    let asset_type: AssetType =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(&code)
            .fetch_optional(&s.pool)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))?;
    if asset_type == AssetType::Etf {
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
        "SELECT payload FROM raw.record WHERE dataset_id = $2 AND thscode = $1 ORDER BY last_seen_at DESC LIMIT 1",
    )
    .bind(code)
    .bind(crate::instrument::ETF_PROFILE_DATASET)
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
fn bar_view(asset_type: AssetType, adjust: &str) -> ApiResult<(&'static str, &'static str)> {
    if asset_type == AssetType::Etf {
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
    let asset_type: Option<AssetType> =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(&code)
            .fetch_optional(&s.pool)
            .await?;
    let asset_type = asset_type.ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))?;
    let (view, factor) = bar_view(asset_type, &adjust)?;
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

#[derive(Deserialize, IntoParams)]
pub struct HotQuery {
    /// 交易日，默认最新；取不晚于它的、本地有日线的最近一天
    pub date: Option<NaiveDate>,
}

/// 今日热点：权威涨停、跌停、炸板池和当天的封板率。
///
/// 涨跌停池由供应商整理，最近几天可能还没更新；`available` 为 false 时
/// `note` 用白话说明原因，页面据此显示空态。计算在 `crate::analysis::hotspots`
#[utoipa::path(get, path = "/market/hot", tag = "market", params(HotQuery),
    responses((status = 200, description = "成功；这一天的涨跌停数据还没同步时 available 为 false，note 说明原因", body = HotMarket)))]
async fn market_hot(
    State(s): State<AppState>,
    Query(q): Query<HotQuery>,
) -> ApiResult<Json<HotMarket>> {
    let Some(trade_date) = hotspots::trade_date(&s.pool, q.date).await? else {
        return Ok(Json(hotspots::no_market_data()));
    };
    let version = CacheVersions::load(&s.pool).await?;
    let key = crate::cache::derived_key(
        "market:hot",
        &version.market_token(),
        &trade_date.to_string(),
    );
    let value = s
        .cache
        .cached_json(&key, || hotspots::compute(&s.pool, trade_date))
        .await?;
    Ok(Json(value))
}
