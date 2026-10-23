//! 从数据库组装生成建议的输入：所有账户共用的部分（交易日、市场、证券信息）读一次，
//! 每个账户再按自己的因子排名读持仓股和候选股的日线（一批证券一次查询，不逐只读）。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate, Weekday};
use sqlx::PgPool;

use super::generate::{CandidateIn, HoldingIn, Input, MarketIn, Snapshot};
use super::ranking;
use crate::account::{self, Account};
use crate::analysis::market;
use crate::analysis::prices::{self, DailyBar};
use crate::analysis::securities::{self, Securities};
use crate::instrument::AssetType;

/// 每只股票读多少根日线：覆盖近 60 个交易日的价格区间
const BARS: i64 = 61;
/// 除了填满空位需要的只数，再多看几只候选（ST、买不起时顺延）
const EXTRA_CANDIDATES: usize = 20;

/// 所有账户共用的输入
pub struct Shared {
    pub trade_date: NaiveDate,
    pub next_date: NaiveDate,
    pub market: Option<MarketIn>,
    sec: Securities,
}

/// 交易日之后的下一个交易日：本地日历有就用，没有时取下一个工作日
pub async fn next_trading_day(pool: &PgPool, d: NaiveDate) -> sqlx::Result<NaiveDate> {
    let next: Option<NaiveDate> =
        sqlx::query_scalar("SELECT min(cal_date) FROM ref.trading_day WHERE cal_date > $1")
            .bind(d)
            .fetch_one(pool)
            .await?;
    Ok(next.unwrap_or_else(|| {
        let mut n = d.succ_opt().unwrap_or(d);
        while matches!(n.weekday(), Weekday::Sat | Weekday::Sun) {
            n = n.succ_opt().unwrap_or(n);
        }
        n
    }))
}

/// 建议的交易日是本地最新一根日线的日期。核心数据的首次回补还没完成、或者还没有日线时为空
pub async fn shared(pool: &PgPool) -> anyhow::Result<Option<Shared>> {
    // 日线只有一部分股票、一部分年份时，算出来的建议不可信，先不生成
    if crate::readiness::core_backfill_left(pool).await? > 0 {
        return Ok(None);
    }
    let Some(trade_date) = market::resolve_date(pool, None).await? else {
        return Ok(None);
    };
    let next_date = next_trading_day(pool, trade_date).await?;
    let market = market::overview(pool, Some(trade_date))
        .await?
        .map(|o| MarketIn {
            above_ma20_pct: o.above_ma20 * 100.0,
        });
    Ok(Some(Shared {
        trade_date,
        next_date,
        market,
        sec: securities::load(pool).await?,
    }))
}

/// 一只股票在最后一根日线上的状态
fn snapshot(bars: &[DailyBar]) -> Option<Snapshot> {
    let last = bars.last()?;
    let n = bars.len();
    let momentum20 = (n > 20).then(|| (last.q_close / bars[n - 21].q_close - 1.0) * 100.0);
    let window = &bars[n.saturating_sub(60)..];
    let hi = window.iter().map(|b| b.q_high).fold(f64::MIN, f64::max);
    let lo = window.iter().map(|b| b.q_low).fold(f64::MAX, f64::min);
    Some(Snapshot {
        close: last.close,
        amount: last.amount,
        momentum20,
        range60_pos: (n >= 60 && hi > lo).then(|| (last.q_close - lo) / (hi - lo)),
    })
}

/// 之前的建议中连续给出、还没执行的卖出：key → 连续天数
async fn streaks(
    pool: &PgPool,
    account_id: i64,
    trade_date: NaiveDate,
) -> anyhow::Result<BTreeMap<String, u32>> {
    let prev: Option<(i64, serde_json::Value)> = sqlx::query_as(
        "SELECT advice_id, result FROM app.advice WHERE account_id = $1 AND trade_date < $2 AND superseded_by IS NULL ORDER BY trade_date DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(trade_date)
    .fetch_optional(pool)
    .await?;
    let Some((advice_id, result)) = prev else {
        return Ok(BTreeMap::new());
    };
    let executed: Vec<String> = sqlx::query_scalar(
        "SELECT item_key FROM app.advice_item_state WHERE advice_id = $1 AND state = 'executed'",
    )
    .bind(advice_id)
    .fetch_all(pool)
    .await?;
    let advice: super::generate::Advice = serde_json::from_value(result)?;
    Ok(advice
        .items
        .iter()
        .filter(|i| i.streak > 0 && !executed.contains(&i.key))
        .map(|i| (i.key.clone(), i.streak))
        .collect())
}

pub async fn input_for(pool: &PgPool, sh: &Shared, acc: &Account) -> anyhow::Result<Input> {
    let rules = acc.rules.clone();
    let ranked = ranking::ranking(pool, sh.trade_date, &rules).await?;
    let rank_of: HashMap<&str, usize> = ranked
        .iter()
        .enumerate()
        .map(|(r, (code, _))| (code.as_str(), r + 1))
        .collect();
    let positions = account::positions(pool, acc.account_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let held: Vec<String> = positions.iter().map(|p| p.thscode.clone()).collect();
    let mut held_bars = prices::recent_bars_batch(pool, &held, Some(sh.trade_date), BARS).await?;
    let mut holdings = vec![];
    for p in &positions {
        let meta = sh.sec.meta.get(&p.thscode);
        let bars = held_bars.remove(&p.thscode).unwrap_or_default();
        holdings.push(HoldingIn {
            thscode: p.thscode.clone(),
            name: sh
                .sec
                .names
                .get(&p.thscode)
                .cloned()
                .unwrap_or_else(|| p.thscode.clone()),
            board: meta.map_or_else(|| "main".into(), |m| m.board.clone()),
            // 证券表里没有的代码（例如已经被供应商删掉）按股票的交易规则估算
            asset_type: meta.map_or(AssetType::Stock, |m| m.asset_type),
            shares: p.shares,
            cost_price: p.cost_price,
            is_st: meta.is_some_and(|m| m.is_st),
            delisted: meta.is_some_and(|m| m.delisted_on.is_some()),
            last_date: bars.last().map(|b| b.date),
            rank: rank_of.get(p.thscode.as_str()).copied(),
            snap: snapshot(&bars),
        });
    }

    // 候选：排名靠前、还没持有的，够填满空位再多看几只。按名次一批批读日线，
    // 没有证券信息或日线的跳过、由后面的名次顺延
    let wanted = rules.top_n as usize + EXTRA_CANDIDATES;
    let mut candidates = vec![];
    let open: Vec<(usize, &(String, f64))> = ranked
        .iter()
        .enumerate()
        .filter(|(_, (code, _))| !held.contains(code) && sh.sec.meta.contains_key(code))
        .collect();
    for chunk in open.chunks(wanted) {
        if candidates.len() >= wanted {
            break;
        }
        let codes: Vec<String> = chunk.iter().map(|(_, (code, _))| code.clone()).collect();
        let mut bars = prices::recent_bars_batch(pool, &codes, Some(sh.trade_date), BARS).await?;
        for (r, (code, score)) in chunk {
            if candidates.len() >= wanted {
                break;
            }
            let Some(snap) = bars.remove(code).and_then(|b| snapshot(&b)) else {
                continue;
            };
            let meta = &sh.sec.meta[code];
            candidates.push(CandidateIn {
                thscode: code.clone(),
                name: sh.sec.names.get(code).cloned().unwrap_or_default(),
                board: meta.board.clone(),
                is_st: meta.is_st,
                rank: r + 1,
                score: *score,
                snap,
            });
        }
    }
    Ok(Input {
        trade_date: sh.trade_date,
        next_date: sh.next_date,
        rules,
        fees: acc.fees,
        cash: acc.cash,
        market: sh.market,
        holdings,
        rebalance: ranking::month_end(sh.trade_date, sh.next_date),
        ranked: ranked.len(),
        candidates,
        streaks: streaks(pool, acc.account_id, sh.trade_date).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reads_momentum_and_range() {
        let closes: Vec<f64> = (0..61).map(|i| 10.0 + i as f64 * 0.1).collect();
        let s = snapshot(&prices::bars_from(&closes)).unwrap();
        assert!((s.momentum20.unwrap() - (16.0 / 14.0 - 1.0) * 100.0).abs() < 1e-9);
        assert!(s.range60_pos.unwrap() > 0.9);
        assert!(
            snapshot(&prices::bars_from(&[10.0]))
                .unwrap()
                .momentum20
                .is_none()
        );
    }
}
