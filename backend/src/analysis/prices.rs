//! 读取分析用的价格。前复权价在 SQL 中按与 `mkt.v_stock_bar_qfq` 相同的表达式计算（原价 × 所在分段的 qfq_factor），
//! 与 K 线接口返回的价格逐位一致；均线等指标在 Rust 中计算（理由见 docs/decisions/indicator_compute.md）。

use std::collections::HashMap;

use chrono::NaiveDate;
use sqlx::PgPool;

/// 一根日线：未复权价用于成交和计算手数，等比前复权价用于收益和指标
#[derive(Clone, Debug)]
pub struct DailyBar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub q_open: f64,
    pub q_close: f64,
}

/// 一只股票截至 end（含）的全部日线，按日期升序
pub async fn stock_bars(pool: &PgPool, code: &str, end: Option<NaiveDate>) -> sqlx::Result<Vec<DailyBar>> {
    let rows: Vec<(NaiveDate, f64, f64, f64, f64, f64, f64)> = sqlx::query_as(
        "SELECT b.trade_date, b.open::float8, b.high::float8, b.low::float8, b.close::float8,
                b.open * coalesce(s.qfq_factor, 1), b.close * coalesce(s.qfq_factor, 1)
         FROM mkt.stock_bar_1d b
         LEFT JOIN mkt.adj_segment s ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to
         WHERE b.thscode = $1 AND ($2::date IS NULL OR b.trade_date <= $2)
         ORDER BY b.trade_date",
    )
    .bind(code)
    .bind(end)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(date, open, high, low, close, q_open, q_close)| DailyBar { date, open, high, low, close, q_open, q_close })
        .collect())
}

/// 前复权收盘价序列，按股票分组、日期升序
pub type CloseSeries = HashMap<String, Vec<(NaiveDate, f64)>>;

// @project-doc docs/domains/ma_signals.md#market_scan
/// 全市场在 (lo, hi] 内的前复权收盘价。日线按日期索引读取，复权分段只有几万行，整表参与关联
pub async fn closes_between(pool: &PgPool, lo: NaiveDate, hi: NaiveDate) -> sqlx::Result<CloseSeries> {
    let mut tx = pool.begin().await?;
    // 这是一次性的大查询：JIT 编译的耗时比它省下的多；排序放在内存里，不落盘
    sqlx::query("SET LOCAL jit = off").execute(&mut *tx).await?;
    sqlx::query("SET LOCAL work_mem = '64MB'").execute(&mut *tx).await?;
    let rows: Vec<(String, NaiveDate, f64)> = sqlx::query_as(
        "SELECT thscode, trade_date, close FROM mkt.v_stock_bar_qfq
         WHERE trade_date > $1 AND trade_date <= $2 ORDER BY thscode, trade_date",
    )
    .bind(lo)
    .bind(hi)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(group(rows))
}

/// 指定股票各自最近 n 根前复权收盘价
pub async fn recent_closes(pool: &PgPool, codes: &[String], n: i64) -> sqlx::Result<CloseSeries> {
    let rows: Vec<(String, NaiveDate, f64)> = sqlx::query_as(
        "SELECT c.code, x.trade_date, x.close FROM unnest($1::text[]) AS c(code)
         CROSS JOIN LATERAL (SELECT trade_date, close FROM mkt.v_stock_bar_qfq v WHERE v.thscode = c.code
                             ORDER BY trade_date DESC LIMIT $2) x
         ORDER BY c.code, x.trade_date",
    )
    .bind(codes)
    .bind(n)
    .fetch_all(pool)
    .await?;
    Ok(group(rows))
}

/// 按代码分组；输入已按 (代码, 日期) 排序
fn group(rows: Vec<(String, NaiveDate, f64)>) -> CloseSeries {
    let mut out: CloseSeries = HashMap::new();
    for (code, date, close) in rows {
        out.entry(code).or_default().push((date, close));
    }
    out
}

/// 一次除权除息：每股现金分红（税前）和每股送转股
#[derive(Clone, Copy, Debug)]
pub struct CorpEvent {
    pub ex_date: NaiveDate,
    pub cash_div: f64,
    pub bonus_ratio: f64,
}

/// 一只股票截至 end（含）的公司行动，按除权日升序。同一天多条时的取舍与复权计算相同（mkt.adj_event_effective）
pub async fn corp_events(pool: &PgPool, code: &str, end: Option<NaiveDate>) -> sqlx::Result<Vec<CorpEvent>> {
    let rows: Vec<(NaiveDate, f64, f64)> = sqlx::query_as(
        "SELECT ex_date, cash_div::float8, bonus_ratio::float8 FROM mkt.adj_event_effective
         WHERE thscode = $1 AND ($2::date IS NULL OR ex_date <= $2) ORDER BY ex_date",
    )
    .bind(code)
    .bind(end)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(ex_date, cash_div, bonus_ratio)| CorpEvent { ex_date, cash_div, bonus_ratio }).collect())
}
