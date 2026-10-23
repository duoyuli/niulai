//! 每日调仓建议：每次同步完成后，为每个账户生成一份「明天怎么操作」。
//!
//! - `ranking`：按账户的因子给全市场排名，判断调仓日；
//! - `generate`：纯函数，从输入算出建议和每条的理由；
//! - `inputs`：从数据库组装输入；
//! - `review`：过去的建议之后 5、10、20 个交易日的实际涨跌；
//! - 本文件：保存、重新生成和每天为所有账户生成建议。
//!
//! 依赖方向：advice 读 account 和 analysis，analysis 不知道账户的存在。规则见 docs/domains/accounts.md。

pub mod generate;
pub mod inputs;
pub mod ranking;
pub mod review;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::account::{self, Account};
use generate::Advice;

/// 一份保存下来的建议，以及用户对每条的处理
#[derive(Serialize, ToSchema)]
pub struct StoredAdvice {
    pub advice_id: i64,
    pub trade_date: NaiveDate,
    pub created_at: DateTime<Utc>,
    pub advice: Advice,
    /// item key → executed / skipped
    pub states: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, ToSchema)]
pub struct ShadowOrder {
    pub shadow_order_id: i64,
    pub advice_id: Option<i64>,
    pub item_key: String,
    pub trade_date: NaiveDate,
    pub thscode: String,
    pub side: String,
    pub planned_shares: i64,
    pub reference_price: f64,
    pub expected_price: Option<f64>,
    pub actual_shares: Option<i64>,
    pub actual_price: Option<f64>,
    pub expected_slippage_bps: Option<f64>,
    pub actual_slippage_bps: Option<f64>,
    pub status: String,
    pub reason: String,
}

type ShadowOrderRow = (
    i64,
    Option<i64>,
    String,
    NaiveDate,
    String,
    String,
    i64,
    f64,
    Option<f64>,
    Option<i64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    String,
    String,
);

pub async fn shadow_orders(
    pool: &PgPool,
    account_id: i64,
    limit: i64,
) -> anyhow::Result<Vec<ShadowOrder>> {
    let rows: Vec<ShadowOrderRow> = sqlx::query_as(
        "SELECT shadow_order_id, advice_id, item_key, trade_date, thscode, side,
                planned_shares, reference_price::float8, expected_price::float8,
                actual_shares, actual_price::float8, expected_slippage_bps::float8,
                actual_slippage_bps::float8, status, reason
         FROM app.shadow_order WHERE account_id = $1 ORDER BY trade_date DESC, shadow_order_id DESC LIMIT $2",
    )
    .bind(account_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ShadowOrder {
            shadow_order_id: r.0,
            advice_id: r.1,
            item_key: r.2,
            trade_date: r.3,
            thscode: r.4,
            side: r.5,
            planned_shares: r.6,
            reference_price: r.7,
            expected_price: r.8,
            actual_shares: r.9,
            actual_price: r.10,
            expected_slippage_bps: r.11,
            actual_slippage_bps: r.12,
            status: r.13,
            reason: r.14,
        })
        .collect())
}

/// 生成并保存一个账户在最新交易日的建议；同一天已有的一份标记为被取代
pub async fn generate_for(
    pool: &PgPool,
    sh: &inputs::Shared,
    acc: &Account,
) -> anyhow::Result<i64> {
    let input = inputs::input_for(pool, sh, acc).await?;
    let advice = generate::generate(&input);
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT 1 FROM app.account WHERE account_id = $1 FOR UPDATE")
        .bind(acc.account_id)
        .execute(&mut *tx)
        .await?;
    let id: i64 =
        sqlx::query_scalar("SELECT nextval(pg_get_serial_sequence('app.advice', 'advice_id'))")
            .fetch_one(&mut *tx)
            .await?;
    // 外键在提交时才检查：先把旧的一份指向新的，再插入新的
    sqlx::query("UPDATE app.advice SET superseded_by = $3 WHERE account_id = $1 AND trade_date = $2 AND superseded_by IS NULL")
        .bind(acc.account_id)
        .bind(sh.trade_date)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO app.advice (advice_id, account_id, trade_date, input, result) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(acc.account_id)
        .bind(sh.trade_date)
        .bind(serde_json::to_value(&input)?)
        .bind(serde_json::to_value(&advice)?)
        .execute(&mut *tx)
        .await?;
    for item in &advice.items {
        let side = match item.action {
            generate::Action::Buy => Some("buy"),
            generate::Action::Sell => Some("sell"),
            _ => None,
        };
        let Some(side) = side else { continue };
        if item.shares <= 0 || item.ref_price <= 0.0 {
            continue;
        }
        sqlx::query(
            "INSERT INTO app.shadow_order
             (account_id, advice_id, item_key, trade_date, thscode, side, planned_shares, reference_price, expected_price)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT (advice_id, item_key) DO NOTHING",
        )
        .bind(acc.account_id)
        .bind(id)
        .bind(&item.key)
        .bind(advice.next_date)
        .bind(&item.thscode)
        .bind(side)
        .bind(item.shares)
        .bind(item.ref_price)
        .bind(item.limit_price.unwrap_or(item.ref_price))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// 持仓、现金或设置改了之后：立即重新生成这个账户的建议。核心数据没齐或还没有日线时不生成
pub async fn regenerate(pool: &PgPool, acc: &Account) -> anyhow::Result<Option<i64>> {
    let Some(sh) = inputs::shared(pool).await? else {
        return Ok(None);
    };
    Ok(Some(generate_for(pool, &sh, acc).await?))
}

/// 后台重新生成中的账户：值为 true 表示生成期间又有修改，结束后再生成一次
fn pending() -> &'static std::sync::Mutex<std::collections::HashMap<i64, bool>> {
    static PENDING: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<i64, bool>>> =
        std::sync::OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// 这个账户的建议是否正在后台重新生成
pub fn regenerating(account_id: i64) -> bool {
    pending().lock().is_ok_and(|m| m.contains_key(&account_id))
}

/// 在后台重新生成这个账户的建议，保存持仓、现金或设置的请求不用等它。同一账户同时只生成一份；
/// 生成期间又有修改时，结束后按最新的账户再生成一次，所以最后一份总是最新的。失败只写日志
pub fn regenerate_soon(pool: PgPool, account_id: i64) {
    {
        let Ok(mut m) = pending().lock() else {
            return;
        };
        if let Some(again) = m.get_mut(&account_id) {
            *again = true;
            return;
        }
        m.insert(account_id, false);
    }
    tokio::spawn(async move {
        loop {
            let r = async {
                match account::get_by_id(&pool, account_id).await? {
                    Some(acc) => regenerate(&pool, &acc).await.map(|_| ()),
                    None => Ok(()),
                }
            }
            .await;
            if let Err(e) = r {
                tracing::error!(account_id, error = %format!("{e:#}"), "重新生成建议失败");
            }
            let Ok(mut m) = pending().lock() else {
                return;
            };
            match m.get_mut(&account_id) {
                Some(again) if *again => *again = false,
                _ => {
                    m.remove(&account_id);
                    return;
                }
            }
        }
    });
}

async fn current_id(
    pool: &PgPool,
    account_id: i64,
    trade_date: NaiveDate,
) -> sqlx::Result<Option<(i64, bool)>> {
    sqlx::query_as(
        "SELECT a.advice_id, EXISTS (SELECT 1 FROM app.advice_item_state s WHERE s.advice_id = a.advice_id)
         FROM app.advice a WHERE a.account_id = $1 AND a.trade_date = $2 AND a.superseded_by IS NULL",
    )
    .bind(account_id)
    .bind(trade_date)
    .fetch_optional(pool)
    .await
}

/// 为所有账户生成最新交易日的建议。`replace` 为 true 时（同步刚完成，日线有变化）清掉因子排名缓存、
/// 连已有的一起重新生成，但用户已经处理过某一条（执行或不做）的保留，免得建议在手上变了
pub async fn refresh_all(pool: &PgPool, replace: bool) -> anyhow::Result<usize> {
    if replace {
        ranking::clear();
    } else {
        let Some(trade_date) = crate::analysis::market::resolve_date(pool, None).await? else {
            return Ok(0);
        };
        let needed: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM app.account a
                WHERE NOT EXISTS (
                    SELECT 1 FROM app.advice x
                    WHERE x.account_id = a.account_id AND x.trade_date = $1 AND x.superseded_by IS NULL
                )
            )",
        )
        .bind(trade_date)
        .fetch_one(pool)
        .await?;
        if !needed {
            account::corp::apply(pool, trade_date).await?;
            return Ok(0);
        }
    }
    let Some(sh) = inputs::shared(pool).await? else {
        return Ok(0);
    };
    account::corp::apply(pool, sh.trade_date).await?;
    let mut n = 0;
    for acc in account::all(pool)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
    {
        let cur = current_id(pool, acc.account_id, sh.trade_date).await?;
        let wanted = match cur {
            None => true,
            Some((_, touched)) => replace && !touched,
        };
        if !wanted {
            continue;
        }
        match generate_for(pool, &sh, &acc).await {
            Ok(_) => n += 1,
            Err(e) => {
                tracing::error!(account_id = acc.account_id, error = %format!("{e:#}"), "生成建议失败")
            }
        }
    }
    if n > 0 {
        tracing::info!(trade_date = %sh.trade_date, accounts = n, "已生成调仓建议");
    }
    Ok(n)
}

/// 执行循环每分钟调一次：还没有最新交易日建议的账户补上（例如刚建账户、刚部署）。
/// 同步还在运行时日线可能只有一部分，等它结束后由同步完成事件统一生成
pub async fn maintain(pool: &PgPool) -> anyhow::Result<()> {
    let syncing: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ops.run WHERE kind = 'incremental' AND status = 'running')",
    )
    .fetch_one(pool)
    .await?;
    if !syncing {
        refresh_all(pool, false).await?;
    }
    Ok(())
}

/// 一个账户某个交易日的建议；date 为空时取最新一份
pub async fn load(
    pool: &PgPool,
    account_id: i64,
    date: Option<NaiveDate>,
) -> anyhow::Result<Option<StoredAdvice>> {
    let row: Option<(i64, NaiveDate, DateTime<Utc>, serde_json::Value)> = sqlx::query_as(
        "SELECT advice_id, trade_date, created_at, result FROM app.advice
         WHERE account_id = $1 AND superseded_by IS NULL AND ($2::date IS NULL OR trade_date = $2) ORDER BY trade_date DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(date)
    .fetch_optional(pool)
    .await?;
    let Some((advice_id, trade_date, created_at, result)) = row else {
        return Ok(None);
    };
    let states: Vec<(String, String)> =
        sqlx::query_as("SELECT item_key, state FROM app.advice_item_state WHERE advice_id = $1")
            .bind(advice_id)
            .fetch_all(pool)
            .await?;
    Ok(Some(StoredAdvice {
        advice_id,
        trade_date,
        created_at,
        advice: serde_json::from_value(result)?,
        states: states.into_iter().collect(),
    }))
}

/// 一份建议，不论是否被取代（执行时用）
pub async fn load_by_id(
    pool: &PgPool,
    account_id: i64,
    advice_id: i64,
) -> anyhow::Result<Option<Advice>> {
    let v: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT result FROM app.advice WHERE account_id = $1 AND advice_id = $2",
    )
    .bind(account_id)
    .bind(advice_id)
    .fetch_optional(pool)
    .await?;
    Ok(v.map(serde_json::from_value).transpose()?)
}

#[derive(Serialize, ToSchema)]
pub struct AdviceDay {
    pub advice_id: i64,
    pub trade_date: NaiveDate,
    pub headline: String,
    pub sells: usize,
    pub buys: usize,
    /// 用户点了「已执行」的条数
    pub executed: i64,
    pub skipped: i64,
}

/// 每个交易日的最终一份建议，新的在前
pub async fn history(pool: &PgPool, account_id: i64, limit: i64) -> anyhow::Result<Vec<AdviceDay>> {
    let rows: Vec<(i64, NaiveDate, serde_json::Value, i64, i64)> = sqlx::query_as(
        "SELECT a.advice_id, a.trade_date, a.result,
                (SELECT count(*) FROM app.advice_item_state s WHERE s.advice_id = a.advice_id AND s.state = 'executed'),
                (SELECT count(*) FROM app.advice_item_state s WHERE s.advice_id = a.advice_id AND s.state = 'skipped')
         FROM app.advice a WHERE a.account_id = $1 AND a.superseded_by IS NULL ORDER BY a.trade_date DESC LIMIT $2",
    )
    .bind(account_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(advice_id, trade_date, v, executed, skipped)| {
            let a: Advice = serde_json::from_value(v)?;
            let count = |x| a.items.iter().filter(|i| i.action == x).count();
            Ok(AdviceDay {
                advice_id,
                trade_date,
                headline: a.headline.clone(),
                sells: count(generate::Action::Sell),
                buys: count(generate::Action::Buy),
                executed,
                skipped,
            })
        })
        .collect()
}
