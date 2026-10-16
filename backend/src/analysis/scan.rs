//! 全市场均线交叉扫描，带进程内结果缓存。
//!
//! 同一个 (交易日, 快线, 慢线) 的结果只取决于前复权价，而前复权价只在复权因子重算后变化。
//! 缓存以 `ops.derived_version` 中 adj_factor 的时间戳为版本，重算完成后版本变化，旧结果不再命中。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::ma::{CrossSignal, ma_cross};
use super::prices;

/// 最多缓存多少组参数的结果。一组全市场结果只有几百条命中，占用很小
const CAPACITY: usize = 64;

#[derive(Serialize, ToSchema, Clone)]
pub struct ScanHit {
    pub thscode: String,
    pub name: Option<String>,
    pub signal: CrossSignal,
    pub close: f64,
    pub ma_fast: f64,
    pub ma_slow: f64,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct ScanResult {
    pub trade_date: NaiveDate,
    pub fast: i32,
    pub slow: i32,
    pub scanned: usize,
    pub hits: Vec<ScanHit>,
}

type Key = (NaiveDate, usize, usize);
/// (参数, 复权价版本, 结果)
type Entry = (Key, DateTime<Utc>, Arc<ScanResult>);

#[derive(Default)]
pub struct ScanCache {
    entries: Mutex<VecDeque<Entry>>,
}

impl ScanCache {
    /// date 为空时取本地最新的交易日
    pub async fn scan(&self, pool: &PgPool, date: Option<NaiveDate>, fast: usize, slow: usize) -> anyhow::Result<Arc<ScanResult>> {
        let version: DateTime<Utc> = sqlx::query_scalar("SELECT updated_at FROM ops.derived_version WHERE name = 'adj_factor'").fetch_one(pool).await?;
        let date = match date {
            Some(d) => d,
            None => match sqlx::query_scalar::<_, Option<NaiveDate>>("SELECT max(trade_date) FROM mkt.stock_bar_1d").fetch_one(pool).await? {
                Some(d) => d,
                None => return Ok(Arc::new(ScanResult { trade_date: crate::time::today_cn(), fast: fast as i32, slow: slow as i32, scanned: 0, hits: vec![] })),
            },
        };
        let key = (date, fast, slow);
        if let Some(hit) = self.get(key, version) {
            return Ok(hit);
        }
        let result = Arc::new(compute(pool, date, fast, slow).await?);
        self.put(key, version, result.clone());
        Ok(result)
    }

    fn get(&self, key: Key, version: DateTime<Utc>) -> Option<Arc<ScanResult>> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.iter().find(|(k, v, _)| *k == key && *v == version).map(|(.., r)| r.clone())
    }

    fn put(&self, key: Key, version: DateTime<Utc>, result: Arc<ScanResult>) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        // 版本变了的结果再也不会命中，一并清掉
        entries.retain(|(k, v, _)| *k != key && *v == version);
        if entries.len() >= CAPACITY {
            entries.pop_front();
        }
        entries.push_back((key, version, result));
    }
}

// @project-doc docs/domains/ma_signals.md#market_scan
async fn compute(pool: &PgPool, date: NaiveDate, fast: usize, slow: usize) -> anyhow::Result<ScanResult> {
    // 每只股票取截至该日、往前 (slow + 60) × 2 个自然日内的 K 线，在这个范围内按与单只股票相同的规则判断交叉
    let lookback = ((slow + 60) * 2) as i64;
    let series = prices::closes_between(pool, date - chrono::Duration::days(lookback), date).await?;
    let mut hits = vec![];
    let mut scanned = 0usize;
    for (code, pts) in &series {
        if pts.last().is_none_or(|p| p.0 != date) {
            continue; // 当天停牌
        }
        let closes: Vec<f64> = pts.iter().map(|p| p.1).collect();
        // 范围内不足 slow 根、当天没有完整均线的股票不算参与扫描
        let Some(Some(row)) = ma_cross(&closes, fast, slow).last().copied() else { continue };
        scanned += 1;
        if let Some(signal) = row.signal {
            hits.push(ScanHit { thscode: code.clone(), name: None, signal, close: *closes.last().unwrap(), ma_fast: row.ma_fast, ma_slow: row.ma_slow });
        }
    }
    hits.sort_by(|a, b| a.thscode.cmp(&b.thscode));
    let codes: Vec<&str> = hits.iter().map(|h| h.thscode.as_str()).collect();
    let names: Vec<(String, String)> = sqlx::query_as("SELECT thscode, name FROM ref.security WHERE thscode = ANY($1)").bind(&codes).fetch_all(pool).await?;
    let names: std::collections::HashMap<_, _> = names.into_iter().collect();
    for h in &mut hits {
        h.name = names.get(&h.thscode).cloned();
    }
    Ok(ScanResult { trade_date: date, fast: fast as i32, slow: slow as i32, scanned, hits })
}
