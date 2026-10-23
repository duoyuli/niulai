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
use crate::account::corp::PendingRights;
use crate::account::equity::EquityCurve;
use crate::account::paste::{self, Known, ParsedRow};
use crate::account::valuation::{self, Valuation};
use crate::account::{
    self, Account, AccountError, AccountUpdate, NewAccount, Position, PositionInput, Trade,
    TradeInput, TradeSource,
};
use crate::advice::generate::Action;
use crate::advice::handling::{self, Fill, HandleError};
use crate::advice::review::{self, Review};
use crate::advice::{self, AdviceDay, StoredAdvice};
use crate::error::{ApiError, ApiResult};
use crate::instrument::AssetType;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_accounts, create_account))
        .routes(routes!(get_account, update_account, delete_account))
        .routes(routes!(mark_checked))
        .routes(routes!(parse_positions))
        .routes(routes!(calibrate))
        .routes(routes!(list_trades, add_trade))
        .routes(routes!(undo_trade))
        .routes(routes!(list_corp_actions))
        .routes(routes!(subscribe_rights))
        .routes(routes!(ignore_rights))
        .routes(routes!(get_equity))
        .routes(routes!(current_advice))
        .routes(routes!(refresh_advice))
        .routes(routes!(advice_history))
        .routes(routes!(shadow_orders))
        .routes(routes!(execute_item))
        .routes(routes!(skip_item, clear_item))
        .routes(routes!(account_review))
}

impl From<HandleError> for ApiError {
    fn from(e: HandleError) -> Self {
        match e {
            HandleError::NotFound(m) => ApiError::NotFound(m),
            HandleError::Invalid(m) => ApiError::BadRequest(m),
            HandleError::Account(e) => e.into(),
            HandleError::Internal(e) => ApiError::Internal(e),
        }
    }
}

impl From<AccountError> for ApiError {
    fn from(e: AccountError) -> Self {
        match e {
            AccountError::Invalid(m) => ApiError::BadRequest(m),
            AccountError::NotFound | AccountError::TradeNotFound => {
                ApiError::NotFound(e.to_string())
            }
            AccountError::Taken(_) => ApiError::Conflict(e.to_string()),
            AccountError::Internal(e) => ApiError::Internal(e),
        }
    }
}

/// 一份建议的摘要（账户列表和首页用）
#[derive(Serialize, ToSchema)]
pub struct AdviceBrief {
    pub trade_date: NaiveDate,
    pub next_date: NaiveDate,
    pub headline: String,
    pub sells: usize,
    pub buys: usize,
    /// 卖出、买入中还没处理（执行或不做）的条数
    pub pending: usize,
}

#[derive(Serialize, ToSchema)]
pub struct AccountSummary {
    pub account: Account,
    pub positions: usize,
    pub market_value: f64,
    pub total_assets: f64,
    /// 当日盈亏：最新交易日有日线的持仓按当天的涨跌算，停牌的记 0（口径与每日净值相同）
    pub day_pnl: f64,
    /// 已实现盈亏（元）：已经卖出的部分，按卖出时的平均成本算，含买卖费用；有卖出没填成本价时为空
    pub realized_pnl: Option<f64>,
    pub advice: Option<AdviceBrief>,
    /// 持仓很久没变动、又有没执行的建议时，提醒核对
    pub check_hint: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PositionView {
    pub position: Position,
    pub name: String,
    pub board: String,
    pub asset_type: AssetType,
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
    pub delisted: bool,
}

#[derive(Serialize, ToSchema)]
pub struct AccountDetail {
    pub summary: AccountSummary,
    pub positions: Vec<PositionView>,
}

async fn brief(s: &AppState, account_id: i64) -> ApiResult<Option<AdviceBrief>> {
    let Some(a) = advice::load(&s.pool, account_id, None).await? else {
        return Ok(None);
    };
    let count = |x| a.advice.items.iter().filter(|i| i.action == x).count();
    let pending = a
        .advice
        .items
        .iter()
        .filter(|i| advice::actionable(i.action) && !a.states.contains_key(&i.key))
        .count();
    Ok(Some(AdviceBrief {
        trade_date: a.trade_date,
        next_date: a.advice.next_date,
        headline: a.advice.headline.clone(),
        sells: count(Action::Sell),
        buys: count(Action::Buy),
        pending,
    }))
}

/// 持仓 10 个交易日（约两周）没有任何变动、期间的建议又没执行时，提醒和券商核对一次
async fn check_hint(s: &AppState, acc: &Account) -> ApiResult<Option<String>> {
    let last = account::last_change(&s.pool, acc.account_id)
        .await?
        .max(acc.checked_at)
        .unwrap_or(acc.created_at);
    if Utc::now() - last < chrono::Duration::days(14) {
        return Ok(None);
    }
    let unhandled = advice::has_unhandled_since(&s.pool, acc.account_id, last).await?;
    Ok(unhandled.then(|| {
        "持仓两周多没有变动，期间的建议也都没有处理。持仓可能和券商不一致，花一分钟核对一下"
            .to_string()
    }))
}

/// 账户摘要；持仓和估值由调用方读好传进来，详情页不用再读一遍
async fn summary(
    s: &AppState,
    acc: Account,
    positions: &[Position],
    value: &Valuation,
) -> ApiResult<AccountSummary> {
    let realized = account::realized(&s.pool, acc.account_id).await?;
    Ok(AccountSummary {
        positions: positions.len(),
        market_value: value.market_value,
        total_assets: acc.cash + value.market_value,
        day_pnl: value.day_pnl,
        realized_pnl: realized.total,
        advice: brief(s, acc.account_id).await?,
        check_hint: check_hint(s, &acc).await?,
        account: acc,
    })
}

async fn summary_of(s: &AppState, acc: Account) -> ApiResult<AccountSummary> {
    let positions = account::positions(&s.pool, acc.account_id).await?;
    let value = valuation::load(&s.pool, &positions).await?;
    summary(s, acc, &positions, &value).await
}

async fn detail(s: &AppState, me: &CurrentUser, id: i64) -> ApiResult<AccountDetail> {
    let acc = account::get(&s.pool, me.user_id, id).await?;
    let positions = account::positions(&s.pool, id).await?;
    let value = valuation::load(&s.pool, &positions).await?;
    let names: HashMap<String, (String, String, AssetType, bool)> = sqlx::query_as::<_, (String, String, String, AssetType, bool)>(
        "SELECT thscode, name, board, asset_type, delisted_on IS NOT NULL FROM ref.instrument WHERE thscode = ANY($1)",
    )
    .bind(positions.iter().map(|p| p.thscode.clone()).collect::<Vec<_>>())
    .fetch_all(&s.pool)
    .await?
    .into_iter()
    .map(|(c, n, b, a, d)| (c, (n, b, a, d)))
    .collect();
    let summary = summary(s, acc, &positions, &value).await?;
    let mut views: Vec<PositionView> = positions
        .into_iter()
        .map(|p| {
            let q = value.quotes.get(&p.thscode).copied();
            let close = q.map(|q| q.close);
            let mv = value.market_value_of(&p);
            let (name, board, asset_type, delisted) = names
                .get(&p.thscode)
                .cloned()
                .unwrap_or_else(|| (p.thscode.clone(), "main".into(), AssetType::Stock, false));
            PositionView {
                name,
                board,
                asset_type,
                close,
                last_date: q.map(|q| q.date),
                change_pct: q.and_then(|q| q.change_pct),
                market_value: mv,
                weight_pct: mv / summary.total_assets.max(1.0) * 100.0,
                pnl: p
                    .cost_price
                    .zip(close)
                    .map(|(c, x)| (x - c) * p.shares as f64),
                pnl_pct: p.cost_price.zip(close).map(|(c, x)| (x / c - 1.0) * 100.0),
                delisted,
                position: p,
            }
        })
        .collect();
    views.sort_by(|a, b| b.market_value.total_cmp(&a.market_value));
    Ok(AccountDetail {
        summary,
        positions: views,
    })
}

/// 改了持仓、现金或设置之后：账户模块已经在同一个事务里记下了重算待办（`account::stale`），
/// 这里只是唤醒后台的消费者早点开始，不唤醒也会在几秒内被处理
fn wake_regen(s: &AppState) {
    s.advice_wake.notify_one();
}

/// 我的账户
#[utoipa::path(get, path = "/accounts", tag = "accounts", responses((status = 200, description = "成功", body = [AccountSummary])))]
async fn list_accounts(
    State(s): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<AccountSummary>>> {
    let mut out = vec![];
    for acc in account::list(&s.pool, me.user_id).await? {
        out.push(summary_of(&s, acc).await?);
    }
    Ok(Json(out))
}

/// 新建账户：名字、现在的现金，规则用均衡型的参考值（之后在设置里改）。持仓之后添加或粘贴
#[utoipa::path(post, path = "/accounts", tag = "accounts", request_body = NewAccount,
    responses((status = 201, description = "已创建", body = AccountDetail),
              (status = 400, description = "名字不合法、现金为负或账户太多", body = crate::error::ErrorBody),
              (status = 409, description = "已经有同名账户", body = crate::error::ErrorBody)))]
async fn create_account(
    State(s): State<AppState>,
    me: CurrentUser,
    Json(input): Json<NewAccount>,
) -> ApiResult<(StatusCode, Json<AccountDetail>)> {
    let acc = account::create(&s.pool, me.user_id, &input).await?;
    wake_regen(&s);
    Ok((
        StatusCode::CREATED,
        Json(detail(&s, &me, acc.account_id).await?),
    ))
}

/// 账户详情：持仓带现价、市值、占比和盈亏
#[utoipa::path(get, path = "/accounts/{account_id}", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 200, description = "成功", body = AccountDetail), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn get_account(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<AccountDetail>> {
    Ok(Json(detail(&s, &me, id).await?))
}

/// 修改名字、选股和风控规则、费率、备注；之后重新生成运营建议
#[utoipa::path(put, path = "/accounts/{account_id}", tag = "accounts", params(("account_id" = i64, Path)), request_body = AccountUpdate,
    responses((status = 200, description = "已保存", body = AccountDetail),
              (status = 400, description = "参数不合法", body = crate::error::ErrorBody),
              (status = 404, description = "不存在", body = crate::error::ErrorBody),
              (status = 409, description = "已经有同名账户", body = crate::error::ErrorBody)))]
async fn update_account(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Json(input): Json<AccountUpdate>,
) -> ApiResult<Json<AccountDetail>> {
    account::update(&s.pool, me.user_id, id, &input).await?;
    wake_regen(&s);
    Ok(Json(detail(&s, &me, id).await?))
}

/// 删除账户，连同持仓、流水和历史建议
#[utoipa::path(delete, path = "/accounts/{account_id}", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 204, description = "已删除"), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn delete_account(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    account::delete(&s.pool, me.user_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 确认持仓和券商一致（只记时间，用于核对提醒）
#[utoipa::path(post, path = "/accounts/{account_id}/checked", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 204, description = "已记下"), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn mark_checked(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
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
async fn parse_positions(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Json(input): Json<PasteInput>,
) -> ApiResult<Json<Vec<ParsedRow>>> {
    account::get(&s.pool, me.user_id, id).await?;
    if input.text.chars().count() > 20_000 {
        return Err(ApiError::BadRequest("粘贴的文字太长".into()));
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT thscode, ticker, name FROM ref.instrument WHERE delisted_on IS NULL",
    )
    .fetch_all(&s.pool)
    .await?;
    let known: Vec<Known> = rows
        .iter()
        .map(|(c, t, n)| Known {
            thscode: c,
            ticker: t,
            name: n,
        })
        .collect();
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
async fn calibrate(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Json(input): Json<CalibrateInput>,
) -> ApiResult<Json<AccountDetail>> {
    account::calibrate(&s.pool, me.user_id, id, input.cash, &input.positions).await?;
    wake_regen(&s);
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
async fn list_trades(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Query(q): Query<TradesQuery>,
) -> ApiResult<Json<Vec<Trade>>> {
    Ok(Json(
        account::trades(
            &s.pool,
            me.user_id,
            id,
            q.limit.unwrap_or(100).clamp(1, 500),
        )
        .await?,
    ))
}

/// 手动记一笔：买入、卖出（按成交价和股数，费用不填时按账户费率估算）、转入或转出现金；之后重新生成建议
#[utoipa::path(post, path = "/accounts/{account_id}/trades", tag = "accounts", params(("account_id" = i64, Path)), request_body = TradeInput,
    responses((status = 200, description = "已记下", body = AccountDetail),
              (status = 400, description = "现金不够、股数不对等", body = crate::error::ErrorBody),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn add_trade(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Json(input): Json<TradeInput>,
) -> ApiResult<Json<AccountDetail>> {
    account::record(&s.pool, me.user_id, id, &input, TradeSource::User).await?;
    wake_regen(&s);
    Ok(Json(detail(&s, &me, id).await?))
}

/// 撤销一条手动记的流水（红冲）：现金和持仓、成本、已实现盈亏重新算一遍。按建议执行、自动记账、校准的流水不能单独撤销
#[utoipa::path(delete, path = "/accounts/{account_id}/trades/{trade_id}", tag = "accounts",
    params(("account_id" = i64, Path), ("trade_id" = i64, Path, description = "流水编号")),
    responses((status = 200, description = "已撤销，返回重算后的账户", body = AccountDetail),
              (status = 400, description = "不是手动流水、已经撤销过，或撤销后现金、持仓会变成负数", body = crate::error::ErrorBody),
              (status = 404, description = "账户或流水不存在", body = crate::error::ErrorBody)))]
async fn undo_trade(
    State(s): State<AppState>,
    me: CurrentUser,
    Path((id, trade_id)): Path<(i64, i64)>,
) -> ApiResult<Json<AccountDetail>> {
    account::undo_trade(&s.pool, me.user_id, id, trade_id).await?;
    wake_regen(&s);
    Ok(Json(detail(&s, &me, id).await?))
}

/// 待处理的公司行动（目前是配股）：除权日已经过去、还没认购也没选「暂不处理」的配股
#[utoipa::path(get, path = "/accounts/{account_id}/corp-actions", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 200, description = "成功；没有待处理的配股时是空列表", body = [PendingRights]),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn list_corp_actions(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<PendingRights>>> {
    account::get(&s.pool, me.user_id, id).await?;
    let Some(through) = crate::analysis::market::resolve_date(&s.pool, None).await? else {
        return Ok(Json(vec![]));
    };
    Ok(Json(
        account::corp::pending_rights(&s.pool, id, through).await?,
    ))
}

/// 记一笔配股认购：按除权日持有的股数和公告的配股比例算出认购股数，扣配股款和费用，按平均成本更新持仓；
/// 提醒随之消失，之后重新生成建议
#[utoipa::path(post, path = "/accounts/{account_id}/corp-actions/rights/{thscode}/{ex_date}/subscribe", tag = "accounts",
    params(("account_id" = i64, Path), ("thscode" = String, Path, description = "股票代码"), ("ex_date" = NaiveDate, Path, description = "除权日")),
    responses((status = 200, description = "已记下，返回更新后的账户", body = AccountDetail),
              (status = 400, description = "没有这次配股、已经处理过、按股数不足 1 股或现金不够", body = crate::error::ErrorBody),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn subscribe_rights(
    State(s): State<AppState>,
    me: CurrentUser,
    Path((id, thscode, ex_date)): Path<(i64, String, NaiveDate)>,
) -> ApiResult<Json<AccountDetail>> {
    account::corp::subscribe_rights(&s.pool, me.user_id, id, &thscode, ex_date).await?;
    wake_regen(&s);
    Ok(Json(detail(&s, &me, id).await?))
}

/// 暂不处理这次配股：不买、不扣钱，只记下处理状态，提醒消失
#[utoipa::path(post, path = "/accounts/{account_id}/corp-actions/rights/{thscode}/{ex_date}/ignore", tag = "accounts",
    params(("account_id" = i64, Path), ("thscode" = String, Path, description = "股票代码"), ("ex_date" = NaiveDate, Path, description = "除权日")),
    responses((status = 204, description = "已记下，不再提醒"),
              (status = 400, description = "没有这次配股、已经处理过或不足 1 股", body = crate::error::ErrorBody),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn ignore_rights(
    State(s): State<AppState>,
    me: CurrentUser,
    Path((id, thscode, ex_date)): Path<(i64, String, NaiveDate)>,
) -> ApiResult<StatusCode> {
    account::corp::ignore_rights(&s.pool, me.user_id, id, &thscode, ex_date).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, IntoParams)]
pub struct EquityQuery {
    /// 最近多少个交易日，默认 180，最多 1,825（约 7 年）
    pub days: Option<i64>,
}

/// 每日净值曲线：每个交易日的现金、持仓市值、总资产和当天盈亏，返回最近 `days` 个交易日
#[utoipa::path(get, path = "/accounts/{account_id}/equity", tag = "accounts", params(("account_id" = i64, Path), EquityQuery),
    responses((status = 200, description = "成功；还没有日线或账户还没有资金时是空曲线", body = EquityCurve),
              (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn get_equity(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Query(q): Query<EquityQuery>,
) -> ApiResult<Json<EquityCurve>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(
        account::equity::refresh(&s.pool, id, q.days.unwrap_or(180).clamp(5, 1825), None).await?,
    ))
}

#[derive(Serialize, ToSchema)]
pub struct AdviceResponse {
    /// 没有建议时的原因，例如还没有日线
    pub waiting: Option<String>,
    pub advice: Option<StoredAdvice>,
    /// 最新日线的日期：比建议的交易日新时，说明新的运营建议还在生成
    pub latest_bar: Option<NaiveDate>,
    /// 改了持仓、现金或设置之后正在后台重新生成；为 true 时 `advice` 还是改之前的一份
    pub regenerating: bool,
    /// 上一次重新生成失败的原因；后台会自动重试，成功后为空
    pub regenerate_error: Option<String>,
}

#[derive(Deserialize, IntoParams)]
pub struct AdviceQuery {
    /// 交易日；默认最新一份
    pub date: Option<NaiveDate>,
}

async fn advice_response(
    s: &AppState,
    id: i64,
    date: Option<NaiveDate>,
) -> ApiResult<AdviceResponse> {
    let advice = advice::load(&s.pool, id, date).await?;
    let latest_bar = crate::analysis::market::resolve_date(&s.pool, None).await?;
    let core_left = crate::sync::audit::core_backfill_left(&s.pool).await?;
    let waiting = match (&advice, latest_bar) {
        (Some(_), _) => None,
        (None, _) if core_left > 0 => Some(crate::sync::audit::data_status(&s.pool).await?.message),
        (None, None) => {
            Some("本地还没有日线：数据同步完成后自动生成（新部署的系统要等首次同步）".into())
        }
        (None, Some(_)) => Some("正在生成，稍等一分钟后刷新".into()),
    };
    let (pending, regenerate_error) = account::stale::status(&s.pool, id).await?;
    Ok(AdviceResponse {
        waiting,
        advice,
        latest_bar,
        regenerating: pending && regenerate_error.is_none(),
        regenerate_error,
    })
}

/// 当前的建议（某个交易日的最终一份），以及每条的处理状态
#[utoipa::path(get, path = "/accounts/{account_id}/advice", tag = "accounts", params(("account_id" = i64, Path), AdviceQuery),
    responses((status = 200, description = "成功", body = AdviceResponse), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn current_advice(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Query(q): Query<AdviceQuery>,
) -> ApiResult<Json<AdviceResponse>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(advice_response(&s, id, q.date).await?))
}

/// 按现在的持仓和最新的因子排名重新生成运营建议
#[utoipa::path(post, path = "/accounts/{account_id}/advice/refresh", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 200, description = "已重新生成", body = AdviceResponse), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn refresh_advice(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<AdviceResponse>> {
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
async fn advice_history(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<Vec<AdviceDay>>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(
        advice::history(&s.pool, id, q.limit.unwrap_or(60).clamp(1, 250)).await?,
    ))
}

#[derive(Deserialize, IntoParams)]
pub struct ShadowOrderQuery {
    pub limit: Option<i64>,
}

/// 影子执行记录：理论订单与实际成交的价差和未成交状态
#[utoipa::path(get, path = "/accounts/{account_id}/shadow-orders", tag = "accounts", params(("account_id" = i64, Path), ShadowOrderQuery),
    responses((status = 200, description = "成功", body = [crate::advice::ShadowOrder]), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn shadow_orders(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
    Query(q): Query<ShadowOrderQuery>,
) -> ApiResult<Json<Vec<advice::ShadowOrder>>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(
        advice::shadow_orders(&s.pool, id, q.limit.unwrap_or(250).clamp(1, 1000)).await?,
    ))
}

#[derive(Deserialize, ToSchema, Default)]
pub struct ExecuteInput {
    /// 实际成交的股数；默认按建议
    #[serde(default)]
    pub shares: Option<i64>,
    /// 实际成交价，必填：参考价是前一天的收盘价，不能当成交价记
    #[serde(default)]
    pub price: Option<f64>,
    /// 实际费用；默认按账户费率估算
    #[serde(default)]
    pub fee: Option<f64>,
    /// 成交日；默认这份建议的操作日
    #[serde(default)]
    pub trade_date: Option<NaiveDate>,
}

/// 已执行：按实际成交记一笔买入或卖出，持仓和现金跟着更新。这份建议不重新生成，其余条目照常
#[utoipa::path(post, path = "/accounts/{account_id}/advice/{advice_id}/items/{key}/execute", tag = "accounts",
    params(("account_id" = i64, Path), ("advice_id" = i64, Path), ("key" = String, Path, description = "例：buy-601012.SH")), request_body = ExecuteInput,
    responses((status = 200, description = "已记下", body = AdviceResponse),
              (status = 400, description = "这条不能执行、已经执行过、没填成交价，或现金、股数不够", body = crate::error::ErrorBody),
              (status = 404, description = "账户、建议或条目不存在", body = crate::error::ErrorBody)))]
async fn execute_item(
    State(s): State<AppState>,
    me: CurrentUser,
    Path((id, advice_id, key)): Path<(i64, i64, String)>,
    Json(input): Json<ExecuteInput>,
) -> ApiResult<Json<AdviceResponse>> {
    let fill = Fill {
        shares: input.shares,
        price: input.price,
        fee: input.fee,
        trade_date: input.trade_date,
    };
    let date = handling::execute(&s.pool, me.user_id, id, advice_id, &key, &fill).await?;
    Ok(Json(advice_response(&s, id, Some(date)).await?))
}

/// 不做这一条（之后可以撤销）
#[utoipa::path(post, path = "/accounts/{account_id}/advice/{advice_id}/items/{key}/skip", tag = "accounts",
    params(("account_id" = i64, Path), ("advice_id" = i64, Path), ("key" = String, Path)),
    responses((status = 200, description = "已记下", body = AdviceResponse),
              (status = 400, description = "这一条已经执行过", body = crate::error::ErrorBody),
              (status = 404, description = "账户、建议或条目不存在", body = crate::error::ErrorBody)))]
async fn skip_item(
    State(s): State<AppState>,
    me: CurrentUser,
    Path((id, advice_id, key)): Path<(i64, i64, String)>,
) -> ApiResult<Json<AdviceResponse>> {
    account::get(&s.pool, me.user_id, id).await?;
    let date = handling::skip(&s.pool, id, advice_id, &key).await?;
    Ok(Json(advice_response(&s, id, Some(date)).await?))
}

/// 撤销「不做」。已执行的不能撤销：记错了在持仓页校准
#[utoipa::path(delete, path = "/accounts/{account_id}/advice/{advice_id}/items/{key}/skip", tag = "accounts",
    params(("account_id" = i64, Path), ("advice_id" = i64, Path), ("key" = String, Path)),
    responses((status = 200, description = "已撤销；不是「不做」的条目不变", body = AdviceResponse), (status = 404, description = "账户、建议或条目不存在", body = crate::error::ErrorBody)))]
async fn clear_item(
    State(s): State<AppState>,
    me: CurrentUser,
    Path((id, advice_id, key)): Path<(i64, i64, String)>,
) -> ApiResult<Json<AdviceResponse>> {
    account::get(&s.pool, me.user_id, id).await?;
    let date = handling::unskip(&s.pool, id, advice_id, &key).await?;
    Ok(Json(advice_response(&s, id, Some(date)).await?))
}

/// 过去的建议之后 5、10、20 个交易日的实际涨跌
#[utoipa::path(get, path = "/accounts/{account_id}/review", tag = "accounts", params(("account_id" = i64, Path)),
    responses((status = 200, description = "成功", body = Review), (status = 404, description = "账户不存在", body = crate::error::ErrorBody)))]
async fn account_review(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<i64>,
) -> ApiResult<Json<Review>> {
    account::get(&s.pool, me.user_id, id).await?;
    Ok(Json(review::review(&s.pool, id).await?))
}
