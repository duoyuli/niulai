//! 股票池：排除 ST、低价、成交太少、上市太短和已退市的股票。因子研究和账户的因子排名共用。

use serde::Serialize;
use utoipa::ToSchema;

use super::prices::DailyBar;
use super::securities::{StPeriod, StockMeta, st_on};
use crate::instrument::AssetType;

#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct Universe {
    /// 只看这些板块：main 主板、chinext 创业板、star 科创板、bse 北交所；为空表示全部
    pub boards: Vec<String>,
    /// 排除当天处于 ST 的股票
    pub exclude_st: bool,
    /// 最低股价（元，未复权收盘价）
    pub min_price: Option<f64>,
    /// 最低成交额（元）
    pub min_amount: Option<f64>,
    /// 上市满多少个交易日（按本地日线根数）
    pub min_listed_days: Option<u32>,
}

impl Default for Universe {
    fn default() -> Self {
        Universe {
            boards: vec![],
            exclude_st: true,
            min_price: Some(2.0),
            min_amount: Some(5e7),
            min_listed_days: Some(60),
        }
    }
}

impl Universe {
    /// 第 i 根日线当天这只股票是否在股票池里；bars 是这只股票按日期升序的全部日线
    pub fn contains(&self, bars: &[DailyBar], i: usize, meta: &StockMeta, st: &[StPeriod]) -> bool {
        let b = &bars[i];
        meta.asset_type == AssetType::Stock
            && (self.boards.is_empty() || self.boards.contains(&meta.board))
            && !(self.exclude_st && st_on(meta, st, b.date))
            && self.min_price.is_none_or(|p| b.close >= p)
            && self.min_amount.is_none_or(|a| b.amount >= a)
            && meta.delisted_on.is_none_or(|d| b.date < d)
            && self.min_listed_days.is_none_or(|d| i + 1 >= d as usize)
    }

    /// 白话描述
    pub fn describe(&self) -> Vec<String> {
        let mut out = vec![];
        if !self.boards.is_empty() {
            let names: Vec<&str> = self
                .boards
                .iter()
                .map(|b| match b.as_str() {
                    "main" => "主板",
                    "chinext" => "创业板",
                    "star" => "科创板",
                    _ => "北交所",
                })
                .collect();
            out.push(format!("只看{}", names.join("、")));
        }
        if self.exclude_st {
            out.push("排除 ST 股".into());
        }
        if let Some(p) = self.min_price {
            out.push(format!("股价不低于 {p} 元"));
        }
        if let Some(a) = self.min_amount {
            let text = if a >= 1e8 {
                format!("{} 亿元", a / 1e8)
            } else {
                format!("{} 万元", a / 1e4)
            };
            out.push(format!("成交额不低于 {text}"));
        }
        if let Some(d) = self.min_listed_days {
            out.push(format!("上市满 {d} 个交易日"));
        }
        out
    }
}
