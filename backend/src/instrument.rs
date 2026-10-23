//! 资产类型：股票和交易所 ETF 的日线表和交易规则（每手股数、印花税）。

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetType {
    #[default]
    Stock,
    Etf,
}

impl AssetType {
    pub fn from_db(value: Option<&str>) -> Self {
        if value == Some("etf") {
            Self::Etf
        } else {
            Self::Stock
        }
    }

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
}
