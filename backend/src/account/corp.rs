//! 分红送转自动记账：持有的股票到了除权除息日，按公告（`mkt.adj_event_effective`）给账户记账，
//! 用户不用自己改持仓。只处理股数最近一次变动之后（`position.since` 之后）的除权日：
//! 某天买入的股票，下一个交易日起的除权才有份；校准过的持仓，校准之前的除权不再重复记。
//!
//! - 送转股：股数按比例增加，不足 1 股舍去；
//! - 现金分红：按税前金额加到现金（实际到账要扣红利税，和持有时间有关，提示用户可以校准）；
//! - 成本价摊薄：(原成本 × 原股数 − 分红) ÷ 新股数；
//! - 配股不自动记账：要不要认购由用户决定。除权日过去、还没处理过的配股，在账户页「待处理公司行动」里提醒
//!   （只看最近 180 天，缴款期早就过了的不再提醒），用户可以「记一笔配股认购」或「暂不处理」；
//!   处理状态记在 `app.account_corp_action`，处理过就不再提醒。
//!
//! 规则见 docs/domains/accounts.md#corp_actions。

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::lots::{Fees, round2};
use super::{
    AccountError, Result as AccResult, TradeKind, TradeSource, account_fees, add_cash,
    insert_trade, invalid, lock,
};

/// 一只待处理的配股
#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct PendingRights {
    pub thscode: String,
    pub name: String,
    /// 除权日
    pub ex_date: NaiveDate,
    /// 每 10 股配几股
    pub rights_per_10: f64,
    /// 配股价（元）
    pub rights_price: f64,
    /// 除权日持有的股数（按有效流水算）
    pub held_shares: i64,
    /// 这次可以认购的股数（股数 × 配股比例，不足 1 股舍去）
    pub rights_shares: i64,
    /// 配股款（配股价 × 认购股数），不含费用
    pub amount: f64,
    /// 按账户费率估算的费用
    pub fee: f64,
}

/// 配股提醒只看最近多少天：缴款期通常只有几个交易日，更早的已经没法认购，不再提醒
const RIGHTS_WINDOW_DAYS: i64 = 180;

// @project-doc docs/domains/accounts.md#corp_actions
/// 这个账户待处理的配股：除权日已过、按流水算当时持有股票、还没认购也没选「暂不处理」的。
/// `through` 是最新交易日，晚于它的除权日还没到，不提醒。
pub async fn pending_rights(
    pool: &PgPool,
    account_id: i64,
    through: NaiveDate,
) -> anyhow::Result<Vec<PendingRights>> {
    let since = through - chrono::Duration::days(RIGHTS_WINDOW_DAYS);
    let rows: Vec<(String, String, NaiveDate, f64, f64, i64)> = sqlx::query_as(
        "SELECT p.thscode, s.name, e.ex_date, e.rights_ratio::float8, e.rights_price::float8,
                COALESCE((SELECT sum(t.shares) FROM app.account_trade t
                          WHERE t.account_id = p.account_id AND t.thscode = p.thscode
                            AND t.reversed_at IS NULL AND t.trade_date <= e.ex_date), 0)::bigint
         FROM app.position p
         JOIN mkt.adj_event_effective e ON e.thscode = p.thscode AND e.ex_date <= $2 AND e.ex_date >= $3
         JOIN ref.security s ON s.thscode = p.thscode
         WHERE p.account_id = $1 AND e.rights_ratio > 0 AND e.rights_price > 0
           AND NOT EXISTS (SELECT 1 FROM app.account_corp_action c
                           WHERE c.account_id = p.account_id AND c.thscode = p.thscode
                             AND c.ex_date = e.ex_date AND c.kind = 'rights')
         ORDER BY e.ex_date, p.thscode",
    )
    .bind(account_id)
    .bind(through)
    .bind(since)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let (commission_rate, min_commission): (f64, f64) =
        sqlx::query_as("SELECT commission_rate::float8, min_commission::float8 FROM app.account WHERE account_id = $1").bind(account_id).fetch_one(pool).await?;
    let fees = Fees {
        commission_rate,
        min_commission,
    };
    Ok(rows
        .into_iter()
        .filter_map(|(thscode, name, ex_date, ratio, price, held)| {
            let rights_shares = entitlement(held, 0.0, ratio).0;
            if rights_shares <= 0 {
                return None;
            }
            let amount = round2(rights_shares as f64 * price);
            let fee = fees.buy_fee(amount);
            Some(PendingRights {
                thscode,
                name,
                ex_date,
                rights_per_10: round2(ratio * 10.0),
                rights_price: price,
                held_shares: held,
                rights_shares,
                amount,
                fee,
            })
        })
        .collect())
}

/// 除权日持有的股数：这一天（含）之前全部有效流水的股数变化之和
async fn held_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: i64,
    code: &str,
    ex_date: NaiveDate,
) -> AccResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(sum(shares), 0)::bigint FROM app.account_trade
         WHERE account_id = $1 AND thscode = $2 AND reversed_at IS NULL AND trade_date <= $3",
    )
    .bind(account_id)
    .bind(code)
    .bind(ex_date)
    .fetch_one(&mut **tx)
    .await?)
}

/// 处理过的配股状态；空表示还没处理过
async fn handled(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    account_id: i64,
    code: &str,
    ex_date: NaiveDate,
) -> AccResult<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT state FROM app.account_corp_action WHERE account_id = $1 AND thscode = $2 AND ex_date = $3 AND kind = 'rights'",
    )
    .bind(account_id)
    .bind(code)
    .bind(ex_date)
    .fetch_optional(&mut **tx)
    .await?)
}

fn already_handled(state: &str) -> AccountError {
    AccountError::Invalid(match state {
        "ignored" => "这笔配股已经选过「暂不处理」了；要认购请在持仓页校准".to_string(),
        _ => "这笔配股已经记过认购了".to_string(),
    })
}

/// 记一笔配股认购：按除权日持有的股数 × 配股比例算出认购股数，扣配股款和费用，持仓按平均成本更新，
/// 同一事务里记流水和「已认购」状态。现金不够时拒绝。
pub async fn subscribe_rights(
    pool: &PgPool,
    user_id: i64,
    account_id: i64,
    code: &str,
    ex_date: NaiveDate,
) -> AccResult<i64> {
    let mut tx = pool.begin().await?;
    let cash = lock(&mut tx, user_id, account_id).await?;
    let fees = account_fees(&mut tx, account_id).await?;
    let event: Option<(f64, f64)> = sqlx::query_as("SELECT rights_ratio::float8, rights_price::float8 FROM mkt.adj_event_effective WHERE thscode = $1 AND ex_date = $2")
        .bind(code)
        .bind(ex_date)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((ratio, price)) = event.filter(|(r, p)| *r > 0.0 && *p > 0.0) else {
        return invalid("这一天没有配股记录");
    };
    if let Some(state) = handled(&mut tx, account_id, code, ex_date).await? {
        return Err(already_handled(&state));
    }
    let held = held_on(&mut tx, account_id, code, ex_date).await?;
    let rights_shares = entitlement(held, 0.0, ratio).0;
    if rights_shares <= 0 {
        return invalid("按除权日持有的股数，这次配股不足 1 股，不用认购");
    }
    let pos: Option<(i64, Option<f64>)> =
        sqlx::query_as("SELECT shares, cost_price::float8 FROM app.position WHERE account_id = $1 AND thscode = $2 FOR UPDATE")
            .bind(account_id)
            .bind(code)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((shares, cost)) = pos else {
        return invalid("这只股票已经没有持仓了，这笔配股不用处理");
    };
    let amount = round2(rights_shares as f64 * price);
    let fee = fees.buy_fee(amount);
    let pay = amount + fee;
    if pay > cash + 0.005 {
        return invalid(format!(
            "现金不够：认购 {rights_shares} 股要 ¥{pay:.2}（配股款 ¥{amount:.2} 加费用 ¥{fee:.2}），现金只有 ¥{cash:.2}。先转入现金，或在持仓页校准现金"
        ));
    }
    let new_shares = shares + rights_shares;
    let new_cost = cost.map(|c| (c * shares as f64 + pay) / new_shares as f64);
    let today = crate::time::today_cn();
    sqlx::query(
        "UPDATE app.position SET shares = $3, cost_price = round($4::float8::numeric, 4), since = $5 WHERE account_id = $1 AND thscode = $2",
    )
    .bind(account_id)
    .bind(code)
    .bind(new_shares)
    .bind(new_cost)
    .bind(today)
    .execute(&mut *tx)
    .await?;
    let note = format!(
        "配股认购：每 10 股配 {:.2} 股，配股价 ¥{:.4}，除权日 {ex_date}，认购 {rights_shares} 股，配股款 ¥{amount:.2}",
        round2(ratio * 10.0),
        price
    );
    let trade_id = insert_trade(
        &mut tx,
        account_id,
        TradeKind::Rights,
        today,
        Some(code),
        Some(rights_shares),
        Some(price),
        -pay,
        fee,
        TradeSource::CorpAction,
        &note,
    )
    .await?;
    sqlx::query(
        "INSERT INTO app.account_corp_action (account_id, thscode, ex_date, kind, state, shares, trade_id)
         VALUES ($1, $2, $3, 'rights', 'subscribed', $4, $5)",
    )
    .bind(account_id)
    .bind(code)
    .bind(ex_date)
    .bind(rights_shares)
    .bind(trade_id)
    .execute(&mut *tx)
    .await?;
    add_cash(&mut tx, account_id, -pay).await?;
    tx.commit().await?;
    Ok(trade_id)
}

/// 暂不处理这次配股：不买、不扣钱，只记下处理状态，提醒消失
pub async fn ignore_rights(
    pool: &PgPool,
    user_id: i64,
    account_id: i64,
    code: &str,
    ex_date: NaiveDate,
) -> AccResult<()> {
    let mut tx = pool.begin().await?;
    lock(&mut tx, user_id, account_id).await?;
    let event: Option<f64> = sqlx::query_scalar("SELECT rights_ratio::float8 FROM mkt.adj_event_effective WHERE thscode = $1 AND ex_date = $2")
        .bind(code)
        .bind(ex_date)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(ratio) = event.filter(|r| *r > 0.0) else {
        return invalid("这一天没有配股记录");
    };
    if let Some(state) = handled(&mut tx, account_id, code, ex_date).await? {
        return Err(already_handled(&state));
    }
    let held = held_on(&mut tx, account_id, code, ex_date).await?;
    if entitlement(held, 0.0, ratio).0 <= 0 {
        return invalid("按除权日持有的股数，这次配股不足 1 股，不用处理");
    }
    sqlx::query("INSERT INTO app.account_corp_action (account_id, thscode, ex_date, kind, state) VALUES ($1, $2, $3, 'rights', 'ignored')")
        .bind(account_id)
        .bind(code)
        .bind(ex_date)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

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
    // 每天生成建议前会走到这里，顺手把当天的净值快照写上，让净值曲线不断天。
    // 净值自己按流水和收盘价重算；写失败只记日志，不影响分红送转记账。
    if let Err(e) = super::equity::refresh_all(pool, through).await {
        tracing::warn!(error = %format!("{e:#}"), "写每日净值快照失败");
    }
    Ok(n)
}

async fn apply_one(
    pool: &PgPool,
    account_id: i64,
    code: &str,
    ex_date: NaiveDate,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT 1 FROM app.account WHERE account_id = $1 FOR UPDATE")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    let pos: Option<(i64, Option<f64>, NaiveDate)> =
        sqlx::query_as("SELECT shares, cost_price::float8, since FROM app.position WHERE account_id = $1 AND thscode = $2 FOR UPDATE")
            .bind(account_id)
            .bind(code)
            .fetch_optional(&mut *tx)
            .await?;
    // 排队期间持仓可能已经变了：重新判断
    let Some((shares, cost, since)) = pos.filter(|p| p.2 < ex_date) else {
        return Ok(false);
    };
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
        let note =
            format!("每股分红 {cash_div:.4} 元（税前，实际到账要扣红利税，可以在持仓页校准现金）");
        insert_trade(
            &mut tx,
            account_id,
            TradeKind::Dividend,
            ex_date,
            Some(code),
            None,
            Some(cash_div),
            dividend,
            0.0,
            TradeSource::CorpAction,
            &note,
        )
        .await?;
    }
    if bonus > 0 {
        let note = format!(
            "每 10 股送转 {:.2} 股，{shares} 股 → {new_shares} 股",
            bonus_ratio * 10.0
        );
        insert_trade(
            &mut tx,
            account_id,
            TradeKind::Bonus,
            ex_date,
            Some(code),
            Some(bonus),
            None,
            0.0,
            0.0,
            TradeSource::CorpAction,
            &note,
        )
        .await?;
    }
    tracing::debug!(account_id, code, %since, %ex_date, bonus, dividend, "分红送转记账");
    tx.commit().await?;
    Ok(true)
}

/// 应得的送转股（不足 1 股舍去）和税前分红
fn entitlement(shares: i64, cash_div: f64, bonus_ratio: f64) -> (i64, f64) {
    (
        (shares as f64 * bonus_ratio + 1e-9).floor() as i64,
        round2(shares as f64 * cash_div),
    )
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
