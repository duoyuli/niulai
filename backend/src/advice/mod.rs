//! 每日调仓建议：每次同步完成后，为每个账户生成一份「明天怎么操作」。
//!
//! - `ranking`：按账户的因子给全市场排名，判断调仓日；
//! - `generate`：纯函数，从输入算出建议和每条的理由；
//! - `inputs`：从数据库组装输入；
//! - `review`：过去的建议之后 5、10、20 个交易日的实际涨跌；
//! - `handling`：用户对一条建议的处理（已执行、不做、撤销不做），流水、处理状态和影子订单在一个事务里；
//! - `regen`：改了账户之后重新生成建议的常驻消费者（待办在 `account::stale`）；
//! - 本文件：保存、为所有账户生成建议、维护和清理被取代的建议。
//!
//! 依赖方向：advice 读 account 和 analysis，analysis 不知道账户的存在。规则见 docs/domains/accounts.md。

pub mod generate;
pub mod handling;
pub mod inputs;
pub mod ranking;
pub mod regen;
pub mod review;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::account::{self, Account};
use generate::{Action, Advice};

/// 用户对一条建议的处理，存在 `app.advice_item_state.state`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    /// 已执行：对应一笔按建议记的流水
    Executed,
    /// 不做（可以撤销）
    Skipped,
}

impl ItemState {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "executed" => Some(Self::Executed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// 需要用户处理（执行或不做）的动作
pub fn actionable(action: Action) -> bool {
    matches!(action, Action::Buy | Action::Sell)
}

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
            Action::Buy => Some("buy"),
            Action::Sell => Some("sell"),
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

/// 按现在的账户立即重新生成建议（页面上的「重新生成」和 [`regen`] 用）。核心数据没齐或还没有日线时不生成
pub async fn regenerate(pool: &PgPool, acc: &Account) -> anyhow::Result<Option<i64>> {
    let Some(sh) = inputs::shared(pool).await? else {
        return Ok(None);
    };
    Ok(Some(generate_for(pool, &sh, acc).await?))
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

/// 为所有账户生成最新交易日的建议。`replace` 为 true 时（同步刚完成，行情、涨跌停池或因子输入有变化）
/// 连已有的一起重新生成，但用户已经处理过某一条（执行或不做）的保留，免得建议在手上变了。
/// 因子排名缓存按数据版本失效（见 [`ranking`]），这里不用清。某个账户生成失败时转成它的重算待办
/// （[`account::stale::defer`]），由 [`regen`] 重试，不影响其他账户，也不让调用方把同步事件一直挂着
pub async fn refresh_all(pool: &PgPool, replace: bool) -> anyhow::Result<usize> {
    if !replace {
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
                let msg = format!("{e:#}");
                tracing::error!(account_id = acc.account_id, error = %msg, "生成建议失败，转为重算待办");
                account::stale::defer(pool, acc.account_id, &msg).await?;
            }
        }
    }
    if n > 0 {
        tracing::info!(trade_date = %sh.trade_date, accounts = n, "已生成调仓建议");
    }
    Ok(n)
}

/// 执行循环每分钟调一次：还没有最新交易日建议的账户补上（例如刚建账户、刚部署），每天清理一次被取代的建议。
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
    prune_if_due(pool).await
}

/// 被重新生成取代超过多少天、没有处理记录的建议删掉（改一次持仓或设置就会取代一份，会越积越多）
pub const SUPERSEDED_KEEP_DAYS: i32 = 30;

/// 每天一次（抢占 `ops.scheduler` 的 `advice_prune` 作业，多个进程并存时也只做一次）
async fn prune_if_due(pool: &PgPool) -> anyhow::Result<()> {
    let today = crate::time::today_cn();
    if !crate::scheduling::claim(pool, "advice_prune", today, "day").await? {
        return Ok(());
    }
    match prune(pool).await {
        Ok(n) => {
            if n > 0 {
                tracing::info!(advice = n, "已清理被取代的调仓建议");
            }
            Ok(())
        }
        Err(e) => {
            crate::scheduling::release_claim(pool, "advice_prune", today).await?;
            Err(e)
        }
    }
}

// @project-doc docs/domains/accounts.md#generation
/// 删掉被取代超过 [`SUPERSEDED_KEEP_DAYS`] 天、没有处理记录的建议，分批删，不长时间锁表
pub async fn prune(pool: &PgPool) -> anyhow::Result<u64> {
    let mut total = 0;
    loop {
        let n = sqlx::query(
            "DELETE FROM app.advice WHERE advice_id IN (
               SELECT a.advice_id FROM app.advice a
               WHERE a.superseded_by IS NOT NULL AND a.created_at < now() - make_interval(days => $1)
                 AND NOT EXISTS (SELECT 1 FROM app.advice_item_state s WHERE s.advice_id = a.advice_id)
               LIMIT 20000)",
        )
        .bind(SUPERSEDED_KEEP_DAYS)
        .execute(pool)
        .await?
        .rows_affected();
        total += n;
        if n < 20_000 {
            return Ok(total);
        }
    }
}

/// 账户在 `since` 之后生成的建议里，有没有一份有要操作的条目、却一条都没处理（核对提醒用）
pub async fn has_unhandled_since(
    pool: &PgPool,
    account_id: i64,
    since: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let rows: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT a.result FROM app.advice a WHERE a.account_id = $1 AND a.superseded_by IS NULL AND a.created_at > $2
           AND NOT EXISTS (SELECT 1 FROM app.advice_item_state s WHERE s.advice_id = a.advice_id)",
    )
    .bind(account_id)
    .bind(since)
    .fetch_all(pool)
    .await?;
    for v in rows {
        let a: Advice = serde_json::from_value(v)?;
        if a.items.iter().any(|i| actionable(i.action)) {
            return Ok(true);
        }
    }
    Ok(false)
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
                (SELECT count(*) FROM app.advice_item_state s WHERE s.advice_id = a.advice_id AND s.state = $3),
                (SELECT count(*) FROM app.advice_item_state s WHERE s.advice_id = a.advice_id AND s.state = $4)
         FROM app.advice a WHERE a.account_id = $1 AND a.superseded_by IS NULL ORDER BY a.trade_date DESC LIMIT $2",
    )
    .bind(account_id)
    .bind(limit)
    .bind(ItemState::Executed.key())
    .bind(ItemState::Skipped.key())
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
                sells: count(Action::Sell),
                buys: count(Action::Buy),
                executed,
                skipped,
            })
        })
        .collect()
}
