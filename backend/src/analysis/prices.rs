//! 读取分析用的价格。股票和 ETF 都从 `mkt.v_analysis_bar` 读：股票的前复权价与 `mkt.v_stock_bar_qfq`
//! 同一表达式（K 线接口用后者，集成测试逐根比对两者），ETF 用供应商前复权价。这里不按资产类型分表，
//! 也不在 Rust 里再写一遍复权表达式；因子在 Rust 中计算（理由见 docs/decisions/indicator_compute.md）。

use std::collections::HashMap;

use chrono::NaiveDate;
use sqlx::PgPool;

use super::compute;

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

/// (代码, 日期, 开, 高, 低, 收, 前复权开, 高, 低, 收, 成交量, 成交额)，列顺序与 [`COLUMNS`] 一致
type BarRow = (
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

const COLUMNS: &str =
    "thscode, trade_date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount";

fn split(
    (code, date, open, high, low, close, q_open, q_high, q_low, q_close, volume, amount): BarRow,
) -> (String, DailyBar) {
    (
        code,
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
        },
    )
}

/// 一只证券截至 end（含）的全部日线，按日期升序
pub async fn instrument_bars(
    pool: &PgPool,
    code: &str,
    end: Option<NaiveDate>,
) -> sqlx::Result<Vec<DailyBar>> {
    // 审计：只拼接了本文件中的常量
    let sql = sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM mkt.v_analysis_bar WHERE thscode = $1 AND ($2::date IS NULL OR trade_date <= $2) ORDER BY trade_date"
    ));
    let rows: Vec<BarRow> = sqlx::query_as(sql)
        .bind(code)
        .bind(end)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| split(r).1).collect())
}

/// 一批证券截至 end（含，为空时不限）的最近 n 根日线，按日期升序。股票与 ETF 一次查询读完，
/// 列表页面和建议生成不再为每只证券发起 N+1 查询；没有日线的证券不出现在结果里
pub async fn recent_bars_batch(
    pool: &PgPool,
    codes: &[String],
    end: Option<NaiveDate>,
    n: i64,
) -> sqlx::Result<HashMap<String, Vec<DailyBar>>> {
    if codes.is_empty() || n <= 0 {
        return Ok(HashMap::new());
    }
    // 审计：只拼接了本文件中的常量
    let sql = sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM (
             SELECT b.*, row_number() OVER (PARTITION BY b.thscode ORDER BY b.trade_date DESC) AS rn
             FROM mkt.v_analysis_bar b WHERE b.thscode = ANY($1) AND ($3::date IS NULL OR b.trade_date <= $3)
         ) x WHERE rn <= $2 ORDER BY thscode, trade_date"
    ));
    let rows: Vec<BarRow> = sqlx::query_as(sql)
        .bind(codes)
        .bind(n)
        .bind(end)
        .fetch_all(pool)
        .await?;
    let mut out: HashMap<String, Vec<DailyBar>> = HashMap::new();
    for row in rows {
        let (code, bar) = split(row);
        out.entry(code).or_default().push(bar);
    }
    Ok(out)
}

// @project-doc docs/architecture/sync_pipeline.md#rate_limit
/// 全市场股票在 (lo, hi] 内的日线，按股票代码顺序逐只交给 f，同一时刻只有一批股票的数据在内存中。
/// 市场概况、因子研究和账户的因子排名都用它：按批次读取，避免在 CPU 计算期间长期持有数据库事务。
/// 调用方没有占着全市场计算的槽位时，这里按前台计算占一个（见 [`compute::run`]）；f 里的计算交给 [`compute::run_cpu`]
pub async fn for_each_stock<F>(
    pool: &PgPool,
    lo: NaiveDate,
    hi: NaiveDate,
    mut f: F,
) -> sqlx::Result<()>
where
    F: FnMut(&str, &[DailyBar]),
{
    compute::run(compute::Budget::Foreground, async {
        const BATCH_CODES: usize = 128;
        let codes: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT thscode FROM mkt.stock_bar_1d WHERE trade_date > $1 AND trade_date <= $2 ORDER BY thscode",
        )
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
            // 审计：只拼接了本文件中的常量
            let sql = sqlx::AssertSqlSafe(format!(
                "SELECT {COLUMNS} FROM mkt.v_analysis_bar
                 WHERE asset_type = 'stock' AND thscode = ANY($1) AND trade_date > $2 AND trade_date <= $3
                 ORDER BY thscode, trade_date"
            ));
            let rows: Vec<BarRow> = sqlx::query_as(sql)
                .bind(chunk)
                .bind(lo)
                .bind(hi)
                .fetch_all(&mut *tx)
                .await?;
            tx.commit().await?;
            let mut grouped: HashMap<String, Vec<DailyBar>> = HashMap::new();
            for row in rows {
                let (code, bar) = split(row);
                grouped.entry(code).or_default().push(bar);
            }
            for code in chunk {
                if let Some(bars) = grouped.get(code) {
                    compute::run_cpu(|| f(code, bars));
                }
            }
        }
        Ok(())
    })
    .await
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
