//! 账户的选股和风控规则：参考哪些策略（新账户保存创建时的策略快照）、至少几个策略同时选中才买、最多持几只、
//! 单只最多占多少、留多少现金、弱市怎么办、追高上限。
//! 规则不绑定具体策略：候选股按同一套优先级排序（见 advice::generate）。
//!
//! 保守、稳健、进取只是三组参考值（`Preset`），在高级设置里作为辅助：说明这类账户一般怎么设计，点一下套用后还可以再改。
//! 新账户用稳健的参考值。规则见 docs/domains/accounts.md#rules。
//!
//! `factor` 不为空时账户改用多因子组合选股：不看勾选的策略，按因子合成得分每月调仓，见 docs/domains/accounts.md#factor_plan。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::analysis::factors::FactorId;

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

/// 多因子组合里的一个因子和它的权重（按因子的预期方向合成）
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq)]
pub struct FactorWeight {
    pub factor: FactorId,
    pub weight: f64,
}

/// 多因子组合选股：每月最后一个交易日收盘按合成得分排名，持有前 `top_n` 只，等权买入
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct FactorPlan {
    pub factors: Vec<FactorWeight>,
    /// 持有几只
    pub top_n: u32,
    /// 已持有的股票排名在 top_n × (1 + buffer) 以内时继续拿，减少换手
    pub buffer: f64,
}

impl FactorPlan {
    /// 研究记录里的组合 E：波动率、特质波动率、单季利润增速、应计利润、成交额等权，持有 30 只，缓冲 1.5
    pub fn combo_e() -> Self {
        let w = |factor| FactorWeight {
            factor,
            weight: 1.0,
        };
        FactorPlan {
            factors: vec![
                w(FactorId::Volatility),
                w(FactorId::IdioVol),
                w(FactorId::ProfitGrowth),
                w(FactorId::Accruals),
                w(FactorId::Liquidity),
            ],
            top_n: 30,
            buffer: 1.5,
        }
    }

    /// 排名在这个名次以内（从 0 算）的持仓继续拿
    pub fn keep_limit(&self) -> usize {
        ((self.top_n as f64) * (1.0 + self.buffer)).ceil() as usize
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.factors.is_empty() || self.factors.len() > 12 {
            return Err("多因子组合至少 1 个因子，最多 12 个".into());
        }
        let mut seen = std::collections::HashSet::new();
        if self.factors.iter().any(|f| !seen.insert(f.factor)) {
            return Err("多因子组合的因子不能重复".into());
        }
        if self
            .factors
            .iter()
            .any(|f| !f.weight.is_finite() || f.weight == 0.0 || f.weight.abs() > 10.0)
        {
            return Err("因子权重应是 -10 到 10 之间、不为 0 的数".into());
        }
        if !(5..=50).contains(&self.top_n) {
            return Err("多因子组合持有只数应在 5 到 50 之间".into());
        }
        if !self.buffer.is_finite() || !(0.0..=2.0).contains(&self.buffer) {
            return Err("多因子组合的缓冲区应在 0 到 2 之间".into());
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
    pub weak_market: WeakMarket,
    /// 全市场站上 20 日均线的股票比例低于多少（%）算弱市
    pub weak_threshold_pct: f64,
    /// 第二天开盘比参考价高多少（%）就不买
    pub chase_limit_pct: f64,
    /// 持有的股票被 ST 时卖出
    pub sell_st: bool,
    /// 多因子组合选股；不为空时不看勾选的策略、至少几个策略、最多持有只数和弱市规则
    #[serde(default)]
    pub factor: Option<FactorPlan>,
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
        if let Some(plan) = &self.factor {
            plan.validate()?;
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

/// 参考值：只是辅助，套用后还可以改。前三组按勾选的策略选股，`Factor` 改用多因子组合
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Conservative,
    Balanced,
    Aggressive,
    Factor,
}

impl Preset {
    pub const ALL: [Preset; 4] = [
        Preset::Conservative,
        Preset::Balanced,
        Preset::Aggressive,
        Preset::Factor,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Preset::Conservative => "conservative",
            Preset::Balanced => "balanced",
            Preset::Aggressive => "aggressive",
            Preset::Factor => "factor",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Conservative => "稳健型",
            Preset::Balanced => "均衡型",
            Preset::Aggressive => "积极型",
            Preset::Factor => "多因子月度调仓",
        }
    }

    /// 一句话：适合什么样的账户
    pub fn tagline(self) -> &'static str {
        match self {
            Preset::Conservative => "像存一笔不能亏太多的钱：宁可少赚，也要少亏",
            Preset::Balanced => "大多数账户的起点：有机会就参与，留一点余地",
            Preset::Aggressive => "能承受大起大落的闲钱：仓位集中，给股票更多波动空间",
            Preset::Factor => "不看买卖信号，每月按几个因子给全市场打分，分散持有排名靠前的 30 只",
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
                "策略决定什么时候卖出，账户只负责组合约束",
                "全市场偏弱时不买新股票；开盘比参考价高 3% 以上不追",
            ],
            Preset::Balanced => vec![
                "1 个勾选的策略看好就考虑买",
                "最多 6 只，单只不超过 25%，留 10% 现金",
                "策略决定什么时候卖出，账户只负责组合约束",
                "全市场偏弱时用于买新股票的钱减半；开盘比参考价高 5% 以上不追",
            ],
            Preset::Aggressive => vec![
                "勾选的策略都可以提供候选，候选最多",
                "1 个勾选的策略看好就买，几乎满仓",
                "最多 8 只，单只可以到 30%",
                "策略决定什么时候卖出，给策略更多波动空间",
                "全市场偏弱也照常买；开盘比参考价高 7% 以内都追；持有的股票被 ST 也不急着卖",
            ],
            Preset::Factor => vec![
                "因子：低波动率、低特质波动率、单季利润增速高、应计利润低、成交额小，五个等权",
                "每月最后一个交易日收盘排名，下一个交易日开盘调仓；空仓时马上建仓",
                "持有排名前 30 只，每只约占总资产的 1/30；已持有的跌出前 75 名才卖",
                "不看勾选的策略和弱市；持有的股票被 ST 时卖出，开盘比参考价高 5% 以上不追",
                "纯做多，跟着大盘涨跌，回撤可能到 30% 左右",
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
            weak_market: WeakMarket::Half,
            weak_threshold_pct: 30.0,
            chase_limit_pct: 5.0,
            sell_st: true,
            factor: None,
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
                weak_market: WeakMarket::Pause,
                chase_limit_pct: 3.0,
                ..base
            },
            Preset::Balanced => base,
            Preset::Aggressive => Rules {
                max_positions: 8,
                max_weight_pct: 30.0,
                min_cash_pct: 0.0,
                weak_market: WeakMarket::Normal,
                chase_limit_pct: 7.0,
                sell_st: false,
                ..base
            },
            Preset::Factor => Rules {
                min_cash_pct: 0.0,
                weak_market: WeakMarket::Normal,
                factor: Some(FactorPlan::combo_e()),
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
        assert!(c.min_picks >= b.min_picks && b.min_picks >= a.min_picks);
        assert_eq!(Rules::default(), b);
        let f = Preset::Factor.rules().factor.unwrap();
        assert_eq!((f.top_n, f.keep_limit()), (30, 75));
        assert!(
            FactorPlan {
                top_n: 60,
                ..f.clone()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn missing_fields_use_balanced_values_and_empty_means_all() {
        let r: Rules = serde_json::from_value(serde_json::json!({})).unwrap();
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
        assert!(r.validate().is_ok());
    }
}
