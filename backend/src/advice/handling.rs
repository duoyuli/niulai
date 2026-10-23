//! 用户对一条建议的处理：「已执行」「不做」和撤销「不做」。
//!
//! 每个动作的全部写入在一个事务里提交：执行时的流水、持仓和现金（`account`），处理状态（`app.advice_item_state`）
//! 和影子订单（`app.shadow_order`）。不会出现账记上了、影子订单还显示未成交，或撤销了「不做」、影子订单还是已取消。
//! 处理状态的取值只在 [`super::ItemState`]。规则见 docs/domains/accounts.md#generation。

use chrono::NaiveDate;
use sqlx::{PgPool, Postgres, Transaction};

use super::ItemState;
use super::generate::{Action, Advice, Item};
use crate::account::{self, AccountError, TradeInput, TradeKind, TradeSource};

#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Account(#[from] AccountError),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for HandleError {
    fn from(e: sqlx::Error) -> Self {
        HandleError::Internal(e.into())
    }
}

type Result<T> = std::result::Result<T, HandleError>;

/// 实际成交；没填的按建议和账户费率
#[derive(Clone, Debug, Default)]
pub struct Fill {
    pub shares: Option<i64>,
    pub price: Option<f64>,
    pub fee: Option<f64>,
    pub trade_date: Option<NaiveDate>,
}

/// 读出一份建议（不论是否被取代），锁住这条的处理状态，返回建议的交易日和这一条
async fn item_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    account_id: i64,
    advice_id: i64,
    key: &str,
) -> Result<(Advice, Item, Option<ItemState>)> {
    let v: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT result FROM app.advice WHERE account_id = $1 AND advice_id = $2",
    )
    .bind(account_id)
    .bind(advice_id)
    .fetch_optional(&mut **tx)
    .await?;
    let advice: Advice =
        serde_json::from_value(v.ok_or_else(|| HandleError::NotFound("建议不存在".into()))?)
            .map_err(anyhow::Error::from)?;
    let item = advice
        .items
        .iter()
        .find(|i| i.key == key)
        .cloned()
        .ok_or_else(|| HandleError::NotFound("没有这一条".into()))?;
    let state: Option<String> = sqlx::query_scalar(
        "SELECT state FROM app.advice_item_state WHERE advice_id = $1 AND item_key = $2 FOR UPDATE",
    )
    .bind(advice_id)
    .bind(key)
    .fetch_optional(&mut **tx)
    .await?;
    Ok((advice, item, state.as_deref().and_then(ItemState::parse)))
}

/// 已执行：按实际成交记一笔买入或卖出，记下处理状态，补记影子订单的成交。返回建议的交易日。
/// 同一条并发请求也只能产生一笔流水；这份建议不重新生成
pub async fn execute(
    pool: &PgPool,
    user_id: i64,
    account_id: i64,
    advice_id: i64,
    key: &str,
    fill: &Fill,
) -> Result<NaiveDate> {
    let mut tx = pool.begin().await?;
    // 先锁账户，同一账户的执行排队，再锁这一条的状态
    let (advice, item, state) = {
        sqlx::query("SELECT 1 FROM app.account WHERE user_id = $1 AND account_id = $2 FOR UPDATE")
            .bind(user_id)
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(HandleError::Account(AccountError::NotFound))?;
        item_in_tx(&mut tx, account_id, advice_id, key).await?
    };
    let kind = match item.action {
        Action::Buy => TradeKind::Buy,
        Action::Sell => TradeKind::Sell,
        _ => return Err(HandleError::Invalid("这一条不需要操作".into())),
    };
    if state == Some(ItemState::Executed) {
        return Err(HandleError::Invalid("这一条已经记过了".into()));
    }
    let price = fill
        .price
        .filter(|p| *p > 0.0)
        .ok_or_else(|| HandleError::Invalid("请填实际成交价".into()))?;
    let shares = fill.shares.unwrap_or(item.shares);
    let trade = TradeInput {
        kind,
        trade_date: Some(fill.trade_date.unwrap_or(advice.next_date)),
        thscode: Some(item.thscode.clone()),
        shares: Some(shares),
        price: Some(price),
        amount: None,
        fee: fill.fee,
        note: format!("按 {} 的建议：{}", advice.trade_date, item.headline),
    };
    let trade_id =
        account::record_in_tx_for(&mut tx, user_id, account_id, &trade, TradeSource::Advice)
            .await?;
    sqlx::query(
        "INSERT INTO app.advice_item_state (advice_id, item_key, state, trade_id) VALUES ($1, $2, $3, $4)
         ON CONFLICT (advice_id, item_key) DO UPDATE SET state = EXCLUDED.state, trade_id = EXCLUDED.trade_id, updated_at = now()",
    )
    .bind(advice_id)
    .bind(key)
    .bind(ItemState::Executed.key())
    .bind(trade_id)
    .execute(&mut *tx)
    .await?;
    let slippage_bps = (price / item.ref_price - 1.0) * 10_000.0;
    sqlx::query(
        "UPDATE app.shadow_order
         SET actual_shares = $4, actual_price = $5, actual_slippage_bps = $6, reason = '',
             status = CASE WHEN $4 >= planned_shares THEN 'filled' ELSE 'partial' END,
             updated_at = now()
         WHERE account_id = $1 AND advice_id = $2 AND item_key = $3",
    )
    .bind(account_id)
    .bind(advice_id)
    .bind(key)
    .bind(shares)
    .bind(price)
    .bind(slippage_bps.is_finite().then_some(slippage_bps))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(advice.trade_date)
}

/// 不做这一条：记下处理状态，影子订单记为已取消。已执行的不能改成不做
pub async fn skip(pool: &PgPool, account_id: i64, advice_id: i64, key: &str) -> Result<NaiveDate> {
    let mut tx = pool.begin().await?;
    let (advice, _, state) = item_in_tx(&mut tx, account_id, advice_id, key).await?;
    match state {
        Some(ItemState::Executed) => {
            return Err(HandleError::Invalid(
                "这一条已经执行过了：记错了在持仓页校准".into(),
            ));
        }
        Some(ItemState::Skipped) => {}
        None => {
            sqlx::query(
                "INSERT INTO app.advice_item_state (advice_id, item_key, state) VALUES ($1, $2, $3)",
            )
            .bind(advice_id)
            .bind(key)
            .bind(ItemState::Skipped.key())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE app.shadow_order SET status = 'cancelled', reason = '用户选择不做', updated_at = now()
                 WHERE account_id = $1 AND advice_id = $2 AND item_key = $3",
            )
            .bind(account_id)
            .bind(advice_id)
            .bind(key)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(advice.trade_date)
}

/// 撤销「不做」：删掉处理状态，影子订单回到计划中。已执行的不能撤销：记错了在持仓页校准
pub async fn unskip(
    pool: &PgPool,
    account_id: i64,
    advice_id: i64,
    key: &str,
) -> Result<NaiveDate> {
    let mut tx = pool.begin().await?;
    let (advice, _, state) = item_in_tx(&mut tx, account_id, advice_id, key).await?;
    if state == Some(ItemState::Skipped) {
        sqlx::query("DELETE FROM app.advice_item_state WHERE advice_id = $1 AND item_key = $2")
            .bind(advice_id)
            .bind(key)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE app.shadow_order SET status = 'planned', reason = '', updated_at = now()
             WHERE account_id = $1 AND advice_id = $2 AND item_key = $3 AND status = 'cancelled'",
        )
        .bind(account_id)
        .bind(advice_id)
        .bind(key)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(advice.trade_date)
}
