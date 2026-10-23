//! 资产类型：股票和交易所 ETF 的日线表和交易规则（每手股数、印花税）。
//!
//! 数据库（`ref.instrument.asset_type`）、接口和建议的输入都直接用这个枚举，不认识的值在解码时报错，
//! 不会被静默当成股票。分析用的价格不按资产类型分表读取，统一读 `mkt.v_analysis_bar`（见 `analysis::prices`）。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// ETF 资料（规模、净值、管理人、费率）的数据集：只写 `raw.record`，ETF 的个股资料页从这里读
pub const ETF_PROFILE_DATASET: &str = "fund.profile";

/// stock / etf
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum AssetType {
    Stock,
    Etf,
}

impl std::str::FromStr for AssetType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "stock" => Ok(Self::Stock),
            "etf" => Ok(Self::Etf),
            other => anyhow::bail!("不认识的资产类型：{other}"),
        }
    }
}

impl AssetType {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Stock => "stock",
            Self::Etf => "etf",
        }
    }

    pub const fn bar_table(self) -> &'static str {
        match self {
            Self::Stock => "mkt.stock_bar_1d",
            Self::Etf => "mkt.etf_bar_1d",
        }
    }

    /// 卖出印花税率；ETF 不收
    pub const fn stamp_tax_rate(self) -> f64 {
        match self {
            Self::Stock => 0.0005,
            Self::Etf => 0.0,
        }
    }

    /// (买入最少, 递增)：科创板 200 股起、1 股递增，北交所 100 股起、1 股递增，其余 100 股
    pub fn lot_rule(self, board: &str) -> (i64, i64) {
        match (self, board) {
            (Self::Stock, "star") => (200, 1),
            (Self::Stock, "bse") => (100, 1),
            _ => (100, 100),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_and_trade_rules_follow_asset_type() {
        assert_eq!(AssetType::Stock.bar_table(), "mkt.stock_bar_1d");
        assert_eq!(AssetType::Etf.bar_table(), "mkt.etf_bar_1d");
        assert_eq!(AssetType::Stock.lot_rule("star"), (200, 1));
        assert_eq!(AssetType::Etf.lot_rule("star"), (100, 100));
        assert_eq!(AssetType::Etf.stamp_tax_rate(), 0.0);
    }

    #[test]
    fn unknown_asset_type_is_an_error() {
        assert_eq!("etf".parse::<AssetType>().unwrap(), AssetType::Etf);
        assert!("bond".parse::<AssetType>().is_err());
        assert_eq!(
            serde_json::to_string(&AssetType::Stock).unwrap(),
            "\"stock\""
        );
    }
}
