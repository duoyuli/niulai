//! 证券的静态信息：板块、资产类型、上市日、退市日、简称和 ST 历史。全市场扫描、因子研究和账户建议共用。

use std::collections::HashMap;

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::instrument::AssetType;

#[derive(Clone, Debug)]
pub struct StockMeta {
    /// main / chinext / star / bse
    pub board: String,
    /// 当前简称含 ST
    pub is_st: bool,
    pub list_date: Option<NaiveDate>,
    /// 本地数据推断的退市日；交易日不早于它时不再进入股票池
    pub delisted_on: Option<NaiveDate>,
    pub asset_type: AssetType,
}

/// 从开始采集那天起记录的简称区间
#[derive(Clone, Debug)]
pub struct StPeriod {
    pub valid_from: NaiveDate,
    pub valid_to: Option<NaiveDate>,
    pub is_st: bool,
}

/// 某一天是否处于 ST：简称历史覆盖的日期按历史，没有覆盖的（大多是开始采集以前）回退当前简称
pub fn st_on(meta: &StockMeta, history: &[StPeriod], date: NaiveDate) -> bool {
    history
        .iter()
        .find(|p| p.valid_from <= date && p.valid_to.is_none_or(|to| date < to))
        .map_or(meta.is_st, |p| p.is_st)
}

/// 全部证券的静态信息和简称
pub struct Securities {
    pub meta: HashMap<String, StockMeta>,
    pub names: HashMap<String, String>,
    pub st_history: HashMap<String, Vec<StPeriod>>,
}

impl Securities {
    pub fn st_history(&self, code: &str) -> &[StPeriod] {
        self.st_history.get(code).map(Vec::as_slice).unwrap_or(&[])
    }
}

type InstrumentRow = (
    String,
    String,
    String,
    Option<String>,
    Option<NaiveDate>,
    Option<NaiveDate>,
);

pub async fn load(pool: &PgPool) -> sqlx::Result<Securities> {
    let rows: Vec<InstrumentRow> = sqlx::query_as(
        "SELECT thscode, name, board, asset_type, list_date, delisted_on FROM ref.instrument",
    )
    .fetch_all(pool)
    .await?;
    let mut meta = HashMap::new();
    let mut names = HashMap::new();
    for (code, name, board, asset_type, list_date, delisted_on) in rows {
        meta.insert(
            code.clone(),
            StockMeta {
                board,
                is_st: name.contains("ST"),
                list_date,
                delisted_on,
                asset_type: AssetType::from_db(asset_type.as_deref()),
            },
        );
        names.insert(code, name);
    }
    let rows: Vec<(String, NaiveDate, Option<NaiveDate>, bool)> = sqlx::query_as(
        "SELECT thscode, valid_from, valid_to, is_st FROM ref.security_name_hist ORDER BY thscode, valid_from",
    )
    .fetch_all(pool)
    .await?;
    let mut st_history: HashMap<String, Vec<StPeriod>> = HashMap::new();
    for (code, valid_from, valid_to, is_st) in rows {
        st_history.entry(code).or_default().push(StPeriod {
            valid_from,
            valid_to,
            is_st,
        });
    }
    Ok(Securities {
        meta,
        names,
        st_history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn st_follows_history_and_falls_back_to_current_name() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let meta = StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
            delisted_on: None,
            asset_type: AssetType::Stock,
        };
        let history = [StPeriod {
            valid_from: d(3, 1),
            valid_to: Some(d(6, 1)),
            is_st: true,
        }];
        assert!(st_on(&meta, &history, d(4, 1)));
        assert!(!st_on(&meta, &history, d(6, 1)));
        assert!(!st_on(&meta, &history, d(1, 1)));
    }
}
