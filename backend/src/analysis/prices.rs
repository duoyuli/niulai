//! 读取分析用的价格。前复权价在 SQL 中按与 `mkt.v_stock_bar_qfq` 相同的表达式计算（原价 × 所在分段的 qfq_factor），
//! 与 K 线接口返回的价格逐位一致；均线等指标在 Rust 中计算（理由见 docs/decisions/indicator_compute.md）。

use std::collections::HashMap;

use chrono::NaiveDate;
use sqlx::{AssertSqlSafe, PgPool};

/// 一根日线：未复权价用于成交、计算手数和涨跌停，等比前复权价（q_*）用于收益和指标
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
const BAR_COLUMNS: &str = "b.trade_date, b.open::float8, b.high::float8, b.low::float8, b.close::float8,
     b.open * coalesce(s.qfq_factor, 1), b.high * coalesce(s.qfq_factor, 1),
     b.low * coalesce(s.qfq_factor, 1), b.close * coalesce(s.qfq_factor, 1),
     b.volume::float8, b.amount::float8";
const BAR_JOIN: &str = "mkt.stock_bar_1d b
     LEFT JOIN mkt.adj_segment s ON s.thscode = b.thscode AND b.trade_date >= s.valid_from AND b.trade_date < s.valid_to";

type BarRow = (NaiveDate, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64);

fn bar((date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount): BarRow) -> DailyBar {
    DailyBar { date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount }
}

/// 一只股票截至 end（含）的全部日线，按日期升序
pub async fn stock_bars(pool: &PgPool, code: &str, end: Option<NaiveDate>) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接了本文件中的常量
    let sql = format!("SELECT {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.thscode = $1 AND ($2::date IS NULL OR b.trade_date <= $2) ORDER BY b.trade_date");
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(code).bind(end).fetch_all(pool).await?;
    Ok(rows.into_iter().map(bar).collect())
}

/// 一只股票最近 n 根日线，按日期升序
pub async fn stock_bars_between(pool: &PgPool, code: &str, start: NaiveDate, end: NaiveDate) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接本文件的列名和 JOIN 常量，证券代码和日期均绑定。
    let sql = format!("SELECT {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.thscode = $1 AND b.trade_date BETWEEN $2 AND $3 ORDER BY b.trade_date");
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(code).bind(start).bind(end).fetch_all(pool).await?;
    Ok(rows.into_iter().map(bar).collect())
}

/// 一只股票最近 n 根日线，按日期升序
pub async fn recent_bars(pool: &PgPool, code: &str, n: i64) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接了本文件中的常量
    let sql = format!("SELECT * FROM (SELECT {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.thscode = $1 ORDER BY b.trade_date DESC LIMIT $2) t ORDER BY 1");
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(code).bind(n).fetch_all(pool).await?;
    Ok(rows.into_iter().map(bar).collect())
}

/// 一只股票截至 end（含）的最近 n 根日线，按日期升序
pub async fn recent_bars_until(pool: &PgPool, code: &str, end: NaiveDate, n: i64) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接了本文件中的常量
    let sql = format!("SELECT * FROM (SELECT {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.thscode = $1 AND b.trade_date <= $3 ORDER BY b.trade_date DESC LIMIT $2) t ORDER BY 1");
    let rows: Vec<BarRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(code).bind(n).bind(end).fetch_all(pool).await?;
    Ok(rows.into_iter().map(bar).collect())
}

/// 同时进行的全市场计算最多几个。每个都要读几百万行日线、算几秒到十几秒，
/// 不限制的话几个页面同时打开就会占满数据库连接和 CPU，拖慢接口和同步任务；多出来的排队等待
pub const MARKET_SCANS: usize = 2;
static SCAN_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MARKET_SCANS);
/// 后台计算（每日巡检、调仓建议、每周的策略统计和参考值回测）另有一个槽位：页面上同时开几个回测，也不会让它们排不上
static BACKGROUND_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

tokio::task_local! {
    static BACKGROUND: ();
}

/// 在后台计算的槽位里执行 f 中的全市场计算（见 [`BACKGROUND_SLOT`]）
pub async fn in_background<F: std::future::Future>(f: F) -> F::Output {
    BACKGROUND.scope((), f).await
}

// @project-doc docs/architecture/sync_pipeline.md#rate_limit
/// 全市场在 (lo, hi] 内的日线，按股票代码顺序逐只交给 f，同一时刻只有一只股票的数据在内存中。
/// 每日巡检、市场概况、信号的过去表现和组合回测都用它：几年的全市场日线有几百万行，一次性读入会占用大量内存。
/// 同时最多 [`MARKET_SCANS`] 个（后台计算另有一个，见 [`in_background`]）；f 里的计算交给 [`run_cpu`]，不占住异步线程
pub async fn for_each_stock<F>(pool: &PgPool, lo: NaiveDate, hi: NaiveDate, mut f: F) -> sqlx::Result<()>
where
    F: FnMut(&str, &[DailyBar]),
{
    use futures_util::TryStreamExt;
    let slots = if BACKGROUND.try_with(|_| ()).is_ok() { &BACKGROUND_SLOT } else { &SCAN_SLOTS };
    let _slot = slots.acquire().await.expect("信号量不会被关闭");
    let mut tx = pool.begin().await?;
    // 一次性的大查询：JIT 编译的耗时比它省下的多；排序放在内存里，不落盘
    sqlx::query("SET LOCAL jit = off").execute(&mut *tx).await?;
    sqlx::query("SET LOCAL work_mem = '128MB'").execute(&mut *tx).await?;
    // 审计：只拼接了本文件中的常量
    let sql = format!("SELECT b.thscode, {BAR_COLUMNS} FROM {BAR_JOIN} WHERE b.trade_date > $1 AND b.trade_date <= $2 ORDER BY b.thscode, b.trade_date");
    let mut rows = sqlx::query_as::<_, (String, NaiveDate, f64, f64, f64, f64, f64, f64, f64, f64, f64, f64)>(AssertSqlSafe(sql))
        .bind(lo)
        .bind(hi)
        .fetch(&mut *tx);
    let mut code = String::new();
    let mut bars: Vec<DailyBar> = Vec::with_capacity(512);
    while let Some((c, d, o, h, l, cl, qo, qh, ql, qc, v, a)) = rows.try_next().await? {
        if c != code {
            if !bars.is_empty() {
                run_cpu(|| f(&code, &bars));
                bars.clear();
            }
            code = c;
        }
        bars.push(bar((d, o, h, l, cl, qo, qh, ql, qc, v, a)));
    }
    if !bars.is_empty() {
        run_cpu(|| f(&code, &bars));
    }
    drop(rows);
    tx.commit().await?;
    Ok(())
}

/// 执行一段耗 CPU 的计算：多线程运行时（服务）里用 block_in_place，让这个线程上的其他异步任务（接口、同步任务的续租）
/// 挪到别的线程继续；单线程运行时（测试）里直接执行
pub fn run_cpu<R>(f: impl FnOnce() -> R) -> R {
    use tokio::runtime::{Handle, RuntimeFlavor};
    if Handle::try_current().is_ok_and(|h| h.runtime_flavor() == RuntimeFlavor::MultiThread) { tokio::task::block_in_place(f) } else { f() }
}

/// 前复权收盘价序列，按股票分组、日期升序
pub type CloseSeries = HashMap<String, Vec<(NaiveDate, f64)>>;

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

/// 一段时间内全部股票的公司行动，按代码和除权日分组。
/// 全市场回测使用，避免在逐股票扫描时为每只股票重复发 SQL。
pub async fn corp_events_for_window(
    pool: &PgPool,
    lo: NaiveDate,
    hi: NaiveDate,
) -> sqlx::Result<HashMap<String, Vec<CorpEvent>>> {
    let rows: Vec<(String, NaiveDate, f64, f64)> = sqlx::query_as(
        "SELECT thscode, ex_date, cash_div::float8, bonus_ratio::float8
         FROM mkt.adj_event_effective
         WHERE ex_date > $1 AND ex_date <= $2
         ORDER BY thscode, ex_date",
    )
    .bind(lo)
    .bind(hi)
    .fetch_all(pool)
    .await?;
    let mut out: HashMap<String, Vec<CorpEvent>> = HashMap::new();
    for (code, ex_date, cash_div, bonus_ratio) in rows {
        out.entry(code).or_default().push(CorpEvent { ex_date, cash_div, bonus_ratio });
    }
    Ok(out)
}
