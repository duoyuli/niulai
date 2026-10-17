//! 账户、持仓和流水。账户只属于一个用户：这里的每个函数都带用户编号，别人的账户一律当作不存在（管理员也一样）。
//!
//! 持仓和现金的每一次变动都记一条流水（`app.account_trade`）：买入、卖出、转入转出现金、分红、送转、校准。
//! 持仓表和现金是流水的结果，两者在同一个事务里更新，所以总能回答「我的持仓为什么变成这样」。
//! 规则见 docs/domains/accounts.md。

pub mod corp;
pub mod lots;
pub mod paste;
pub mod style;

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use utoipa::ToSchema;

use lots::{Fees, round2};
use style::{Style, StyleOverrides, StyleParams};

/// 每个用户最多几个账户
pub const MAX_ACCOUNTS: i64 = 10;
/// 每个账户最多几只持仓
pub const MAX_POSITIONS: usize = 50;

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct Account {
    pub account_id: i64,
    pub name: String,
    pub style: Style,
    /// 高级设置中改过的参数；为空对象表示全部跟着风格
    pub overrides: StyleOverrides,
    /// 现金（元）
    pub cash: f64,
    pub fees: Fees,
    pub note: String,
    /// 最近一次确认「和券商一致」的时刻
    pub checked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Account {
    /// 实际生效的风格参数
    pub fn params(&self) -> StyleParams {
        self.style.effective(&self.overrides)
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[schema(as = AccountPosition)]
pub struct Position {
    pub thscode: String,
    pub shares: i64,
    /// 每股成本（未复权；分红送转后自动摊薄）；没填时为空
    pub cost_price: Option<f64>,
    /// 买入日；没填时为空
    pub opened_on: Option<NaiveDate>,
    /// 按哪个策略的买入建议买的；手动录入的为空
    pub strategy_id: Option<i64>,
    /// 股数最近一次变动的日期：之后的除权除息才自动记账
    pub since: NaiveDate,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TradeKind {
    Buy,
    Sell,
    /// 现金分红（税前），系统按公告自动记账
    Dividend,
    /// 送转股，系统按公告自动记账
    Bonus,
    CashIn,
    CashOut,
    /// 校准：直接改持仓或现金
    Adjust,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TradeSource {
    /// 用户手动记的
    User,
    /// 在建议上点了「已执行」
    Advice,
    /// 系统按分红送转公告自动记的
    CorpAction,
}

impl TradeKind {
    fn key(self) -> &'static str {
        match self {
            TradeKind::Buy => "buy",
            TradeKind::Sell => "sell",
            TradeKind::Dividend => "dividend",
            TradeKind::Bonus => "bonus",
            TradeKind::CashIn => "cash_in",
            TradeKind::CashOut => "cash_out",
            TradeKind::Adjust => "adjust",
        }
    }
}

impl TradeSource {
    fn key(self) -> &'static str {
        match self {
            TradeSource::User => "user",
            TradeSource::Advice => "advice",
            TradeSource::CorpAction => "corp_action",
        }
    }
}

fn parse_enum<T: serde::de::DeserializeOwned>(s: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(s.to_string()))?)
}

#[derive(Serialize, ToSchema, Clone, Debug)]
#[schema(as = AccountTrade)]
pub struct Trade {
    pub trade_id: i64,
    pub kind: TradeKind,
    pub trade_date: NaiveDate,
    pub thscode: Option<String>,
    pub name: Option<String>,
    /// 持仓股数的变化：买入、送转为正，卖出为负
    pub shares: Option<i64>,
    pub price: Option<f64>,
    /// 现金的变化（含费用）：买入为负，卖出、分红为正
    pub amount: f64,
    pub fee: f64,
    pub source: TradeSource,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("{0}")]
    Invalid(String),
    #[error("账户不存在")]
    NotFound,
    #[error("已经有叫「{0}」的账户了")]
    Taken(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for AccountError {
    fn from(e: sqlx::Error) -> Self {
        AccountError::Internal(e.into())
    }
}

type Result<T> = std::result::Result<T, AccountError>;

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(AccountError::Invalid(msg.into()))
}

#[derive(Deserialize, ToSchema)]
pub struct NewAccount {
    pub name: String,
    pub style: Style,
    /// 现在的现金（元）
    #[serde(default)]
    pub cash: f64,
    #[serde(default)]
    pub note: String,
}

#[derive(Deserialize, ToSchema)]
pub struct AccountUpdate {
    pub name: String,
    pub style: Style,
    #[serde(default)]
    pub overrides: StyleOverrides,
    pub fees: Fees,
    #[serde(default)]
    pub note: String,
}

#[derive(Deserialize, ToSchema, Clone, Debug)]
pub struct PositionInput {
    pub thscode: String,
    pub shares: i64,
    #[serde(default)]
    pub cost_price: Option<f64>,
    #[serde(default)]
    pub opened_on: Option<NaiveDate>,
}

/// 手动记一笔：买入、卖出、转入或转出现金
#[derive(Deserialize, ToSchema, Clone, Debug)]
pub struct TradeInput {
    pub kind: TradeKind,
    /// 成交日，默认今天
    #[serde(default)]
    pub trade_date: Option<NaiveDate>,
    #[serde(default)]
    pub thscode: Option<String>,
    /// 买入、卖出的股数
    #[serde(default)]
    pub shares: Option<i64>,
    /// 成交价
    #[serde(default)]
    pub price: Option<f64>,
    /// 转入、转出的金额
    #[serde(default)]
    pub amount: Option<f64>,
    /// 实际费用；为空时按账户费率估算
    #[serde(default)]
    pub fee: Option<f64>,
    #[serde(default)]
    pub note: String,
}

type AccountRow = (i64, String, String, serde_json::Value, f64, f64, f64, String, Option<DateTime<Utc>>, DateTime<Utc>, DateTime<Utc>);
const ACCOUNT_COLS: &str = "account_id, name, style, overrides, cash::float8, commission_rate::float8, min_commission::float8, note, checked_at, created_at, updated_at";

fn account((account_id, name, style, overrides, cash, commission_rate, min_commission, note, checked_at, created_at, updated_at): AccountRow) -> anyhow::Result<Account> {
    Ok(Account {
        account_id,
        name,
        style: Style::parse(&style)?,
        overrides: serde_json::from_value(overrides)?,
        cash,
        fees: Fees { commission_rate, min_commission },
        note,
        checked_at,
        created_at,
        updated_at,
    })
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    let n = name.chars().count();
    if n == 0 || n > 30 {
        return invalid("账户名要 1 到 30 个字");
    }
    Ok(name.to_string())
}

fn validate_money(v: f64, what: &str) -> Result<f64> {
    if !v.is_finite() || v < 0.0 || v > 1e12 {
        return invalid(format!("{what}不合法"));
    }
    Ok(round2(v))
}

fn validate_fees(f: &Fees) -> Result<()> {
    if !(0.0..=0.003).contains(&f.commission_rate) || !(0.0..=100.0).contains(&f.min_commission) {
        return invalid("佣金率应在 0 到千分之 3 之间，最低佣金在 0 到 100 元之间");
    }
    Ok(())
}

pub async fn list(pool: &PgPool, user_id: i64) -> Result<Vec<Account>> {
    // 审计：ACCOUNT_COLS 是本文件中的常量列名
    let rows: Vec<AccountRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ACCOUNT_COLS} FROM app.account WHERE user_id = $1 ORDER BY account_id"))).bind(user_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(account).collect::<anyhow::Result<_>>()?)
}

pub async fn get(pool: &PgPool, user_id: i64, account_id: i64) -> Result<Account> {
    // 审计：ACCOUNT_COLS 是本文件中的常量列名
    let row: Option<AccountRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ACCOUNT_COLS} FROM app.account WHERE user_id = $1 AND account_id = $2")))
        .bind(user_id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?;
    row.map(account).transpose()?.ok_or(AccountError::NotFound)
}

/// 全部账户（后台每天生成建议用，不按用户过滤）
pub async fn all(pool: &PgPool) -> Result<Vec<Account>> {
    // 审计：ACCOUNT_COLS 是本文件中的常量列名
    let rows: Vec<AccountRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ACCOUNT_COLS} FROM app.account ORDER BY account_id"))).fetch_all(pool).await?;
    Ok(rows.into_iter().map(account).collect::<anyhow::Result<_>>()?)
}

pub async fn create(pool: &PgPool, user_id: i64, input: &NewAccount) -> Result<Account> {
    let name = validate_name(&input.name)?;
    let cash = validate_money(input.cash, "现金")?;
    let mut tx = pool.begin().await?;
    // 锁住这个用户，两个请求同时建账户时数量也不会超
    sqlx::query("SELECT 1 FROM app.user WHERE user_id = $1 FOR UPDATE").bind(user_id).execute(&mut *tx).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM app.account WHERE user_id = $1").bind(user_id).fetch_one(&mut *tx).await?;
    if n >= MAX_ACCOUNTS {
        return invalid(format!("每人最多 {MAX_ACCOUNTS} 个账户"));
    }
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO app.account (user_id, name, style, cash, note) VALUES ($1, $2, $3, round($4::float8::numeric, 2), $5)
         ON CONFLICT (user_id, lower(name)) DO NOTHING RETURNING account_id",
    )
    .bind(user_id)
    .bind(&name)
    .bind(input.style.key())
    .bind(cash)
    .bind(input.note.trim())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(id) = id else { return Err(AccountError::Taken(name)) };
    if cash > 0.0 {
        insert_trade(&mut tx, id, TradeKind::CashIn, crate::time::today_cn(), None, None, None, cash, 0.0, TradeSource::User, "建账户时的现金").await?;
    }
    tx.commit().await?;
    get(pool, user_id, id).await
}

pub async fn update(pool: &PgPool, user_id: i64, account_id: i64, input: &AccountUpdate, known: &[&str]) -> Result<Account> {
    let name = validate_name(&input.name)?;
    validate_fees(&input.fees)?;
    input.style.effective(&input.overrides).validate(known).map_err(AccountError::Invalid)?;
    let r = sqlx::query(
        "UPDATE app.account SET name = $3, style = $4, overrides = $5, commission_rate = $6, min_commission = $7, note = $8, updated_at = now()
         WHERE user_id = $1 AND account_id = $2",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&name)
    .bind(input.style.key())
    .bind(serde_json::to_value(&input.overrides).map_err(anyhow::Error::from)?)
    .bind(input.fees.commission_rate)
    .bind(input.fees.min_commission)
    .bind(input.note.trim())
    .execute(pool)
    .await;
    match r {
        Ok(r) if r.rows_affected() == 0 => Err(AccountError::NotFound),
        Ok(_) => get(pool, user_id, account_id).await,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(AccountError::Taken(name)),
        Err(e) => Err(e.into()),
    }
}

pub async fn delete(pool: &PgPool, user_id: i64, account_id: i64) -> Result<()> {
    let n = sqlx::query("DELETE FROM app.account WHERE user_id = $1 AND account_id = $2").bind(user_id).bind(account_id).execute(pool).await?.rows_affected();
    if n == 0 { Err(AccountError::NotFound) } else { Ok(()) }
}

pub async fn mark_checked(pool: &PgPool, user_id: i64, account_id: i64) -> Result<()> {
    let n = sqlx::query("UPDATE app.account SET checked_at = now() WHERE user_id = $1 AND account_id = $2").bind(user_id).bind(account_id).execute(pool).await?.rows_affected();
    if n == 0 { Err(AccountError::NotFound) } else { Ok(()) }
}

type PositionRow = (String, i64, Option<f64>, Option<NaiveDate>, Option<i64>, NaiveDate);
/// 已有持仓的 (股数, 成本价, 买入日, 买入依据的策略)
type HeldRow = (i64, Option<f64>, Option<NaiveDate>, Option<i64>);

pub async fn positions(pool: &PgPool, account_id: i64) -> Result<Vec<Position>> {
    let rows: Vec<PositionRow> = sqlx::query_as(
        "SELECT thscode, shares, cost_price::float8, opened_on, strategy_id, since FROM app.position WHERE account_id = $1 ORDER BY thscode",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(thscode, shares, cost_price, opened_on, strategy_id, since)| Position { thscode, shares, cost_price, opened_on, strategy_id, since })
        .collect())
}

/// 最近一次持仓或现金的变动（流水），用于提醒核对
pub async fn last_change(pool: &PgPool, account_id: i64) -> Result<Option<DateTime<Utc>>> {
    Ok(sqlx::query_scalar("SELECT max(created_at) FROM app.account_trade WHERE account_id = $1").bind(account_id).fetch_one(pool).await?)
}

/// 锁住账户（同一账户的变动排队），返回现金
async fn lock(tx: &mut Transaction<'_, Postgres>, user_id: i64, account_id: i64) -> Result<f64> {
    let cash: Option<f64> = sqlx::query_scalar("SELECT cash::float8 FROM app.account WHERE user_id = $1 AND account_id = $2 FOR UPDATE")
        .bind(user_id)
        .bind(account_id)
        .fetch_optional(&mut **tx)
        .await?;
    cash.ok_or(AccountError::NotFound)
}

#[allow(clippy::too_many_arguments)]
async fn insert_trade(
    tx: &mut Transaction<'_, Postgres>,
    account_id: i64,
    kind: TradeKind,
    date: NaiveDate,
    thscode: Option<&str>,
    shares: Option<i64>,
    price: Option<f64>,
    amount: f64,
    fee: f64,
    source: TradeSource,
    note: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO app.account_trade (account_id, kind, trade_date, thscode, shares, price, amount, fee, source, note)
         VALUES ($1, $2, $3, $4, $5, round($6::float8::numeric, 4), round($7::float8::numeric, 2), round($8::float8::numeric, 2), $9, $10) RETURNING trade_id",
    )
    .bind(account_id)
    .bind(kind.key())
    .bind(date)
    .bind(thscode)
    .bind(shares)
    .bind(price)
    .bind(amount)
    .bind(fee)
    .bind(source.key())
    .bind(note)
    .fetch_one(&mut **tx)
    .await?)
}

async fn add_cash(tx: &mut Transaction<'_, Postgres>, account_id: i64, delta: f64) -> Result<()> {
    sqlx::query("UPDATE app.account SET cash = cash + round($2::float8::numeric, 2), updated_at = now() WHERE account_id = $1")
        .bind(account_id)
        .bind(delta)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn security_board(tx: &mut Transaction<'_, Postgres>, code: &str) -> Result<Option<String>> {
    Ok(sqlx::query_scalar("SELECT board FROM ref.security WHERE thscode = $1").bind(code).fetch_optional(&mut **tx).await?)
}

/// 买入或卖出成交后更新持仓、现金并记流水；返回流水编号。`strategy_id` 是按哪个策略的建议买的
pub async fn record(pool: &PgPool, user_id: i64, account_id: i64, input: &TradeInput, source: TradeSource, strategy_id: Option<i64>) -> Result<i64> {
    let date = input.trade_date.unwrap_or_else(crate::time::today_cn);
    let mut tx = pool.begin().await?;
    let cash = lock(&mut tx, user_id, account_id).await?;
    let fees = account_fees(&mut tx, account_id).await?;
    let id = match input.kind {
        TradeKind::CashIn | TradeKind::CashOut => {
            let amount = input.amount.filter(|a| a.is_finite() && *a > 0.0).ok_or_else(|| AccountError::Invalid("金额要大于 0".into()))?;
            let amount = validate_money(amount, "金额")?;
            let delta = if input.kind == TradeKind::CashIn { amount } else { -amount };
            if cash + delta < -0.005 {
                return invalid(format!("现金只有 ¥{cash:.2}，不够转出 ¥{amount:.2}"));
            }
            add_cash(&mut tx, account_id, delta).await?;
            insert_trade(&mut tx, account_id, input.kind, date, None, None, None, delta, 0.0, source, input.note.trim()).await?
        }
        TradeKind::Buy | TradeKind::Sell => {
            let code = input.thscode.as_deref().map(str::trim).filter(|c| !c.is_empty()).ok_or_else(|| AccountError::Invalid("缺少股票代码".into()))?;
            if security_board(&mut tx, code).await?.is_none() {
                return invalid(format!("没有这只股票：{code}"));
            }
            let shares = input.shares.filter(|n| *n > 0).ok_or_else(|| AccountError::Invalid("股数要大于 0".into()))?;
            let price = input.price.filter(|p| p.is_finite() && *p > 0.0).ok_or_else(|| AccountError::Invalid("成交价要大于 0".into()))?;
            let gross = round2(shares as f64 * price);
            let current: Option<HeldRow> = sqlx::query_as(
                "SELECT shares, cost_price::float8, opened_on, strategy_id FROM app.position WHERE account_id = $1 AND thscode = $2 FOR UPDATE",
            )
            .bind(account_id)
            .bind(code)
            .fetch_optional(&mut *tx)
            .await?;
            if input.kind == TradeKind::Buy {
                let fee = input.fee.unwrap_or_else(|| fees.buy_fee(gross));
                let pay = gross + fee;
                if pay > cash + 0.005 {
                    return invalid(format!("现金不够：这笔要 ¥{pay:.2}，现金只有 ¥{cash:.2}。先转入现金，或在持仓页校准现金"));
                }
                let (old, old_cost, opened, old_strategy) = current.unwrap_or((0, None, None, None));
                // 加权平均成本（含费用）；原来没填成本的，加仓后仍不知道整体成本
                let cost = match (old, old_cost) {
                    (0, _) => Some((pay) / shares as f64),
                    (o, Some(c)) => Some((c * o as f64 + pay) / (o + shares) as f64),
                    (_, None) => None,
                };
                let n: i64 = sqlx::query_scalar("SELECT count(*) FROM app.position WHERE account_id = $1").bind(account_id).fetch_one(&mut *tx).await?;
                if old == 0 && n as usize >= MAX_POSITIONS {
                    return invalid(format!("每个账户最多 {MAX_POSITIONS} 只持仓"));
                }
                sqlx::query(
                    "INSERT INTO app.position (account_id, thscode, shares, cost_price, opened_on, strategy_id, since)
                     VALUES ($1, $2, $3, round($4::float8::numeric, 4), $5, $6, $7)
                     ON CONFLICT (account_id, thscode) DO UPDATE SET shares = EXCLUDED.shares, cost_price = EXCLUDED.cost_price,
                       opened_on = EXCLUDED.opened_on, strategy_id = EXCLUDED.strategy_id, since = EXCLUDED.since",
                )
                .bind(account_id)
                .bind(code)
                .bind(old + shares)
                .bind(cost)
                .bind(opened.or(Some(date)))
                .bind(old_strategy.or(strategy_id))
                .bind(date)
                .execute(&mut *tx)
                .await?;
                add_cash(&mut tx, account_id, -pay).await?;
                insert_trade(&mut tx, account_id, TradeKind::Buy, date, Some(code), Some(shares), Some(price), -pay, fee, source, input.note.trim()).await?
            } else {
                let held = current.as_ref().map_or(0, |c| c.0);
                if shares > held {
                    return invalid(format!("只持有 {held} 股，不能卖 {shares} 股"));
                }
                let fee = input.fee.unwrap_or_else(|| fees.sell_fee(gross));
                if held == shares {
                    sqlx::query("DELETE FROM app.position WHERE account_id = $1 AND thscode = $2").bind(account_id).bind(code).execute(&mut *tx).await?;
                } else {
                    sqlx::query("UPDATE app.position SET shares = shares - $3, since = $4 WHERE account_id = $1 AND thscode = $2")
                        .bind(account_id)
                        .bind(code)
                        .bind(shares)
                        .bind(date)
                        .execute(&mut *tx)
                        .await?;
                }
                add_cash(&mut tx, account_id, gross - fee).await?;
                insert_trade(&mut tx, account_id, TradeKind::Sell, date, Some(code), Some(-shares), Some(price), gross - fee, fee, source, input.note.trim()).await?
            }
        }
        _ => return invalid("只能记买入、卖出、转入或转出现金"),
    };
    tx.commit().await?;
    Ok(id)
}

async fn account_fees(tx: &mut Transaction<'_, Postgres>, account_id: i64) -> Result<Fees> {
    let (commission_rate, min_commission): (f64, f64) =
        sqlx::query_as("SELECT commission_rate::float8, min_commission::float8 FROM app.account WHERE account_id = $1").bind(account_id).fetch_one(&mut **tx).await?;
    Ok(Fees { commission_rate, min_commission })
}

/// 校准：把持仓和现金整体换成用户填的（例如照着券商 App 抄一遍）。和原来不同的地方各记一条「校准」流水
pub async fn calibrate(pool: &PgPool, user_id: i64, account_id: i64, cash: f64, list: &[PositionInput]) -> Result<()> {
    let cash = validate_money(cash, "现金")?;
    if list.len() > MAX_POSITIONS {
        return invalid(format!("每个账户最多 {MAX_POSITIONS} 只持仓"));
    }
    let today = crate::time::today_cn();
    let mut tx = pool.begin().await?;
    let old_cash = lock(&mut tx, user_id, account_id).await?;
    let mut seen = std::collections::HashSet::new();
    for p in list {
        let code = p.thscode.trim();
        if !seen.insert(code.to_string()) {
            return invalid(format!("{code} 出现了两次"));
        }
        if security_board(&mut tx, code).await?.is_none() {
            return invalid(format!("没有这只股票：{code}"));
        }
        if p.shares <= 0 {
            return invalid(format!("{code} 的股数要大于 0"));
        }
        if p.cost_price.is_some_and(|c| !c.is_finite() || c <= 0.0) {
            return invalid(format!("{code} 的成本价要大于 0"));
        }
    }
    let old: HashMap<String, Position> = positions_tx(&mut tx, account_id).await?.into_iter().map(|p| (p.thscode.clone(), p)).collect();
    for p in list {
        let code = p.thscode.trim();
        let before = old.get(code);
        let cost = p.cost_price.map(|c| (c * 10_000.0).round() / 10_000.0);
        let unchanged = before.is_some_and(|b| b.shares == p.shares && b.cost_price == cost && b.opened_on == p.opened_on);
        if unchanged {
            continue;
        }
        let since = match before {
            Some(b) if b.shares == p.shares => b.since,
            _ => today,
        };
        sqlx::query(
            "INSERT INTO app.position (account_id, thscode, shares, cost_price, opened_on, strategy_id, since)
             VALUES ($1, $2, $3, $4::float8::numeric, $5, NULL, $6)
             ON CONFLICT (account_id, thscode) DO UPDATE SET shares = EXCLUDED.shares, cost_price = EXCLUDED.cost_price,
               opened_on = EXCLUDED.opened_on, since = EXCLUDED.since",
        )
        .bind(account_id)
        .bind(code)
        .bind(p.shares)
        .bind(cost)
        .bind(p.opened_on)
        .bind(since)
        .execute(&mut *tx)
        .await?;
        let delta = p.shares - before.map_or(0, |b| b.shares);
        let note = match before {
            None => "校准：新增持仓".to_string(),
            Some(b) if delta != 0 => format!("校准：股数 {} → {}", b.shares, p.shares),
            Some(_) => "校准：成本价或买入日".to_string(),
        };
        insert_trade(&mut tx, account_id, TradeKind::Adjust, today, Some(code), Some(delta), cost, 0.0, 0.0, TradeSource::User, &note).await?;
    }
    for (code, b) in &old {
        if !list.iter().any(|p| p.thscode.trim() == code) {
            sqlx::query("DELETE FROM app.position WHERE account_id = $1 AND thscode = $2").bind(account_id).bind(code).execute(&mut *tx).await?;
            insert_trade(&mut tx, account_id, TradeKind::Adjust, today, Some(code), Some(-b.shares), None, 0.0, 0.0, TradeSource::User, "校准：移除持仓").await?;
        }
    }
    let delta = round2(cash - old_cash);
    if delta.abs() >= 0.005 {
        add_cash(&mut tx, account_id, delta).await?;
        insert_trade(&mut tx, account_id, TradeKind::Adjust, today, None, None, None, delta, 0.0, TradeSource::User, &format!("校准：现金 {old_cash:.2} → {cash:.2}")).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn positions_tx(tx: &mut Transaction<'_, Postgres>, account_id: i64) -> Result<Vec<Position>> {
    let rows: Vec<PositionRow> = sqlx::query_as(
        "SELECT thscode, shares, cost_price::float8, opened_on, strategy_id, since FROM app.position WHERE account_id = $1 FOR UPDATE",
    )
    .bind(account_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(thscode, shares, cost_price, opened_on, strategy_id, since)| Position { thscode, shares, cost_price, opened_on, strategy_id, since })
        .collect())
}

type TradeRow = (i64, String, NaiveDate, Option<String>, Option<String>, Option<i64>, Option<f64>, f64, f64, String, String, DateTime<Utc>);

/// 流水，新的在前
pub async fn trades(pool: &PgPool, user_id: i64, account_id: i64, limit: i64) -> Result<Vec<Trade>> {
    get(pool, user_id, account_id).await?;
    let rows: Vec<TradeRow> = sqlx::query_as(
        "SELECT t.trade_id, t.kind, t.trade_date, t.thscode, s.name, t.shares, t.price::float8, t.amount::float8, t.fee::float8, t.source, t.note, t.created_at
         FROM app.account_trade t LEFT JOIN ref.security s USING (thscode)
         WHERE t.account_id = $1 ORDER BY t.trade_id DESC LIMIT $2",
    )
    .bind(account_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(trade_id, kind, trade_date, thscode, name, shares, price, amount, fee, source, note, created_at)| {
            Ok(Trade { trade_id, kind: parse_enum(&kind)?, trade_date, thscode, name, shares, price, amount, fee, source: parse_enum(&source)?, note, created_at })
        })
        .collect::<anyhow::Result<_>>()
        .map_err(AccountError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_keys_match_serde_names() {
        for k in [TradeKind::Buy, TradeKind::Sell, TradeKind::Dividend, TradeKind::Bonus, TradeKind::CashIn, TradeKind::CashOut, TradeKind::Adjust] {
            assert_eq!(parse_enum::<TradeKind>(k.key()).unwrap(), k);
        }
        for s in [TradeSource::User, TradeSource::Advice, TradeSource::CorpAction] {
            assert_eq!(parse_enum::<TradeSource>(s.key()).unwrap(), s);
        }
    }
}
