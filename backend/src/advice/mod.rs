//! 每日调仓建议：每次同步加巡检完成后，为每个账户生成一份「明天怎么操作」。
//!
//! - `generate`：纯函数，从输入算出建议和每条的理由；
//! - `inputs`：从数据库组装输入；
//! - `stats`、`backtest`：策略的过去表现和账户规则回测，供用户主动研究；不参与账户建议，也不由后台定期重算；
//! - `review`：过去的建议之后 5、10、20 个交易日的实际涨跌；
//! - 本文件：保存、重新生成和每天为所有账户生成建议。
//!
//! 依赖方向：advice 读 account 和 analysis，analysis 不知道账户的存在。规则见 docs/domains/accounts.md。

pub mod backtest;
pub mod generate;
pub mod inputs;
pub mod review;
pub mod stats;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::account::{self, Account};
use generate::{Advice, RULES_VERSION};

/// 一份保存下来的建议，以及用户对每条的处理
#[derive(Serialize, ToSchema)]
pub struct StoredAdvice {
    pub advice_id: i64,
    pub trade_date: NaiveDate,
    pub created_at: DateTime<Utc>,
    pub rules_version: i32,
    pub advice: Advice,
    /// item key → executed / skipped
    pub states: std::collections::BTreeMap<String, String>,
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
    sqlx::query("INSERT INTO app.advice (advice_id, account_id, trade_date, rules_version, input, result) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(id)
        .bind(acc.account_id)
        .bind(sh.trade_date)
        .bind(RULES_VERSION)
        .bind(serde_json::to_value(&input)?)
        .bind(serde_json::to_value(&advice)?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}

/// 持仓、现金或设置改了之后：立即重新生成这个账户的建议。还没有巡检结果时不生成
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

/// 为所有账户生成最新交易日的建议。`replace` 为 true 时（巡检刚重跑过、策略表现刚统计完）连已有的一起重新生成，
/// 但用户已经处理过某一条（执行或不做）的保留，免得建议在手上变了
pub async fn refresh_all(pool: &PgPool, replace: bool) -> anyhow::Result<usize> {
    if !replace {
        let Some(trade_date) = inputs::latest_screen_date(pool).await? else {
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

/// 执行循环每分钟调一次：
/// - 参加巡检的策略在最新交易日还没有巡检（例如刚部署、刚加了内置策略）时补一次巡检，每天最多一次；
/// - 还没有当天建议的账户补上；
/// - 账户建议只使用用户勾选的策略和当前规则，不用历史回测数字做决策
pub async fn maintain(pool: &PgPool) -> anyhow::Result<()> {
    let latest_bar = crate::analysis::market::resolve_date(pool, None).await?;
    let screened = inputs::latest_screen_date(pool).await?;
    let needs_screen = match latest_bar {
        Some(latest) => match screened {
            Some(d) if d == latest => !inputs::screening_complete(pool, latest).await?,
            _ => true,
        },
        None => false,
    };
    if needs_screen
        && crate::sync::scheduler::claim(pool, "builtin_screen", crate::time::today_cn(), "day")
            .await?
    {
        tracing::info!("参加巡检的策略在最新交易日还没有巡检，补一次");
        crate::analysis::screener::run_and_store(pool, None, None).await?;
    }
    refresh_all(pool, false).await?;
    Ok(())
}

/// 一个账户某个交易日的建议；date 为空时取最新一份
pub async fn load(
    pool: &PgPool,
    account_id: i64,
    date: Option<NaiveDate>,
) -> anyhow::Result<Option<StoredAdvice>> {
    let row: Option<(i64, NaiveDate, DateTime<Utc>, i32, serde_json::Value)> = sqlx::query_as(
        "SELECT advice_id, trade_date, created_at, rules_version, result FROM app.advice
         WHERE account_id = $1 AND superseded_by IS NULL AND ($2::date IS NULL OR trade_date = $2) ORDER BY trade_date DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(date)
    .fetch_optional(pool)
    .await?;
    let Some((advice_id, trade_date, created_at, rules_version, result)) = row else {
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
        rules_version,
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
    pub reduces: usize,
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
                reduces: count(generate::Action::Reduce),
                buys: count(generate::Action::Buy),
                executed,
                skipped,
            })
        })
        .collect()
}
