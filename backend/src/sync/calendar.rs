//! 交易日历与交易日校验。
//!
//! - 本地日历 ref.trading_day：供应商日历（最近一年，source = api）+ 从行情推导的日期（source = derived）。
//! - 交易日校验：快照类接口只返回「当前」的数据，`data.timestamp` 是请求时刻而不是数据日期，
//!   所以执行前先确认运行交易日是交易日、已经收盘，并且就是供应商最新的交易日。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use chrono::{Datelike, NaiveDate, NaiveTime};
use serde_json::Value;
use sqlx::PgPool;

use super::spec::{DatasetSpec, Partition};
use crate::time::{cn_date_end_ms, cn_date_start_ms, now_cn, today_cn};
use crate::vendor::MarketDataClient;

/// 交易日收盘后多久开始采集当天的快照类数据（Asia/Shanghai）
pub const CLOSE_CUTOFF: NaiveTime = NaiveTime::from_hms_opt(15, 30, 0).unwrap();
/// 判断供应商最新交易日所用的参照指数：上证指数
pub const REFERENCE_INDEX: &str = "000001.SH";
/// 供应商最新交易日的缓存时长
pub const LATEST_DAY_TTL: Duration = Duration::from_secs(600);

/// 现取供应商交易日历（最近一年）并写入 ref.trading_day，返回日历中的日期
pub async fn refresh(
    pool: &PgPool,
    client: &dyn MarketDataClient,
) -> anyhow::Result<Vec<NaiveDate>> {
    let resp = client
        .get("/api/a-share/calendar/trading-days", &[])
        .await?;
    let dates: Vec<NaiveDate> = resp
        .body
        .pointer("/data/item")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|i| i.get("date").and_then(Value::as_str))
                .filter_map(|s| NaiveDate::parse_from_str(s, "%Y%m%d").ok())
                .collect()
        })
        .unwrap_or_default();
    sqlx::query("INSERT INTO ref.trading_day (cal_date, source) SELECT unnest($1::date[]), 'api' ON CONFLICT (cal_date) DO UPDATE SET source = 'api'")
        .bind(&dates)
        .execute(pool)
        .await?;
    Ok(dates)
}

/// 本地日历中是否有这一天
pub async fn is_known_trading_day(pool: &PgPool, d: NaiveDate) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ref.trading_day WHERE cal_date = $1)")
            .bind(d)
            .fetch_one(pool)
            .await?,
    )
}

/// 确定不是交易日：周末，或者供应商日历覆盖了这一天却没有它。日历还没覆盖到的日子不下结论
pub async fn is_closed_day(pool: &PgPool, d: NaiveDate) -> anyhow::Result<bool> {
    if d.weekday().number_from_monday() > 5 {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT coalesce(min(cal_date) <= $1 AND max(cal_date) >= $1, false)
                AND NOT EXISTS (SELECT 1 FROM ref.trading_day WHERE cal_date = $1)
         FROM ref.trading_day WHERE source = 'api'",
    )
    .bind(d)
    .fetch_one(pool)
    .await?)
}

// @project-doc docs/architecture/sync_pipeline.md#stale_dates
/// 按交易日分区的任务，执行前（以及兜底补偿清理失败任务时）再确认一次日期。规划时日历可能还没同步，
/// 任务排队期间日期也可能滑出接口的历史范围（例如只能查一年内）。返回跳过的原因
pub async fn stale_partition_date(
    pool: &PgPool,
    spec: &DatasetSpec,
    d: NaiveDate,
) -> anyhow::Result<Option<String>> {
    let Partition::TradingDay {
        from,
        natural_calendar,
        dates_from,
    } = &spec.partition
    else {
        return Ok(None);
    };
    let earliest = from.resolve(today_cn());
    if d < earliest {
        return Ok(Some(format!(
            "{d} 已超出接口的历史范围（只提供 {earliest} 之后的数据）"
        )));
    }
    if !natural_calendar && dates_from.is_none() && is_closed_day(pool, d).await? {
        return Ok(Some(format!("{d} 不是交易日")));
    }
    Ok(None)
}

/// 本地供应商日历覆盖到的最后一天
async fn latest_api_day(pool: &PgPool) -> anyhow::Result<Option<NaiveDate>> {
    Ok(
        sqlx::query_scalar("SELECT max(cal_date) FROM ref.trading_day WHERE source = 'api'")
            .fetch_one(pool)
            .await?,
    )
}

// @project-doc docs/domains/market_data_rules.md#trading_days
/// 区间内的交易日：本地日历中的日期；早于本地最早日期的部分，退回到周一到周五
pub async fn trading_days(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
) -> anyhow::Result<Vec<NaiveDate>> {
    let known: Vec<NaiveDate> = sqlx::query_scalar(
        "SELECT cal_date FROM ref.trading_day WHERE cal_date BETWEEN $1 AND $2 ORDER BY 1",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    let earliest = known
        .first()
        .copied()
        .unwrap_or(to + chrono::Duration::days(1));
    let mut out: Vec<NaiveDate> = from
        .iter_days()
        .take_while(|d| *d < earliest)
        .filter(|d| d.weekday().number_from_monday() <= 5)
        .collect();
    out.extend(known);
    Ok(out)
}

/// 交易日校验的结论
#[derive(Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// 这一天没有数据：不是交易日，或在未来
    Skip(String),
    /// 这一天是交易日，但供应商已经进入下一个交易日，只有当天值的数据再也取不到了。记为失败，不能静默跳过
    Missed(String),
    /// 数据尚未就绪，稍后再试
    Defer(String),
}

/// 交易日校验。每个执行者一个实例；供应商最新交易日在进程内缓存。
pub struct TradeDateGuard {
    pool: PgPool,
    client: Arc<dyn MarketDataClient>,
}

impl TradeDateGuard {
    pub fn new(pool: PgPool, client: Arc<dyn MarketDataClient>) -> Self {
        TradeDateGuard { pool, client }
    }

    // @project-doc docs/domains/market_data_rules.md#trade_date_guard
    pub async fn check(&self, trade_date: Option<NaiveDate>) -> anyhow::Result<Readiness> {
        let Some(d) = trade_date else {
            return Ok(Readiness::Skip("运行没有交易日".into()));
        };
        let now = now_cn();
        if d > now.date() {
            return Ok(Readiness::Skip(format!("运行交易日 {d} 在未来")));
        }
        let mut known = is_known_trading_day(&self.pool, d).await?;
        // 本地日历可能还没有最近的交易日（日历在当天之前取过）：刷新一次再判断
        if !known && latest_api_day(&self.pool).await?.is_none_or(|m| m < d) {
            refresh(&self.pool, self.client.as_ref()).await?;
            known = is_known_trading_day(&self.pool, d).await?;
        }
        if !known {
            // 日历已经覆盖到这一天却没有它，或者是周末：确实不是交易日。
            // 否则是供应商日历还没更新到这一天（例如当天凌晨），不能据此判定，交给下面的最新日线判断
            let covered = latest_api_day(&self.pool).await?.is_some_and(|m| m >= d);
            if covered || d.weekday().number_from_monday() > 5 {
                return Ok(Readiness::Skip(format!("{d} 不是交易日")));
            }
        }
        if d == now.date() && now.time() < CLOSE_CUTOFF {
            return Ok(Readiness::Defer(format!(
                "{d} 尚未收盘，{CLOSE_CUTOFF} 之后才采集当天数据"
            )));
        }
        Ok(match compare_latest(d, self.latest_vendor_day().await?) {
            // 日历里没有这一天、供应商又已经有更晚的日线：这一天是节假日
            Readiness::Missed(_) if !known => {
                Readiness::Skip(format!("{d} 不是交易日（供应商没有这一天的日线）"))
            }
            r => r,
        })
    }

    async fn latest_vendor_day(&self) -> anyhow::Result<NaiveDate> {
        latest_vendor_day(self.client.as_ref()).await
    }
}

/// 供应商最新交易日的进程内缓存，执行者和完整性核对共用
static LATEST: Mutex<Option<(Instant, NaiveDate)>> = Mutex::new(None);

/// 供应商最新的交易日：参照指数最新一根日线的日期，LATEST_DAY_TTL 内复用
pub async fn latest_vendor_day(client: &dyn MarketDataClient) -> anyhow::Result<NaiveDate> {
    if let Some((at, d)) = *LATEST.lock().unwrap()
        && at.elapsed() < LATEST_DAY_TTL
    {
        return Ok(d);
    }
    let today = today_cn();
    let query = [
        ("thscode".to_string(), REFERENCE_INDEX.to_string()),
        ("interval".to_string(), "1d".to_string()),
        (
            "start".to_string(),
            cn_date_start_ms(today - chrono::Duration::days(20)).to_string(),
        ),
        ("end".to_string(), cn_date_end_ms(today).to_string()),
    ];
    let resp = client
        .get("/api/a-share-index/prices/historical", &query)
        .await?;
    let d = resp
        .body
        .pointer("/data/item")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .filter_map(|i| i.get("date_ms").and_then(Value::as_i64))
                .max()
        })
        .and_then(crate::time::ms_to_cn_date)
        .context("参照指数没有返回日线")?;
    *LATEST.lock().unwrap() = Some((Instant::now(), d));
    Ok(d)
}

/// 运行交易日与供应商最新交易日比较
fn compare_latest(d: NaiveDate, latest: NaiveDate) -> Readiness {
    match latest.cmp(&d) {
        std::cmp::Ordering::Equal => Readiness::Ready,
        std::cmp::Ordering::Greater => Readiness::Missed(format!(
            "已错过：供应商最新交易日已是 {latest}，{d} 只有当天值的数据已无法获取"
        )),
        std::cmp::Ordering::Less => {
            Readiness::Defer(format!("供应商最新交易日仍是 {latest}，{d} 的数据尚未就绪"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_taken_only_on_the_vendor_latest_day() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        assert_eq!(compare_latest(d, d), Readiness::Ready);
        assert!(matches!(
            compare_latest(d, d.pred_opt().unwrap()),
            Readiness::Defer(_)
        ));
        assert!(matches!(
            compare_latest(d, d.succ_opt().unwrap()),
            Readiness::Missed(_)
        ));
    }
}
