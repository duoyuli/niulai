//! 账户的因子排名：用因子研究的同一套计算（`analysis::factor_lab`）给全市场打分，以及调仓日的判断。
//! 规则见 docs/domains/accounts.md#ranking。

use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Datelike, NaiveDate};
use sqlx::PgPool;

use crate::account::rules::Rules;
use crate::analysis::factor_lab::{self, Prep, Spec};
use crate::analysis::universe::Universe;

/// 交易日是当月最后一个交易日（下一个交易日在下个月）
pub fn month_end(trade_date: NaiveDate, next_date: NaiveDate) -> bool {
    (trade_date.year(), trade_date.month()) != (next_date.year(), next_date.month())
}

/// 代码和合成得分，从高到低
pub type Ranking = Arc<Vec<(String, f64)>>;

/// 同一天、同一组因子的排名只算一次；同步完成后 [`clear`] 清掉，用新数据重算
fn cache() -> &'static Mutex<Vec<(String, Ranking)>> {
    static CACHE: OnceLock<Mutex<Vec<(String, Ranking)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// 数据变了（同步完成）之后清空排名缓存
pub fn clear() {
    if let Ok(mut c) = cache().lock() {
        c.clear();
    }
}

// @project-doc docs/domains/accounts.md#ranking
/// 交易日收盘的全市场排名。股票池、去极值、行业和规模中性化与因子研究的默认值相同；当天日线太少时为空
pub async fn ranking(pool: &PgPool, date: NaiveDate, rules: &Rules) -> anyhow::Result<Ranking> {
    let key = format!("{date}|{}", serde_json::to_string(&rules.factors)?);
    if let Some((_, r)) = cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("因子排名缓存不可用"))?
        .iter()
        .find(|(k, _)| *k == key)
    {
        return Ok(r.clone());
    }
    let spec = Spec {
        start: date,
        end: date,
        factors: rules.factors.iter().map(|f| f.factor).collect(),
        universe: Universe::default(),
        step: 1,
    };
    let weights: Vec<f64> = rules.factors.iter().map(|f| f.weight).collect();
    let prep = Prep {
        neutralize: true,
        exclude_small_pct: 0.0,
    };
    let ranked = match factor_lab::load(pool, &spec).await? {
        Some(panel) if panel.calendar.last() == Some(&date) => {
            crate::analysis::prices::run_cpu(|| factor_lab::ranking(&panel, prep, &weights))
        }
        _ => vec![],
    };
    let ranked = Arc::new(ranked);
    // 没算出来的不缓存：多半是数据还没同步完，下次再算
    if !ranked.is_empty() {
        let mut c = cache()
            .lock()
            .map_err(|_| anyhow::anyhow!("因子排名缓存不可用"))?;
        c.retain(|(k, _)| *k != key);
        c.push((key, ranked.clone()));
        if c.len() > 4 {
            c.remove(0);
        }
    }
    Ok(ranked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_end_detection() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        assert!(month_end(d(10, 30), d(11, 2)));
        assert!(!month_end(d(10, 29), d(10, 30)));
    }
}
