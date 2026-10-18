//! 账户、持仓、流水和每日调仓建议。账户只属于当前用户：每个查询都按当前用户的编号过滤，
//! 别人的账户（包括管理员看普通用户的）一律返回 404。规则见 docs/domains/accounts.md。

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::auth::CurrentUser;
use crate::account::paste::{self, Known, ParsedRow};
use crate::account::rules::{Preset, Rules};
use crate::account::{self, Account, AccountError, AccountUpdate, NewAccount, Position, PositionInput, Trade, TradeInput, TradeKind, TradeSource};
use crate::advice::backtest::{self, RulesBacktest};
use crate::advice::generate::Action;
use crate::advice::review::{self, Review};
use crate::advice::stats::{self, StrategyStat};
use crate::advice::{self, AdviceDay, StoredAdvice, inputs};
use crate::analysis::indicators::{Frame, IndicatorId, IndicatorRef};
use crate::analysis::{prices, screener};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(rules_guide))
        .routes(routes!(list_accounts, create_account))
        .routes(routes!(get_account, update_account, delete_account))
        .routes(routes!(mark_checked))
        .routes(routes!(parse_positions))
        .routes(routes!(calibrate))
        .routes(routes!(list_trades, add_trade))
        .routes(routes!(current_advice))
        .routes(routes!(refresh_advice))
        .routes(routes!(advice_history))
        .routes(routes!(execute_item))
        .routes(routes!(skip_item, clear_item))
        .routes(routes!(account_review))
        .routes(routes!(account_backtest))
}

impl From<AccountError> for ApiError {
    fn from(e: AccountError) -> Self {
        match e {
            AccountError::Invalid(m) => ApiError::BadRequest(m),
            AccountError::NotFound => ApiError::NotFound(e.to_string()),
            AccountError::Taken(_) => ApiError::Conflict(e.to_string()),
            AccountError::Internal(e) => ApiError::Internal(e),
        }
    }
}

/// 一个参考策略：过去表现和是否算作有效策略
#[derive(Serialize, ToSchema)]
pub struct StrategyQuality {
    pub strategy_id: i64,
    pub name: String,
    pub builtin: bool,
    /// 过去 3 年的表现；还没统计时为空
    pub stat: Option<StrategyStat>,
    /// 买点之后 20 日平均好于随便买；还没统计时为空
    pub effective: Option<bool>,
}

/// 一组参考值：适合什么样的账户、怎么设计、过去 3 年的回测
#[derive(Serialize, ToSchema)]
pub struct PresetInfo {
    pub preset: Preset,
    pub name: String,
    pub tagline: String,
    /// 这类账户一般怎么设计，每条说明一个参数为什么这样取
    pub design: Vec<String>,
    pub rules: Rules,
    /// 按这组规则过去 3 年的回测；还没算时为空
    pub backtest: Option<RulesBacktest>,
}

#[derive(Serialize, ToSchema)]
pub struct RulesGuide {
    pub presets: Vec<PresetInfo>,
    /// 全部参加巡检的策略，过去表现好的在前
    pub strategies: Vec<StrategyQuality>,
}

/// 设置规则时的辅助：三组参考值（保守型、稳健型、进取型）和参考策略的过去表现
#[utoipa::path(get, path = "/rules/guide", tag = "accounts", responses((status = 200, description = "成功", body = RulesGuide)))]
async fn rules_guide(State(s): State<AppState>) -> ApiResult<Json<RulesGuide>> {
    let list: Vec<(i64, String, bool)> =
        sqlx::query_as("SELECT strategy_id, name, builtin_key IS NOT NULL FROM app.strategy WHERE screening ORDER BY strategy_id").fetch_all(&s.pool).await?;
    let stats = stats::load(&s.pool, &list.iter().map(|x| x.0).collect::<Vec<_>>()).await?;
    let mut strategies: Vec<StrategyQuality> = list
        .into_iter()
        .map(|(strategy_id, name, builtin)| {
            let stat = stats.get(&strategy_id).cloned();
            let effective = stat.as_ref().and_then(StrategyStat::excess20_pct).map(|e| e > 0.0);
            StrategyQuality { strategy_id, name, builtin, stat, effective }
        })
        .collect();
    let ex = |q: &StrategyQuality| q.stat.as_ref().and_then(StrategyStat::excess20_pct).unwrap_or(f64::MIN);
    strategies.sort_by(|a, b| ex(b).total_cmp(&ex(a)));
    let tests = backtest::load_presets(&s.pool).await?;
    let presets = Preset::ALL
        .into_iter()
        .map(|p| PresetInfo {
            preset: p,
            name: p.name().into(),
            tagline: p.tagline().into(),
            design: p.design().into_iter().map(String::from).collect(),
            rules: p.rules(),
            backtest: tests.get(p.key()).cloned(),
        })
        .collect();
    Ok(Json(RulesGuide { presets, strategies }))
}

/// 一份建议的摘要（账户列表和今日页用）
#[derive(Serialize, ToSchema)]
pub struct AdviceBrief {
    pub trade_date: NaiveDate,
    pub next_date: NaiveDate,
    pub headline: String,
    pub sells: usize,
    pub reduces: usize,
    pub buys: usize,
    /// 卖出、减仓、买入中还没处理（执行或不做）的条数
    pub pending: usize,
}

#[derive(Serialize, ToSchema)]
pub struct AccountSummary {
    pub account: Account,
    pub positions: usize,
    pub market_value: f64,
    pub total_assets: f64,
    /// 按最近一个交易日的涨跌估算的今日盈亏
    pub day_pnl: f64,
    pub advice: Option<AdviceBrief>,
    /// 持仓很久没变动、又有没执行的建议时，提醒核对
    pub check_hint: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PositionView {
    pub position: Position,
    pub name: String,
    pub board: String,
    /// 最近一根日线的收盘价（未复权）和日期；没有日线时为空
    pub close: Option<f64>,
    pub last_date: Option<NaiveDate>,
    /// 最近一个交易日的涨跌（%）
    pub change_pct: Option<f64>,
    pub market_value: f64,
    pub weight_pct: f64,
    /// 浮动盈亏（元、%）；没填成本价时为空
    pub pnl: Option<f64>,
    pub pnl_pct: Option<f64>,
    /// 按本账户止损线算的止损价
    pub stop_price: Option<f64>,
    /// 本账户趋势均线的值，以及收盘价相对它的位置（%）
    pub trend_ma: Option<f64>,
    pub trend_gap_pct: Option<f64>,
    /// 买入依据的策略
    pub strategy_name: Option<String>,
    pub delisted: bool,
}

#[derive(Serialize, ToSchema)]
pub struct AccountDetail {
    pub summary: AccountSummary,
    pub positions: Vec<PositionView>,
}

/// 最近两根日线：(日期, 收盘价, 涨跌 %)
async fn quote(s: &AppState, code: &str, n: i64) -> ApiResult<(Vec<prices::DailyBar>, Option<(NaiveDate, f64, Option<f64>)>)> {
    let bars = prices::recent_bars(&s.pool, code, n).await?;
    let q = bars.last().map(|b| {
        let ch = bars.len().checked_sub(2).map(|i| (b.q_close / bars[i].q_close - 1.0) * 100.0).filter(|v| v.is_finite());
        (b.date, b.close, ch)
    });
    Ok((bars, q))
}

async fn brief(s: &AppState, account_id: i64) -> ApiResult<Option<AdviceBrief>> {
    let Some(a) = advice::load(&s.pool, account_id, None).await? else { return Ok(None) };
    let count = |x| a.advice.items.iter().filter(|i| i.action == x).count();
    let pending = a.advice.items.iter().filter(|i| matches!(i.action, Action::Sell | Action::Reduce | Action::Buy) && !a.states.contains_key(&i.key)).count();
    Ok(Some(AdviceBrief {
        trade_date: a.trade_date,
        next_date: a.advice.next_date,
        headline: a.advice.headline.clone(),
        sells: count(Action::Sell),
        reduces: count(Action::Reduce),
        buys: count(Action::Buy),
        pending,
    }))
}

/// 持仓 10 个交易日（约两周）没有任何变动、期间的建议又没执行时，提醒和券商核对一次
async fn check_hint(s: &AppState, acc: &Account) -> ApiResult<Option<String>> {
    let last = account::last_change(&s.pool, acc.account_id).await?.max(acc.checked_at).unwrap_or(acc.created_at);
    if Utc::now() - last < chrono::Duration::days(14) {
        return Ok(None);
    }
    let unexecuted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM app.advice a WHERE a.account_id = $1 AND a.superseded_by IS NULL AND a.created_at > $2
           AND NOT EXISTS (SELECT 1 FROM app.advice_item_state s WHERE s.advice_id = a.advice_id)
           AND jsonb_path_exists(a.result, '$.items[*] ? (@.action == \"sell\" || @.action == \"buy\" || @.action == \"reduce\")')",
    )
    .bind(acc.account_id)
    .bind(last)
    .fetch_one(&s.pool)
    .await?;
    Ok((unexecuted > 0).then(|| "持仓两周多没有变动，期间的建议也都没有处理。持仓可能和券商不一致，花一分钟核对一下".to_string()))
}

async fn summary(s: &AppState, acc: Account) -> ApiResult<AccountSummary> {
    let positions = account::positions(&s.pool, acc.account_id).await?;
    let (mut mv, mut day) = (0.0, 0.0);
    for p in &positions {
        if let (_, Some((_, close, ch))) = quote(s, &p.thscode, 2).await? {
            let v = p.shares as f64 * close;
            mv += v;
            if let Some(c) = ch {
                day += v - v / (1.0 + c / 100.0);
            }
        }
    }
    Ok(AccountSummary {
        positions: positions.len(),
        market_value: mv,
        total_assets: acc.cash + mv,
        day_pnl: day,
        advice: brief(s, acc.account_id).await?,
        check_hint: check_hint(s, &acc).await?,
        account: acc,
    })
}

async fn detail(s: &AppState, me: &CurrentUser, id: i64) -> ApiResult<AccountDetail> {
    let acc = account::get(&s.pool, me.user_id, id).await?;
    let params = acc.rules.clone();
    let positions = account::positions(&s.pool, id).await?;
    let names: HashMap<String, (String, String, bool)> = sqlx::query_as::<_, (String, String, String, bool)>(
        "SELECT thscode, name, board, delisted_on IS NOT NULL FROM ref.security WHERE thscode = ANY($1)",
    )
    .bind(positions.iter().map(|p| p.thscode.clone()).collect::<Vec<_>>())
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|(c, n, b, d)| (c, (n, b, d)))
    .collect();
    let strategy_names: HashMap<i64, String> = sqlx::query_as::<_, (i64, String)>("SELECT strategy_id, name FROM app.strategy").fetch_all(&s.pool).await?.into_iter().collect();
    let mut views = vec![];
    for p in positions {
        let (bars, q) = quote(s, &p.thscode, params.trend_ma as i64 + 5).await?;
        let trend_ma = (!bars.is_empty()).then(|| Frame::new(&bars, None).at(IndicatorRef::new(IndicatorId::Ma, Some(params.trend_ma)), bars.len() - 1)).filter(|v| v.is_finite());
        let close = q.map(|q| q.1);
        let mv = close.map_or(0.0, |c| c * p.shares as f64);
        let (name, board, delisted) = names.get(&p.thscode).cloned().unwrap_or_else(|| (p.thscode.clone(), "main".into(), false));
        views.push(PositionView {
            name,
            board,
            close,
            last_date: q.map(|q| q.0),
            change_pct: q.and_then(|q| q.2),
            market_value: mv,
            weight_pct: 0.0,
            pnl: p.cost_price.zip(close).map(|(c, x)| (x - c) * p.shares as f64),
            pnl_pct: p.cost_price.zip(close).map(|(c, x)| (x / c - 1.0) * 100.0),
            stop_price: p.cost_price.map(|c| c * (1.0 - params.stop_loss_pct / 100.0)),
            trend_gap_pct: trend_ma.zip(close).map(|(m, c)| (c / m - 1.0) * 100.0),
            trend_ma,
            strategy_name: p.strategy_id.and_then(|id| strategy_names.get(&id).cloned()),
            delisted,
            position: p,
        });
    }
    let summary = summary(s, acc).await?;
    for v in &mut views {
        v.weight_pct = v.market_value / summary.total_assets.max(1.0) * 100.0;
    }
    views.sort_by(|a, b| b.market_value.total_cmp(&a.market_value));
    Ok(AccountDetail { summary, positions: views })
}

/// 改了持仓、现金或设置后重新生成操作计划；失败只写日志，不影响这次修改
async fn regenerate(s: &AppState, me: &CurrentUser, id: i64) {
    let r = async {
        let acc = account::get(&s.pool, me.user_id, id).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        advice::regenerate(&s.pool, &acc).await
    }
    .await;
    if let Err(e) = r {
        tracing::error!(account_id = id, error = %format!("{e:#}"), "重新生成建议失败");
    }
}

/// 我的账户
#[utoipa::path(get, path = "/accounts", tag = "accounts", responses((status = 200, description = "成功", body = [AccountSummary])))]
async fn list_accounts(State(s): State<AppState>, me: CurrentUser) -> ApiResult<Json<Vec<AccountSummary>>> {
    let mut out = vec![];
    for acc in account::list(&s.pool, me.user_id).await? {
        out.push(summary(&s, acc).await?);
    }
    Ok(Json(out))
}

/// 新建账户：名字、现在的现金，规则用稳健型的参考值（之后在高级设置里改）。持仓之后添加或粘贴
#[utoipa::path(post, path = "/accounts", tag = "accounts", request_body = NewAccount,
    responses((status = 201, description = "已创建", body = AccountDetail),
              (status = 400, description = "名字不合法、现金为负或账户太多", body = crate::error::ErrorBody),
              (status = 409, description = "已经有同名账户", body = crate::error::ErrorBody)))]
async fn create_account(State(s): State<AppState>, me: CurrentUser, Json(input): Json<NewAccount>) -> ApiResult<(StatusCode, Json<AccountDetail>)> {
    let acc = account::create(&s.pool, me.user_id, &input).await?;
    regenerate(&s, &me, acc.account_id).await;
    Ok((StatusCode::CREATED, Json(detail(&s, &me, acc.account_id).await?)))
}

/// 账户详情：持仓带现价、市值、占比、盈亏、止损价和趋势均线
#[utoipa::path(get, path = "/accounts/{account_id}", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 200, description = "成功", body = AccountDetail), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn get_account(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>) -> ApiResult<Json<AccountDetail>> {
    Ok(Json(detail(&s, &me, id).await?))
}

/// 修改名字、选股和风控规则、费率、备注；之后重新生成操作计划
#[utoipa::path(put, path = "/accounts/{account_id}", tag = "accounts", params(("account_id" = i64, Path)), request_body = AccountUpdate,
    responses((status = 200, description = "已保存", body = AccountDetail),
              (status = 400, description = "参数不合法", body = crate::error::ErrorBody),
              (status = 404, description = "不存在", body = crate::error::ErrorBody),
              (status = 409, description = "已经有同名账户", body = crate::error::ErrorBody)))]
async fn update_account(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Json(input): Json<AccountUpdate>) -> ApiResult<Json<AccountDetail>> {
    account::update(&s.pool, me.user_id, id, &input).await?;
    regenerate(&s, &me, id).await;
    Ok(Json(detail(&s, &me, id).await?))
}

/// 删除账户，连同持仓、流水和历史建议
#[utoipa::path(delete, path = "/accounts/{account_id}", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 204, description = "已删除"), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn delete_account(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    account::delete(&s.pool, me.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 确认持仓和券商一致（只记时间，用于核对提醒）
#[utoipa::path(post, path = "/accounts/{account_id}/checked", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 204, description = "已记下"), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn mark_checked(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    account::mark_checked(&s.pool, me.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct PasteInput {
    /// 从券商 App 复制的持仓文字
    pub text: String,
}

/// 识别粘贴的持仓文字，只返回预览，不保存
#[utoipa::path(post, path = "/accounts/{account_id}/positions/parse", tag = "accounts", params(("account_id" = i64, Path)), request_body = PasteInput,
    responses((status = 200, description = "识别结果", body = [ParsedRow]), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn parse_positions(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Json(input): Json<PasteInput>) -> ApiResult<Json<Vec<ParsedRow>>> {
    account::get(&s.pool, me.user_id, id).await?;
    if input.text.chars().count() > 20_000 {
        return Err(ApiError::BadRequest("粘贴的文字太长".into()));
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT thscode, ticker, name FROM ref.security WHERE delisted_on IS NULL").fetch_all(&s.pool).await?;
    let known: Vec<Known> = rows.iter().map(|(c, t, n)| Known { thscode: c, ticker: t, name: n }).collect();
    Ok(Json(paste::parse(&input.text, &known)))
}

#[derive(Deserialize, ToSchema)]
pub struct CalibrateInput {
    /// 现在的现金
    pub cash: f64,
    /// 现在的全部持仓：没列出的视为已经卖掉
    pub positions: Vec<PositionInput>,
}

/// 校准：把持仓和现金整体换成填的（照着券商 App 抄一遍）；和原来不同的地方各记一条流水，之后重新生成建议
#[utoipa::path(put, path = "/accounts/{account_id}/positions", tag = "accounts", params(("account_id" = i64, Path)), request_body = CalibrateInput,
    responses((status = 200, description = "已保存", body = AccountDetail),
              (status = 400, description = "代码不存在、股数不对或重复", body = crate::error::ErrorBody),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn calibrate(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Json(input): Json<CalibrateInput>) -> ApiResult<Json<AccountDetail>> {
    account::calibrate(&s.pool, me.user_id, id, input.cash, &input.positions).await?;
    regenerate(&s, &me, id).await;
    Ok(Json(detail(&s, &me, id).await?))
}

#[derive(Deserialize, IntoParams)]
pub struct TradesQuery {
    /// 默认 100，最多 500
    pub limit: Option<i64>,
}

/// 流水：每一次持仓和现金的变动，新的在前
#[utoipa::path(get, path = "/accounts/{account_id}/trades", tag = "accounts", params(("account_id" = i64, Path), TradesQuery),
    responses((status = 200, description = "成功", body = [Trade]), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn list_trades(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Query(q): Query<TradesQuery>) -> ApiResult<Json<Vec<Trade>>> {
    Ok(Json(account::trades(&s.pool, me.user_id, id, q.limit.unwrap_or(100).clamp(1, 500)).await?))
}

/// 手动记一笔：买入、卖出（按成交价和股数，费用不填时按账户费率估算）、转入或转出现金；之后重新生成建议
#[utoipa::path(post, path = "/accounts/{account_id}/trades", tag = "accounts", params(("account_id" = i64, Path)), request_body = TradeInput,
    responses((status = 200, description = "已记下", body = AccountDetail),
              (status = 400, description = "现金不够、股数不对等", body = crate::error::ErrorBody),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn add_trade(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Json(input): Json<TradeInput>) -> ApiResult<Json<AccountDetail>> {
    account::record(&s.pool, me.user_id, id, &input, TradeSource::User, None).await?;
    regenerate(&s, &me, id).await;
    Ok(Json(detail(&s, &me, id).await?))
}

#[derive(Serialize, ToSchema)]
pub struct AdviceResponse {
    /// 没有建议时的原因，例如还没有巡检结果
    pub waiting: Option<String>,
    pub advice: Option<StoredAdvice>,
    /// 最新日线的日期：比建议的交易日新时，说明新的操作计划还在等巡检
    pub latest_bar: Option<NaiveDate>,
}

#[derive(Deserialize, IntoParams)]
pub struct AdviceQuery {
    /// 交易日；默认最新一份
    pub date: Option<NaiveDate>,
}

async fn advice_response(s: &AppState, id: i64, date: Option<NaiveDate>) -> ApiResult<AdviceResponse> {
    let advice = advice::load(&s.pool, id, date).await?;
    let latest_bar = crate::analysis::market::resolve_date(&s.pool, None).await?;
    let waiting = match (&advice, inputs::latest_screen_date(&s.pool).await?) {
        (Some(_), _) => None,
        (None, None) => Some("内置策略还没有巡检结果：数据同步、巡检完成后自动生成（新部署的系统要等首次同步）".into()),
        (None, Some(_)) => Some("正在生成，稍等一分钟后刷新".into()),
    };
    Ok(AdviceResponse { waiting, advice, latest_bar })
}

/// 当前的建议（某个交易日的最终一份），以及每条的处理状态
#[utoipa::path(get, path = "/accounts/{account_id}/advice", tag = "accounts", params(("account_id" = i64, Path), AdviceQuery),
    responses((status = 200, description = "成功", body = AdviceResponse), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn current_advice(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Query(q): Query<AdviceQuery>) -> ApiResult<Json<AdviceResponse>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(advice_response(&s, id, q.date).await?))
}

/// 按现在的持仓和最新的巡检结果重新生成操作计划
#[utoipa::path(post, path = "/accounts/{account_id}/advice/refresh", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 200, description = "已重新生成", body = AdviceResponse), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn refresh_advice(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>) -> ApiResult<Json<AdviceResponse>> {
    let acc = account::get(&s.pool, me.user_id, id).await?;
    advice::regenerate(&s.pool, &acc).await?;
    Ok(Json(advice_response(&s, id, None).await?))
}

#[derive(Deserialize, IntoParams)]
pub struct HistoryQuery {
    /// 默认 60，最多 250
    pub limit: Option<i64>,
}

/// 历史建议：每个交易日一行，新的在前
#[utoipa::path(get, path = "/accounts/{account_id}/advice/history", tag = "accounts", params(("account_id" = i64, Path), HistoryQuery),
    responses((status = 200, description = "成功", body = [AdviceDay]), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn advice_history(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Query(q): Query<HistoryQuery>) -> ApiResult<Json<Vec<AdviceDay>>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(advice::history(&s.pool, id, q.limit.unwrap_or(60).clamp(1, 250)).await?))
}

#[derive(Deserialize, ToSchema, Default)]
pub struct ExecuteInput {
    /// 实际成交的股数；默认按建议
    #[serde(default)]
    pub shares: Option<i64>,
    /// 实际成交价；默认按参考价
    #[serde(default)]
    pub price: Option<f64>,
    /// 实际费用；默认按账户费率估算
    #[serde(default)]
    pub fee: Option<f64>,
    /// 成交日；默认今天
    #[serde(default)]
    pub trade_date: Option<NaiveDate>,
}

/// 已执行：按实际成交记一笔买入或卖出，持仓和现金跟着更新。这份建议不重新生成，其余条目照常
#[utoipa::path(post, path = "/accounts/{account_id}/advice/{advice_id}/items/{key}/execute", tag = "accounts",
    params(("account_id" = i64, Path), ("advice_id" = i64, Path), ("key" = String, Path, description = "例：buy-601012.SH")), request_body = ExecuteInput,
    responses((status = 200, description = "已记下", body = AdviceResponse),
              (status = 400, description = "这条不能执行、已经执行过，或现金、股数不够", body = crate::error::ErrorBody),
              (status = 404, description = "账户、建议或条目不存在", body = crate::error::ErrorBody)))]
async fn execute_item(
    State(s): State<AppState>,
    me: CurrentUser,
    Path((id, advice_id, key)): Path<(i64, i64, String)>,
    Json(input): Json<ExecuteInput>,
) -> ApiResult<Json<AdviceResponse>> {
    account::get(&s.pool, me.user_id, id).await?;
    let a = advice::load_by_id(&s.pool, id, advice_id).await?.ok_or_else(|| ApiError::NotFound("建议不存在".into()))?;
    let item = a.items.iter().find(|i| i.key == key).ok_or_else(|| ApiError::NotFound("没有这一条".into()))?;
    let kind = match item.action {
        Action::Buy => TradeKind::Buy,
        Action::Sell | Action::Reduce => TradeKind::Sell,
        _ => return Err(ApiError::BadRequest("这一条不需要操作".into())),
    };
    let done: Option<String> = sqlx::query_scalar("SELECT state FROM app.advice_item_state WHERE advice_id = $1 AND item_key = $2")
        .bind(advice_id)
        .bind(&key)
        .fetch_optional(&s.pool)
        .await?;
    if done.as_deref() == Some("executed") {
        return Err(ApiError::BadRequest("这一条已经记过了".into()));
    }
    let trade = TradeInput {
        kind,
        trade_date: input.trade_date,
        thscode: Some(item.thscode.clone()),
        shares: Some(input.shares.unwrap_or(item.shares)),
        price: Some(input.price.unwrap_or(item.ref_price)),
        amount: None,
        fee: input.fee,
        note: format!("按 {} 的建议：{}", a.trade_date, item.headline),
    };
    let strategy = if kind == TradeKind::Buy { item.strategy_id } else { None };
    let trade_id = account::record(&s.pool, me.user_id, id, &trade, TradeSource::Advice, strategy).await?;
    sqlx::query(
        "INSERT INTO app.advice_item_state (advice_id, item_key, state, trade_id) VALUES ($1, $2, 'executed', $3)
         ON CONFLICT (advice_id, item_key) DO UPDATE SET state = 'executed', trade_id = EXCLUDED.trade_id, updated_at = now()",
    )
    .bind(advice_id)
    .bind(&key)
    .bind(trade_id)
    .execute(&s.pool)
    .await?;
    Ok(Json(advice_response(&s, id, Some(a.trade_date)).await?))
}

/// 不做这一条（之后可以撤销）
#[utoipa::path(post, path = "/accounts/{account_id}/advice/{advice_id}/items/{key}/skip", tag = "accounts",
    params(("account_id" = i64, Path), ("advice_id" = i64, Path), ("key" = String, Path)),
    responses((status = 200, description = "已记下", body = AdviceResponse), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn skip_item(State(s): State<AppState>, me: CurrentUser, Path((id, advice_id, key)): Path<(i64, i64, String)>) -> ApiResult<Json<AdviceResponse>> {
    account::get(&s.pool, me.user_id, id).await?;
    let a = advice::load_by_id(&s.pool, id, advice_id).await?.ok_or_else(|| ApiError::NotFound("建议不存在".into()))?;
    if !a.items.iter().any(|i| i.key == key) {
        return Err(ApiError::NotFound("没有这一条".into()));
    }
    sqlx::query("INSERT INTO app.advice_item_state (advice_id, item_key, state) VALUES ($1, $2, 'skipped') ON CONFLICT (advice_id, item_key) DO NOTHING")
        .bind(advice_id)
        .bind(&key)
        .execute(&s.pool)
        .await?;
    Ok(Json(advice_response(&s, id, Some(a.trade_date)).await?))
}

/// 撤销「不做」。已执行的不能撤销：记错了在持仓页校准
#[utoipa::path(delete, path = "/accounts/{account_id}/advice/{advice_id}/items/{key}/skip", tag = "accounts",
    params(("account_id" = i64, Path), ("advice_id" = i64, Path), ("key" = String, Path)),
    responses((status = 200, description = "已撤销", body = AdviceResponse), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn clear_item(State(s): State<AppState>, me: CurrentUser, Path((id, advice_id, key)): Path<(i64, i64, String)>) -> ApiResult<Json<AdviceResponse>> {
    account::get(&s.pool, me.user_id, id).await?;
    let date: Option<NaiveDate> =
        sqlx::query_scalar("SELECT trade_date FROM app.advice WHERE account_id = $1 AND advice_id = $2").bind(id).bind(advice_id).fetch_optional(&s.pool).await?;
    let date = date.ok_or_else(|| ApiError::NotFound("建议不存在".into()))?;
    sqlx::query("DELETE FROM app.advice_item_state WHERE advice_id = $1 AND item_key = $2 AND state = 'skipped'").bind(advice_id).bind(&key).execute(&s.pool).await?;
    Ok(Json(advice_response(&s, id, Some(date)).await?))
}

/// 过去的建议之后 5、10、20 个交易日的实际涨跌
#[utoipa::path(get, path = "/accounts/{account_id}/review", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 200, description = "成功", body = Review), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn account_review(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>) -> ApiResult<Json<Review>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(review::review(&s.pool, id).await?))
}

#[derive(Deserialize, IntoParams)]
pub struct BacktestQuery {
    /// 开始日期，默认今年 1 月 1 日
    pub start: Option<NaiveDate>,
    /// 结束日期，默认最新交易日；晚于最新交易日时取最新交易日
    pub end: Option<NaiveDate>,
}

/// 用这个账户的规则回测一段时间（两头都包含），默认今年以来，最长 5 年。全市场计算，要几十秒到一两分钟
#[utoipa::path(post, path = "/accounts/{account_id}/backtest", tag = "accounts", params(("account_id" = i64, Path), BacktestQuery),
    responses((status = 200, description = "成功", body = RulesBacktest),
              (status = 400, description = "区间不对，或本地还没有日线", body = crate::error::ErrorBody),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn account_backtest(State(s): State<AppState>, me: CurrentUser, Path(id): Path<i64>, Query(q): Query<BacktestQuery>) -> ApiResult<Json<RulesBacktest>> {
    let acc = account::get(&s.pool, me.user_id, id).await?;
    let latest = crate::analysis::market::resolve_date(&s.pool, None).await?.ok_or_else(|| ApiError::BadRequest("本地还没有日线".into()))?;
    let (start, end) = backtest::resolve_range(latest, q.start, q.end).map_err(ApiError::BadRequest)?;
    let defs = screener::screening_strategies(&s.pool).await?;
    backtest::run(&s.pool, &acc.rules, &defs, start, end).await?.map(Json).ok_or_else(|| ApiError::BadRequest("本地还没有足够的日线，或勾选的策略都没有参加巡检".into()))
}
