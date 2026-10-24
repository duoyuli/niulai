//! 个股和 ETF 的基本面：最近的估值、三张财报的摘要、财务能力指标，以及 ETF 的资料和行情概览。
//!
//! 财报和财务指标只写在 `raw.record`，这里是读取它们、解析供应商字段名的唯一位置（因子研究读同样的三张财报，
//! 数据集 ID 用这里的常量）。HTTP 参数和响应在 `api::market`。

use chrono::NaiveDate;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::instrument::{AssetType, ETF_PROFILE_DATASET};

/// 利润表（季度），只写 `raw.record`
pub const INCOME: &str = "a_share.income_q";
/// 资产负债表（季度）
pub const BALANCE: &str = "a_share.balance_q";
/// 现金流量表（季度）
pub const CASHFLOW: &str = "a_share.cashflow_q";
/// 财务能力指标（成长、盈利、偿债、营运、现金流）
pub const INDICATORS: &str = "a_share.fin_indicator";

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

/// 一只证券的基本面；证券不存在时为空。`limit` 是返回最近多少个交易日的估值
pub async fn load(pool: &PgPool, code: &str, limit: i64) -> sqlx::Result<Option<Fundamentals>> {
    let asset_type: Option<AssetType> =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(code)
            .fetch_optional(pool)
            .await?;
    let Some(asset_type) = asset_type else {
        return Ok(None);
    };
    if asset_type == AssetType::Etf {
        return Ok(Some(Fundamentals {
            thscode: code.to_string(),
            asset_type,
            valuation: vec![],
            income: None,
            balance: None,
            cashflow: None,
            indicators: vec![],
            etf: Some(etf_fundamentals(pool, code).await?),
        }));
    }

    let valuation_rows: Vec<ValuationRow> = sqlx::query_as(
        "SELECT trade_date, float_market_cap::text, pe_ttm::text, pe_mrq::text, pb_mrq::text, ps_ttm::text, pcf_ttm::text
         FROM fin.valuation_1d WHERE thscode = $1 ORDER BY trade_date DESC LIMIT $2",
    )
    .bind(code)
    .bind(limit)
    .fetch_all(pool)
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

    let income = latest_statement(pool, INCOME, code).await?;
    let balance = latest_statement(pool, BALANCE, code).await?;
    let cashflow = latest_statement(pool, CASHFLOW, code).await?;
    let indicator_rows: Vec<(Option<NaiveDate>, Value)> = sqlx::query_as(
        "SELECT obs_date, payload FROM raw.record
         WHERE dataset_id = $2 AND thscode = $1
         ORDER BY obs_date DESC NULLS LAST, last_seen_at DESC LIMIT 1",
    )
    .bind(code)
    .bind(INDICATORS)
    .fetch_all(pool)
    .await?;
    let indicators = indicator_rows
        .into_iter()
        .flat_map(|(_, payload)| parse_abilities(&payload))
        .collect();

    Ok(Some(Fundamentals {
        thscode: code.to_string(),
        asset_type,
        valuation,
        income,
        balance,
        cashflow,
        indicators,
        etf: None,
    }))
}

async fn etf_fundamentals(pool: &PgPool, code: &str) -> sqlx::Result<EtfFundamentals> {
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
    .bind(ETF_PROFILE_DATASET)
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
    pool: &PgPool,
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
        INCOME => &[
            ("operating_income", "营业收入"),
            ("operating_profit", "营业利润"),
            ("parent_holder_net_profit", "归母净利润"),
            ("net_profit", "净利润"),
            ("basic_eps", "基本每股收益"),
        ],
        BALANCE => &[
            ("assets_total", "资产总额"),
            ("total_debt", "负债总额"),
            ("holder_equity_total", "所有者权益"),
            ("cash", "货币资金"),
            ("accounts_receivable", "应收账款"),
        ],
        CASHFLOW => &[
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
    // 供应商的毫秒时间戳按上海时间换算，和因子研究推断财报可用日的口径一致
    crate::time::ms_to_cn_date(ms)
}
