//! 账户当前的估值：持仓市值、总资产和当日盈亏。口径与每日净值（`equity`）相同：
//!
//! - 持仓市值 = 股数 × 最近一根日线的收盘价（未复权）；停牌、退市时沿用之前最近的收盘价；
//! - 当日盈亏只算最新交易日有日线的持仓：（今天收盘 − 昨天按除权调整的收盘）× 股数，用前复权价之比还原；
//!   停牌的持仓今天价格没变，当日盈亏记 0，不会把几天前那根 K 线的涨跌算成今天的。
//!
//! 全部持仓的日线一次查询读完（`prices::recent_bars_batch`），不按持仓逐只查询。规则见 docs/domains/accounts.md#equity。

use std::collections::HashMap;

use chrono::NaiveDate;
use sqlx::PgPool;

use super::Position;
use crate::analysis::{market, prices};

/// 一只持仓的现价
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quote {
    /// 最近一根日线的日期和收盘价（未复权）
    pub date: NaiveDate,
    pub close: f64,
    /// 最近一根日线相对前一根的涨跌（%，按前复权价）；只有一根日线时为空
    pub change_pct: Option<f64>,
    /// 最近一根日线是不是最新交易日的：不是说明停牌或已退市，当日盈亏记 0
    pub current: bool,
}

/// 一组持仓的估值
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Valuation {
    pub quotes: HashMap<String, Quote>,
    pub market_value: f64,
    pub day_pnl: f64,
}

impl Valuation {
    pub fn market_value_of(&self, p: &Position) -> f64 {
        self.quotes
            .get(&p.thscode)
            .map_or(0.0, |q| q.close * p.shares as f64)
    }
}

/// 按最近两根日线算出持仓的现价、市值和当日盈亏；`latest` 是本地最新交易日
pub fn value(
    positions: &[Position],
    bars: &HashMap<String, Vec<prices::DailyBar>>,
    latest: Option<NaiveDate>,
) -> Valuation {
    let mut out = Valuation::default();
    for p in positions {
        let Some(series) = bars.get(&p.thscode) else {
            continue;
        };
        let Some(last) = series.last() else {
            continue;
        };
        let change_pct = series
            .len()
            .checked_sub(2)
            .map(|i| (last.q_close / series[i].q_close - 1.0) * 100.0)
            .filter(|v| v.is_finite());
        let current = latest == Some(last.date);
        let mv = p.shares as f64 * last.close;
        out.market_value += mv;
        if current && let Some(c) = change_pct {
            out.day_pnl += mv - mv / (1.0 + c / 100.0);
        }
        out.quotes.insert(
            p.thscode.clone(),
            Quote {
                date: last.date,
                close: last.close,
                change_pct,
                current,
            },
        );
    }
    out
}

/// 读出这些持仓最近两根日线并估值
pub async fn load(pool: &PgPool, positions: &[Position]) -> anyhow::Result<Valuation> {
    let codes: Vec<String> = positions.iter().map(|p| p.thscode.clone()).collect();
    let bars = prices::recent_bars_batch(pool, &codes, None, 2).await?;
    let latest = market::resolve_date(pool, None).await?;
    Ok(value(positions, &bars, latest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(code: &str, shares: i64) -> Position {
        Position {
            thscode: code.into(),
            shares,
            cost_price: None,
            opened_on: None,
            since: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        }
    }

    #[test]
    fn suspended_holdings_keep_value_but_add_no_day_pnl() {
        let mut bars = HashMap::new();
        // 两只都从 10 涨到 11：一只最后一根是最新交易日，另一只停牌在更早的一天
        let a = prices::bars_from(&[10.0, 11.0]);
        let latest = a[1].date;
        bars.insert("A".to_string(), a);
        let mut b = prices::bars_from(&[10.0, 11.0]);
        for bar in &mut b {
            bar.date -= chrono::Duration::days(3);
        }
        bars.insert("B".to_string(), b);
        let v = value(&[pos("A", 100), pos("B", 100)], &bars, Some(latest));
        assert!((v.market_value - 2200.0).abs() < 1e-9);
        assert!((v.day_pnl - 100.0).abs() < 1e-9, "只算 A 今天的涨跌");
        assert!(!v.quotes["B"].current);
    }
}
