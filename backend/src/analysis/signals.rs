//! 信号：某一天发生的一件事（例如「5 日均线上穿 20 日均线」），或某一天所处的状态（例如「均线多头排列」）。
//! 每个信号在一只股票的日线上算出与日线等长的布尔序列。目录和口径见 docs/domains/strategies.md#signals。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::indicators::{Frame, IndicatorId, IndicatorRef};
use super::ma::{CrossSignal, cross_signals};

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SignalId {
    MaCrossUp,
    MaCrossDown,
    MaBullish,
    MaBearish,
    PriceCrossAboveMa,
    PriceCrossBelowMa,
    MacdGolden,
    MacdDeath,
    MacdTurnPositive,
    KdjGolden,
    KdjDeath,
    RsiLeaveOversold,
    RsiEnterOverbought,
    BollBreakUpper,
    BollBreakLower,
    NewHigh,
    NewLow,
    VolumeSurge,
    BigUp,
    GapUp,
    LongLowerShadow,
    LimitUp,
    LimitDown,
    BrokenLimitUp,
    ConsecutiveLimitUp,
}

/// 信号参数的说明
#[derive(Serialize, ToSchema, Clone, Copy)]
pub struct ParamInfo {
    pub key: &'static str,
    pub label: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    /// 是否只能取整数
    pub integer: bool,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// 通常被看作走强
    Bullish,
    /// 通常被看作走弱
    Bearish,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// 事件：只在发生的那一天为真
    Event,
    /// 状态：满足期间每天都为真
    State,
}

#[derive(Serialize, ToSchema, Clone, Copy)]
pub struct SignalInfo {
    pub id: SignalId,
    /// 名称，参数用 {key} 占位，例如「{fast} 日均线上穿 {slow} 日均线」
    pub name: &'static str,
    pub group: &'static str,
    pub tone: Tone,
    pub kind: SignalKind,
    pub description: &'static str,
    /// 怎么看、要注意什么，给没有金融背景的人看
    pub usage: &'static str,
    pub params: &'static [ParamInfo],
}

const fn p(key: &'static str, label: &'static str, default: f64, min: f64, max: f64, integer: bool) -> ParamInfo {
    ParamInfo { key, label, default, min, max, integer }
}

const MA_PAIR: &[ParamInfo] = &[p("fast", "快线周期", 5.0, 2.0, 120.0, true), p("slow", "慢线周期", 20.0, 3.0, 250.0, true)];
const MA_TRIPLE: &[ParamInfo] = &[p("short", "短期", 5.0, 2.0, 60.0, true), p("mid", "中期", 20.0, 3.0, 120.0, true), p("long", "长期", 60.0, 5.0, 250.0, true)];
const MA_ONE: &[ParamInfo] = &[p("n", "均线周期", 20.0, 2.0, 250.0, true)];
const DAYS: &[ParamInfo] = &[p("n", "天数", 60.0, 5.0, 250.0, true)];
const NONE: &[ParamInfo] = &[];

use SignalId as S;
use SignalKind::{Event, State};
use Tone::{Bearish, Bullish};

pub const CATALOG: &[SignalInfo] = &[
    SignalInfo { id: S::MaCrossUp, name: "{fast} 日均线上穿 {slow} 日均线", group: "均线", tone: Bullish, kind: Event, description: "金叉：短期均线从下往上穿过长期均线，常被看作走势转强", usage: "最常用的转强信号。周期越短越灵敏，假信号也越多；股价横盘时金叉死叉会来回出现，最好结合成交量或更长的均线一起看。", params: MA_PAIR },
    SignalInfo { id: S::MaCrossDown, name: "{fast} 日均线下穿 {slow} 日均线", group: "均线", tone: Bearish, kind: Event, description: "死叉：短期均线从上往下穿过长期均线，常被看作走势转弱", usage: "常用的转弱信号，很多人把它当作卖出提示。和金叉一样，横盘时容易反复出现。", params: MA_PAIR },
    SignalInfo { id: S::MaBullish, name: "均线多头排列（{short} > {mid} > {long}）", group: "均线", tone: Bullish, kind: State, description: "短期均线在上、长期均线在下，处在上升趋势中", usage: "短、中、长期走势一致向上，是趋势最清楚的状态之一。出现时往往已经涨了一段，适合顺着趋势做，而不是抄底。", params: MA_TRIPLE },
    SignalInfo { id: S::MaBearish, name: "均线空头排列（{short} < {mid} < {long}）", group: "均线", tone: Bearish, kind: State, description: "短期均线在下、长期均线在上，处在下降趋势中", usage: "下降趋势最清楚的状态。处在空头排列的股票，反弹常常到均线附近就受阻。", params: MA_TRIPLE },
    SignalInfo { id: S::PriceCrossAboveMa, name: "收盘站上 {n} 日均线", group: "均线", tone: Bullish, kind: Event, description: "前一天收在均线下方，当天收在上方", usage: "比均线交叉出现得早，但也更容易是假突破，之后几天能不能站稳更重要。20 日均线常被看作中短期强弱的分界线。", params: MA_ONE },
    SignalInfo { id: S::PriceCrossBelowMa, name: "收盘跌破 {n} 日均线", group: "均线", tone: Bearish, kind: Event, description: "前一天收在均线上方，当天收在下方", usage: "常用的止损或减仓提示，策略模板默认的卖出规则就是跌破 20 日均线。", params: MA_ONE },
    SignalInfo { id: S::MacdGolden, name: "MACD 金叉", group: "MACD", tone: Bullish, kind: Event, description: "MACD 快线（DIF）上穿慢线（DEA）", usage: "MACD 是两条指数均线的差，反应比普通均线快。出现在零轴上方（DIF 大于 0）的金叉比零轴下方的更强。", params: NONE },
    SignalInfo { id: S::MacdDeath, name: "MACD 死叉", group: "MACD", tone: Bearish, kind: Event, description: "MACD 快线（DIF）下穿慢线（DEA）", usage: "动能转弱。零轴上方的死叉常常只是上涨中的回调，零轴下方的死叉说明弱势还在延续。", params: NONE },
    SignalInfo { id: S::MacdTurnPositive, name: "MACD 上穿零轴", group: "MACD", tone: Bullish, kind: Event, description: "DIF 由负转正：12 日指数均线重新站上 26 日指数均线", usage: "中期由弱转强的标志。比 MACD 金叉出现得晚，但更稳。", params: NONE },
    SignalInfo { id: S::KdjGolden, name: "KDJ 金叉", group: "超买超卖", tone: Bullish, kind: Event, description: "K 值上穿 D 值", usage: "KDJ 很灵敏，适合看几天之内的短线节奏。在低位（K、D 小于 20）出现的金叉更有参考价值，高位的金叉容易很快失效。", params: NONE },
    SignalInfo { id: S::KdjDeath, name: "KDJ 死叉", group: "超买超卖", tone: Bearish, kind: Event, description: "K 值下穿 D 值", usage: "短线转弱。在高位（K、D 大于 80）出现时更值得注意。", params: NONE },
    SignalInfo { id: S::RsiLeaveOversold, name: "RSI 走出超卖（上穿 {level}）", group: "超买超卖", tone: Bullish, kind: Event, description: "RSI(14) 从 {level} 以下回到上方，跌势可能告一段落", usage: "说明连续下跌之后买盘开始回来。强烈的下跌中 RSI 可以在超卖区停很久，走出超卖不等于见底。", params: &[p("level", "超卖线", 30.0, 5.0, 50.0, false)] },
    SignalInfo { id: S::RsiEnterOverbought, name: "RSI 进入超买（上穿 {level}）", group: "超买超卖", tone: Bearish, kind: Event, description: "RSI(14) 上穿 {level}，短期涨得较多，可能回调", usage: "涨得快不等于马上会跌，强势股常常长时间处在超买区。更适合用来提醒「别追高」，而不是当作卖出信号。", params: &[p("level", "超买线", 70.0, 50.0, 95.0, false)] },
    SignalInfo { id: S::BollBreakUpper, name: "突破布林带上轨", group: "突破", tone: Bullish, kind: Event, description: "收盘价从上轨下方升到上方，走势很强", usage: "布林带上下轨之间大约包住了 95% 的正常波动，收盘突破上轨说明涨势超出平常。可能是新一轮上涨的开始，也可能是短期过热。", params: NONE },
    SignalInfo { id: S::BollBreakLower, name: "跌破布林带下轨", group: "突破", tone: Bearish, kind: Event, description: "收盘价从下轨上方跌到下方，走势很弱", usage: "跌势超出平常，可能是恐慌性下跌，也可能是弱势加速。单独出现时不宜当作买入理由。", params: NONE },
    SignalInfo { id: S::NewHigh, name: "创 {n} 日新高", group: "突破", tone: Bullish, kind: Event, description: "收盘价高于之前 {n} 个交易日的所有收盘价", usage: "创新高说明这段时间买入的人都在赚钱，上方没有被套的人等着解套卖出，强势股常见。注意追高的风险。", params: DAYS },
    SignalInfo { id: S::NewLow, name: "创 {n} 日新低", group: "突破", tone: Bearish, kind: Event, description: "收盘价低于之前 {n} 个交易日的所有收盘价", usage: "这段时间买入的人都在亏钱，弱势明显。「跌到新低就便宜」是常见误区。", params: DAYS },
    SignalInfo { id: S::VolumeSurge, name: "放量（{ratio} 倍以上）", group: "量能", tone: Bullish, kind: Event, description: "成交量达到前 5 个交易日平均的 {ratio} 倍以上，说明关注度明显上升", usage: "成交量反映关注度和资金进出。放量上涨说明买的人多，放量下跌说明卖压重，要结合当天的涨跌一起看。", params: &[p("ratio", "倍数", 2.0, 1.2, 10.0, false)] },
    SignalInfo { id: S::BigUp, name: "大阳线（涨 {pct}% 以上）", group: "K 线形态", tone: Bullish, kind: Event, description: "当天涨幅不小于 {pct}%，且收盘高于开盘", usage: "当天买方力量很强。出现在长期下跌之后可能是转折，出现在连续上涨之后可能是加速，也可能是尾声。", params: &[p("pct", "涨幅 %", 5.0, 2.0, 20.0, false)] },
    SignalInfo { id: S::GapUp, name: "向上跳空", group: "K 线形态", tone: Bullish, kind: Event, description: "当天最低价高于前一天最高价，留下缺口", usage: "开盘就比前一天的最高价还高，说明买方很急。缺口如果几天内被回补（价格跌回缺口里），说明上涨的力量不够持续。", params: NONE },
    SignalInfo { id: S::LongLowerShadow, name: "长下影线", group: "K 线形态", tone: Bullish, kind: Event, description: "盘中跌下去又被拉回：下影线不短于实体的 2 倍，且不短于股价的 3%", usage: "盘中跌下去又被买回来，说明下方有人接。出现在低位时更有意义，出现在高位可能是有人借机卖出。", params: NONE },
    SignalInfo { id: S::LimitUp, name: "涨停", group: "涨跌停", tone: Bullish, kind: Event, description: "收盘价达到涨停价", usage: "A 股的单日涨幅上限（主板 10%，创业板、科创板 20%，北交所 30%）。涨停说明短期资金追捧，但第二天走势分化很大；一字涨停的股票普通人往往买不进。", params: NONE },
    SignalInfo { id: S::LimitDown, name: "跌停", group: "涨跌停", tone: Bearish, kind: Event, description: "收盘价达到跌停价", usage: "单日跌幅下限，往往伴随重大利空或恐慌。跌停封死时很难卖出。", params: NONE },
    SignalInfo { id: S::BrokenLimitUp, name: "炸板", group: "涨跌停", tone: Bearish, kind: Event, description: "盘中触及涨停价，收盘没有封住", usage: "冲上涨停却没封住，说明高位有大量卖盘，第二天表现通常偏弱。", params: NONE },
    SignalInfo { id: S::ConsecutiveLimitUp, name: "连续 {n} 个涨停", group: "涨跌停", tone: Bullish, kind: Event, description: "连续 {n} 个交易日收盘涨停", usage: "市场炒作最热的股票，波动和风险都极大，不适合普通投资者追。", params: &[p("n", "连板数", 2.0, 2.0, 10.0, true)] },
];

pub fn info_of(id: SignalId) -> &'static SignalInfo {
    CATALOG.iter().find(|s| s.id == id).expect("每个信号都在目录中")
}

pub type Params = BTreeMap<String, f64>;

/// 参数值：没给的用默认值
pub fn param(id: SignalId, params: &Params, key: &str) -> f64 {
    params.get(key).copied().unwrap_or_else(|| info_of(id).params.iter().find(|p| p.key == key).map_or(0.0, |p| p.default))
}

pub fn validate(id: SignalId, params: &Params) -> Result<(), String> {
    let info = info_of(id);
    for (k, v) in params {
        let Some(pi) = info.params.iter().find(|p| p.key == k) else {
            return Err(format!("信号「{}」没有参数 {k}", label(id, &Params::new())));
        };
        if !(pi.min..=pi.max).contains(v) || (pi.integer && v.fract() != 0.0) {
            return Err(format!("「{}」的{}应在 {} 到 {} 之间{}", label(id, &Params::new()), pi.label, pi.min, pi.max, if pi.integer { "的整数" } else { "" }));
        }
    }
    match id {
        S::MaCrossUp | S::MaCrossDown if param(id, params, "fast") >= param(id, params, "slow") => Err("快线周期要小于慢线周期".into()),
        S::MaBullish | S::MaBearish if !(param(id, params, "short") < param(id, params, "mid") && param(id, params, "mid") < param(id, params, "long")) => {
            Err("均线排列要求短期 < 中期 < 长期".into())
        }
        _ => Ok(()),
    }
}

/// 白话名称，参数代入后的结果，例如「5 日均线上穿 20 日均线」
pub fn label(id: SignalId, params: &Params) -> String {
    let info = info_of(id);
    let mut s = info.name.to_string();
    for pi in info.params {
        let v = param(id, params, pi.key);
        let text = if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") };
        s = s.replace(&format!("{{{}}}", pi.key), &text);
    }
    s
}

/// 计算需要的历史日线根数
pub fn warmup(id: SignalId, params: &Params) -> u32 {
    let v = |k| param(id, params, k) as u32;
    match id {
        S::MaCrossUp | S::MaCrossDown => v("slow") + 2,
        S::MaBullish | S::MaBearish => v("long") + 1,
        S::PriceCrossAboveMa | S::PriceCrossBelowMa => v("n") + 2,
        S::NewHigh | S::NewLow => v("n") + 1,
        S::MacdGolden | S::MacdDeath | S::MacdTurnPositive => 120,
        S::RsiLeaveOversold | S::RsiEnterOverbought => 60,
        S::KdjGolden | S::KdjDeath => 40,
        S::BollBreakUpper | S::BollBreakLower => 22,
        S::ConsecutiveLimitUp => v("n") + 6,
        _ => 6,
    }
}

/// 两条线的上穿（golden = true）或下穿事件，与均线交叉同一套容差规则（见 ma::cross_signals）
fn crossing(a: &[f64], b: &[f64], up: bool) -> Vec<bool> {
    let pairs: Vec<(f64, f64)> = a.iter().zip(b).map(|(x, y)| (*x, *y)).collect();
    // 数据不足的开头部分（NaN）不参与，状态从第一个完整的点开始
    let start = pairs.iter().position(|(x, y)| !x.is_nan() && !y.is_nan()).unwrap_or(pairs.len());
    let mut out = vec![false; start];
    let want = if up { CrossSignal::Golden } else { CrossSignal::Death };
    out.extend(cross_signals(&pairs[start..]).into_iter().map(|s| s == Some(want)));
    out
}

/// 一条线越过一个固定水平
fn crossing_level(a: &[f64], level: f64, up: bool) -> Vec<bool> {
    crossing(a, &vec![level; a.len()], up)
}

/// 信号序列，同一只股票上相同的信号和参数只算一次
pub fn series(f: &Frame, id: SignalId, params: &Params) -> std::rc::Rc<Vec<bool>> {
    f.memo_bools(format!("{id:?}{params:?}"), || compute(f, id, params))
}

// @project-doc docs/domains/strategies.md#signals
/// 信号序列（与日线等长）
pub fn compute(f: &Frame, id: SignalId, params: &Params) -> Vec<bool> {
    let ind = |id: IndicatorId, n: Option<u32>| f.get(IndicatorRef::new(id, n));
    let pv = |k: &str| param(id, params, k);
    let n = f.len();
    let bars = f.bars;
    match id {
        S::MaCrossUp | S::MaCrossDown => {
            let fast = ind(IndicatorId::Ma, Some(pv("fast") as u32));
            let slow = ind(IndicatorId::Ma, Some(pv("slow") as u32));
            crossing(&fast, &slow, id == S::MaCrossUp)
        }
        S::MaBullish | S::MaBearish => {
            let s = ind(IndicatorId::Ma, Some(pv("short") as u32));
            let m = ind(IndicatorId::Ma, Some(pv("mid") as u32));
            let l = ind(IndicatorId::Ma, Some(pv("long") as u32));
            (0..n).map(|i| if id == S::MaBullish { s[i] > m[i] && m[i] > l[i] } else { s[i] < m[i] && m[i] < l[i] }).collect()
        }
        S::PriceCrossAboveMa | S::PriceCrossBelowMa => {
            let c = ind(IndicatorId::Close, None);
            let m = ind(IndicatorId::Ma, Some(pv("n") as u32));
            crossing(&c, &m, id == S::PriceCrossAboveMa)
        }
        S::MacdGolden | S::MacdDeath => crossing(&ind(IndicatorId::MacdDif, None), &ind(IndicatorId::MacdDea, None), id == S::MacdGolden),
        S::MacdTurnPositive => crossing_level(&ind(IndicatorId::MacdDif, None), 0.0, true),
        S::KdjGolden | S::KdjDeath => crossing(&ind(IndicatorId::KdjK, None), &ind(IndicatorId::KdjD, None), id == S::KdjGolden),
        S::RsiLeaveOversold => crossing_level(&ind(IndicatorId::Rsi, Some(14)), pv("level"), true),
        S::RsiEnterOverbought => crossing_level(&ind(IndicatorId::Rsi, Some(14)), pv("level"), true),
        S::BollBreakUpper => crossing(&ind(IndicatorId::Close, None), &ind(IndicatorId::BollUpper, None), true),
        S::BollBreakLower => crossing(&ind(IndicatorId::Close, None), &ind(IndicatorId::BollLower, None), false),
        S::NewHigh | S::NewLow => {
            let c = ind(IndicatorId::Close, None);
            let k = pv("n") as usize;
            (0..n)
                .map(|i| {
                    if i < k {
                        return false;
                    }
                    let prior = &c[i - k..i];
                    if id == S::NewHigh { prior.iter().all(|v| c[i] > *v) } else { prior.iter().all(|v| c[i] < *v) }
                })
                .collect()
        }
        S::VolumeSurge => {
            let r = ind(IndicatorId::VolRatio, Some(5));
            r.iter().map(|v| *v >= pv("ratio")).collect()
        }
        S::BigUp => {
            let ch = ind(IndicatorId::ChangePct, None);
            (0..n).map(|i| ch[i] >= pv("pct") && bars[i].q_close > bars[i].q_open).collect()
        }
        S::GapUp => (0..n).map(|i| i > 0 && bars[i].q_low > bars[i - 1].q_high).collect(),
        S::LongLowerShadow => bars
            .iter()
            .map(|b| {
                let lower = b.q_open.min(b.q_close) - b.q_low;
                let body = (b.q_close - b.q_open).abs();
                lower >= 2.0 * body && lower >= 0.03 * b.q_close
            })
            .collect(),
        S::LimitUp => f.limits().limit_up.clone(),
        S::LimitDown => f.limits().limit_down.clone(),
        S::BrokenLimitUp => f.limits().broken.clone(),
        S::ConsecutiveLimitUp => {
            let lu = &f.limits().limit_up;
            let k = pv("n") as usize;
            let mut run = 0usize;
            lu.iter()
                .map(|&u| {
                    run = if u { run + 1 } else { 0 };
                    run >= k
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::tests::bars_from;

    #[test]
    fn every_catalog_entry_is_explained() {
        for s in CATALOG {
            assert!(!s.description.is_empty() && !s.usage.is_empty(), "信号 {:?} 缺少说明", s.id);
        }
        for i in crate::analysis::indicators::CATALOG {
            assert!(!i.description.is_empty() && !i.usage.is_empty(), "指标 {:?} 缺少说明", i.id);
        }
    }

    #[test]
    fn ma_cross_matches_existing_rule() {
        let closes = [5.0, 4.0, 3.0, 2.0, 3.0, 5.0, 8.0];
        let b = bars_from(&closes);
        let f = Frame::new(&b, None);
        let mut params = Params::new();
        params.insert("fast".into(), 2.0);
        params.insert("slow".into(), 3.0);
        let sig = compute(&f, SignalId::MaCrossUp, &params);
        // 与 ma::ma_cross 的测试相同：第 6 根上穿
        assert_eq!(sig, vec![false, false, false, false, false, true, false]);
        assert_eq!(label(SignalId::MaCrossUp, &params), "2 日均线上穿 3 日均线");
    }

    #[test]
    fn new_high_and_volume_surge() {
        let b = bars_from(&[10.0, 11.0, 10.5, 12.0]);
        let f = Frame::new(&b, None);
        let mut p = Params::new();
        p.insert("n".into(), 3.0);
        assert_eq!(compute(&f, SignalId::NewHigh, &p), vec![false, false, false, true]);
        // 成交量逐日增加 1000：第 4 天 4000 是前 5 天平均（不足 5 天）——不足时不判断
        assert!(compute(&f, SignalId::VolumeSurge, &Params::new()).iter().all(|v| !v));
    }

    #[test]
    fn params_are_validated() {
        let mut p = Params::new();
        p.insert("fast".into(), 30.0);
        p.insert("slow".into(), 20.0);
        assert!(validate(SignalId::MaCrossUp, &p).is_err());
        p.insert("bogus".into(), 1.0);
        assert!(validate(SignalId::MaCrossUp, &p).is_err());
        assert!(validate(SignalId::MacdGolden, &Params::new()).is_ok());
    }
}
