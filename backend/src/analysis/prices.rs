//! 读取分析用的价格。前复权价在 SQL 中按与 `mkt.v_stock_bar_qfq` 相同的表达式计算（原价 × 所在分段的 qfq_factor），
//! 与 K 线接口返回的价格逐位一致；因子在 Rust 中计算（理由见 docs/decisions/indicator_compute.md）。

use std::collections::HashMap;

use chrono::NaiveDate;
use sqlx::{AssertSqlSafe, PgPool};

use crate::instrument::AssetType;

/// 一根日线：未复权价用于成交、计算手数和涨跌停，等比前复权价（q_*）用于收益和因子
#[derive(Clone, Debug)]
pub struct DailyBar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub q_open: f64,
    pub q_high: f64,
    pub q_low: f64,
    pub q_close: f64,
    /// 成交量（股）
    pub volume: f64,
    /// 成交额（元）
    pub amount: f64,
}

/// 查询一行日线的列，顺序与 `BarRow` 一致；前复权价的表达式与 `mkt.v_stock_bar_qfq` 相同
const BAR_COLUMNS: &str =
    "b.trade_date, b.open::float8, b.high::float8, b.low::float8, b.close::float8,
     b.open * coalesce(s.qfq_factor, 1), b.high * coalesce(s.qfq_factor, 1),
     b.low * coalesce(s.qfq_factor, 1), b.close * coalesce(s.qfq_factor, 1),
     b.volume::float8, b.amount::float8";
const BAR_JOIN: &str = "mkt.stock_bar_1d b
     LEFT JOIN mkt.adj_segment s ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to";
const ETF_BAR_COLUMNS: &str =
    "b.trade_date, b.open::float8, b.high::float8, b.low::float8, b.close::float8,
     b.open::float8, b.high::float8, b.low::float8, b.close::float8,
     b.volume::float8, b.amount::float8";
const ETF_BAR_JOIN: &str = "mkt.etf_bar_1d b";
const STOCK_BATCH_COLUMNS: &str = "b.trade_date AS trade_date, b.open::float8 AS open, b.high::float8 AS high, b.low::float8 AS low, b.close::float8 AS close,
     (b.open * coalesce(s.qfq_factor, 1))::float8 AS q_open, (b.high * coalesce(s.qfq_factor, 1))::float8 AS q_high,
     (b.low * coalesce(s.qfq_factor, 1))::float8 AS q_low, (b.close * coalesce(s.qfq_factor, 1))::float8 AS q_close,
     b.volume::float8 AS volume, b.amount::float8 AS amount";
const ETF_BATCH_COLUMNS: &str = "b.trade_date AS trade_date, b.open::float8 AS open, b.high::float8 AS high, b.low::float8 AS low, b.close::float8 AS close,
     b.open::float8 AS q_open, b.high::float8 AS q_high, b.low::float8 AS q_low, b.close::float8 AS q_close,
     b.volume::float8 AS volume, b.amount::float8 AS amount";

type BarRow = (NaiveDate, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64);
type StockBarRow = (
    String,
    NaiveDate,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
);

fn bar(
    (date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount): BarRow,
) -> DailyBar {
    DailyBar {
        date,
        open,
        high,
        low,
        close,
        q_open,
        q_high,
        q_low,
        q_close,
        volume,
        amount,
    }
}

/// 一只股票截至 end（含）的全部日线，按日期升序
pub async fn stock_bars(
    pool: &PgPool,
    code: &str,
    end: Option<NaiveDate>,
) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接了本文件中的常量
    let sql = format!(
        "SELECT {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.thscode = $1 AND ($2::date IS NULL OR b.trade_date <= $2) ORDER BY b.trade_date"
    );
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(code)
        .bind(end)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(bar).collect())
}

/// 一只证券截至 end（含）的全部日线；ETF 使用供应商前复权价格作为分析价格。
pub async fn instrument_bars(
    pool: &PgPool,
    code: &str,
    end: Option<NaiveDate>,
) -> sqlx::Result<Vec<DailyBar>> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(code)
            .fetch_optional(pool)
            .await?;
    if AssetType::from_db(kind.as_deref()).bar_table() == "mkt.etf_bar_1d" {
        let sql = format!(
            "SELECT {ETF_BAR_COLUMNS} FROM {ETF_BAR_JOIN} WHERE b.thscode = $1 AND ($2::date IS NULL OR b.trade_date <= $2) ORDER BY b.trade_date"
        );
        let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(code)
            .bind(end)
            .fetch_all(pool)
            .await?;
        Ok(rows.into_iter().map(bar).collect())
    } else {
        stock_bars(pool, code, end).await
    }
}

/// 一只股票最近 n 根日线，按日期升序
pub async fn recent_bars(pool: &PgPool, code: &str, n: i64) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接了本文件中的常量
    let sql = format!(
        "SELECT * FROM (SELECT {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.thscode = $1 ORDER BY b.trade_date DESC LIMIT $2) t ORDER BY 1"
    );
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(code)
        .bind(n)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(bar).collect())
}

/// 一只证券最近 n 根日线，按日期升序。
pub async fn recent_instrument_bars(
    pool: &PgPool,
    code: &str,
    n: i64,
) -> sqlx::Result<Vec<DailyBar>> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(code)
            .fetch_optional(pool)
            .await?;
    if AssetType::from_db(kind.as_deref()).bar_table() == "mkt.etf_bar_1d" {
        let sql = format!(
            "SELECT * FROM (SELECT {ETF_BAR_COLUMNS} FROM {ETF_BAR_JOIN} WHERE b.thscode = $1 ORDER BY b.trade_date DESC LIMIT $2) t ORDER BY 1"
        );
        let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(code)
            .bind(n)
            .fetch_all(pool)
            .await?;
        Ok(rows.into_iter().map(bar).collect())
    } else {
        recent_bars(pool, code, n).await
    }
}

/// 一批证券最近 n 根日线，按证券和日期升序返回。股票与 ETF 共用一个入口，避免列表页面为每只证券发起 N+1 查询。
pub async fn recent_instrument_bars_batch(
    pool: &PgPool,
    codes: &[String],
    n: i64,
) -> sqlx::Result<HashMap<String, Vec<DailyBar>>> {
    if codes.is_empty() || n <= 0 {
        return Ok(HashMap::new());
    }
    let stock_sql = format!(
        "SELECT thscode, trade_date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount FROM (
             SELECT b.thscode, {STOCK_BATCH_COLUMNS}, row_number() OVER (PARTITION BY b.thscode ORDER BY b.trade_date DESC) AS rn
             FROM {BAR_JOIN} WHERE b.thscode = ANY($1)
         ) x WHERE rn <= $2 ORDER BY thscode, trade_date"
    );
    let stock_rows: Vec<StockBarRow> = sqlx::query_as(AssertSqlSafe(stock_sql))
        .bind(codes)
        .bind(n)
        .fetch_all(pool)
        .await?;
    let etf_sql = format!(
        "SELECT thscode, trade_date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount FROM (
             SELECT b.thscode, {ETF_BATCH_COLUMNS}, row_number() OVER (PARTITION BY b.thscode ORDER BY b.trade_date DESC) AS rn
             FROM {ETF_BAR_JOIN} WHERE b.thscode = ANY($1)
         ) x WHERE rn <= $2 ORDER BY thscode, trade_date"
    );
    let etf_rows: Vec<StockBarRow> = sqlx::query_as(AssertSqlSafe(etf_sql))
        .bind(codes)
        .bind(n)
        .fetch_all(pool)
        .await?;
    let mut out: HashMap<String, Vec<DailyBar>> = HashMap::new();
    for (code, date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount) in
        stock_rows.into_iter().chain(etf_rows)
    {
        out.entry(code).or_default().push(DailyBar {
            date,
            open,
            high,
            low,
            close,
            q_open,
            q_high,
            q_low,
            q_close,
            volume,
            amount,
        });
    }
    Ok(out)
}

/// 一只股票截至 end（含）的最近 n 根日线，按日期升序
pub async fn recent_bars_until(
    pool: &PgPool,
    code: &str,
    end: NaiveDate,
    n: i64,
) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接了本文件中的常量
    let sql = format!(
        "SELECT * FROM (SELECT {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.thscode = $1 AND b.trade_date <= $3 ORDER BY b.trade_date DESC LIMIT $2) t ORDER BY 1"
    );
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(code)
        .bind(n)
        .bind(end)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(bar).collect())
}

pub async fn recent_instrument_bars_until(
    pool: &PgPool,
    code: &str,
    end: NaiveDate,
    n: i64,
) -> sqlx::Result<Vec<DailyBar>> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT asset_type FROM ref.instrument WHERE thscode = $1")
            .bind(code)
            .fetch_optional(pool)
            .await?;
    if AssetType::from_db(kind.as_deref()).bar_table() == "mkt.etf_bar_1d" {
        let sql = format!(
            "SELECT * FROM (SELECT {ETF_BAR_COLUMNS} FROM {ETF_BAR_JOIN} WHERE b.thscode = $1 AND b.trade_date <= $3 ORDER BY b.trade_date DESC LIMIT $2) t ORDER BY 1"
        );
        let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(code)
            .bind(n)
            .bind(end)
            .fetch_all(pool)
            .await?;
        Ok(rows.into_iter().map(bar).collect())
    } else {
        recent_bars_until(pool, code, end, n).await
    }
}

/// 同时进行的全市场计算最多几个。每个都要读几百万行日线、算几秒到十几秒，
/// 不限制的话几个页面同时打开就会占满数据库连接和 CPU，拖慢接口和同步任务；多出来的排队等待
pub const MARKET_SCANS: usize = 2;
static SCAN_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MARKET_SCANS);
/// 后台计算（调仓建议）另有一个槽位：页面上同时开几个回测，也不会让它们排不上
static BACKGROUND_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

tokio::task_local! {
    static BACKGROUND: ();
}

/// 在后台计算的槽位里执行 f 中的全市场计算（见 [`BACKGROUND_SLOT`]）
pub async fn in_background<F: std::future::Future>(f: F) -> F::Output {
    BACKGROUND.scope((), f).await
}

// @project-doc docs/architecture/sync_pipeline.md#rate_limit
/// 全市场在 (lo, hi] 内的日线，按股票代码顺序逐只交给 f，同一时刻只有一批股票的数据在内存中。
/// 市场概况、因子研究和账户的因子排名都用它：按批次读取，避免在 CPU 计算期间长期持有数据库事务。
/// 同时最多 [`MARKET_SCANS`] 个（后台计算另有一个，见 [`in_background`]）；f 里的计算交给 [`run_cpu`]，不占住异步线程
pub async fn for_each_stock<F>(
    pool: &PgPool,
    lo: NaiveDate,
    hi: NaiveDate,
    mut f: F,
) -> sqlx::Result<()>
where
    F: FnMut(&str, &[DailyBar]),
{
    for_each_table(pool, lo, hi, BAR_JOIN, BAR_COLUMNS, &mut f).await
}

async fn for_each_table<F>(
    pool: &PgPool,
    lo: NaiveDate,
    hi: NaiveDate,
    join: &str,
    columns: &str,
    mut f: F,
) -> sqlx::Result<()>
where
    F: FnMut(&str, &[DailyBar]),
{
    let slots = if BACKGROUND.try_with(|_| ()).is_ok() {
        &BACKGROUND_SLOT
    } else {
        &SCAN_SLOTS
    };
    let _slot = slots.acquire().await.expect("信号量不会被关闭");
    const BATCH_CODES: usize = 128;
    let code_sql = format!(
        "SELECT DISTINCT b.thscode FROM {join} WHERE b.trade_date > $1 AND b.trade_date <= $2 ORDER BY b.thscode"
    );
    let codes: Vec<String> = sqlx::query_scalar(AssertSqlSafe(code_sql))
        .bind(lo)
        .bind(hi)
        .fetch_all(pool)
        .await?;
    for chunk in codes.chunks(BATCH_CODES) {
        let mut tx = pool.begin().await?;
        sqlx::query("SET LOCAL jit = off").execute(&mut *tx).await?;
        sqlx::query("SET LOCAL work_mem = '128MB'")
            .execute(&mut *tx)
            .await?;
        let sql = format!(
            "SELECT b.thscode, {columns} FROM {join} WHERE b.thscode = ANY($1) AND b.trade_date > $2 AND b.trade_date <= $3 ORDER BY b.thscode, b.trade_date"
        );
        let rows: Vec<StockBarRow> = sqlx::query_as(AssertSqlSafe(sql))
            .bind(chunk)
            .bind(lo)
            .bind(hi)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let mut grouped: HashMap<String, Vec<DailyBar>> = HashMap::new();
        for (code, d, o, h, l, cl, qo, qh, ql, qc, v, a) in rows {
            grouped
                .entry(code)
                .or_default()
                .push(bar((d, o, h, l, cl, qo, qh, ql, qc, v, a)));
        }
        for code in chunk {
            if let Some(bars) = grouped.get(code) {
                run_cpu(|| f(code, bars));
            }
        }
    }
    Ok(())
}

/// 执行一段耗 CPU 的计算：多线程运行时（服务）里用 block_in_place，让这个线程上的其他异步任务（接口、同步任务的续租）
/// 挪到别的线程继续；单线程运行时（测试）里直接执行
pub fn run_cpu<R>(f: impl FnOnce() -> R) -> R {
    use tokio::runtime::{Handle, RuntimeFlavor};
    if Handle::try_current().is_ok_and(|h| h.runtime_flavor() == RuntimeFlavor::MultiThread) {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

/// 单元测试用的合成日线：每天一根，开高低收按收盘价上下浮动 1%
#[cfg(test)]
pub fn bars_from(closes: &[f64]) -> Vec<DailyBar> {
    let d0 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| DailyBar {
            date: d0 + chrono::Duration::days(i as i64),
            open: c,
            high: c * 1.01,
            low: c * 0.99,
            close: c,
            q_open: c,
            q_high: c * 1.01,
            q_low: c * 0.99,
            q_close: c,
            volume: 1000.0 * (i as f64 + 1.0),
            amount: c * 1000.0 * (i as f64 + 1.0),
        })
        .collect()
}
