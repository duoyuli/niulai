//! 权威涨跌停池：供应商提供的涨停、跌停、炸板名单和连板数，只看 `raw.record`（没有类型化表）。
//!
//! 涨停池从 2020-07-22 起每个交易日齐全，也是判断「池这一天有没有数据」的权威交易日历；
//! 跌停池、炸板池只有 2025-10-16 起，更早的日期和北交所（池里没有）回退到 `indicators::Frame::limits` 的自算规则。
//! 为什么以池为准、两段分界和各处分界见 docs/decisions/authoritative_limits.md，口径见 docs/domains/strategies.md#limits。

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;

/// 涨停池：历史最深（2020-07-22 起），同时用作权威交易日历
pub const UP: &str = "special.limit_up_pool";
/// 跌停池：2025-10-16 起
pub const DOWN: &str = "special.limit_down_pool";
/// 炸板池：2025-10-16 起
pub const BREAK: &str = "special.limit_break_pool";
/// 三个池的数据集 ID：同步运行结束后据此判断 `limit_pool` 版本是否要更新
pub const DATASETS: [&str; 3] = [UP, DOWN, BREAK];

/// 一个池的数据覆盖范围（整个数据集的最小、最大观察日期）。日期在范围内才以池为准
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coverage {
    pub from: Option<NaiveDate>,
    pub through: Option<NaiveDate>,
}

impl Coverage {
    /// 这一天是否在池的覆盖范围内；池还没有数据时为 false
    pub fn covers(&self, date: NaiveDate) -> bool {
        matches!((self.from, self.through), (Some(from), Some(through)) if from <= date && date <= through)
    }

    /// 把日期并入覆盖范围（新记录的日期一定在已加载的窗口内）
    fn include(&mut self, date: NaiveDate) {
        self.from = Some(self.from.map_or(date, |d| d.min(date)));
        self.through = Some(self.through.map_or(date, |d| d.max(date)));
    }

    /// 只保留和 [lo, hi] 重叠的部分。加载窗口外的日期没有读入池的行，不能当作「不在池里」
    fn clip(&mut self, lo: NaiveDate, hi: NaiveDate) {
        self.from = self.from.map(|d| d.max(lo));
        self.through = self.through.map(|d| d.min(hi));
    }
}

/// 一只股票的权威涨跌停记录，日期升序
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct CodeLimits {
    /// 涨停日与连板数（供应商的 `continue_day_cnt`，至少 1）
    pub up: Vec<(NaiveDate, u32)>,
    pub down: Vec<NaiveDate>,
    pub broken: Vec<NaiveDate>,
}

/// 三个池在一段时间内的权威数据，按股票聚合；`Coverage` 是整个数据集的范围，不只是查询窗口
#[derive(Default, Clone, Debug)]
pub struct LimitPool {
    pub up: Coverage,
    pub down: Coverage,
    pub broken: Coverage,
    /// 涨停池出现过的日期，即权威交易日历。跌停、炸板只在这样的日期以池为准
    pub up_days: HashSet<NaiveDate>,
    pub by_code: HashMap<String, CodeLimits>,
}

impl LimitPool {
    /// 没有任何权威数据：所有日期、所有股票都回退自算。池为空时（例如本地还没有同步过）也走这里
    pub fn empty() -> Self {
        Self::default()
    }

    /// 登记一天的涨停。连板数缺失或为 0 时按 1 处理
    pub fn push_up(&mut self, code: &str, date: NaiveDate, continue_day_cnt: u32) {
        self.by_code
            .entry(code.to_string())
            .or_default()
            .up
            .push((date, continue_day_cnt.max(1)));
        self.up.include(date);
        self.up_days.insert(date);
    }

    /// 登记一天的跌停
    pub fn push_down(&mut self, code: &str, date: NaiveDate) {
        self.by_code
            .entry(code.to_string())
            .or_default()
            .down
            .push(date);
        self.down.include(date);
    }

    /// 登记一天的炸板
    pub fn push_broken(&mut self, code: &str, date: NaiveDate) {
        self.by_code
            .entry(code.to_string())
            .or_default()
            .broken
            .push(date);
        self.broken.include(date);
    }

    /// 一只股票的权威记录；不在池里时为空
    pub fn code(&self, code: &str) -> Option<&CodeLimits> {
        self.by_code.get(code)
    }

    /// 这一天是不是权威交易日：涨停池有这一天的数据
    pub fn is_trading_day(&self, date: NaiveDate) -> bool {
        self.up_days.contains(&date)
    }

    /// 全市场在 lo 到 hi（含）之间的权威数据。全市场扫描用这个，一次读入整个窗口
    pub async fn load(pool: &PgPool, lo: NaiveDate, hi: NaiveDate) -> sqlx::Result<Self> {
        Self::load_inner(pool, None, lo, hi).await
    }

    /// 指定股票在 lo 到 hi（含）之间的权威数据。单只股票、自选股这类查询用这个，只读需要的行
    pub async fn load_for(
        pool: &PgPool,
        codes: &[String],
        lo: NaiveDate,
        hi: NaiveDate,
    ) -> sqlx::Result<Self> {
        Self::load_inner(pool, Some(codes), lo, hi).await
    }

    async fn load_inner(
        pool: &PgPool,
        codes: Option<&[String]>,
        lo: NaiveDate,
        hi: NaiveDate,
    ) -> sqlx::Result<Self> {
        let mut out = Self::empty();
        // 覆盖边界是整个数据集的最小、最大日期：查询窗口外的日期也要知道能不能以池为准。
        // 分成单条 min、max 查询，能直接走 (dataset_id, obs_date) 索引
        let bounds = |dataset: &'static str| async move {
            let from: Option<NaiveDate> =
                sqlx::query_scalar("SELECT min(obs_date) FROM raw.record WHERE dataset_id = $1")
                    .bind(dataset)
                    .fetch_one(pool)
                    .await?;
            let through: Option<NaiveDate> =
                sqlx::query_scalar("SELECT max(obs_date) FROM raw.record WHERE dataset_id = $1")
                    .bind(dataset)
                    .fetch_one(pool)
                    .await?;
            Ok::<Coverage, sqlx::Error>(Coverage { from, through })
        };
        out.up = bounds(UP).await?;
        out.down = bounds(DOWN).await?;
        out.broken = bounds(BREAK).await?;
        // 查询窗口外的日期没有读入池的行：把覆盖范围裁到窗口内，这些日期回退自算
        for c in [&mut out.up, &mut out.down, &mut out.broken] {
            c.clip(lo, hi);
        }
        // 权威交易日历：涨停池有数据的日期。只需要加载窗口内的日期（窗口外的日期一律回退自算）
        out.up_days = sqlx::query_scalar("SELECT DISTINCT obs_date FROM raw.record WHERE dataset_id = $1 AND obs_date BETWEEN $2 AND $3")
            .bind(UP)
            .bind(lo)
            .bind(hi)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
        // 只取判断需要的列：连板数按文本取回、在 Rust 里解析，不传输整条 payload（几万行时差别明显）
        // 审计：只拼接了本文件中的常量条件；日期和股票代码都是绑定参数
        let filter = if codes.is_some() {
            "AND thscode = ANY($4)"
        } else {
            ""
        };
        let sql = format!(
            "SELECT thscode, dataset_id, obs_date, payload ->> 'continue_day_cnt' FROM raw.record
             WHERE dataset_id = ANY($1) AND obs_date BETWEEN $2 AND $3 AND thscode IS NOT NULL {filter}
             ORDER BY thscode, obs_date"
        );
        let mut q = sqlx::query_as::<_, (String, String, NaiveDate, Option<String>)>(
            sqlx::AssertSqlSafe(sql),
        )
        .bind(&DATASETS[..])
        .bind(lo)
        .bind(hi);
        if let Some(codes) = codes {
            q = q.bind(codes);
        }
        for (code, dataset, date, continue_day_cnt) in q.fetch_all(pool).await? {
            out.push_row(&code, &dataset, date, continue_day_cnt.as_deref());
        }
        Ok(out)
    }

    /// 解析一行 `raw.record`：涨停池取连板数（缺失、不是数字时按 1），跌停、炸板只看日期
    fn push_row(
        &mut self,
        code: &str,
        dataset: &str,
        date: NaiveDate,
        continue_day_cnt: Option<&str>,
    ) {
        match dataset {
            UP => {
                let n = continue_day_cnt
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(1);
                self.push_up(code, date, n);
            }
            DOWN => self.push_down(code, date),
            BREAK => self.push_broken(code, date),
            _ => {}
        }
    }
}

/// 进程内缓存用的数据版本：adj_factor 和 limit_pool 中任一更新，依赖它的结果就不再命中。
/// `limit_pool` 行由迁移 20261018000013 加入；升级前的库里查不到，按 None 处理
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CacheVersions {
    pub adj_factor: Option<DateTime<Utc>>,
    pub limit_pool: Option<DateTime<Utc>>,
}

impl CacheVersions {
    pub async fn load(pool: &PgPool) -> sqlx::Result<Self> {
        let rows: Vec<(String, DateTime<Utc>)> =
            sqlx::query_as("SELECT name, updated_at FROM ops.derived_version WHERE name = ANY($1)")
                .bind(&["adj_factor", "limit_pool"][..])
                .fetch_all(pool)
                .await?;
        let mut v = Self::default();
        for (name, at) in rows {
            match name.as_str() {
                "adj_factor" => v.adj_factor = Some(at),
                "limit_pool" => v.limit_pool = Some(at),
                _ => {}
            }
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn coverage_boundaries() {
        let c = Coverage {
            from: Some(d(1)),
            through: Some(d(20)),
        };
        assert!(c.covers(d(1)) && c.covers(d(20)), "边界包含在内");
        assert!(!c.covers(d(21)) && !c.covers(NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()));
        assert!(
            !Coverage::default().covers(d(1)),
            "池还没有数据时什么日期都不覆盖"
        );
        let one = Coverage {
            from: Some(d(5)),
            through: Some(d(5)),
        };
        assert!(one.covers(d(5)) && !one.covers(d(6)));
    }

    #[test]
    fn row_gives_continue_count_and_dates() {
        let mut p = LimitPool::empty();
        p.push_row("600519.SH", UP, d(10), Some("3"));
        // 连板数缺失、为 0、不是数字时按 1 处理
        p.push_row("000001.SZ", UP, d(10), None);
        p.push_row("000002.SZ", UP, d(11), Some("0"));
        p.push_row("000003.SZ", UP, d(11), Some(""));
        p.push_row("000004.SZ", UP, d(11), Some("板"));
        p.push_row("600519.SH", DOWN, d(10), None);
        p.push_row("600519.SH", BREAK, d(11), None);
        let limits = p.code("600519.SH").expect("有记录");
        assert_eq!(limits.up, vec![(d(10), 3)]);
        assert_eq!(limits.down, vec![d(10)]);
        assert_eq!(limits.broken, vec![d(11)]);
        assert_eq!(p.code("000001.SZ").unwrap().up, vec![(d(10), 1)]);
        assert_eq!(p.code("000002.SZ").unwrap().up, vec![(d(11), 1)]);
        assert_eq!(p.code("000003.SZ").unwrap().up, vec![(d(11), 1)]);
        assert_eq!(p.code("000004.SZ").unwrap().up, vec![(d(11), 1)]);
        assert_eq!(p.code("000005.SZ"), None);
        assert!(p.is_trading_day(d(10)) && p.is_trading_day(d(11)) && !p.is_trading_day(d(12)));
    }

    #[test]
    fn pool_keeps_dates_ascending_when_loaded_in_order() {
        // 加载 SQL 按 (thscode, obs_date) 排序，三种池的日期各自升序，指标合并时可以直接顺序推进
        let mut p = LimitPool::empty();
        for day in [1, 3, 5] {
            p.push_row("600519.SH", UP, d(day), Some(&day.to_string()));
        }
        assert_eq!(
            p.code("600519.SH")
                .unwrap()
                .up
                .iter()
                .map(|(day, _)| *day)
                .collect::<Vec<_>>(),
            vec![d(1), d(3), d(5)]
        );
    }
}
