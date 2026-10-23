//! 因子研究接口：因子目录、因子检验、多因子组合回测。只计算、不改业务数据；每次运行写入 `ops.backtest_run`，
//! 用于统计试过多少组配置。规则见 docs/domains/factor_research.md。

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::backtest::{
    BacktestCosts, CostOptions, capital, check_range, costs, costs_echo, coverage, manifest,
};
use crate::analysis::execution::DealPrice;
use crate::analysis::factor_lab::{
    self, FactorPortfolioOutcome, FactorTestOutcome, PortfolioParams, Prep, Spec, TestParams,
};
use crate::analysis::factors::{self, FactorId, FactorInfo};
use crate::analysis::market;
use crate::analysis::research::{self, BacktestManifest};
use crate::analysis::strategy::{Definition, Universe};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(factor_catalog))
        .routes(routes!(factor_test))
        .routes(routes!(factor_portfolio))
}

/// 因子目录：每个因子的口径、类别、预期方向和依据
#[utoipa::path(get, path = "/research/factors", tag = "research",
    responses((status = 200, description = "成功", body = [FactorInfo])))]
async fn factor_catalog() -> Json<Vec<FactorInfo>> {
    Json(factors::CATALOG.to_vec())
}

/// 股票池和预处理的公共参数。查询参数不能用 `serde(flatten)`（数字和布尔值会被当成字符串解析失败），
/// 所以两个接口的参数结构各自声明这些字段，再用 [`pool_query!`] 取出来
struct PoolQuery {
    neutralize: Option<bool>,
    exclude_small_pct: Option<f64>,
    boards: Option<String>,
    exclude_st: Option<bool>,
    min_price: Option<f64>,
    min_amount: Option<f64>,
    min_listed_days: Option<u32>,
}

macro_rules! pool_query {
    ($q:expr) => {
        PoolQuery {
            neutralize: $q.neutralize,
            exclude_small_pct: $q.exclude_small_pct,
            boards: $q.boards.clone(),
            exclude_st: $q.exclude_st,
            min_price: $q.min_price,
            min_amount: $q.min_amount,
            min_listed_days: $q.min_listed_days,
        }
    };
}

impl PoolQuery {
    fn universe(&self) -> ApiResult<Universe> {
        let d = Universe::default();
        let boards: Vec<String> = self
            .boards
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(String::from)
            .collect();
        if let Some(b) = boards
            .iter()
            .find(|b| !["main", "chinext", "star", "bse"].contains(&b.as_str()))
        {
            return Err(ApiError::BadRequest(format!("不认识的板块：{b}")));
        }
        for (name, v) in [
            ("min_price", self.min_price),
            ("min_amount", self.min_amount),
        ] {
            if let Some(v) = v
                && (!v.is_finite() || v < 0.0)
            {
                return Err(ApiError::BadRequest(format!("{name} 不能为负")));
            }
        }
        Ok(Universe {
            asset_types: vec!["stock".into()],
            boards,
            exclude_st: self.exclude_st.unwrap_or(true),
            min_price: self.min_price.or(d.min_price),
            min_amount: self.min_amount.or(d.min_amount),
            min_listed_days: self.min_listed_days.or(d.min_listed_days),
            sector: None,
        })
    }

    fn prep(&self) -> ApiResult<Prep> {
        let pct = self.exclude_small_pct.unwrap_or(0.0);
        if !pct.is_finite() || !(0.0..=50.0).contains(&pct) {
            return Err(ApiError::BadRequest(
                "exclude_small_pct 应在 0 到 50 之间".into(),
            ));
        }
        Ok(Prep {
            neutralize: self.neutralize.unwrap_or(true),
            exclude_small_pct: pct,
        })
    }
}

/// 解析 `reversal,idio_vol:0.5,roe`：因子 ID，可带权重（默认 1，按预期方向合成，负数表示反过来用）
fn parse_factors(s: &str) -> ApiResult<(Vec<FactorId>, Vec<f64>)> {
    let mut ids = vec![];
    let mut weights = vec![];
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (id, w) = match part.split_once(':') {
            Some((id, w)) => (
                id.trim(),
                w.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|w| w.is_finite() && w.abs() <= 100.0)
                    .ok_or_else(|| ApiError::BadRequest(format!("权重不合法：{part}")))?,
            ),
            None => (part, 1.0),
        };
        let id: FactorId = serde_json::from_value(serde_json::Value::String(id.to_string()))
            .map_err(|_| ApiError::BadRequest(format!("不认识的因子：{id}")))?;
        if ids.contains(&id) {
            return Err(ApiError::BadRequest(format!("因子重复：{part}")));
        }
        ids.push(id);
        weights.push(w);
    }
    if ids.is_empty() {
        return Err(ApiError::BadRequest("至少选择一个因子".into()));
    }
    if weights.iter().all(|w| *w == 0.0) {
        return Err(ApiError::BadRequest("权重不能全为 0".into()));
    }
    Ok((ids, weights))
}

/// 结束日（默认本地最新交易日）和开始日（默认结束日前 years 年），并检查最长区间
async fn range(
    s: &AppState,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    default_years: i64,
    max_years: i64,
) -> ApiResult<(NaiveDate, NaiveDate)> {
    let end = market::resolve_date(&s.pool, end)
        .await?
        .ok_or_else(|| ApiError::BadRequest("本地还没有日线".into()))?;
    let start = start.unwrap_or(end - chrono::Duration::days(365 * default_years));
    check_range(Some(start), Some(end))?;
    if (end - start).num_days() > 366 * max_years {
        return Err(ApiError::BadRequest(format!("区间最长 {max_years} 年")));
    }
    if (end - start).num_days() < 90 {
        return Err(ApiError::BadRequest("区间至少 90 天".into()));
    }
    Ok((start, end))
}

/// 配置的可复现描述，写入运行记录并计算哈希
#[derive(Serialize, ToSchema, Clone)]
pub struct FactorConfig {
    pub factors: Vec<FactorId>,
    pub weights: Vec<f64>,
    pub neutralize: bool,
    pub exclude_small_pct: f64,
    /// 股票池的白话描述
    pub universe: Vec<String>,
    /// 检验的持有期或组合的调仓间隔（交易日）
    pub horizon: usize,
    /// 组合持有只数；因子检验为空
    pub top_n: Option<usize>,
    pub buffer: Option<f64>,
}

async fn record(
    pool: &sqlx::PgPool,
    mode: &str,
    start: NaiveDate,
    end: NaiveDate,
    manifest: &BacktestManifest,
    summary: &impl Serialize,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO ops.backtest_run (mode, start_date, end_date, definition_hash, manifest, summary)
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
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

/// 运行清单：价格口径、试验次数、覆盖范围、幸存者偏差说明
async fn research_manifest(
    s: &AppState,
    config: &FactorConfig,
    start: NaiveDate,
    end: NaiveDate,
    costs: &crate::analysis::backtest::Costs,
    kind: &str,
    trials: u64,
) -> ApiResult<BacktestManifest> {
    let mut m = manifest(&s.pool, config, start, end, costs).await?;
    m.price_basis = "forward_adjusted".into();
    m.validation_kind = kind.into();
    m.trial_count = trials;
    m.coverage = coverage(&s.pool, start, end, true, false).await?;
    m.delisted_policy =
        "survivorship_biased:no_history_before_collection;current_industry;fundamentals_at_statutory_deadline".into();
    Ok(m)
}

/// 同一模式下此前试过多少组不同配置（按配置哈希去重，含这一次），以及各组最近一次的每期 Sharpe
async fn trials(
    s: &AppState,
    mode: &str,
    hash: &str,
    sharpe_path: Option<&str>,
) -> ApiResult<(u64, Vec<f64>)> {
    let rows: Vec<(String, Option<f64>)> = sqlx::query_as(
        "SELECT DISTINCT ON (definition_hash) definition_hash, (summary #>> $2::text[])::float8
         FROM ops.backtest_run WHERE mode = $1 ORDER BY definition_hash, created_at DESC",
    )
    .bind(mode)
    .bind(sharpe_path.map(|p| p.split('.').collect::<Vec<_>>()))
    .fetch_all(&s.pool)
    .await?;
    let seen = rows.iter().any(|(h, _)| h == hash);
    let n = rows.len() as u64 + u64::from(!seen);
    Ok((
        n,
        rows.into_iter()
            .filter_map(|(h, v)| (h != hash).then_some(v).flatten())
            .collect(),
    ))
}

#[derive(Deserialize, IntoParams)]
pub struct FactorTestQuery {
    /// 逗号分隔的因子 ID，例如 `reversal,idio_vol,roe`；两个以上时额外检验等权合成
    pub factors: String,
    /// 开始日，默认结束日前 5 年；区间最长 8 年
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    /// 持有期（交易日）：5、10 或 20，默认 5；取样间隔等于持有期，前后两期不重叠
    pub horizon: Option<usize>,
    /// 分几组，3 到 10，默认 5
    pub quantiles: Option<usize>,
    /// 对行业和规模（成交额）中性化，默认是
    pub neutralize: Option<bool>,
    /// 每个取样日剔除成交额最小的百分之几，0 到 50，默认 0
    pub exclude_small_pct: Option<f64>,
    /// 只看这些板块，逗号分隔：main、chinext、star、bse；为空表示全部
    pub boards: Option<String>,
    /// 排除 ST，默认是
    pub exclude_st: Option<bool>,
    /// 最低股价（元），默认 2
    pub min_price: Option<f64>,
    /// 最低当日成交额（元），默认 5000 万
    pub min_amount: Option<f64>,
    /// 上市满多少个交易日，默认 60
    pub min_listed_days: Option<u32>,
}

#[derive(Serialize, ToSchema)]
pub struct FactorTestResult {
    pub config: FactorConfig,
    pub manifest: BacktestManifest,
    #[serde(flatten)]
    pub outcome: FactorTestOutcome,
}

// @project-doc docs/domains/factor_research.md#test
/// 因子检验：按持有期取样，统计 RankIC、分组收益、多空收益、换手、衰减和挑选偏差（PBO、Deflated Sharpe）
#[utoipa::path(get, path = "/research/factor-test", tag = "research", params(FactorTestQuery),
    responses((status = 200, description = "成功", body = FactorTestResult),
              (status = 400, description = "参数不合法，或区间内没有足够的数据", body = crate::error::ErrorBody)))]
async fn factor_test(
    State(s): State<AppState>,
    Query(q): Query<FactorTestQuery>,
) -> ApiResult<Json<FactorTestResult>> {
    let (ids, _) = parse_factors(&q.factors)?;
    let horizon = q.horizon.unwrap_or(5);
    if ![5, 10, 20].contains(&horizon) {
        return Err(ApiError::BadRequest("horizon 应为 5、10 或 20".into()));
    }
    let quantiles = q.quantiles.unwrap_or(5);
    if !(3..=10).contains(&quantiles) {
        return Err(ApiError::BadRequest("quantiles 应在 3 到 10 之间".into()));
    }
    let pool = pool_query!(q);
    let universe = pool.universe()?;
    let prep = pool.prep()?;
    let (start, end) = range(&s, q.start, q.end, 5, factor_lab::MAX_TEST_YEARS).await?;
    let spec = Spec {
        start,
        end,
        factors: ids.clone(),
        universe: universe.clone(),
        step: horizon,
    };
    let panel = factor_lab::load(&s.pool, &spec)
        .await?
        .ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    let outcome = crate::analysis::prices::run_cpu(|| {
        factor_lab::evaluate(
            &panel,
            &TestParams {
                horizon,
                quantiles,
                prep,
            },
        )
    })
    .ok_or_else(|| ApiError::BadRequest("区间太短，没有完整的持有期".into()))?;
    drop(panel);
    let config = FactorConfig {
        weights: vec![1.0; ids.len()],
        factors: ids,
        neutralize: prep.neutralize,
        exclude_small_pct: prep.exclude_small_pct,
        universe: Definition {
            universe,
            ..Definition::any_stock()
        }
        .describe_universe(),
        horizon,
        top_n: None,
        buffer: None,
    };
    let costs = crate::analysis::backtest::Costs::default();
    let hash = research::definition_hash(&config);
    let (n, _) = trials(&s, "factor_test", &hash, None).await?;
    let m = research_manifest(
        &s,
        &config,
        outcome.start,
        outcome.end,
        &costs,
        "cross_sectional_factor_test",
        n,
    )
    .await?;
    #[derive(Serialize)]
    struct Brief<'a> {
        factors: Vec<(&'a str, Option<f64>, Option<f64>)>,
        pbo: Option<f64>,
    }
    record(
        &s.pool,
        "factor_test",
        outcome.start,
        outcome.end,
        &m,
        &Brief {
            factors: outcome
                .factors
                .iter()
                .map(|f| (f.name.as_str(), f.rank_ic_mean, f.rank_ic_t))
                .collect(),
            pbo: outcome.overfit.pbo,
        },
    )
    .await?;
    Ok(Json(FactorTestResult {
        config,
        manifest: m,
        outcome,
    }))
}

#[derive(Deserialize, IntoParams)]
pub struct FactorPortfolioQuery {
    /// 逗号分隔的因子，可带权重：`reversal:1,idio_vol:1,roe:0.5`。权重作用在按预期方向统一后的得分上
    pub factors: String,
    /// 开始日，默认结束日前 3 年；区间最长 5 年
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    /// 每隔几个交易日调仓，5 到 60，默认 20（约一个月）
    pub rebalance: Option<usize>,
    /// 持有只数，5 到 100，默认 30
    pub top_n: Option<usize>,
    /// 缓冲区：已持有的股票排名在 top_n × (1 + buffer) 以内时不卖，0 到 2，默认 0.5
    pub buffer: Option<f64>,
    pub capital: Option<f64>,
    pub commission_rate: Option<f64>,
    pub min_commission: Option<f64>,
    pub stamp_tax_rate: Option<f64>,
    pub transfer_fee_rate: Option<f64>,
    /// 滑点（基点），默认 10
    pub slippage_bps: Option<f64>,
    /// 对行业和规模（成交额）中性化，默认是
    pub neutralize: Option<bool>,
    /// 每个取样日剔除成交额最小的百分之几，0 到 50，默认 0
    pub exclude_small_pct: Option<f64>,
    /// 只看这些板块，逗号分隔：main、chinext、star、bse；为空表示全部
    pub boards: Option<String>,
    /// 排除 ST，默认是
    pub exclude_st: Option<bool>,
    /// 最低股价（元），默认 2
    pub min_price: Option<f64>,
    /// 最低当日成交额（元），默认 5000 万
    pub min_amount: Option<f64>,
    /// 上市满多少个交易日，默认 60
    pub min_listed_days: Option<u32>,
}

#[derive(Serialize, ToSchema)]
pub struct FactorPortfolioResult {
    pub config: FactorConfig,
    pub costs: BacktestCosts,
    pub manifest: BacktestManifest,
    /// 考虑这类回测此前试过的配置数之后，真实 Sharpe 大于 0 的概率；通常要求高于 0.95
    pub deflated_sharpe: Option<f64>,
    #[serde(flatten)]
    pub outcome: FactorPortfolioOutcome,
}

// @project-doc docs/domains/factor_research.md#portfolio
/// 多因子组合回测：每隔 rebalance 个交易日按合成得分持有前 top_n 只，下一个交易日开盘调仓
#[utoipa::path(get, path = "/research/factor-portfolio", tag = "research", params(FactorPortfolioQuery),
    responses((status = 200, description = "成功", body = FactorPortfolioResult),
              (status = 400, description = "参数不合法，或区间内没有足够的数据", body = crate::error::ErrorBody)))]
async fn factor_portfolio(
    State(s): State<AppState>,
    Query(q): Query<FactorPortfolioQuery>,
) -> ApiResult<Json<FactorPortfolioResult>> {
    let (ids, weights) = parse_factors(&q.factors)?;
    let rebalance = q.rebalance.unwrap_or(20);
    if !(5..=60).contains(&rebalance) {
        return Err(ApiError::BadRequest("rebalance 应在 5 到 60 之间".into()));
    }
    let top_n = q.top_n.unwrap_or(30);
    if !(5..=100).contains(&top_n) {
        return Err(ApiError::BadRequest("top_n 应在 5 到 100 之间".into()));
    }
    let buffer = q.buffer.unwrap_or(0.5);
    if !buffer.is_finite() || !(0.0..=2.0).contains(&buffer) {
        return Err(ApiError::BadRequest("buffer 应在 0 到 2 之间".into()));
    }
    let cost = costs(CostOptions {
        commission_rate: q.commission_rate,
        min_commission: q.min_commission,
        stamp_tax_rate: q.stamp_tax_rate,
        transfer_fee_rate: q.transfer_fee_rate,
        dividend_tax_rate: None,
        cash_interest_rate: None,
        deal_price: Some(DealPrice::Open),
        slippage_bps: Some(q.slippage_bps.unwrap_or(10.0)),
        participation_rate: None,
    })?;
    let capital = capital(q.capital)?;
    let pool = pool_query!(q);
    let universe = pool.universe()?;
    let prep = pool.prep()?;
    let (start, end) = range(&s, q.start, q.end, 3, factor_lab::MAX_PORTFOLIO_YEARS).await?;
    let spec = Spec {
        start,
        end,
        factors: ids.clone(),
        universe: universe.clone(),
        step: rebalance,
    };
    let names: HashMap<String, String> = market::securities(&s.pool).await?.names;
    let panel = factor_lab::load(&s.pool, &spec)
        .await?
        .ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    let params = PortfolioParams {
        weights: weights.clone(),
        prep,
        top_n,
        buffer,
        capital,
        costs: cost,
    };
    let outcome =
        crate::analysis::prices::run_cpu(|| factor_lab::simulate(&panel, &params, &names))
            .ok_or_else(|| ApiError::BadRequest("所选区间内没有日线".into()))?;
    drop(panel);
    let config = FactorConfig {
        factors: ids,
        weights,
        neutralize: prep.neutralize,
        exclude_small_pct: prep.exclude_small_pct,
        universe: Definition {
            universe,
            ..Definition::any_stock()
        }
        .describe_universe(),
        horizon: rebalance,
        top_n: Some(top_n),
        buffer: Some(buffer),
    };
    let hash = research::definition_hash(&config);
    let (n, others) = trials(&s, "factor_portfolio", &hash, Some("risk.sharpe")).await?;
    // 运行记录里是年化 Sharpe，换成每日 Sharpe 再算各组之间的方差
    let daily: Vec<f64> = others.iter().map(|v| v / 252.0_f64.sqrt()).collect();
    let returns: Vec<f64> = outcome
        .equity
        .windows(2)
        .map(|w| w[1].equity / w[0].equity - 1.0)
        .collect();
    let mut all = daily.clone();
    if let Some(sr) = outcome.summary.risk.sharpe {
        all.push(sr / 252.0_f64.sqrt());
    }
    let var = (all.len() >= 2).then(|| {
        let m = all.iter().sum::<f64>() / all.len() as f64;
        all.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (all.len() - 1) as f64
    });
    let dsr = research::deflated_sharpe(&returns, n, var);
    let m = research_manifest(
        &s,
        &config,
        outcome.summary.start,
        outcome.summary.end,
        &cost,
        "factor_portfolio_deflated_sharpe",
        n,
    )
    .await?;
    record(
        &s.pool,
        "factor_portfolio",
        outcome.summary.start,
        outcome.summary.end,
        &m,
        &outcome.summary,
    )
    .await?;
    Ok(Json(FactorPortfolioResult {
        config,
        costs: costs_echo(&cost),
        manifest: m,
        deflated_sharpe: dsr,
        outcome,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_factor_list_with_weights() {
        let (ids, w) = parse_factors("reversal, idio_vol:0.5,roe:-1").unwrap();
        assert_eq!(
            ids,
            vec![FactorId::Reversal, FactorId::IdioVol, FactorId::Roe]
        );
        assert_eq!(w, vec![1.0, 0.5, -1.0]);
        assert!(parse_factors("").is_err());
        assert!(parse_factors("nope").is_err());
        assert!(parse_factors("reversal,reversal").is_err());
        assert!(parse_factors("reversal:abc").is_err());
        assert!(parse_factors("reversal:0").is_err());
    }
}
