//! 账户的风格：一组预先定好的规则——参考哪些内置策略、最多持几只、单只最多占多少、留多少现金、
//! 止损线、趋势卖出看哪条均线、弱市怎么办、追高上限。建账户时三选一，高级设置里可以改每一项。
//! 数字是初始值，风格回测（`advice::backtest`）的结果展示在风格卡片上。规则见 docs/domains/accounts.md#styles。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Conservative,
    Balanced,
    Aggressive,
}

/// 弱市（全市场站上 20 日均线的股票比例低于阈值）时怎么买新股票
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum WeakMarket {
    /// 不买新股票
    Pause,
    /// 新买入的金额减半
    Half,
    /// 照常
    Normal,
}

/// 一个风格的全部规则
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct StyleParams {
    /// 参考的内置策略（模板 key）
    pub strategies: Vec<String>,
    /// 最多同时持有几只
    pub max_positions: u32,
    /// 单只最多占总资产的百分比
    pub max_weight_pct: f64,
    /// 至少留多少现金（占总资产的百分比）
    pub min_cash_pct: f64,
    /// 止损线：比成本价亏多少（%）就卖
    pub stop_loss_pct: f64,
    /// 收盘跌破几日均线就卖
    pub trend_ma: u32,
    pub weak_market: WeakMarket,
    /// 全市场站上 20 日均线的股票比例低于多少（%）算弱市
    pub weak_threshold_pct: f64,
    /// 第二天开盘比参考价高多少（%）就不买
    pub chase_limit_pct: f64,
    /// 只采纳过去 3 年 20 日平均涨跌好于「随便买」的策略
    pub require_edge: bool,
    /// 持有的股票被 ST 时卖出
    pub sell_st: bool,
}

/// 高级设置中改过的参数；没改的为空，跟着风格走
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default, PartialEq)]
pub struct StyleOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategies: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_positions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_weight_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_cash_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_loss_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend_ma: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weak_market: Option<WeakMarket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weak_threshold_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chase_limit_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_edge: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sell_st: Option<bool>,
}

impl StyleOverrides {
    pub fn is_empty(&self) -> bool {
        *self == StyleOverrides::default()
    }
}

/// 趋势卖出可选的均线
pub const TREND_MAS: [u32; 5] = [10, 20, 30, 60, 120];

impl Style {
    pub const ALL: [Style; 3] = [Style::Conservative, Style::Balanced, Style::Aggressive];

    pub fn key(self) -> &'static str {
        match self {
            Style::Conservative => "conservative",
            Style::Balanced => "balanced",
            Style::Aggressive => "aggressive",
        }
    }

    pub fn parse(s: &str) -> anyhow::Result<Self> {
        Style::ALL.into_iter().find(|x| x.key() == s).ok_or_else(|| anyhow::anyhow!("未知的风格：{s}"))
    }

    pub fn name(self) -> &'static str {
        match self {
            Style::Conservative => "保守",
            Style::Balanced => "稳健",
            Style::Aggressive => "进取",
        }
    }

    /// 一句话：这个风格在做什么
    pub fn tagline(self) -> &'static str {
        match self {
            Style::Conservative => "只跟随已经走稳的上涨趋势，宁可少赚也要少亏",
            Style::Balanced => "跟随趋势，也接受刚刚启动的突破",
            Style::Aggressive => "追逐强势股和突破，波动大，可能大赚也可能大亏",
        }
    }

    // @project-doc docs/domains/accounts.md#styles
    pub fn params(self) -> StyleParams {
        let keys = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let conservative = ["adx_ma_golden", "bullish_trend", "shrink_pullback"];
        let balanced = [&conservative[..], &["volume_breakout", "macd_golden", "pullback_ma20"]].concat();
        let aggressive = [&balanced[..], &["new_high_60", "strong_pullback", "supertrend_follow"]].concat();
        match self {
            Style::Conservative => StyleParams {
                strategies: keys(&conservative),
                max_positions: 5,
                max_weight_pct: 20.0,
                min_cash_pct: 20.0,
                stop_loss_pct: 6.0,
                trend_ma: 20,
                weak_market: WeakMarket::Pause,
                weak_threshold_pct: 30.0,
                chase_limit_pct: 3.0,
                require_edge: true,
                sell_st: true,
            },
            Style::Balanced => StyleParams {
                strategies: keys(&balanced),
                max_positions: 6,
                max_weight_pct: 25.0,
                min_cash_pct: 10.0,
                stop_loss_pct: 8.0,
                trend_ma: 20,
                weak_market: WeakMarket::Half,
                weak_threshold_pct: 30.0,
                chase_limit_pct: 5.0,
                require_edge: true,
                sell_st: true,
            },
            Style::Aggressive => StyleParams {
                strategies: keys(&aggressive),
                max_positions: 8,
                max_weight_pct: 30.0,
                min_cash_pct: 0.0,
                stop_loss_pct: 10.0,
                trend_ma: 60,
                weak_market: WeakMarket::Normal,
                weak_threshold_pct: 30.0,
                chase_limit_pct: 7.0,
                require_edge: false,
                sell_st: false,
            },
        }
    }

    /// 风格加上高级设置中改过的参数
    pub fn effective(self, o: &StyleOverrides) -> StyleParams {
        let p = self.params();
        StyleParams {
            strategies: o.strategies.clone().unwrap_or(p.strategies),
            max_positions: o.max_positions.unwrap_or(p.max_positions),
            max_weight_pct: o.max_weight_pct.unwrap_or(p.max_weight_pct),
            min_cash_pct: o.min_cash_pct.unwrap_or(p.min_cash_pct),
            stop_loss_pct: o.stop_loss_pct.unwrap_or(p.stop_loss_pct),
            trend_ma: o.trend_ma.unwrap_or(p.trend_ma),
            weak_market: o.weak_market.unwrap_or(p.weak_market),
            weak_threshold_pct: o.weak_threshold_pct.unwrap_or(p.weak_threshold_pct),
            chase_limit_pct: o.chase_limit_pct.unwrap_or(p.chase_limit_pct),
            require_edge: o.require_edge.unwrap_or(p.require_edge),
            sell_st: o.sell_st.unwrap_or(p.sell_st),
        }
    }
}

impl StyleParams {
    /// 参数是否在合理范围内；`known` 是现有的内置策略 key
    pub fn validate(&self, known: &[&str]) -> Result<(), String> {
        if self.strategies.is_empty() {
            return Err("至少参考一个策略".into());
        }
        if let Some(k) = self.strategies.iter().find(|k| !known.contains(&k.as_str())) {
            return Err(format!("没有这个内置策略：{k}"));
        }
        let range = |v: f64, lo: f64, hi: f64, what: &str| if (lo..=hi).contains(&v) { Ok(()) } else { Err(format!("{what}应在 {lo} 到 {hi} 之间")) };
        range(self.max_positions as f64, 1.0, 20.0, "最多持有只数")?;
        range(self.max_weight_pct, 5.0, 100.0, "单只上限（%）")?;
        range(self.min_cash_pct, 0.0, 90.0, "保留现金（%）")?;
        range(self.stop_loss_pct, 2.0, 30.0, "止损线（%）")?;
        range(self.weak_threshold_pct, 5.0, 60.0, "弱市阈值（%）")?;
        range(self.chase_limit_pct, 0.0, 20.0, "追高上限（%）")?;
        if !TREND_MAS.contains(&self.trend_ma) {
            return Err(format!("趋势均线只能是 {TREND_MAS:?} 日之一"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid_and_use_existing_templates() {
        let keys: Vec<&str> = crate::analysis::strategy::templates().iter().map(|t| t.key).collect();
        for s in Style::ALL {
            s.params().validate(&keys).unwrap_or_else(|e| panic!("{}：{e}", s.name()));
            assert_eq!(Style::parse(s.key()).unwrap(), s);
        }
        // 越进取，参考的策略越多、能持的只数越多、止损越宽
        let (c, b, a) = (Style::Conservative.params(), Style::Balanced.params(), Style::Aggressive.params());
        assert!(c.strategies.len() < b.strategies.len() && b.strategies.len() < a.strategies.len());
        assert!(c.stop_loss_pct < b.stop_loss_pct && b.stop_loss_pct < a.stop_loss_pct);
    }

    #[test]
    fn overrides_replace_only_what_was_changed() {
        let o = StyleOverrides { stop_loss_pct: Some(5.0), ..Default::default() };
        let p = Style::Balanced.effective(&o);
        assert_eq!(p.stop_loss_pct, 5.0);
        assert_eq!(p.max_positions, Style::Balanced.params().max_positions);
        assert!(StyleOverrides::default().is_empty() && !o.is_empty());
        let bad = StyleParams { trend_ma: 7, ..p };
        assert!(bad.validate(&["adx_ma_golden"]).is_err());
    }
}
