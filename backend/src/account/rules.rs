//! 账户的选股和风控规则：参考哪些策略（默认全部参加巡检的策略）、至少几个有效策略同时选中才买、最多持几只、
//! 单只最多占多少、留多少现金、止损线、趋势卖出看哪条均线、弱市怎么办、追高上限。
//! 规则不绑定具体策略：候选股按同一套优先级排序（见 advice::generate）。
//!
//! 保守、稳健、进取只是三组参考值（`Preset`），在高级设置里作为辅助：说明这类账户一般怎么设计，点一下套用后还可以再改。
//! 新账户用稳健的参考值。规则见 docs/domains/accounts.md#rules。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// 弱市（全市场站上 20 日均线的股票比例低于阈值）时怎么买新股票
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum WeakMarket {
    /// 不买新股票
    Pause,
    /// 用于买新股票的钱减半
    Half,
    /// 照常
    Normal,
}

/// 一个账户的全部规则。缺的字段按稳健的参考值补齐
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Rules {
    /// 只参考这些策略（策略编号）；为空（null）表示全部参加巡检的策略，以后新增的也自动参考
    pub strategies: Option<Vec<i64>>,
    /// 有效策略的门槛：过去 3 年买点之后 20 个交易日，平均比随便买至少多涨几个百分点
    pub min_edge_pct: f64,
    /// 至少几个有效策略同时选中才买
    pub min_picks: u32,
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
    /// 持有的股票被 ST 时卖出
    pub sell_st: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Preset::Balanced.rules()
    }
}

/// 趋势卖出可选的均线
pub const TREND_MAS: [u32; 5] = [10, 20, 30, 60, 120];

impl Rules {
    /// 参数是否在合理范围内
    pub fn validate(&self) -> Result<(), String> {
        let range = |v: f64, lo: f64, hi: f64, what: &str| if (lo..=hi).contains(&v) { Ok(()) } else { Err(format!("{what}应在 {lo} 到 {hi} 之间")) };
        range(self.min_edge_pct, 0.0, 10.0, "有效策略的门槛（%）")?;
        range(self.min_picks as f64, 1.0, 5.0, "至少几个有效策略同时选中")?;
        range(self.max_positions as f64, 1.0, 20.0, "最多持有只数")?;
        range(self.max_weight_pct, 5.0, 100.0, "单只上限（%）")?;
        range(self.min_cash_pct, 0.0, 90.0, "保留现金（%）")?;
        range(self.stop_loss_pct, 2.0, 30.0, "止损线（%）")?;
        range(self.weak_threshold_pct, 5.0, 60.0, "弱市阈值（%）")?;
        range(self.chase_limit_pct, 0.0, 20.0, "追高上限（%）")?;
        if !TREND_MAS.contains(&self.trend_ma) {
            return Err(format!("趋势均线只能是 {TREND_MAS:?} 日之一"));
        }
        match &self.strategies {
            Some(l) if l.is_empty() => return Err("至少勾选一个策略".into()),
            Some(l) if l.len() > 64 => return Err("最多勾选 64 个策略".into()),
            _ => {}
        }
        Ok(())
    }

    /// 这个策略是否参考
    pub fn uses(&self, strategy_id: i64) -> bool {
        self.strategies.as_ref().is_none_or(|l| l.contains(&strategy_id))
    }

    /// 策略是否够得上有效：超额（过去 3 年买点之后 20 日平均比同一天随便买多涨的百分点）超过门槛，
    /// 并且超过标准误的 2 倍（不是运气）。edge 是（超额, 标准误）；还没有统计、买点太少（None）的在观察期，
    /// 不算有效：新建的策略不会当晚就被拿来买入
    pub fn effective(&self, edge: Option<(f64, f64)>) -> bool {
        edge.is_some_and(|(e, se)| e > self.min_edge_pct && e > 2.0 * se)
    }
}

/// 三组参考值：只是辅助，套用后还可以改
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Conservative,
    Balanced,
    Aggressive,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Conservative, Preset::Balanced, Preset::Aggressive];

    pub fn key(self) -> &'static str {
        match self {
            Preset::Conservative => "conservative",
            Preset::Balanced => "balanced",
            Preset::Aggressive => "aggressive",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Conservative => "保守型",
            Preset::Balanced => "稳健型",
            Preset::Aggressive => "进取型",
        }
    }

    /// 一句话：适合什么样的账户
    pub fn tagline(self) -> &'static str {
        match self {
            Preset::Conservative => "像存一笔不能亏太多的钱：宁可少赚，也要少亏",
            Preset::Balanced => "大多数账户的起点：有机会就参与，留一点余地",
            Preset::Aggressive => "能承受大起大落的闲钱：仓位集中，给股票更多波动空间",
        }
    }

    /// 这类账户一般怎么设计：每条说明一个参数为什么这样取
    pub fn design(self) -> Vec<&'static str> {
        match self {
            Preset::Conservative => vec![
                "只信过去表现明显好的策略：比随便买至少多涨 1 个百分点才算有效",
                "选股门槛高：至少 2 个有效策略同时看好才买，宁可错过",
                "分散：最多 5 只，单只不超过 20%，一只出问题影响有限",
                "留 20% 现金，跌的时候有余地",
                "止损紧（6%），收盘跌破 20 日均线就走，不扛单",
                "全市场偏弱时不买新股票；开盘比参考价高 3% 以上不追",
            ],
            Preset::Balanced => vec![
                "比随便买多涨 0.3 个百分点以上的策略算有效",
                "1 个有效策略看好就考虑买",
                "最多 6 只，单只不超过 25%，留 10% 现金",
                "止损 8%，收盘跌破 20 日均线就卖",
                "全市场偏弱时用于买新股票的钱减半；开盘比参考价高 5% 以上不追",
            ],
            Preset::Aggressive => vec![
                "只要比随便买好的策略都算有效，候选最多",
                "1 个有效策略看好就买，几乎满仓",
                "最多 8 只，单只可以到 30%",
                "止损宽（10%），趋势看 60 日均线，给股票更多波动空间，但单笔亏损会更大",
                "全市场偏弱也照常买；开盘比参考价高 7% 以内都追；持有的股票被 ST 也不急着卖",
            ],
        }
    }

    // @project-doc docs/domains/accounts.md#rules
    pub fn rules(self) -> Rules {
        let base = Rules {
            strategies: None,
            min_edge_pct: 0.3,
            min_picks: 1,
            max_positions: 6,
            max_weight_pct: 25.0,
            min_cash_pct: 10.0,
            stop_loss_pct: 8.0,
            trend_ma: 20,
            weak_market: WeakMarket::Half,
            weak_threshold_pct: 30.0,
            chase_limit_pct: 5.0,
            sell_st: true,
        };
        match self {
            Preset::Conservative => Rules {
                min_edge_pct: 1.0,
                min_picks: 2,
                max_positions: 5,
                max_weight_pct: 20.0,
                min_cash_pct: 20.0,
                stop_loss_pct: 6.0,
                weak_market: WeakMarket::Pause,
                chase_limit_pct: 3.0,
                ..base
            },
            Preset::Balanced => base,
            Preset::Aggressive => Rules {
                min_edge_pct: 0.0,
                max_positions: 8,
                max_weight_pct: 30.0,
                min_cash_pct: 0.0,
                stop_loss_pct: 10.0,
                trend_ma: 60,
                weak_market: WeakMarket::Normal,
                chase_limit_pct: 7.0,
                sell_st: false,
                ..base
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid_and_ordered() {
        for p in Preset::ALL {
            p.rules().validate().unwrap_or_else(|e| panic!("{}：{e}", p.name()));
        }
        let (c, b, a) = (Preset::Conservative.rules(), Preset::Balanced.rules(), Preset::Aggressive.rules());
        assert!(c.max_positions < b.max_positions && b.max_positions < a.max_positions);
        assert!(c.stop_loss_pct < b.stop_loss_pct && b.stop_loss_pct < a.stop_loss_pct);
        assert!(c.min_picks >= b.min_picks && b.min_picks >= a.min_picks);
        assert!(c.min_edge_pct > b.min_edge_pct && b.min_edge_pct > a.min_edge_pct, "越保守，有效策略的门槛越高");
        assert_eq!(Rules::default(), b);
        assert!(!b.effective(None) && b.effective(Some((0.5, 0.1))) && !b.effective(Some((0.3, 0.1))));
        assert!(!b.effective(Some((1.0, 0.6))), "超额不到标准误的 2 倍，多半是运气");
    }

    #[test]
    fn missing_fields_use_balanced_values_and_empty_means_all() {
        let r: Rules = serde_json::from_value(serde_json::json!({"stop_loss_pct": 5.0})).unwrap();
        assert_eq!(r.stop_loss_pct, 5.0);
        assert_eq!(r.max_positions, Preset::Balanced.rules().max_positions);
        assert!(r.uses(42));
        let r = Rules { strategies: Some(vec![1, 2]), ..r };
        assert!(r.uses(1) && !r.uses(3));
        assert!(Rules { strategies: Some(vec![]), ..r.clone() }.validate().is_err(), "一个都不勾不行，不会当成全部");
        assert!(Rules { trend_ma: 7, ..r }.validate().is_err());
    }
}
