//! 选股策略：一段 JSON 定义，由股票池、买入条件、卖出规则和排序组成，不是代码。
//! 每日巡检、试运行、个股诊断和回测都用这里的判断逻辑。定义格式见 docs/domains/strategies.md#definition。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::indicators::{self, Frame, IndicatorId, IndicatorRef, StockMeta};
use super::signals::{self, Params, SignalId};

/// 「上市满 N 个交易日」的上限：N 决定巡检要读多少根历史日线
const MAX_LISTED_DAYS: u32 = 500;
/// 条件数量上限：太多的条件难以理解，也几乎选不出股票
const MAX_CONDITIONS: usize = 10;
/// 信号「最近 N 个交易日内发生过」的 N 上限
const MAX_WITHIN_DAYS: u32 = 20;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct Definition {
    #[serde(default)]
    pub universe: Universe,
    /// 全部满足
    #[serde(default)]
    pub all: Vec<Condition>,
    /// 至少满足一个；为空时不要求
    #[serde(default)]
    pub any: Vec<Condition>,
    /// 卖出规则：每日巡检据此给出卖点，回测据此卖出
    #[serde(default)]
    pub exit: ExitRule,
    #[serde(default)]
    pub sort: Sort,
    /// 每天最多保留多少只（按排序取前面的）
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// 条件里引用的自定义信号；读出定义后由 `with_lib` 填上，不存进 JSON
    #[serde(skip)]
    pub lib: SignalLib,
}

fn default_limit() -> u32 {
    50
}

/// 股票池：先排除不想要的股票，再判断条件
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct Universe {
    /// 只看这些板块：main 主板、chinext 创业板、star 科创板、bse 北交所；为空表示全部
    #[serde(default)]
    pub boards: Vec<String>,
    /// 排除简称含 ST 的股票
    #[serde(default = "yes")]
    pub exclude_st: bool,
    /// 最低股价（元，未复权收盘价）
    #[serde(default)]
    pub min_price: Option<f64>,
    /// 最低成交额（元）：排除成交太少、买卖不方便的股票
    #[serde(default)]
    pub min_amount: Option<f64>,
    /// 上市满多少个交易日：排除走势还不稳定的新股
    #[serde(default)]
    pub min_listed_days: Option<u32>,
    /// 只看强势板块中的股票（只用于每日巡检，回测时不使用）
    #[serde(default)]
    pub sector: Option<SectorFilter>,
}

fn yes() -> bool {
    true
}

impl Default for Universe {
    fn default() -> Self {
        Universe { boards: vec![], exclude_st: true, min_price: Some(2.0), min_amount: Some(5e7), min_listed_days: Some(60), sector: None }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct SectorFilter {
    /// industry 同花顺行业，cn_concept 同花顺概念
    pub category: String,
    /// 按最近多少个交易日的涨幅排名
    pub days: u32,
    /// 只保留排名前百分之多少的板块中的股票
    pub top_pct: f64,
}

// @project-doc docs/domains/strategies.md#definition
/// 条件：一句话的判断，结果是「是」或「否」。信号、策略的买入和卖出都用这一种写法。
/// 时间范围：默认看当天；within_days > 1 表示最近 N 个交易日内出现过，streak_days > 1 表示连续 N 个交易日都满足
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    /// 出现内置信号
    Signal {
        signal: SignalId,
        #[serde(default)]
        params: Params,
        #[serde(default = "one")]
        within_days: u32,
        #[serde(default = "one")]
        streak_days: u32,
    },
    /// 出现自己定义的信号（app.signal）
    MySignal {
        signal_id: i64,
        #[serde(default = "one")]
        within_days: u32,
        #[serde(default = "one")]
        streak_days: u32,
    },
    /// 指标与一个数或另一个指标比较
    Compare {
        left: IndicatorRef,
        op: Op,
        right: Operand,
        #[serde(default = "one")]
        within_days: u32,
        #[serde(default = "one")]
        streak_days: u32,
    },
}

fn one() -> u32 {
    1
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
    /// 上穿：前一天在下方（或相等），当天在上方；与均线金叉同一套容差规则
    #[serde(rename = "crosses_above")]
    CrossesAbove,
    /// 下穿
    #[serde(rename = "crosses_below")]
    CrossesBelow,
}

impl Op {
    fn word(self) -> &'static str {
        match self {
            Op::Gt => "高于",
            Op::Ge => "不低于",
            Op::Lt => "低于",
            Op::Le => "不高于",
            Op::CrossesAbove => "上穿",
            Op::CrossesBelow => "下穿",
        }
    }

    fn is_cross(self) -> bool {
        matches!(self, Op::CrossesAbove | Op::CrossesBelow)
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[serde(untagged)]
pub enum Operand {
    Value { value: f64 },
    Indicator(IndicatorRef),
}

/// 卖出规则：满足任一条件，或触发止损、止盈、最长持有天数时，在下一个交易日开盘卖出
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, Default)]
pub struct ExitRule {
    #[serde(default)]
    pub any: Vec<Condition>,
    /// 亏损达到这个百分比时卖出，例如 8 表示跌 8%
    #[serde(default)]
    pub stop_loss_pct: Option<f64>,
    /// 盈利达到这个百分比时卖出
    #[serde(default)]
    pub take_profit_pct: Option<f64>,
    /// 最多持有多少个交易日
    #[serde(default)]
    pub max_hold_days: Option<u32>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct Sort {
    pub by: IndicatorRef,
    #[serde(default = "yes")]
    pub desc: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { by: IndicatorRef::new(IndicatorId::Amount, None), desc: true }
    }
}

/// 信号的定义：几条条件，全部满足或任一满足。只能用指标和内置信号，不能引用别的自定义信号
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct SignalDef {
    /// true：全部满足；false：任一满足
    #[serde(default = "yes")]
    pub match_all: bool,
    pub conditions: Vec<Condition>,
}

/// 一个自定义信号：名称、偏多或偏空、定义
#[derive(Clone, Debug)]
pub struct CustomSignal {
    pub name: String,
    pub bullish: bool,
    pub def: SignalDef,
}

/// 判断时可用的自定义信号，按 signal_id
pub type SignalLib = std::sync::Arc<std::collections::BTreeMap<i64, CustomSignal>>;

impl SignalDef {
    pub fn validate(&self, lib: &SignalLib) -> Result<(), String> {
        if self.conditions.is_empty() {
            return Err("信号至少要有一条条件".into());
        }
        if self.conditions.len() > MAX_CONDITIONS {
            return Err(format!("信号的条件最多 {MAX_CONDITIONS} 条"));
        }
        for c in &self.conditions {
            if matches!(c, Condition::MySignal { .. }) {
                return Err("信号只能由指标和内置信号组成，不能引用别的自定义信号".into());
            }
            c.validate(lib)?;
        }
        Ok(())
    }

    pub fn warmup(&self, lib: &SignalLib) -> u32 {
        self.conditions.iter().map(|c| c.warmup(lib)).max().unwrap_or(0)
    }

    /// 每天是否满足，与日线等长
    pub fn series(&self, f: &Frame, lib: &SignalLib) -> Vec<bool> {
        let parts: Vec<std::rc::Rc<Vec<bool>>> = self.conditions.iter().map(|c| c.series(f, lib)).collect();
        (0..f.len()).map(|i| if self.match_all { parts.iter().all(|p| p[i]) } else { parts.iter().any(|p| p[i]) }).collect()
    }

    /// 每条条件的白话
    pub fn describe(&self, lib: &SignalLib) -> Vec<String> {
        self.conditions.iter().map(|c| c.describe(lib)).collect()
    }

    /// 是事件（某天发生）还是状态（持续成立）：含上穿下穿、事件类内置信号或时间范围时算事件
    pub fn is_event(&self) -> bool {
        self.conditions.iter().any(|c| match c {
            Condition::Signal { signal, .. } => signals::info_of(*signal).kind == signals::SignalKind::Event,
            Condition::Compare { op, .. } => op.is_cross(),
            Condition::MySignal { .. } => false,
        })
    }

    /// 用到的指标（去重）
    pub fn indicators(&self) -> Vec<IndicatorRef> {
        let mut out: Vec<IndicatorRef> = vec![];
        for c in &self.conditions {
            if let Condition::Compare { left, right, .. } = c {
                out.push(*left);
                if let Operand::Indicator(r) = right {
                    out.push(*r);
                }
            }
        }
        out.dedup();
        out
    }
}

impl Condition {
    fn window(&self) -> (u32, u32) {
        match self {
            Condition::Signal { within_days, streak_days, .. }
            | Condition::MySignal { within_days, streak_days, .. }
            | Condition::Compare { within_days, streak_days, .. } => (*within_days, *streak_days),
        }
    }

    pub fn validate(&self, lib: &SignalLib) -> Result<(), String> {
        let (within, streak) = self.window();
        if !(1..=MAX_WITHIN_DAYS).contains(&within) || !(1..=MAX_WITHIN_DAYS).contains(&streak) {
            return Err(format!("「最近 N 个交易日」「连续 N 个交易日」的 N 应在 1 到 {MAX_WITHIN_DAYS} 之间"));
        }
        if within > 1 && streak > 1 {
            return Err("「最近 N 天内出现过」和「连续 N 天」只能选一个".into());
        }
        match self {
            Condition::Signal { signal, params, .. } => signals::validate(*signal, params),
            Condition::MySignal { signal_id, .. } => {
                if lib.contains_key(signal_id) { Ok(()) } else { Err(format!("自定义信号不存在：{signal_id}")) }
            }
            Condition::Compare { left, op, right, .. } => {
                left.validate()?;
                match right {
                    Operand::Indicator(r) => r.validate()?,
                    Operand::Value { value } if !value.is_finite() => return Err("比较的数值不合法".into()),
                    Operand::Value { .. } if op.is_cross() => {}
                    Operand::Value { .. } => {}
                }
                Ok(())
            }
        }
    }

    fn warmup(&self, lib: &SignalLib) -> u32 {
        let (within, streak) = self.window();
        let base = match self {
            Condition::Signal { signal, params, .. } => signals::warmup(*signal, params),
            Condition::MySignal { signal_id, .. } => lib.get(signal_id).map_or(0, |s| s.def.warmup(lib)),
            Condition::Compare { left, op, right, .. } => {
                let r = match right {
                    Operand::Indicator(r) => r.warmup(),
                    Operand::Value { .. } => 0,
                };
                left.warmup().max(r) + if op.is_cross() { 2 } else { 0 }
            }
        };
        base + within.max(streak)
    }

    /// 不看时间范围、只看当天的结果
    fn raw_series(&self, f: &Frame, lib: &SignalLib) -> std::rc::Rc<Vec<bool>> {
        match self {
            Condition::Signal { signal, params, .. } => signals::series(f, *signal, params),
            Condition::MySignal { signal_id, .. } => {
                let key = format!("my:{signal_id}");
                f.memo_bools(key, || lib.get(signal_id).map_or_else(|| vec![false; f.len()], |s| s.def.series(f, lib)))
            }
            Condition::Compare { left, op, right, .. } => {
                let key = format!("cmp:{left:?}{op:?}{right:?}");
                f.memo_bools(key, || {
                    let l = f.get(*left);
                    let r: std::rc::Rc<Vec<f64>> = match right {
                        Operand::Value { value } => std::rc::Rc::new(vec![*value; f.len()]),
                        Operand::Indicator(r) => f.get(*r),
                    };
                    match op {
                        Op::CrossesAbove => signals::crossing(&l, &r, true),
                        Op::CrossesBelow => signals::crossing(&l, &r, false),
                        _ => l
                            .iter()
                            .zip(r.iter())
                            .map(|(&a, &b)| {
                                !a.is_nan()
                                    && !b.is_nan()
                                    && match op {
                                        Op::Gt => a > b,
                                        Op::Ge => a >= b,
                                        Op::Lt => a < b,
                                        _ => a <= b,
                                    }
                            })
                            .collect(),
                    }
                })
            }
        }
    }

    /// 每天是否满足（含时间范围），与日线等长
    pub fn series(&self, f: &Frame, lib: &SignalLib) -> std::rc::Rc<Vec<bool>> {
        let (within, streak) = self.window();
        let raw = self.raw_series(f, lib);
        if within <= 1 && streak <= 1 {
            return raw;
        }
        let n = f.len();
        let out = (0..n)
            .map(|i| {
                if within > 1 {
                    let from = (i + 1).saturating_sub(within as usize);
                    raw[from..=i].iter().any(|v| *v)
                } else {
                    i + 1 >= streak as usize && raw[i + 1 - streak as usize..=i].iter().all(|v| *v)
                }
            })
            .collect();
        std::rc::Rc::new(out)
    }

    /// 在第 i 天是否满足
    pub fn eval(&self, f: &Frame, i: usize, lib: &SignalLib) -> bool {
        self.series(f, lib)[i]
    }

    /// 白话描述，例如「最近 3 个交易日内出现过「5 日均线上穿 20 日均线」」「RSI（14）上穿 30」「连续 3 个交易日收盘价高于 20 日均线」
    pub fn describe(&self, lib: &SignalLib) -> String {
        let (within, streak) = self.window();
        let core = match self {
            Condition::Signal { signal, params, .. } => {
                let state = signals::info_of(*signal).kind == signals::SignalKind::State;
                return wrap_signal(&signals::label(*signal, params), state, within, streak);
            }
            Condition::MySignal { signal_id, .. } => {
                let (name, state) = lib.get(signal_id).map_or((format!("自定义信号 {signal_id}"), false), |s| (s.name.clone(), !s.def.is_event()));
                return wrap_signal(&name, state, within, streak);
            }
            Condition::Compare { left, op, right, .. } => {
                let r = match right {
                    Operand::Value { value } => format_value(*value, indicators::info_of(left.id).unit),
                    Operand::Indicator(r) => r.label(),
                };
                format!("{}{} {r}", left.label(), op.word())
            }
        };
        if within > 1 {
            format!("最近 {within} 个交易日内出现过「{core}」")
        } else if streak > 1 {
            format!("连续 {streak} 个交易日{core}")
        } else {
            core
        }
    }
}

fn wrap_signal(name: &str, state: bool, within: u32, streak: u32) -> String {
    if streak > 1 {
        return format!("连续 {streak} 个交易日{}「{name}」", if state { "处于" } else { "出现" });
    }
    match (within <= 1, state) {
        (true, false) => format!("当天出现「{name}」"),
        (true, true) => format!("当天处于「{name}」"),
        (false, false) => format!("最近 {within} 个交易日内出现过「{name}」"),
        (false, true) => format!("最近 {within} 个交易日内处于过「{name}」"),
    }
}

fn format_value(v: f64, unit: &str) -> String {
    let n = if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") };
    match unit {
        "pct" => format!("{n}%"),
        "ratio" => format!("{n} 倍"),
        "yuan" if v.abs() >= 1e8 => format!("{} 亿元", v / 1e8),
        "yuan" if v.abs() >= 1e4 => format!("{} 万元", v / 1e4),
        "yuan" => format!("{n} 元"),
        "price" => format!("{n} 元"),
        _ => n,
    }
}

/// 一次判断的结果：是否入选，以及入选时命中的条件
pub struct Evaluation {
    pub matched: bool,
    pub reasons: Vec<String>,
}

impl Definition {
    pub fn validate(&self) -> Result<(), String> {
        if self.all.is_empty() && self.any.is_empty() {
            return Err("至少要有一个条件".into());
        }
        if self.all.len() + self.any.len() > MAX_CONDITIONS || self.exit.any.len() > MAX_CONDITIONS {
            return Err(format!("条件最多 {MAX_CONDITIONS} 个"));
        }
        for c in self.all.iter().chain(&self.any).chain(&self.exit.any) {
            c.validate(&self.lib)?;
        }
        self.sort.by.validate()?;
        if !(1..=200).contains(&self.limit) {
            return Err("每天保留的数量应在 1 到 200 之间".into());
        }
        if self.universe.min_listed_days.is_some_and(|d| d > MAX_LISTED_DAYS) {
            return Err(format!("上市天数最多 {MAX_LISTED_DAYS} 个交易日"));
        }
        for b in &self.universe.boards {
            if !["main", "chinext", "star", "bse"].contains(&b.as_str()) {
                return Err(format!("未知的板块：{b}"));
            }
        }
        if let Some(s) = &self.universe.sector
            && (!["industry", "cn_concept"].contains(&s.category.as_str()) || !(1..=60).contains(&s.days) || !(1.0..=100.0).contains(&s.top_pct))
        {
            return Err("强势板块条件：分类只能是行业或概念，天数 1 到 60，前百分比 1 到 100".into());
        }
        for v in [self.exit.stop_loss_pct, self.exit.take_profit_pct].into_iter().flatten() {
            if !(0.5..=90.0).contains(&v) {
                return Err("止损、止盈的百分比应在 0.5 到 90 之间".into());
            }
        }
        if let Some(d) = self.exit.max_hold_days
            && !(1..=250).contains(&d)
        {
            return Err("最长持有天数应在 1 到 250 之间".into());
        }
        Ok(())
    }

    /// 判断需要的历史日线根数
    /// 判断一天需要往前看多少根日线。「上市满 N 个交易日」按读入的日线根数判断，所以也要覆盖 N
    pub fn warmup(&self) -> u32 {
        let conditions = self.all.iter().chain(&self.any).chain(&self.exit.any).map(|c| c.warmup(&self.lib)).max().unwrap_or(0);
        conditions.max(self.sort.by.warmup()).max(self.universe.min_listed_days.unwrap_or(0)).max(30)
    }

    /// 在第 i 天是否满足买入条件（不含股票池），不生成说明文字；回测逐日判断时用
    pub fn matches(&self, f: &Frame, i: usize) -> bool {
        self.all.iter().all(|c| c.eval(f, i, &self.lib)) && (self.any.is_empty() || self.any.iter().any(|c| c.eval(f, i, &self.lib)))
    }

    /// 在第 i 天是否满足买入条件（不含股票池），以及命中了哪些条件
    pub fn eval(&self, f: &Frame, i: usize) -> Evaluation {
        let mut reasons = vec![];
        for c in &self.all {
            if !c.eval(f, i, &self.lib) {
                return Evaluation { matched: false, reasons: vec![] };
            }
            reasons.push(c.describe(&self.lib));
        }
        if !self.any.is_empty() {
            let hit: Vec<String> = self.any.iter().filter(|c| c.eval(f, i, &self.lib)).map(|c| c.describe(&self.lib)).collect();
            if hit.is_empty() {
                return Evaluation { matched: false, reasons: vec![] };
            }
            reasons.extend(hit);
        }
        Evaluation { matched: true, reasons }
    }

    /// 第 i 天是否满足卖出条件（不含止损、止盈、持有天数）
    pub fn exit_signal(&self, f: &Frame, i: usize) -> bool {
        self.exit.any.iter().any(|c| c.eval(f, i, &self.lib))
    }

    /// 每天是否满足买入条件（不含股票池），一次算出整段，供买卖点和回测逐日使用
    pub fn buy_series(&self, f: &Frame) -> Vec<bool> {
        let all: Vec<_> = self.all.iter().map(|c| c.series(f, &self.lib)).collect();
        let any: Vec<_> = self.any.iter().map(|c| c.series(f, &self.lib)).collect();
        (0..f.len()).map(|i| all.iter().all(|s| s[i]) && (any.is_empty() || any.iter().any(|s| s[i]))).collect()
    }

    /// 每天是否满足任一卖出条件（不含止损、止盈、持有天数）
    pub fn exit_series(&self, f: &Frame) -> Vec<bool> {
        let parts: Vec<_> = self.exit.any.iter().map(|c| c.series(f, &self.lib)).collect();
        (0..f.len()).map(|i| parts.iter().any(|s| s[i])).collect()
    }

    /// 第 i 天触发的卖出条件（白话），没有时为空
    pub fn exit_reason_text(&self, f: &Frame, i: usize) -> Option<String> {
        self.exit.any.iter().find(|c| c.eval(f, i, &self.lib)).map(|c| c.describe(&self.lib))
    }

    /// 填上可用的自定义信号
    pub fn with_lib(mut self, lib: SignalLib) -> Self {
        self.lib = lib;
        self
    }

    /// 一条条件的白话
    pub fn describe_condition(&self, c: &Condition) -> String {
        c.describe(&self.lib)
    }

    /// 引用的自定义信号
    pub fn my_signals(&self) -> Vec<i64> {
        self.all.iter().chain(&self.any).chain(&self.exit.any).filter_map(|c| if let Condition::MySignal { signal_id, .. } = c { Some(*signal_id) } else { None }).collect()
    }

    /// 引用的内置信号
    pub fn builtin_signals(&self) -> Vec<SignalId> {
        self.all.iter().chain(&self.any).chain(&self.exit.any).filter_map(|c| if let Condition::Signal { signal, .. } = c { Some(*signal) } else { None }).collect()
    }

    /// 引用的指标（条件和排序）
    pub fn indicator_ids(&self) -> Vec<IndicatorId> {
        let mut out = vec![self.sort.by.id];
        for c in self.all.iter().chain(&self.any).chain(&self.exit.any) {
            if let Condition::Compare { left, right, .. } = c {
                out.push(left.id);
                if let Operand::Indicator(r) = right {
                    out.push(r.id);
                }
            }
        }
        out
    }

    /// 股票池过滤（板块、ST、价格、成交额、上市天数）。强势板块在巡检中单独处理
    pub fn in_universe(&self, f: &Frame, meta: &StockMeta, i: usize) -> bool {
        let u = &self.universe;
        let b = &f.bars[i];
        (u.boards.is_empty() || u.boards.contains(&meta.board))
            && !(u.exclude_st && meta.is_st)
            && u.min_price.is_none_or(|p| b.close >= p)
            && u.min_amount.is_none_or(|a| b.amount >= a)
            && u.min_listed_days.is_none_or(|d| i + 1 >= d as usize)
    }

    /// 股票池的白话描述
    pub fn describe_universe(&self) -> Vec<String> {
        let u = &self.universe;
        let mut out = vec![];
        if !u.boards.is_empty() {
            let names: Vec<&str> = u
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
        if u.exclude_st {
            out.push("排除 ST 股".into());
        }
        if let Some(p) = u.min_price {
            out.push(format!("股价不低于 {p} 元"));
        }
        if let Some(a) = u.min_amount {
            out.push(format!("成交额不低于 {}", format_value(a, "yuan")));
        }
        if let Some(d) = u.min_listed_days {
            out.push(format!("上市满 {d} 个交易日"));
        }
        if let Some(s) = &u.sector {
            let cat = if s.category == "industry" { "行业" } else { "概念" };
            out.push(format!("所属{cat}近 {} 日涨幅排名前 {}%", s.days, s.top_pct));
        }
        out
    }
}

/// 内置模板
#[derive(Serialize, ToSchema, Clone)]
pub struct Template {
    pub key: &'static str,
    pub name: &'static str,
    /// 一句话：它在找什么样的股票
    pub description: &'static str,
    pub definition: Definition,
}

fn sig(signal: SignalId, params: &[(&str, f64)], within_days: u32) -> Condition {
    Condition::Signal { signal, params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(), within_days, streak_days: 1 }
}

/// 指标与一个数比较，当天
pub fn cmp(id: IndicatorId, n: Option<u32>, op: Op, value: f64) -> Condition {
    Condition::Compare { left: IndicatorRef::new(id, n), op, right: Operand::Value { value }, within_days: 1, streak_days: 1 }
}

/// 指标与另一个指标比较，当天
pub fn cmp_ind(id: IndicatorId, n: Option<u32>, op: Op, rid: IndicatorId, rn: Option<u32>) -> Condition {
    Condition::Compare { left: IndicatorRef::new(id, n), op, right: Operand::Indicator(IndicatorRef::new(rid, rn)), within_days: 1, streak_days: 1 }
}

/// 模板默认的卖出规则：跌破 20 日均线，或亏损 8%，或持有满 20 个交易日
fn default_exit() -> ExitRule {
    ExitRule { any: vec![sig(SignalId::PriceCrossBelowMa, &[("n", 20.0)], 1)], stop_loss_pct: Some(8.0), take_profit_pct: None, max_hold_days: Some(20) }
}

fn def(all: Vec<Condition>, sort: Sort) -> Definition {
    Definition { universe: Universe::default(), all, any: vec![], exit: default_exit(), sort, limit: 50, lib: SignalLib::default() }
}

fn by(id: IndicatorId, n: Option<u32>, desc: bool) -> Sort {
    Sort { by: IndicatorRef::new(id, n), desc }
}

// @project-doc docs/domains/strategies.md#templates
pub fn templates() -> Vec<Template> {
    use IndicatorId as I;
    use SignalId as S;
    let amount = by(I::Amount, None, true);
    vec![
        Template {
            key: "ma_golden",
            name: "均线金叉",
            description: "5 日均线刚上穿 20 日均线，短期走势开始转强",
            definition: def(vec![sig(S::MaCrossUp, &[("fast", 5.0), ("slow", 20.0)], 1)], amount.clone()),
        },
        Template {
            key: "bullish_trend",
            name: "均线多头",
            description: "5、20、60 日均线从上到下依次排列，股价在 20 日均线上方，处在上升趋势中",
            definition: def(
                vec![sig(S::MaBullish, &[], 1), cmp_ind(I::Close, None, Op::Gt, I::Ma, Some(20))],
                by(I::Momentum, Some(20), true),
            ),
        },
        Template {
            key: "pullback_ma20",
            name: "回踩 20 日均线",
            description: "上升趋势中，股价回落到 20 日均线附近（上下 2% 以内），当天收涨",
            definition: def(
                vec![
                    sig(S::MaBullish, &[], 1),
                    cmp(I::MaGap, Some(20), Op::Ge, -2.0),
                    cmp(I::MaGap, Some(20), Op::Le, 2.0),
                    cmp(I::ChangePct, None, Op::Gt, 0.0),
                ],
                amount.clone(),
            ),
        },
        Template {
            key: "volume_breakout",
            name: "放量突破",
            description: "成交量放大到前 5 天平均的 2 倍以上，同时收盘站上 20 日均线",
            definition: def(vec![sig(S::VolumeSurge, &[("ratio", 2.0)], 1), sig(S::PriceCrossAboveMa, &[("n", 20.0)], 1)], by(I::VolRatio, Some(5), true)),
        },
        Template {
            key: "new_high_60",
            name: "创 60 日新高",
            description: "收盘价创近 3 个月新高，强势股常见的形态",
            definition: def(vec![sig(S::NewHigh, &[("n", 60.0)], 1)], amount.clone()),
        },
        Template {
            key: "macd_golden",
            name: "MACD 金叉",
            description: "MACD 快线上穿慢线，动能由弱转强",
            definition: def(vec![sig(S::MacdGolden, &[], 1)], amount.clone()),
        },
        Template {
            key: "oversold_rebound",
            name: "超跌反弹",
            description: "近 20 个交易日跌了 20% 以上，当天涨超 2%，可能是跌多了的反弹",
            definition: def(vec![cmp(I::Momentum, Some(20), Op::Le, -20.0), cmp(I::ChangePct, None, Op::Gt, 2.0)], by(I::ChangePct, None, true)),
        },
        Template {
            key: "boll_breakout",
            name: "布林带突破",
            description: "收盘价突破布林带上轨，走势明显强于平时",
            definition: def(vec![sig(S::BollBreakUpper, &[], 1)], amount.clone()),
        },
        Template {
            key: "low_volume_surge",
            name: "低位放量",
            description: "股价在近 60 天低点附近（高出不到 10%），成交量明显放大，可能有资金开始进入",
            definition: def(vec![cmp(I::PctFromLow, Some(60), Op::Le, 10.0), sig(S::VolumeSurge, &[("ratio", 2.0)], 1)], by(I::VolRatio, Some(5), true)),
        },
        Template {
            key: "strong_pullback",
            name: "强势股回调",
            description: "近 20 个交易日涨了 20% 以上，最近 3 天小幅回落（不超过 8%），强势股的回调",
            definition: def(
                vec![cmp(I::Momentum, Some(20), Op::Ge, 20.0), cmp(I::Momentum, Some(3), Op::Le, 0.0), cmp(I::Momentum, Some(3), Op::Ge, -8.0)],
                by(I::Momentum, Some(20), true),
            ),
        },
        Template {
            key: "macd_bottom_divergence",
            name: "MACD 底背离",
            description: "股价创了新低，MACD 的低点却抬高了，下跌的力量在减弱；当天收涨，确认有人买",
            definition: def(vec![sig(S::MacdBottomDivergence, &[], 3), cmp(I::ChangePct, None, Op::Gt, 0.0)], amount.clone()),
        },
        Template {
            key: "supertrend_follow",
            name: "Supertrend 趋势跟随",
            description: "Supertrend 翻多时买入，翻空时卖出，中间不管涨跌一直拿着；亏损 10% 止损",
            definition: Definition {
                exit: ExitRule { any: vec![sig(S::SupertrendDown, &[], 1)], stop_loss_pct: Some(10.0), take_profit_pct: None, max_hold_days: None },
                ..def(vec![sig(S::SupertrendUp, &[], 1)], amount.clone())
            },
        },
        Template {
            key: "adx_ma_golden",
            name: "趋势确认的均线金叉",
            description: "5 日均线上穿 20 日均线，同时 ADX 高于 20、+DI 在 −DI 上方：只做趋势明确时的金叉，过滤横盘中的假金叉",
            definition: def(
                vec![sig(S::MaCrossUp, &[("fast", 5.0), ("slow", 20.0)], 1), cmp(I::Adx, Some(14), Op::Gt, 20.0), cmp_ind(I::PlusDi, Some(14), Op::Gt, I::MinusDi, Some(14))],
                by(I::Adx, Some(14), true),
            ),
        },
        Template {
            key: "shrink_pullback",
            name: "缩量回踩 20 日均线",
            description: "上升趋势中回落到 20 日均线附近（上下 2% 以内），成交量缩到前 5 天平均的 0.7 倍以下：回调时卖的人不多",
            definition: def(
                vec![sig(S::MaBullish, &[], 1), cmp(I::MaGap, Some(20), Op::Ge, -2.0), cmp(I::MaGap, Some(20), Op::Le, 2.0), sig(S::VolumeShrink, &[("ratio", 0.7)], 1)],
                amount.clone(),
            ),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::tests::bars_from;

    #[test]
    fn templates_are_valid_and_round_trip() {
        for t in templates() {
            t.definition.validate().unwrap_or_else(|e| panic!("模板 {} 不合法：{e}", t.key));
            let json = serde_json::to_value(&t.definition).unwrap();
            let back: Definition = serde_json::from_value(json).unwrap();
            assert_eq!(back.all.len(), t.definition.all.len());
            // 巡检读入的日线要够判断「上市满 N 个交易日」，否则所有股票都会被排除
            assert!(t.definition.warmup() >= t.definition.universe.min_listed_days.unwrap_or(0), "{}", t.key);
        }
    }

    #[test]
    fn definition_json_and_evaluation() {
        let json = serde_json::json!({
            "all": [
                {"kind": "signal", "signal": "ma_cross_up", "params": {"fast": 2, "slow": 3}, "within_days": 2},
                {"kind": "compare", "left": {"id": "close"}, "op": ">", "right": {"id": "ma", "n": 3}}
            ]
        });
        let d: Definition = serde_json::from_value(json).unwrap();
        d.validate().unwrap();
        let b = bars_from(&[5.0, 4.0, 3.0, 2.0, 3.0, 5.0, 8.0]);
        let f = Frame::new(&b, None);
        // 下标 5 金叉；下标 6 在「最近 2 天内」窗口中，收盘 8 高于 3 日均线 5.33
        let e = d.eval(&f, 6);
        assert!(e.matched);
        assert_eq!(e.reasons, vec!["最近 2 个交易日内出现过「2 日均线上穿 3 日均线」", "收盘价高于 3 日均线"]);
        assert!(!d.eval(&f, 4).matched);
    }

    #[test]
    fn invalid_definitions_are_rejected() {
        let empty: Definition = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(empty.validate().is_err());
        let bad = serde_json::json!({"all": [{"kind": "compare", "left": {"id": "ma", "n": 999}, "op": ">", "right": {"value": 1}}]});
        assert!(serde_json::from_value::<Definition>(bad).unwrap().validate().is_err());
        let unknown = serde_json::json!({"all": [{"kind": "signal", "signal": "no_such_signal"}]});
        assert!(serde_json::from_value::<Definition>(unknown).is_err());
    }

    #[test]
    fn describe_compare_values_with_units() {
        let c = cmp(IndicatorId::Amount, None, Op::Ge, 5e7);
        assert_eq!(c.describe(&SignalLib::default()), "成交额不低于 5000 万元");
        let c = cmp(IndicatorId::Rsi, Some(14), Op::Lt, 70.0);
        assert_eq!(c.describe(&SignalLib::default()), "RSI（14）低于 70");
    }
}
