//! 账户的选股和风控规则：参考哪些策略（新账户保存创建时的策略快照）、至少几个策略同时选中才买、最多持几只、
//! 单只最多占多少、留多少现金、账户保护线、弱市怎么办、追高上限。
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

/// 买入前回看的周期。
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum HistoryWindow {
    ThreeMonths,
    SixMonths,
    OneYear,
    ThreeYears,
    FiveYears,
}

impl HistoryWindow {
    pub const ALL: [Self; 5] = [
        Self::ThreeMonths,
        Self::SixMonths,
        Self::OneYear,
        Self::ThreeYears,
        Self::FiveYears,
    ];

    pub fn months(self) -> u32 {
        match self {
            Self::ThreeMonths => 3,
            Self::SixMonths => 6,
            Self::OneYear => 12,
            Self::ThreeYears => 36,
            Self::FiveYears => 60,
        }
    }

    pub fn start(self, end: chrono::NaiveDate) -> chrono::NaiveDate {
        end.checked_sub_months(chrono::Months::new(self.months()))
            .unwrap_or(chrono::NaiveDate::MIN)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ThreeMonths => "3 个月",
            Self::SixMonths => "6 个月",
            Self::OneYear => "1 年",
            Self::ThreeYears => "3 年",
            Self::FiveYears => "5 年",
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct HistoryCheck {
    pub windows: Vec<HistoryWindow>,
    /// 总收益必须严格大于此百分比，0 表示赚钱；不是收益率小数。
    #[serde(default)]
    pub min_return_pct: f64,
}

impl HistoryCheck {
    pub fn validate(&self) -> Result<(), String> {
        if self.windows.is_empty() || self.windows.len() > HistoryWindow::ALL.len() {
            return Err("买入前回看至少选择一个周期，最多五个".into());
        }
        let mut seen = std::collections::HashSet::new();
        if self.windows.iter().any(|w| !seen.insert(*w)) {
            return Err("买入前回看的周期不能重复".into());
        }
        if !self.min_return_pct.is_finite() || self.min_return_pct < 0.0 {
            return Err("买入前回看的收益门槛必须是大于或等于 0 的有限数值".into());
        }
        Ok(())
    }
}

/// 一个账户的全部规则。缺的字段按均衡型参考值补齐，回看缺省为关闭。
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Rules {
    /// 只参考这些策略（策略编号）；为空时表示当前全部参加巡检的策略，新账户保存当前策略快照
    pub strategies: Option<Vec<i64>>,
    /// 至少几个勾选的策略同时选中才买
    pub min_picks: u32,
    /// 买入前回看；为空时关闭。
    #[serde(default)]
    pub history_check: Option<HistoryCheck>,
    /// 最多同时持有几只
    pub max_positions: u32,
    /// 单只最多占总资产的百分比
    pub max_weight_pct: f64,
    /// 至少留多少现金（占总资产的百分比）
    pub min_cash_pct: f64,
    /// 账户保护线：比成本价亏多少（%）就卖；默认关闭，由账户主动开启
    pub protection_stop_loss_pct: Option<f64>,
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
impl Rules {
    /// 参数是否在合理范围内
    pub fn validate(&self) -> Result<(), String> {
        let range = |v: f64, lo: f64, hi: f64, what: &str| {
            if (lo..=hi).contains(&v) {
                Ok(())
            } else {
                Err(format!("{what}应在 {lo} 到 {hi} 之间"))
            }
        };
        range(self.min_picks as f64, 1.0, 5.0, "至少几个有效策略同时选中")?;
        range(self.max_positions as f64, 1.0, 20.0, "最多持有只数")?;
        range(self.max_weight_pct, 5.0, 100.0, "单只上限（%）")?;
        range(self.min_cash_pct, 0.0, 90.0, "保留现金（%）")?;
        if let Some(v) = self.protection_stop_loss_pct {
            range(v, 2.0, 30.0, "账户保护线（%）")?;
        }
        range(self.weak_threshold_pct, 5.0, 60.0, "弱市阈值（%）")?;
        range(self.chase_limit_pct, 0.0, 20.0, "追高上限（%）")?;
        match &self.strategies {
            Some(l) if l.is_empty() => return Err("至少勾选一个策略".into()),
            Some(l) if l.len() > 64 => return Err("最多勾选 64 个策略".into()),
            _ => {}
        }
        if let Some(check) = &self.history_check {
            check.validate()?;
        }
        Ok(())
    }

    /// 这个策略是否参考
    pub fn uses(&self, strategy_id: i64) -> bool {
        self.strategies
            .as_ref()
            .is_none_or(|l| l.contains(&strategy_id))
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
            Preset::Conservative => "稳健型",
            Preset::Balanced => "均衡型",
            Preset::Aggressive => "积极型",
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
                "至少 2 个勾选的策略同时看好才买，宁可错过",
                "买入前回看：每个策略在这只股票上过去 1 年、3 年的总收益都大于 0",
                "分散：最多 5 只，单只不超过 20%，一只出问题影响有限",
                "留 20% 现金，跌的时候有余地",
                "策略决定什么时候卖出；账户只在主动开启保护线时限制亏损",
                "全市场偏弱时不买新股票；开盘比参考价高 3% 以上不追",
            ],
            Preset::Balanced => vec![
                "1 个勾选的策略看好就考虑买",
                "最多 6 只，单只不超过 25%，留 10% 现金",
                "策略决定什么时候卖出；账户保护线默认关闭",
                "全市场偏弱时用于买新股票的钱减半；开盘比参考价高 5% 以上不追",
            ],
            Preset::Aggressive => vec![
                "勾选的策略都可以提供候选，候选最多",
                "1 个勾选的策略看好就买，几乎满仓",
                "最多 8 只，单只可以到 30%",
                "策略决定什么时候卖出；账户保护线默认关闭，给策略更多波动空间",
                "全市场偏弱也照常买；开盘比参考价高 7% 以内都追；持有的股票被 ST 也不急着卖",
            ],
        }
    }

    // @project-doc docs/domains/accounts.md#rules
    pub fn rules(self) -> Rules {
        let base = Rules {
            strategies: None,
            min_picks: 1,
            history_check: None,
            max_positions: 6,
            max_weight_pct: 25.0,
            min_cash_pct: 10.0,
            protection_stop_loss_pct: None,
            weak_market: WeakMarket::Half,
            weak_threshold_pct: 30.0,
            chase_limit_pct: 5.0,
            sell_st: true,
        };
        match self {
            Preset::Conservative => Rules {
                min_picks: 2,
                history_check: Some(HistoryCheck {
                    windows: vec![HistoryWindow::OneYear, HistoryWindow::ThreeYears],
                    min_return_pct: 0.0,
                }),
                max_positions: 5,
                max_weight_pct: 20.0,
                min_cash_pct: 20.0,
                protection_stop_loss_pct: None,
                weak_market: WeakMarket::Pause,
                chase_limit_pct: 3.0,
                ..base
            },
            Preset::Balanced => base,
            Preset::Aggressive => Rules {
                max_positions: 8,
                max_weight_pct: 30.0,
                min_cash_pct: 0.0,
                protection_stop_loss_pct: None,
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
            p.rules()
                .validate()
                .unwrap_or_else(|e| panic!("{}：{e}", p.name()));
        }
        let (c, b, a) = (
            Preset::Conservative.rules(),
            Preset::Balanced.rules(),
            Preset::Aggressive.rules(),
        );
        assert!(c.max_positions < b.max_positions && b.max_positions < a.max_positions);
        assert!(c.protection_stop_loss_pct.is_none());
        assert!(b.protection_stop_loss_pct.is_none());
        assert!(a.protection_stop_loss_pct.is_none());
        assert!(c.min_picks >= b.min_picks && b.min_picks >= a.min_picks);
        assert_eq!(Rules::default(), b);
    }

    #[test]
    fn missing_fields_use_balanced_values_and_empty_means_all() {
        let r: Rules = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(r.protection_stop_loss_pct.is_none());
        assert_eq!(r.max_positions, Preset::Balanced.rules().max_positions);
        assert!(r.uses(42));
        let r = Rules {
            strategies: Some(vec![1, 2]),
            ..r
        };
        assert!(r.uses(1) && !r.uses(3));
        assert!(
            Rules {
                strategies: Some(vec![]),
                ..r.clone()
            }
            .validate()
            .is_err(),
            "一个都不勾不行，不会当成全部"
        );
        assert!(
            Rules {
                protection_stop_loss_pct: Some(1.0),
                ..r
            }
            .validate()
            .is_err()
        );
    }
}
