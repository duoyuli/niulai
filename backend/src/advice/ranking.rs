//! 账户的因子排名：用因子研究的同一套计算（`analysis::factor_lab`）给全市场打分，以及调仓日的判断。
//! 规则见 docs/domains/accounts.md#ranking。

use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Datelike, NaiveDate};
use sqlx::PgPool;

use crate::account::rules::Rules;
use crate::analysis::compute;
use crate::analysis::factor_lab::{self, Prep, Spec};
use crate::analysis::universe::Universe;
use crate::analysis::versions::CacheVersions;

/// 交易日是当月最后一个交易日（下一个交易日在下个月）
pub fn month_end(trade_date: NaiveDate, next_date: NaiveDate) -> bool {
    (trade_date.year(), trade_date.month()) != (next_date.year(), next_date.month())
}

/// 代码和合成得分，从高到低
pub type Ranking = Arc<Vec<(String, f64)>>;

/// 同一天、同一组因子、同一份数据的排名只算一次。键里带数据版本（复权价和因子的非价格输入，
/// 见 [`CacheVersions::factor_token`]）：同步、`niulai recompute` 或别的进程改了数据，版本变了旧结果自然不再命中，
/// 不依赖哪个进程收到事件后去清空
fn cache() -> &'static Mutex<Vec<(String, Ranking)>> {
    static CACHE: OnceLock<Mutex<Vec<(String, Ranking)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

// @project-doc docs/domains/accounts.md#ranking
/// 交易日收盘的全市场排名。股票池、去极值、行业和规模中性化与因子研究的默认值相同；当天日线太少时为空。
/// 读入和计算整个占一个全市场计算的槽位（后台建议里调用时复用后台的槽位）
pub async fn ranking(pool: &PgPool, date: NaiveDate, rules: &Rules) -> anyhow::Result<Ranking> {
    let version = CacheVersions::load(pool).await?.factor_token();
    let key = format!(
        "{date}|{version}|{}",
        serde_json::to_string(&rules.factors)?
    );
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
    let ranked = compute::run(compute::Budget::Foreground, async {
        anyhow::Ok(match factor_lab::load(pool, &spec).await? {
            Some(panel) if panel.calendar.last() == Some(&date) => {
                compute::run_cpu(|| factor_lab::ranking(&panel, prep, &weights))
            }
            _ => vec![],
        })
    })
    .await?;
    let ranked = Arc::new(ranked);
    // 没算出来的不缓存：多半是数据还没同步完，下次再算
    if !ranked.is_empty() {
        let mut c = cache()
            .lock()
            .map_err(|_| anyhow::anyhow!("因子排名缓存不可用"))?;
        // 旧版本的结果不会再命中，先清掉，免得占着最多 4 份的名额
        c.retain(|(k, _)| *k != key && k.contains(&version));
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
