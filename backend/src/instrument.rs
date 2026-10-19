//! 资产扩展：统一分析、策略和账户主流程，按资产类型加载独立数据与交易规则。

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
}

/// 资产扩展提供数据表和交易规则；上层编排只依赖这个能力边界。
pub trait AssetExtension: Sync {
    fn asset_type(&self) -> AssetType;
    fn bar_table(&self) -> &'static str;
    fn uses_stock_limit_pool(&self) -> bool;
    fn stamp_tax_rate(&self) -> f64;
    fn lot_rule(&self, board: &str) -> (i64, i64);
}

struct StockExtension;
struct EtfExtension;

impl AssetExtension for StockExtension {
    fn asset_type(&self) -> AssetType {
        AssetType::Stock
    }
    fn bar_table(&self) -> &'static str {
        "mkt.stock_bar_1d"
    }
    fn uses_stock_limit_pool(&self) -> bool {
        true
    }
    fn stamp_tax_rate(&self) -> f64 {
        0.0005
    }
    fn lot_rule(&self, board: &str) -> (i64, i64) {
        match board {
            "star" => (200, 1),
            "bse" => (100, 1),
            _ => (100, 100),
        }
    }
}

impl AssetExtension for EtfExtension {
    fn asset_type(&self) -> AssetType {
        AssetType::Etf
    }
    fn bar_table(&self) -> &'static str {
        "mkt.etf_bar_1d"
    }
    fn uses_stock_limit_pool(&self) -> bool {
        false
    }
    fn stamp_tax_rate(&self) -> f64 {
        0.0
    }
    fn lot_rule(&self, _board: &str) -> (i64, i64) {
        (100, 100)
    }
}

static STOCK: StockExtension = StockExtension;
static ETF: EtfExtension = EtfExtension;

pub fn extension(kind: AssetType) -> &'static dyn AssetExtension {
    match kind {
        AssetType::Stock => &STOCK,
        AssetType::Etf => &ETF,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_keep_data_and_trade_rules_separate() {
        let stock = extension(AssetType::Stock);
        let etf = extension(AssetType::Etf);
        assert_eq!(stock.bar_table(), "mkt.stock_bar_1d");
        assert_eq!(etf.bar_table(), "mkt.etf_bar_1d");
        assert!(stock.uses_stock_limit_pool());
        assert!(!etf.uses_stock_limit_pool());
        assert_eq!(stock.lot_rule("star"), (200, 1));
        assert_eq!(etf.lot_rule(""), (100, 100));
        assert_eq!(etf.stamp_tax_rate(), 0.0);
    }
}
