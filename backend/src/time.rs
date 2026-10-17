//! 交易所时间换算。规则与数据库函数 `ops.ms_to_cn_date` 保持一致。
//!
//! 供应商时间戳是 Asia/Shanghai 当地零点的毫秒数。1986–1991 年中国实行夏令时，那几年的
//! "当地零点"按固定 +8 小时换算会落在前一天 23:00；先加 1 小时再取日期，两种情况都能得到正确的日期。
//! 1992 年以后没有夏令时，生成请求窗口时直接按 +8 小时计算。

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};

const CN_OFFSET_MS: i64 = 8 * 3_600_000;
const DST_GUARD_MS: i64 = 3_600_000;

// @project-doc docs/domains/market_data_rules.md#timestamps
/// 供应商毫秒时间戳 → 交易日（Asia/Shanghai）
pub fn ms_to_cn_date(ms: i64) -> Option<NaiveDate> {
    DateTime::from_timestamp_millis(ms + CN_OFFSET_MS + DST_GUARD_MS).map(|t| t.date_naive())
}

/// 交易日 → 当天 00:00（Asia/Shanghai）的毫秒时间戳
pub fn cn_date_start_ms(d: NaiveDate) -> i64 {
    d.and_time(NaiveTime::MIN).and_utc().timestamp_millis() - CN_OFFSET_MS
}

/// 交易日 → 当天 23:59:59.999（Asia/Shanghai）的毫秒时间戳
pub fn cn_date_end_ms(d: NaiveDate) -> i64 {
    cn_date_start_ms(d + Duration::days(1)) - 1
}

/// 当前的上海时间（不带时区信息）
pub fn now_cn() -> chrono::NaiveDateTime {
    (Utc::now() + Duration::milliseconds(CN_OFFSET_MS)).naive_utc()
}

pub fn today_cn() -> NaiveDate {
    now_cn().date()
}

/// 上海时间（不带时区信息）→ UTC 时刻
pub fn cn_to_utc(t: NaiveDateTime) -> DateTime<Utc> {
    (t - Duration::milliseconds(CN_OFFSET_MS)).and_utc()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn vendor_timestamps_map_to_trading_days() {
        assert_eq!(ms_to_cn_date(1_104_768_000_000), Some(d("2005-01-04")));
        assert_eq!(ms_to_cn_date(1_791_993_600_000), Some(d("2026-10-15")));
        // 1991 年夏令时期间的除权日：固定 +8 小时会得到 05-01 23:00
        assert_eq!(ms_to_cn_date(673_110_000_000), Some(d("1991-05-02")));
    }

    #[test]
    fn window_bounds_are_local_midnight() {
        assert_eq!(cn_date_start_ms(d("2026-10-15")), 1_791_993_600_000);
        assert_eq!(cn_date_end_ms(d("2026-10-15")), 1_792_079_999_999);
        assert_eq!(ms_to_cn_date(cn_date_start_ms(d("2021-04-29"))), Some(d("2021-04-29")));
    }
}
