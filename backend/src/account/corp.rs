//! 分红送转自动记账：持有的股票到了除权除息日，按公告（`mkt.adj_event_effective`）给账户记账，
//! 用户不用自己改持仓。只处理股数最近一次变动之后（`position.since` 之后）的除权日：
//! 某天买入的股票，下一个交易日起的除权才有份；校准过的持仓，校准之前的除权不再重复记。
//!
//! - 送转股：股数按比例增加，不足 1 股舍去；
//! - 现金分红：按税前金额加到现金（实际到账要扣红利税，和持有时间有关，提示用户可以校准）；
//! - 成本价摊薄：(原成本 × 原股数 − 分红) ÷ 新股数；
//! - 配股不处理：要不要认购是用户自己的决定，认购后在持仓页校准。
//!
//! 规则见 docs/domains/accounts.md#corp_actions。

use chrono::NaiveDate;
use sqlx::PgPool;

use super::lots::round2;
use super::{TradeKind, TradeSource, insert_trade};

/// 给所有账户补记 through（含）之前的除权除息，返回记了几次
pub async fn apply(pool: &PgPool, through: NaiveDate) -> anyhow::Result<usize> {
    let due: Vec<(i64, String, NaiveDate)> = sqlx::query_as(
        "SELECT p.account_id, p.thscode, e.ex_date FROM app.position p
         JOIN mkt.adj_event_effective e ON e.thscode = p.thscode AND e.ex_date > p.since AND e.ex_date <= $1
         WHERE e.cash_div > 0 OR e.bonus_ratio > 0
         ORDER BY p.account_id, p.thscode, e.ex_date",
    )
    .bind(through)
    .fetch_all(pool)
    .await?;
    let mut n = 0;
    for (account_id, code, ex_date) in due {
        if apply_one(pool, account_id, &code, ex_date).await? {
            n += 1;
        }
    }
    if n > 0 {
        tracing::info!(n, "已按分红送转公告给账户记账");
    }
    Ok(n)
}

async fn apply_one(pool: &PgPool, account_id: i64, code: &str, ex_date: NaiveDate) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT 1 FROM app.account WHERE account_id = $1 FOR UPDATE").bind(account_id).execute(&mut *tx).await?;
    let pos: Option<(i64, Option<f64>, NaiveDate)> =
        sqlx::query_as("SELECT shares, cost_price::float8, since FROM app.position WHERE account_id = $1 AND thscode = $2 FOR UPDATE")
            .bind(account_id)
            .bind(code)
            .fetch_optional(&mut *tx)
            .await?;
    // 排队期间持仓可能已经变了：重新判断
    let Some((shares, cost, since)) = pos.filter(|p| p.2 < ex_date) else { return Ok(false) };
    let (cash_div, bonus_ratio): (f64, f64) =
        sqlx::query_as("SELECT cash_div::float8, bonus_ratio::float8 FROM mkt.adj_event_effective WHERE thscode = $1 AND ex_date = $2")
            .bind(code)
            .bind(ex_date)
            .fetch_one(&mut *tx)
            .await?;
    let (bonus, dividend) = entitlement(shares, cash_div, bonus_ratio);
    let new_shares = shares + bonus;
    let new_cost = cost.map(|c| ((c * shares as f64 - dividend) / new_shares as f64).max(0.0001));
    sqlx::query("UPDATE app.position SET shares = $3, cost_price = round($4::float8::numeric, 4), since = $5 WHERE account_id = $1 AND thscode = $2")
        .bind(account_id)
        .bind(code)
        .bind(new_shares)
        .bind(new_cost)
        .bind(ex_date)
        .execute(&mut *tx)
        .await?;
    if dividend > 0.0 {
        sqlx::query("UPDATE app.account SET cash = cash + round($2::float8::numeric, 2) WHERE account_id = $1").bind(account_id).bind(dividend).execute(&mut *tx).await?;
        let note = format!("每股分红 {cash_div:.4} 元（税前，实际到账要扣红利税，可以在持仓页校准现金）");
        insert_trade(&mut tx, account_id, TradeKind::Dividend, ex_date, Some(code), None, Some(cash_div), dividend, 0.0, TradeSource::CorpAction, &note).await?;
    }
    if bonus > 0 {
        let note = format!("每 10 股送转 {:.2} 股，{shares} 股 → {new_shares} 股", bonus_ratio * 10.0);
        insert_trade(&mut tx, account_id, TradeKind::Bonus, ex_date, Some(code), Some(bonus), None, 0.0, 0.0, TradeSource::CorpAction, &note).await?;
    }
    tracing::debug!(account_id, code, %since, %ex_date, bonus, dividend, "分红送转记账");
    tx.commit().await?;
    Ok(true)
}

/// 应得的送转股（不足 1 股舍去）和税前分红
fn entitlement(shares: i64, cash_div: f64, bonus_ratio: f64) -> (i64, f64) {
    ((shares as f64 * bonus_ratio + 1e-9).floor() as i64, round2(shares as f64 * cash_div))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bonus_rounds_down_and_dividend_is_pre_tax() {
        // 每 10 股送 3 股派 2.5 元
        assert_eq!(entitlement(1_000, 0.25, 0.3), (300, 250.0));
        assert_eq!(entitlement(333, 0.0, 0.3), (99, 0.0), "99.9 股舍去为 99");
        assert_eq!(entitlement(100, 1.234_56, 0.0), (0, 123.46));
    }
}
