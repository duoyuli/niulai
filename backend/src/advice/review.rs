//! 过去的建议准不准：每条买入、卖出、减仓建议之后 5、10、20 个交易日，这只股票实际涨跌多少（前复权，含分红送转）。
//! 买入建议之后涨了是好的；卖出建议之后跌了说明卖对了。区分用户执行了、没做、没处理的，
//! 让用户看到「照着做」和「没照着做」的差别。规则见 docs/domains/accounts.md#review。

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::generate::{Action, Advice};

/// 回看最近多少个交易日的建议
const DAYS: i64 = 180;
const HORIZONS: [usize; 3] = [5, 10, 20];

#[derive(Serialize, ToSchema)]
pub struct ReviewItem {
    pub trade_date: NaiveDate,
    pub action: Action,
    pub thscode: String,
    pub name: String,
    pub headline: String,
    /// executed / skipped；没处理时为空
    pub state: Option<String>,
    /// 之后 5、10、20 个交易日的涨跌（%）；还不到那么多天时为空
    pub after_5d_pct: Option<f64>,
    pub after_10d_pct: Option<f64>,
    pub after_20d_pct: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct ReviewGroup {
    /// 有 20 个交易日后续数据的条数
    pub samples: usize,
    /// 之后 20 个交易日的平均涨跌（%）
    pub avg_20d_pct: Option<f64>,
    /// 其中上涨的比例
    pub up_ratio: Option<f64>,
}

#[derive(Serialize, ToSchema)]
pub struct Review {
    pub items: Vec<ReviewItem>,
    /// 全部买入建议
    pub buys: ReviewGroup,
    /// 全部卖出、减仓建议（之后跌了说明卖对了）
    pub sells: ReviewGroup,
}

fn group(list: Vec<f64>) -> ReviewGroup {
    let n = list.len();
    ReviewGroup {
        samples: n,
        avg_20d_pct: (n > 0).then(|| list.iter().sum::<f64>() / n as f64),
        up_ratio: (n > 0).then(|| list.iter().filter(|r| **r > 0.0).count() as f64 / n as f64),
    }
}

// @project-doc docs/domains/accounts.md#review
pub async fn review(pool: &PgPool, account_id: i64) -> anyhow::Result<Review> {
    let since = crate::time::today_cn() - chrono::Duration::days(DAYS);
    let rows: Vec<(i64, NaiveDate, serde_json::Value)> = sqlx::query_as(
        "SELECT advice_id, trade_date, result FROM app.advice WHERE account_id = $1 AND superseded_by IS NULL AND trade_date >= $2 ORDER BY trade_date DESC",
    )
    .bind(account_id)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let states: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT s.advice_id, s.item_key, s.state FROM app.advice_item_state s JOIN app.advice a USING (advice_id) WHERE a.account_id = $1 AND a.trade_date >= $2",
    )
    .bind(account_id)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let states: HashMap<(i64, String), String> = states.into_iter().map(|(a, k, s)| ((a, k), s)).collect();
    let mut picked = vec![];
    for (advice_id, trade_date, v) in rows {
        let a: Advice = serde_json::from_value(v)?;
        for it in a.items.into_iter().filter(|i| matches!(i.action, Action::Buy | Action::Sell | Action::Reduce)) {
            let state = states.get(&(advice_id, it.key.clone())).cloned();
            picked.push((trade_date, it, state));
        }
    }
    let codes: Vec<String> = picked.iter().map(|(_, i, _)| i.thscode.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let series: Vec<(String, NaiveDate, f64)> = sqlx::query_as(
        "SELECT thscode, trade_date, close FROM mkt.v_stock_bar_qfq WHERE thscode = ANY($1) AND trade_date >= $2 ORDER BY thscode, trade_date",
    )
    .bind(&codes)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let mut by_code: HashMap<String, Vec<(NaiveDate, f64)>> = HashMap::new();
    for (c, d, close) in series {
        by_code.entry(c).or_default().push((d, close));
    }
    let after = |code: &str, date: NaiveDate, n: usize| -> Option<f64> {
        let s = by_code.get(code)?;
        let i = s.binary_search_by_key(&date, |x| x.0).ok()?;
        let later = s.get(i + n)?;
        Some((later.1 / s[i].1 - 1.0) * 100.0)
    };
    let (mut buys, mut sells) = (vec![], vec![]);
    let items: Vec<ReviewItem> = picked
        .into_iter()
        .map(|(trade_date, it, state)| {
            let [a5, a10, a20] = HORIZONS.map(|h| after(&it.thscode, trade_date, h));
            if let Some(r) = a20 {
                if it.action == Action::Buy { buys.push(r) } else { sells.push(r) }
            }
            ReviewItem { trade_date, action: it.action, thscode: it.thscode, name: it.name, headline: it.headline, state, after_5d_pct: a5, after_10d_pct: a10, after_20d_pct: a20 }
        })
        .collect();
    Ok(Review { items, buys: group(buys), sells: group(sells) })
}
