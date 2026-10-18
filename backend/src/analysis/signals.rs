//! 信号：某一天发生的一件事（例如「5 日均线上穿 20 日均线」），或某一天所处的状态（例如「均线多头排列」）。
//! 每个信号在一只股票的日线上算出与日线等长的布尔序列。目录和口径见 docs/domains/strategies.md#signals。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::indicators::{Frame, IndicatorId, IndicatorRef};
use super::ma::{CrossSignal, cross_signals};
use super::strategy::{Condition, Op, SignalDef, SignalLib, cmp, cmp_ind};

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
    MacdTurnNegative,
    KdjGolden,
    KdjDeath,
    KdjLowGolden,
    KdjHighDeath,
    RsiLeaveOversold,
    RsiEnterOverbought,
    RsiLeaveOverbought,
    RsiEnterOversold,
    CciBreakUp,
    CciBreakDown,
    DiCrossUp,
    DiCrossDown,
    AdxStrong,
    SupertrendUp,
    SupertrendDown,
    MacdBottomDivergence,
    MacdTopDivergence,
    RsiBottomDivergence,
    RsiTopDivergence,
    BollBreakUpper,
    BollBreakLower,
    NewHigh,
    NewLow,
    MaSqueezeBreakUp,
    MaSqueezeBreakDown,
    VolumeSurge,
    VolumeShrink,
    VolumePriceUp,
    NewHighLowVolume,
    BigUp,
    BigDown,
    UpStreak,
    DownStreak,
    GapUp,
    GapDown,
    LongLowerShadow,
    LongUpperShadow,
    Doji,
    BullishEngulfing,
    BearishEngulfing,
    MorningStar,
    EveningStar,
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
    /// 本身不说明方向，例如十字星（犹豫）、趋势明确（不管涨跌）
    Neutral,
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
const STREAK: &[ParamInfo] = &[p("n", "天数", 3.0, 2.0, 10.0, true)];
const NONE: &[ParamInfo] = &[];

use SignalId as S;
use SignalKind::{Event, State};
use Tone::{Bearish, Bullish, Neutral};

pub const CATALOG: &[SignalInfo] = &[
    SignalInfo { id: S::MaCrossUp, name: "{fast} 日均线上穿 {slow} 日均线", group: "均线", tone: Bullish, kind: Event, description: "金叉：短期均线从下往上穿过长期均线，常被看作走势转强", usage: "最常用的转强信号。周期越短越灵敏，假信号也越多；股价横盘时金叉死叉会来回出现，最好结合成交量或更长的均线一起看。", params: MA_PAIR },
    SignalInfo { id: S::MaCrossDown, name: "{fast} 日均线下穿 {slow} 日均线", group: "均线", tone: Bearish, kind: Event, description: "死叉：短期均线从上往下穿过长期均线，常被看作走势转弱", usage: "常用的转弱信号，很多人把它当作卖出提示。和金叉一样，横盘时容易反复出现。", params: MA_PAIR },
    SignalInfo { id: S::MaBullish, name: "均线多头排列（{short} > {mid} > {long}）", group: "均线", tone: Bullish, kind: State, description: "短期均线在上、长期均线在下，处在上升趋势中", usage: "短、中、长期走势一致向上，是趋势最清楚的状态之一。出现时往往已经涨了一段，适合顺着趋势做，而不是抄底。", params: MA_TRIPLE },
    SignalInfo { id: S::MaBearish, name: "均线空头排列（{short} < {mid} < {long}）", group: "均线", tone: Bearish, kind: State, description: "短期均线在下、长期均线在上，处在下降趋势中", usage: "下降趋势最清楚的状态。处在空头排列的股票，反弹常常到均线附近就受阻。", params: MA_TRIPLE },
    SignalInfo { id: S::PriceCrossAboveMa, name: "收盘站上 {n} 日均线", group: "均线", tone: Bullish, kind: Event, description: "前一天收在均线下方，当天收在上方", usage: "比均线交叉出现得早，但也更容易是假突破，之后几天能不能站稳更重要。20 日均线常被看作中短期强弱的分界线。", params: MA_ONE },
    SignalInfo { id: S::PriceCrossBelowMa, name: "收盘跌破 {n} 日均线", group: "均线", tone: Bearish, kind: Event, description: "前一天收在均线上方，当天收在下方", usage: "常用的止损或减仓提示，策略模板默认的卖出规则就是跌破 20 日均线。", params: MA_ONE },
    SignalInfo { id: S::MaSqueezeBreakUp, name: "均线粘合后向上发散", group: "均线", tone: Bullish, kind: Event, description: "5、10、20、60 日均线拧在一起横了一段时间后，当天明显散开，收盘在四条均线上方", usage: "均线粘合说明长期没有方向、多空力量接近；散开往往是变盘的开始，向上散开说明选择了向上。之后能否持续，看后面几天能不能站稳。", params: NONE },
    SignalInfo { id: S::MaSqueezeBreakDown, name: "均线粘合后向下发散", group: "均线", tone: Bearish, kind: Event, description: "5、10、20、60 日均线拧在一起横了一段时间后，当天明显散开，收盘在四条均线下方", usage: "横盘之后选择了向下，常常是一段下跌的开始。", params: NONE },
    SignalInfo { id: S::MacdGolden, name: "MACD 金叉", group: "MACD", tone: Bullish, kind: Event, description: "MACD 快线（DIF）上穿慢线（DEA）", usage: "MACD 是两条指数均线的差，反应比普通均线快。出现在零轴上方（DIF 大于 0）的金叉比零轴下方的更强。", params: NONE },
    SignalInfo { id: S::MacdDeath, name: "MACD 死叉", group: "MACD", tone: Bearish, kind: Event, description: "MACD 快线（DIF）下穿慢线（DEA）", usage: "动能转弱。零轴上方的死叉常常只是上涨中的回调，零轴下方的死叉说明弱势还在延续。", params: NONE },
    SignalInfo { id: S::MacdTurnPositive, name: "MACD 上穿零轴", group: "MACD", tone: Bullish, kind: Event, description: "DIF 由负转正：12 日指数均线重新站上 26 日指数均线", usage: "中期由弱转强的标志。比 MACD 金叉出现得晚，但更稳。", params: NONE },
    SignalInfo { id: S::MacdTurnNegative, name: "MACD 下穿零轴", group: "MACD", tone: Bearish, kind: Event, description: "DIF 由正转负：12 日指数均线跌回 26 日指数均线下方", usage: "中期由强转弱的标志，比 MACD 死叉出现得晚，但更稳。", params: NONE },
    SignalInfo { id: S::KdjGolden, name: "KDJ 金叉", group: "超买超卖", tone: Bullish, kind: Event, description: "K 值上穿 D 值", usage: "KDJ 很灵敏，适合看几天之内的短线节奏。在低位（K、D 小于 20）出现的金叉更有参考价值，高位的金叉容易很快失效。", params: NONE },
    SignalInfo { id: S::KdjDeath, name: "KDJ 死叉", group: "超买超卖", tone: Bearish, kind: Event, description: "K 值下穿 D 值", usage: "短线转弱。在高位（K、D 大于 80）出现时更值得注意。", params: NONE },
    SignalInfo { id: S::KdjLowGolden, name: "KDJ 低位金叉（{level} 以下）", group: "超买超卖", tone: Bullish, kind: Event, description: "K 值上穿 D 值，且 D 值低于 {level}", usage: "只看低位的 KDJ 金叉：跌到超卖区后短线开始转强，比普通的 KDJ 金叉少很多假信号。", params: &[p("level", "低位线", 20.0, 5.0, 50.0, false)] },
    SignalInfo { id: S::KdjHighDeath, name: "KDJ 高位死叉（{level} 以上）", group: "超买超卖", tone: Bearish, kind: Event, description: "K 值下穿 D 值，且 D 值高于 {level}", usage: "只看高位的 KDJ 死叉：涨到超买区后短线开始转弱。", params: &[p("level", "高位线", 80.0, 50.0, 95.0, false)] },
    SignalInfo { id: S::RsiLeaveOversold, name: "RSI 走出超卖（上穿 {level}）", group: "超买超卖", tone: Bullish, kind: Event, description: "RSI(14) 从 {level} 以下回到上方，跌势可能告一段落", usage: "说明连续下跌之后买盘开始回来。强烈的下跌中 RSI 可以在超卖区停很久，走出超卖不等于见底。", params: &[p("level", "超卖线", 30.0, 5.0, 50.0, false)] },
    SignalInfo { id: S::RsiEnterOverbought, name: "RSI 进入超买（上穿 {level}）", group: "超买超卖", tone: Bearish, kind: Event, description: "RSI(14) 上穿 {level}，短期涨得较多，可能回调", usage: "涨得快不等于马上会跌，强势股常常长时间处在超买区。更适合用来提醒「别追高」，而不是当作卖出信号。", params: &[p("level", "超买线", 70.0, 50.0, 95.0, false)] },
    SignalInfo { id: S::RsiLeaveOverbought, name: "RSI 超买回落（下穿 {level}）", group: "超买超卖", tone: Bearish, kind: Event, description: "RSI(14) 从 {level} 以上跌回下方，短期涨势可能告一段落", usage: "比「进入超买」更适合当作卖出参考：涨势真正开始减弱的时候才出现。", params: &[p("level", "超买线", 70.0, 50.0, 95.0, false)] },
    SignalInfo { id: S::RsiEnterOversold, name: "RSI 跌入超卖（下穿 {level}）", group: "超买超卖", tone: Bullish, kind: Event, description: "RSI(14) 下穿 {level}，短期跌得较多，可能反弹", usage: "跌得多不等于马上会涨，弱势股可以长时间处在超卖区。更适合用来提醒「别杀跌」，等它走出超卖再看。", params: &[p("level", "超卖线", 30.0, 5.0, 50.0, false)] },
    SignalInfo { id: S::CciBreakUp, name: "CCI 上穿 100", group: "超买超卖", tone: Bullish, kind: Event, description: "CCI(14) 从 100 以下升到上方，价格明显强于近期平均", usage: "常被看作进入强势、可能有一段行情的开始。和 RSI 不同，CCI 不封顶，强势时可以一直在 100 以上。", params: NONE },
    SignalInfo { id: S::CciBreakDown, name: "CCI 下穿 −100", group: "超买超卖", tone: Bearish, kind: Event, description: "CCI(14) 从 −100 以上跌到下方，价格明显弱于近期平均", usage: "进入弱势。", params: NONE },
    SignalInfo { id: S::DiCrossUp, name: "+DI 上穿 −DI", group: "趋势强度", tone: Bullish, kind: Event, description: "DMI 的上升力量（+DI）超过下降力量（−DI），多方开始占优", usage: "DMI 的买入信号。ADX 同时较高（例如 20 以上）时更可靠；ADX 很低说明在盘整，交叉容易来回出现。", params: NONE },
    SignalInfo { id: S::DiCrossDown, name: "−DI 上穿 +DI", group: "趋势强度", tone: Bearish, kind: Event, description: "DMI 的下降力量（−DI）超过上升力量（+DI），空方开始占优", usage: "DMI 的卖出信号。", params: NONE },
    SignalInfo { id: S::AdxStrong, name: "趋势明确（ADX 高于 {level}）", group: "趋势强度", tone: Neutral, kind: State, description: "ADX(14) 高于 {level}：正在走一段明确的趋势，可能是涨也可能是跌", usage: "本身不说明方向，常用来过滤：均线金叉、突破这类趋势信号只在趋势明确时才用，盘整时不用。方向看 +DI 是否在 −DI 上方。", params: &[p("level", "强度线", 25.0, 10.0, 60.0, false)] },
    SignalInfo { id: S::SupertrendUp, name: "Supertrend 翻多", group: "趋势强度", tone: Bullish, kind: Event, description: "收盘价上穿 Supertrend 线（10 日 ATR、3 倍），线从股价上方翻到下方", usage: "趋势由跌转涨的信号，TradingView 上最常用的趋势指标之一。翻多后可以把这条线当作跟随的止损线。", params: NONE },
    SignalInfo { id: S::SupertrendDown, name: "Supertrend 翻空", group: "趋势强度", tone: Bearish, kind: Event, description: "收盘价下穿 Supertrend 线，线从股价下方翻到上方", usage: "趋势由涨转跌，常用作趋势跟随策略的卖出条件。", params: NONE },
    SignalInfo { id: S::MacdBottomDivergence, name: "MACD 底背离", group: "背离", tone: Bullish, kind: Event, description: "股价的低点一个比一个低，但 MACD 快线（DIF）的低点却抬高了，且在零轴下方", usage: "下跌的力量在减弱，最经典的抄底信号之一。要等低点确认（之后 3 天没有更低）才出现，所以比低点晚 3 天。背离之后还可能再背离，最好等价格站稳再动手。", params: NONE },
    SignalInfo { id: S::MacdTopDivergence, name: "MACD 顶背离", group: "背离", tone: Bearish, kind: Event, description: "股价的高点一个比一个高，但 MACD 快线（DIF）的高点却降低了，且在零轴上方", usage: "上涨的力量在减弱，常用的逃顶信号。比高点晚 3 天出现。", params: NONE },
    SignalInfo { id: S::RsiBottomDivergence, name: "RSI 底背离", group: "背离", tone: Bullish, kind: Event, description: "股价的低点一个比一个低，但 RSI(14) 的低点却抬高了", usage: "和 MACD 底背离一样说明跌势在减弱，常常比 MACD 底背离出现得早。比低点晚 3 天出现。", params: NONE },
    SignalInfo { id: S::RsiTopDivergence, name: "RSI 顶背离", group: "背离", tone: Bearish, kind: Event, description: "股价的高点一个比一个高，但 RSI(14) 的高点却降低了", usage: "涨势在减弱。强势股的 RSI 顶背离可能连续出现好几次，单独用来卖出容易卖早。", params: NONE },
    SignalInfo { id: S::BollBreakUpper, name: "突破布林带上轨", group: "突破", tone: Bullish, kind: Event, description: "收盘价从上轨下方升到上方，走势很强", usage: "布林带上下轨之间大约包住了 95% 的正常波动，收盘突破上轨说明涨势超出平常。可能是新一轮上涨的开始，也可能是短期过热。", params: NONE },
    SignalInfo { id: S::BollBreakLower, name: "跌破布林带下轨", group: "突破", tone: Bearish, kind: Event, description: "收盘价从下轨上方跌到下方，走势很弱", usage: "跌势超出平常，可能是恐慌性下跌，也可能是弱势加速。单独出现时不宜当作买入理由。", params: NONE },
    SignalInfo { id: S::NewHigh, name: "创 {n} 日新高", group: "突破", tone: Bullish, kind: Event, description: "收盘价高于之前 {n} 个交易日的所有收盘价", usage: "创新高说明这段时间买入的人都在赚钱，上方没有被套的人等着解套卖出，强势股常见。注意追高的风险。", params: DAYS },
    SignalInfo { id: S::NewLow, name: "创 {n} 日新低", group: "突破", tone: Bearish, kind: Event, description: "收盘价低于之前 {n} 个交易日的所有收盘价", usage: "这段时间买入的人都在亏钱，弱势明显。「跌到新低就便宜」是常见误区。", params: DAYS },
    SignalInfo { id: S::VolumeSurge, name: "放量（{ratio} 倍以上）", group: "量能", tone: Bullish, kind: Event, description: "成交量达到前 5 个交易日平均的 {ratio} 倍以上，说明关注度明显上升", usage: "成交量反映关注度和资金进出。放量上涨说明买的人多，放量下跌说明卖压重，要结合当天的涨跌一起看。", params: &[p("ratio", "倍数", 2.0, 1.2, 10.0, false)] },
    SignalInfo { id: S::VolumeShrink, name: "缩量（{ratio} 倍以下）", group: "量能", tone: Neutral, kind: Event, description: "成交量不到前 5 个交易日平均的 {ratio} 倍，交易明显清淡", usage: "本身不说明方向：上涨中缩量回调说明卖的人不多（惜售），下跌中缩量说明卖盘在枯竭。要结合当天涨跌和所处位置看。", params: &[p("ratio", "倍数", 0.5, 0.1, 0.9, false)] },
    SignalInfo { id: S::VolumePriceUp, name: "量价齐升", group: "量能", tone: Bullish, kind: Event, description: "当天涨 {pct}% 以上，同时成交量达到前 5 天平均的 {ratio} 倍以上", usage: "涨的同时买的人明显变多，比单独的放量或上涨更有说服力。", params: &[p("pct", "涨幅 %", 2.0, 0.5, 10.0, false), p("ratio", "倍数", 2.0, 1.2, 10.0, false)] },
    SignalInfo { id: S::NewHighLowVolume, name: "新高缩量（量价背离）", group: "量能", tone: Bearish, kind: Event, description: "收盘创 {n} 日新高，但成交量低于前 20 天的平均", usage: "价格创新高，跟进买入的人却变少了，上涨的力量可能在减弱。", params: DAYS },
    SignalInfo { id: S::BigUp, name: "大阳线（涨 {pct}% 以上）", group: "K 线形态", tone: Bullish, kind: Event, description: "当天涨幅不小于 {pct}%，且收盘高于开盘", usage: "当天买方力量很强。出现在长期下跌之后可能是转折，出现在连续上涨之后可能是加速，也可能是尾声。", params: &[p("pct", "涨幅 %", 5.0, 2.0, 20.0, false)] },
    SignalInfo { id: S::BigDown, name: "大阴线（跌 {pct}% 以上）", group: "K 线形态", tone: Bearish, kind: Event, description: "当天跌幅不小于 {pct}%，且收盘低于开盘", usage: "当天卖方力量很强，常见于恐慌性下跌或有人大量卖出。出现在长期上涨之后要警惕。", params: &[p("pct", "跌幅 %", 5.0, 2.0, 20.0, false)] },
    SignalInfo { id: S::UpStreak, name: "连续 {n} 天上涨", group: "K 线形态", tone: Bullish, kind: State, description: "连续 {n} 个交易日收盘都比前一天高", usage: "短期持续走强。连涨天数越多，短期回调的可能也越大。", params: STREAK },
    SignalInfo { id: S::DownStreak, name: "连续 {n} 天下跌", group: "K 线形态", tone: Bearish, kind: State, description: "连续 {n} 个交易日收盘都比前一天低", usage: "短期持续走弱。连跌之后可能有反弹，但不要轻易当作抄底理由。", params: STREAK },
    SignalInfo { id: S::GapUp, name: "向上跳空", group: "K 线形态", tone: Bullish, kind: Event, description: "当天最低价高于前一天最高价，留下缺口", usage: "开盘就比前一天的最高价还高，说明买方很急。缺口如果几天内被回补（价格跌回缺口里），说明上涨的力量不够持续。", params: NONE },
    SignalInfo { id: S::GapDown, name: "向下跳空", group: "K 线形态", tone: Bearish, kind: Event, description: "当天最高价低于前一天最低价，留下缺口", usage: "开盘就比前一天的最低价还低，往往伴随利空或恐慌。缺口长时间不回补，说明弱势在延续。", params: NONE },
    SignalInfo { id: S::LongLowerShadow, name: "长下影线", group: "K 线形态", tone: Bullish, kind: Event, description: "盘中跌下去又被拉回：下影线不短于实体的 2 倍，且不短于股价的 3%", usage: "盘中跌下去又被买回来，说明下方有人接。出现在低位时更有意义，出现在高位可能是有人借机卖出。", params: NONE },
    SignalInfo { id: S::LongUpperShadow, name: "长上影线", group: "K 线形态", tone: Bearish, kind: Event, description: "盘中涨上去又被打回：上影线不短于实体的 2 倍，且不短于股价的 3%", usage: "盘中冲高后被卖回来，说明上方卖压重。出现在高位时更要注意。", params: NONE },
    SignalInfo { id: S::Doji, name: "十字星", group: "K 线形态", tone: Neutral, kind: Event, description: "收盘价和开盘价几乎一样（实体不到当天波动的 10%），上下都有影线", usage: "买卖双方打成平手，犹豫不决。出现在一段明显的上涨或下跌之后，常预示可能转向；在横盘中出现意义不大。", params: NONE },
    SignalInfo { id: S::BullishEngulfing, name: "看涨吞没", group: "K 线形态", tone: Bullish, kind: Event, description: "前一天收阴，当天收阳，且当天的实体完全包住前一天的实体", usage: "两根 K 线的反转形态：一天之内把前一天的跌幅全部收回。出现在下跌之后更有意义。", params: NONE },
    SignalInfo { id: S::BearishEngulfing, name: "看跌吞没", group: "K 线形态", tone: Bearish, kind: Event, description: "前一天收阳，当天收阴，且当天的实体完全包住前一天的实体", usage: "两根 K 线的反转形态：一天之内把前一天的涨幅全部吐回。出现在上涨之后要警惕。", params: NONE },
    SignalInfo { id: S::MorningStar, name: "早晨之星", group: "K 线形态", tone: Bullish, kind: Event, description: "三根 K 线：一根较大的阴线，一根实体很小的 K 线落在它下方，再一根阳线收复第一根阴线实体的一半以上", usage: "经典的底部反转形态：跌、犹豫、涨。出现在一段下跌之后更可靠。", params: NONE },
    SignalInfo { id: S::EveningStar, name: "黄昏之星", group: "K 线形态", tone: Bearish, kind: Event, description: "三根 K 线：一根较大的阳线，一根实体很小的 K 线落在它上方，再一根阴线跌掉第一根阳线实体的一半以上", usage: "经典的顶部反转形态：涨、犹豫、跌。出现在一段上涨之后更可靠。", params: NONE },
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
        S::MacdGolden | S::MacdDeath | S::MacdTurnPositive | S::MacdTurnNegative => 120,
        S::RsiLeaveOversold | S::RsiEnterOverbought | S::RsiLeaveOverbought | S::RsiEnterOversold => 60,
        S::KdjGolden | S::KdjDeath | S::KdjLowGolden | S::KdjHighDeath => 40,
        S::BollBreakUpper | S::BollBreakLower => 22,
        S::CciBreakUp | S::CciBreakDown => 20,
        S::DiCrossUp | S::DiCrossDown | S::AdxStrong => 30,
        S::SupertrendUp | S::SupertrendDown => 110,
        // 指标本身的预热，加上两个低点最远相隔的天数和确认低点的天数
        S::MacdBottomDivergence | S::MacdTopDivergence => 120 + DIVERGENCE_MAX_GAP as u32 + PIVOT_SIDE as u32 * 2,
        S::RsiBottomDivergence | S::RsiTopDivergence => 60 + DIVERGENCE_MAX_GAP as u32 + PIVOT_SIDE as u32 * 2,
        S::MaSqueezeBreakUp | S::MaSqueezeBreakDown => 62 + SQUEEZE_LOOKBACK as u32,
        S::NewHighLowVolume => v("n").max(20) + 1,
        S::UpStreak | S::DownStreak => v("n") + 2,
        S::ConsecutiveLimitUp => v("n") + 6,
        _ => 6,
    }
}

/// 两条线的上穿（golden = true）或下穿事件，与均线交叉同一套容差规则（见 ma::cross_signals）
pub fn crossing(a: &[f64], b: &[f64], up: bool) -> Vec<bool> {
    let pairs: Vec<(f64, f64)> = a.iter().zip(b).map(|(x, y)| (*x, *y)).collect();
    // 数据不足的开头部分（NaN）不参与，状态从第一个完整的点开始
    let start = pairs.iter().position(|(x, y)| !x.is_nan() && !y.is_nan()).unwrap_or(pairs.len());
    let mut out = vec![false; start];
    let want = if up { CrossSignal::Golden } else { CrossSignal::Death };
    out.extend(cross_signals(&pairs[start..]).into_iter().map(|s| s == Some(want)));
    out
}

/// 信号序列，同一只股票上相同的信号和参数只算一次
pub fn series(f: &Frame, id: SignalId, params: &Params) -> std::rc::Rc<Vec<bool>> {
    f.memo_bools(format!("{id:?}{params:?}"), || match definition(id, params) {
        Some(d) => d.series(f, &SignalLib::default()),
        None => special(f, id, params),
    })
}

/// 能写成条件的内置信号，它的条件定义（参数代入后）；逻辑特殊、写不成条件的返回 None
// @project-doc docs/domains/strategies.md#signals
pub fn definition(id: SignalId, params: &Params) -> Option<SignalDef> {
    use IndicatorId as I;
    use Op::*;
    let pv = |k: &str| param(id, params, k);
    let n = |k: &str| Some(pv(k) as u32);
    let all = |conditions: Vec<Condition>| Some(SignalDef { match_all: true, conditions });
    match id {
        S::MaCrossUp => all(vec![cmp_ind(I::Ma, n("fast"), CrossesAbove, I::Ma, n("slow"))]),
        S::MaCrossDown => all(vec![cmp_ind(I::Ma, n("fast"), CrossesBelow, I::Ma, n("slow"))]),
        S::MaBullish => all(vec![cmp_ind(I::Ma, n("short"), Gt, I::Ma, n("mid")), cmp_ind(I::Ma, n("mid"), Gt, I::Ma, n("long"))]),
        S::MaBearish => all(vec![cmp_ind(I::Ma, n("short"), Lt, I::Ma, n("mid")), cmp_ind(I::Ma, n("mid"), Lt, I::Ma, n("long"))]),
        S::PriceCrossAboveMa => all(vec![cmp_ind(I::Close, None, CrossesAbove, I::Ma, n("n"))]),
        S::PriceCrossBelowMa => all(vec![cmp_ind(I::Close, None, CrossesBelow, I::Ma, n("n"))]),
        S::MacdGolden => all(vec![cmp_ind(I::MacdDif, None, CrossesAbove, I::MacdDea, None)]),
        S::MacdDeath => all(vec![cmp_ind(I::MacdDif, None, CrossesBelow, I::MacdDea, None)]),
        S::MacdTurnPositive => all(vec![cmp(I::MacdDif, None, CrossesAbove, 0.0)]),
        S::MacdTurnNegative => all(vec![cmp(I::MacdDif, None, CrossesBelow, 0.0)]),
        S::KdjGolden => all(vec![cmp_ind(I::KdjK, None, CrossesAbove, I::KdjD, None)]),
        S::KdjDeath => all(vec![cmp_ind(I::KdjK, None, CrossesBelow, I::KdjD, None)]),
        S::KdjLowGolden => all(vec![cmp_ind(I::KdjK, None, CrossesAbove, I::KdjD, None), cmp(I::KdjD, None, Lt, pv("level"))]),
        S::KdjHighDeath => all(vec![cmp_ind(I::KdjK, None, CrossesBelow, I::KdjD, None), cmp(I::KdjD, None, Gt, pv("level"))]),
        S::RsiLeaveOversold | S::RsiEnterOverbought => all(vec![cmp(I::Rsi, Some(14), CrossesAbove, pv("level"))]),
        S::RsiLeaveOverbought | S::RsiEnterOversold => all(vec![cmp(I::Rsi, Some(14), CrossesBelow, pv("level"))]),
        S::CciBreakUp => all(vec![cmp(I::Cci, Some(14), CrossesAbove, 100.0)]),
        S::CciBreakDown => all(vec![cmp(I::Cci, Some(14), CrossesBelow, -100.0)]),
        S::DiCrossUp => all(vec![cmp_ind(I::PlusDi, Some(14), CrossesAbove, I::MinusDi, Some(14))]),
        S::DiCrossDown => all(vec![cmp_ind(I::PlusDi, Some(14), CrossesBelow, I::MinusDi, Some(14))]),
        S::AdxStrong => all(vec![cmp(I::Adx, Some(14), Gt, pv("level"))]),
        S::SupertrendUp => all(vec![cmp_ind(I::Close, None, CrossesAbove, I::Supertrend, Some(10))]),
        S::SupertrendDown => all(vec![cmp_ind(I::Close, None, CrossesBelow, I::Supertrend, Some(10))]),
        S::BollBreakUpper => all(vec![cmp_ind(I::Close, None, CrossesAbove, I::BollUpper, None)]),
        S::BollBreakLower => all(vec![cmp_ind(I::Close, None, CrossesBelow, I::BollLower, None)]),
        S::NewHigh => all(vec![cmp_ind(I::Close, None, Gt, I::PrevHighClose, n("n"))]),
        S::NewLow => all(vec![cmp_ind(I::Close, None, Lt, I::PrevLowClose, n("n"))]),
        S::VolumeSurge => all(vec![cmp(I::VolRatio, Some(5), Ge, pv("ratio"))]),
        S::VolumeShrink => all(vec![cmp(I::VolRatio, Some(5), Le, pv("ratio"))]),
        S::VolumePriceUp => all(vec![cmp(I::ChangePct, None, Ge, pv("pct")), cmp(I::VolRatio, Some(5), Ge, pv("ratio"))]),
        S::NewHighLowVolume => all(vec![cmp_ind(I::Close, None, Gt, I::PrevHighClose, n("n")), cmp(I::VolRatio, Some(20), Lt, 1.0)]),
        S::BigUp => all(vec![cmp(I::ChangePct, None, Ge, pv("pct")), cmp_ind(I::Close, None, Gt, I::Open, None)]),
        S::BigDown => all(vec![cmp(I::ChangePct, None, Le, -pv("pct")), cmp_ind(I::Close, None, Lt, I::Open, None)]),
        S::UpStreak => all(vec![streak(cmp(I::ChangePct, None, Gt, 0.0), pv("n") as u32)]),
        S::DownStreak => all(vec![streak(cmp(I::ChangePct, None, Lt, 0.0), pv("n") as u32)]),
        S::GapUp
        | S::GapDown
        | S::LongLowerShadow
        | S::LongUpperShadow
        | S::Doji
        | S::BullishEngulfing
        | S::BearishEngulfing
        | S::MorningStar
        | S::EveningStar
        | S::MacdBottomDivergence
        | S::MacdTopDivergence
        | S::RsiBottomDivergence
        | S::RsiTopDivergence
        | S::MaSqueezeBreakUp
        | S::MaSqueezeBreakDown
        | S::LimitUp
        | S::LimitDown
        | S::BrokenLimitUp
        | S::ConsecutiveLimitUp => None,
    }
}

/// 条件改成「连续 n 天都满足」
fn streak(c: Condition, n: u32) -> Condition {
    match c {
        Condition::Compare { left, op, right, .. } => Condition::Compare { left, op, right, within_days: 1, streak_days: n },
        other => other,
    }
}

/// 判断高低点：两侧各这么多根日线都不比它更极端，才算一个高点或低点。要等右侧的日线走完才能确认，所以背离信号比低点晚这么多天
pub const PIVOT_SIDE: usize = 3;
/// 背离的两个高点（或低点）最远相隔多少根日线
pub const DIVERGENCE_MAX_GAP: usize = 60;
/// 均线粘合：四条均线最高最低相差不超过收盘价的这个百分比；发散：超过这个百分比
const SQUEEZE_PCT: f64 = 2.0;
const SPREAD_PCT: f64 = 3.0;
/// 发散当天之前看多少根日线，其中至少 SQUEEZE_MIN_DAYS 根是粘合的
const SQUEEZE_LOOKBACK: usize = 10;
const SQUEEZE_MIN_DAYS: usize = 8;

// @project-doc docs/domains/strategies.md#special_signals
/// 写不成条件的内置信号：依赖前一天的 K 线、K 线形状、高低点，或推算出的涨跌停价
fn special(f: &Frame, id: SignalId, params: &Params) -> Vec<bool> {
    use IndicatorId as I;
    let pv = |k: &str| param(id, params, k);
    let n = f.len();
    let bars = f.bars;
    let body = |i: usize| (bars[i].q_close - bars[i].q_open).abs();
    let up = |i: usize| bars[i].q_close > bars[i].q_open;
    let down = |i: usize| bars[i].q_close < bars[i].q_open;
    match id {
        S::GapUp => (0..n).map(|i| i > 0 && bars[i].q_low > bars[i - 1].q_high).collect(),
        S::GapDown => (0..n).map(|i| i > 0 && bars[i].q_high < bars[i - 1].q_low).collect(),
        S::LongLowerShadow => bars
            .iter()
            .map(|b| {
                let lower = b.q_open.min(b.q_close) - b.q_low;
                let body = (b.q_close - b.q_open).abs();
                lower >= 2.0 * body && lower >= 0.03 * b.q_close
            })
            .collect(),
        S::LongUpperShadow => bars
            .iter()
            .map(|b| {
                let upper = b.q_high - b.q_open.max(b.q_close);
                let body = (b.q_close - b.q_open).abs();
                upper >= 2.0 * body && upper >= 0.03 * b.q_close
            })
            .collect(),
        S::Doji => bars
            .iter()
            .map(|b| {
                let range = b.q_high - b.q_low;
                range > 0.0 && (b.q_close - b.q_open).abs() <= 0.1 * range && b.q_high > b.q_open.max(b.q_close) && b.q_low < b.q_open.min(b.q_close)
            })
            .collect(),
        // 实体包住前一天的实体：开盘不高于前一天收盘、收盘不低于前一天开盘，并且实体更长
        S::BullishEngulfing => (0..n)
            .map(|i| i > 0 && down(i - 1) && up(i) && bars[i].q_open <= bars[i - 1].q_close && bars[i].q_close >= bars[i - 1].q_open && body(i) > body(i - 1))
            .collect(),
        S::BearishEngulfing => (0..n)
            .map(|i| i > 0 && up(i - 1) && down(i) && bars[i].q_open >= bars[i - 1].q_close && bars[i].q_close <= bars[i - 1].q_open && body(i) > body(i - 1))
            .collect(),
        // 第一根实体不小于股价的 2%；第二根实体不超过第一根的 1/3，且整个实体在第一根收盘价的外侧；第三根收复第一根实体的一半以上
        S::MorningStar => (0..n)
            .map(|i| {
                i >= 2 && {
                    let (a, b, c) = (i - 2, i - 1, i);
                    down(a)
                        && body(a) >= 0.02 * bars[a].q_close
                        && body(b) <= body(a) / 3.0
                        && bars[b].q_open.max(bars[b].q_close) <= bars[a].q_close
                        && up(c)
                        && bars[c].q_close >= (bars[a].q_open + bars[a].q_close) / 2.0
                }
            })
            .collect(),
        S::EveningStar => (0..n)
            .map(|i| {
                i >= 2 && {
                    let (a, b, c) = (i - 2, i - 1, i);
                    up(a)
                        && body(a) >= 0.02 * bars[a].q_close
                        && body(b) <= body(a) / 3.0
                        && bars[b].q_open.min(bars[b].q_close) >= bars[a].q_close
                        && down(c)
                        && bars[c].q_close <= (bars[a].q_open + bars[a].q_close) / 2.0
                }
            })
            .collect(),
        S::MacdBottomDivergence | S::MacdTopDivergence | S::RsiBottomDivergence | S::RsiTopDivergence => {
            let ind = match id {
                S::MacdBottomDivergence | S::MacdTopDivergence => f.get(IndicatorRef::new(I::MacdDif, None)),
                _ => f.get(IndicatorRef::new(I::Rsi, Some(14))),
            };
            let bottom = matches!(id, S::MacdBottomDivergence | S::RsiBottomDivergence);
            let macd = matches!(id, S::MacdBottomDivergence | S::MacdTopDivergence);
            divergence(&f.get(IndicatorRef::new(I::Close, None)), &ind, bottom, macd)
        }
        S::MaSqueezeBreakUp | S::MaSqueezeBreakDown => {
            let spread = f.get(IndicatorRef::new(I::MaSpread, None));
            let mas: Vec<_> = [5, 10, 20, 60].iter().map(|k| f.get(IndicatorRef::new(I::Ma, Some(*k)))).collect();
            let want_up = id == S::MaSqueezeBreakUp;
            (0..n)
                .map(|i| {
                    if i < SQUEEZE_LOOKBACK || !(spread[i] > SPREAD_PCT && spread[i - 1] <= SPREAD_PCT) {
                        return false;
                    }
                    let squeezed = spread[i - SQUEEZE_LOOKBACK..i].iter().filter(|v| **v <= SQUEEZE_PCT).count() >= SQUEEZE_MIN_DAYS;
                    let c = bars[i].q_close;
                    squeezed && if want_up { mas.iter().all(|m| c > m[i]) } else { mas.iter().all(|m| c < m[i]) }
                })
                .collect()
        }
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
        _ => unreachable!("能写成条件的信号走 definition"),
    }
}

/// 写不成条件的内置信号，页面上说明它是怎么判断的
pub fn special_rule(id: SignalId) -> Option<&'static str> {
    Some(match id {
        S::GapUp => "当天最低价高于前一天最高价（前复权）",
        S::GapDown => "当天最高价低于前一天最低价（前复权）",
        S::LongLowerShadow => "下影线（开盘、收盘中较低的一个减最低价）不短于实体的 2 倍，且不短于收盘价的 3%",
        S::LongUpperShadow => "上影线（最高价减开盘、收盘中较高的一个）不短于实体的 2 倍，且不短于收盘价的 3%",
        S::Doji => "实体（收盘与开盘之差）不超过当天最高与最低之差的 10%，并且上下都有影线",
        S::BullishEngulfing => "前一天收阴、当天收阳；当天开盘不高于前一天收盘，收盘不低于前一天开盘，且实体比前一天长",
        S::BearishEngulfing => "前一天收阳、当天收阴；当天开盘不低于前一天收盘，收盘不高于前一天开盘，且实体比前一天长",
        S::MorningStar => "第一天收阴，实体不小于股价的 2%；第二天实体不超过第一天的 1/3，且整个实体不高于第一天收盘；第三天收阳，收盘不低于第一天实体的中点",
        S::EveningStar => "第一天收阳，实体不小于股价的 2%；第二天实体不超过第一天的 1/3，且整个实体不低于第一天收盘；第三天收阴，收盘不高于第一天实体的中点",
        S::MacdBottomDivergence => "收盘价的一个低点（前后各 3 天都不比它低）比上一个低点（60 天以内）更低，而这两天的 DIF 后一个更高，且后一个在零轴下方；在低点确认（之后第 3 天）时出现",
        S::MacdTopDivergence => "收盘价的一个高点（前后各 3 天都不比它高）比上一个高点（60 天以内）更高，而这两天的 DIF 后一个更低，且后一个在零轴上方；在高点确认（之后第 3 天）时出现",
        S::RsiBottomDivergence => "收盘价的一个低点（前后各 3 天都不比它低）比上一个低点（60 天以内）更低，而这两天的 RSI(14) 后一个更高；在低点确认（之后第 3 天）时出现",
        S::RsiTopDivergence => "收盘价的一个高点（前后各 3 天都不比它高）比上一个高点（60 天以内）更高，而这两天的 RSI(14) 后一个更低；在高点确认（之后第 3 天）时出现",
        S::MaSqueezeBreakUp => "前 10 个交易日里至少 8 天 5、10、20、60 日均线相差不超过收盘价的 2%（粘合），当天相差超过 3%（前一天还不超过），且收盘价高于四条均线",
        S::MaSqueezeBreakDown => "前 10 个交易日里至少 8 天 5、10、20、60 日均线相差不超过收盘价的 2%（粘合），当天相差超过 3%（前一天还不超过），且收盘价低于四条均线",
        S::LimitUp => "收盘价达到按交易所规则推算的涨停价",
        S::LimitDown => "收盘价达到按交易所规则推算的跌停价",
        S::BrokenLimitUp => "最高价触及涨停价，收盘没有封住",
        S::ConsecutiveLimitUp => "连续 N 个交易日收盘涨停",
        _ => return None,
    })
}

/// 背离：价格的高点（或低点）和指标的高点（或低点）方向相反。
/// 第 j 天是低点：j 前后各 PIVOT_SIDE 天的收盘价都不比它低（并列时取最早的一天），在第 j + PIVOT_SIDE 天才能确认，信号就出在这一天，不用到未来的数据。
/// 底背离：这个低点比上一个低点（相隔不超过 DIVERGENCE_MAX_GAP 天）更低，指标在这两天的值却是后一个更高；
/// MACD 的底背离还要求后一个 DIF 在零轴下方（顶背离在零轴上方）。顶背离相反
fn divergence(close: &[f64], ind: &[f64], bottom: bool, zero_side: bool) -> Vec<bool> {
    let n = close.len();
    let mut out = vec![false; n];
    // 比较：底部看谁更低，顶部看谁更高
    let beats = |a: f64, b: f64| if bottom { a < b } else { a > b };
    let mut last: Option<usize> = None;
    for i in 0..n {
        let Some(j) = i.checked_sub(PIVOT_SIDE).filter(|j| *j >= PIVOT_SIDE) else { continue };
        let w = &close[j - PIVOT_SIDE..=i];
        let pivot = w.iter().enumerate().all(|(k, v)| {
            let at = j - PIVOT_SIDE + k;
            at == j || (if at < j { beats(close[j], *v) } else { !beats(*v, close[j]) })
        });
        if !pivot || ind[j].is_nan() {
            continue;
        }
        if let Some(p) = last
            && j - p <= DIVERGENCE_MAX_GAP
            && beats(close[j], close[p])
            && beats(ind[p], ind[j])
            && (!zero_side || if bottom { ind[j] < 0.0 } else { ind[j] > 0.0 })
        {
            out[i] = true;
        }
        last = Some(j);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::IndicatorRef;
    use crate::analysis::indicators::tests::bars_from;

    fn crossing_level(a: &[f64], level: f64, up: bool) -> Vec<bool> {
        crossing(a, &vec![level; a.len()], up)
    }

    /// 直接按规则写的参照实现，核对这 19 个信号的条件定义逐位一致
    fn reference(f: &Frame, id: SignalId, params: &Params) -> Vec<bool> {
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
            _ => unreachable!("只核对 ORIGINAL 中的信号"),
        }
    }

    /// 有参照实现的 19 个信号
    const ORIGINAL: &[SignalId] = &[
        S::MaCrossUp,
        S::MaCrossDown,
        S::MaBullish,
        S::MaBearish,
        S::PriceCrossAboveMa,
        S::PriceCrossBelowMa,
        S::MacdGolden,
        S::MacdDeath,
        S::MacdTurnPositive,
        S::KdjGolden,
        S::KdjDeath,
        S::RsiLeaveOversold,
        S::RsiEnterOverbought,
        S::BollBreakUpper,
        S::BollBreakLower,
        S::NewHigh,
        S::NewLow,
        S::VolumeSurge,
        S::BigUp,
    ];

    /// 一组起伏明显、有涨停和跳空的日线，覆盖各种信号
    fn wavy(n: usize, seed: u64) -> Vec<crate::analysis::prices::DailyBar> {
        let mut x = seed;
        let mut rnd = || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) as f64 / (1u64 << 31) as f64
        };
        let mut c = 10.0;
        let closes: Vec<f64> = (0..n)
            .map(|i| {
                let drift = ((i as f64) / 17.0).sin() * 0.02;
                let jump = if rnd() < 0.03 { 0.1 } else { (rnd() - 0.5) * 0.06 };
                c *= 1.0 + drift + jump;
                (c * 100.0).round() / 100.0
            })
            .collect();
        let mut b = bars_from(&closes);
        for (i, bar) in b.iter_mut().enumerate() {
            let r = rnd();
            bar.q_open = bar.q_close * (1.0 + (r - 0.5) * 0.04);
            bar.open = bar.q_open;
            bar.q_high = bar.q_close.max(bar.q_open) * (1.0 + rnd() * 0.02);
            bar.high = bar.q_high;
            bar.q_low = bar.q_close.min(bar.q_open) * (1.0 - rnd() * 0.02);
            bar.low = bar.q_low;
            bar.volume = 1e6 * (1.0 + rnd() * if i % 23 == 0 { 6.0 } else { 1.0 });
        }
        b
    }

    #[test]
    fn condition_definitions_match_the_reference_bit_for_bit() {
        let param_sets: Vec<Vec<(&str, f64)>> = vec![
            vec![],
            vec![("fast", 3.0), ("slow", 10.0), ("short", 3.0), ("mid", 8.0), ("long", 21.0), ("n", 10.0), ("level", 40.0), ("ratio", 1.5), ("pct", 3.0)],
        ];
        let mut fired = std::collections::BTreeSet::new();
        for seed in 1..=6 {
            let b = wavy(400, seed);
            for info in CATALOG.iter().filter(|i| ORIGINAL.contains(&i.id)) {
                for ps in &param_sets {
                    let params: Params = ps.iter().filter(|(k, _)| info.params.iter().any(|p| p.key == *k)).map(|(k, v)| (k.to_string(), *v)).collect();
                    let f = Frame::new(&b, None);
                    let new = series(&f, info.id, &params);
                    let old = reference(&f, info.id, &params);
                    assert_eq!(*new, old, "{:?} {params:?} seed {seed}", info.id);
                    if old.iter().any(|v| *v) {
                        fired.insert(info.id);
                    }
                }
            }
        }
        // 每个信号在测试数据里都触发过，比较才有意义
        assert_eq!(fired.len(), ORIGINAL.len(), "没有触发过的：{:?}", ORIGINAL.iter().filter(|id| !fired.contains(id)).collect::<Vec<_>>());
    }

    /// 任何信号在某一天的结果只取决于这一天及以前的日线：截掉之后的日线重算，这一天的结果不变。
    /// 背离要等低点确认后才出现，Supertrend 依赖之前的状态，都要满足这一条，否则回测会偷看未来
    #[test]
    fn no_signal_looks_into_the_future() {
        for seed in [3, 11] {
            let b = wavy(260, seed);
            let full = Frame::new(&b, None);
            for info in CATALOG {
                let all = series(&full, info.id, &Params::new());
                for end in (30..b.len()).step_by(9) {
                    let part = Frame::new(&b[..=end], None);
                    assert_eq!(series(&part, info.id, &Params::new())[end], all[end], "{:?} 第 {end} 天 seed {seed}", info.id);
                }
            }
        }
    }

    /// 用条件写出的新信号与直接按规则算的结果一致
    #[test]
    fn new_definitions_follow_their_rules() {
        use IndicatorId as I;
        let mut fired = std::collections::BTreeSet::new();
        for seed in 1..=6 {
            let b = wavy(400, seed);
            let f = Frame::new(&b, None);
            let ind = |id: IndicatorId, n: Option<u32>| f.get(IndicatorRef::new(id, n));
            let (ch, vr5, vr20, dif, k, d, rsi, cci) =
                (ind(I::ChangePct, None), ind(I::VolRatio, Some(5)), ind(I::VolRatio, Some(20)), ind(I::MacdDif, None), ind(I::KdjK, None), ind(I::KdjD, None), ind(I::Rsi, Some(14)), ind(I::Cci, Some(14)));
            let (pdi, mdi, adx, st, close) = (ind(I::PlusDi, Some(14)), ind(I::MinusDi, Some(14)), ind(I::Adx, Some(14)), ind(I::Supertrend, Some(10)), ind(I::Close, None));
            let lvl = |v: f64| vec![v; b.len()];
            let n = b.len();
            let cases: Vec<(SignalId, Vec<bool>)> = vec![
                (S::BigDown, (0..n).map(|i| ch[i] <= -5.0 && b[i].q_close < b[i].q_open).collect()),
                (S::VolumeShrink, vr5.iter().map(|v| *v <= 0.5).collect()),
                (S::MacdTurnNegative, crossing(&dif, &lvl(0.0), false)),
                (S::UpStreak, (0..n).map(|i| i >= 3 && (i - 2..=i).all(|j| ch[j] > 0.0)).collect()),
                (S::DownStreak, (0..n).map(|i| i >= 3 && (i - 2..=i).all(|j| ch[j] < 0.0)).collect()),
                (S::KdjLowGolden, crossing(&k, &d, true).iter().zip(d.iter()).map(|(c, d)| *c && *d < 20.0).collect()),
                (S::KdjHighDeath, crossing(&k, &d, false).iter().zip(d.iter()).map(|(c, d)| *c && *d > 80.0).collect()),
                (S::RsiLeaveOverbought, crossing(&rsi, &lvl(70.0), false)),
                (S::RsiEnterOversold, crossing(&rsi, &lvl(30.0), false)),
                (S::CciBreakUp, crossing(&cci, &lvl(100.0), true)),
                (S::CciBreakDown, crossing(&cci, &lvl(-100.0), false)),
                (S::DiCrossUp, crossing(&pdi, &mdi, true)),
                (S::DiCrossDown, crossing(&pdi, &mdi, false)),
                (S::AdxStrong, adx.iter().map(|v| *v > 25.0).collect()),
                (S::SupertrendUp, crossing(&close, &st, true)),
                (S::SupertrendDown, crossing(&close, &st, false)),
                (S::VolumePriceUp, (0..n).map(|i| ch[i] >= 2.0 && vr5[i] >= 2.0).collect()),
                (S::NewHighLowVolume, (0..n).map(|i| i >= 60 && close[i - 60..i].iter().all(|c| close[i] > *c) && vr20[i] < 1.0).collect()),
            ];
            for (id, want) in cases {
                assert_eq!(*series(&f, id, &Params::new()), want, "{id:?} seed {seed}");
                if want.iter().any(|v| *v) {
                    fired.insert(id);
                }
            }
        }
        assert!(fired.len() >= 16, "大部分信号在测试数据里都触发过：{fired:?}");
    }

    fn bar(o: f64, h: f64, l: f64, c: f64) -> crate::analysis::prices::DailyBar {
        let mut b = bars_from(&[c]).remove(0);
        (b.q_open, b.q_high, b.q_low, b.q_close) = (o, h, l, c);
        (b.open, b.high, b.low) = (o, h, l);
        b
    }

    fn fires(bars: &[crate::analysis::prices::DailyBar], id: SignalId) -> Vec<usize> {
        let f = Frame::new(bars, None);
        series(&f, id, &Params::new()).iter().enumerate().filter(|(_, v)| **v).map(|(i, _)| i).collect()
    }

    #[test]
    fn candlestick_patterns() {
        // 0 普通阳线，1 阴线，2 吞没它的阳线，3 十字星，4 向下跳空，5 长上影线
        let b = vec![bar(10.0, 10.3, 9.9, 10.2), bar(10.2, 10.25, 9.9, 10.0), bar(9.95, 10.4, 9.9, 10.3), bar(10.3, 10.5, 10.1, 10.31), bar(9.9, 10.0, 9.7, 9.8), bar(9.85, 10.4, 9.75, 9.95)];
        assert_eq!(fires(&b, S::BullishEngulfing), vec![2]);
        assert_eq!(fires(&b, S::Doji), vec![3]);
        assert_eq!(fires(&b, S::GapDown), vec![4]);
        assert_eq!(fires(&b, S::LongUpperShadow), vec![5]);
        assert!(fires(&b, S::BearishEngulfing).is_empty());
        // 早晨之星：大阴线、下方的小实体、收复一半以上的阳线；把第三根改成小阳线就不算
        let star = vec![bar(10.0, 10.0, 10.0, 10.0), bar(10.0, 10.05, 9.4, 9.5), bar(9.4, 9.5, 9.2, 9.35), bar(9.4, 9.9, 9.35, 9.85)];
        assert_eq!(fires(&star, S::MorningStar), vec![3]);
        let weak = vec![star[0].clone(), star[1].clone(), star[2].clone(), bar(9.4, 9.6, 9.35, 9.55)];
        assert!(fires(&weak, S::MorningStar).is_empty());
        let evening = vec![bar(10.0, 10.0, 10.0, 10.0), bar(10.0, 10.6, 9.95, 10.5), bar(10.6, 10.8, 10.5, 10.65), bar(10.6, 10.65, 10.1, 10.15)];
        assert_eq!(fires(&evening, S::EveningStar), vec![3]);
    }

    #[test]
    fn divergence_is_reported_when_the_second_low_is_confirmed() {
        // 两段下跌：第二个低点（第 30 天）比第一个（第 12 天）更低，但跌得慢，RSI 的低点抬高
        let mut c = vec![];
        for i in 0..=12 {
            c.push(20.0 - i as f64 * 0.8);
        }
        for i in 1..=8 {
            c.push(10.4 + i as f64 * 0.5);
        }
        for i in 1..=10 {
            c.push(14.4 - i as f64 * 0.42);
        }
        for i in 1..=8 {
            c.push(10.2 + i as f64 * 0.3);
        }
        let mut b = bars_from(&c);
        // 前面补一段平稳的日线，让 RSI 算得出来
        let mut pre = bars_from(&[20.0; 20]);
        pre.append(&mut b);
        let low2 = 20 + 30;
        let hits = fires(&pre, S::RsiBottomDivergence);
        assert_eq!(hits, vec![low2 + PIVOT_SIDE], "低点确认后（之后第 3 天）才出现");
        assert!(fires(&pre, S::RsiTopDivergence).is_empty());
    }

    #[test]
    fn ma_squeeze_then_break_up() {
        // 70 天几乎不动（均线粘合），之后连续大涨，均线散开
        let mut c: Vec<f64> = (0..70).map(|i| 10.0 + if i % 2 == 0 { 0.02 } else { -0.02 }).collect();
        c.extend((1..=8).map(|i| 10.0 * 1.05f64.powi(i)));
        let b = bars_from(&c);
        let up = fires(&b, S::MaSqueezeBreakUp);
        assert_eq!(up.len(), 1, "只在第一次散开时出现：{up:?}");
        assert!(up[0] > 70);
        assert!(fires(&b, S::MaSqueezeBreakDown).is_empty());
    }

    #[test]
    fn every_catalog_entry_is_explained() {
        for s in CATALOG {
            assert!(!s.description.is_empty() && !s.usage.is_empty(), "信号 {:?} 缺少说明", s.id);
        }
        for i in crate::analysis::indicators::CATALOG {
            assert!(!i.description.is_empty() && !i.usage.is_empty() && !i.formula.is_empty(), "指标 {:?} 缺少说明", i.id);
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
        let sig = series(&f, SignalId::MaCrossUp, &params);
        // 与 ma::ma_cross 的测试相同：第 6 根上穿
        assert_eq!(*sig, vec![false, false, false, false, false, true, false]);
        assert_eq!(label(SignalId::MaCrossUp, &params), "2 日均线上穿 3 日均线");
    }

    #[test]
    fn new_high_and_volume_surge() {
        let b = bars_from(&[10.0, 11.0, 10.5, 12.0]);
        let f = Frame::new(&b, None);
        let mut p = Params::new();
        p.insert("n".into(), 3.0);
        assert_eq!(*series(&f, SignalId::NewHigh, &p), vec![false, false, false, true]);
        // 成交量逐日增加 1000：第 4 天 4000 是前 5 天平均（不足 5 天）——不足时不判断
        assert!(series(&f, SignalId::VolumeSurge, &Params::new()).iter().all(|v| !v));
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
