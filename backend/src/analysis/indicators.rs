//! 技术指标：在一只股票按日期升序的日线上计算，价格一律用等比前复权价，成交量和成交额用原值。
//! 每个指标返回与日线等长的序列，数据不够算出的位置是 NaN。口径见 docs/domains/strategies.md#indicators。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::limit_pool::LimitPool;
use super::ma::sma;
use super::prices::DailyBar;
pub use crate::instrument::AssetType;

/// 指标周期 n 的上限：巡检和回测按它确定要读多少根历史日线
pub const MAX_WINDOW: u32 = 250;

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorId {
    Close,
    Open,
    High,
    Low,
    ChangePct,
    Amplitude,
    Amount,
    Volume,
    Ma,
    Ema,
    MaGap,
    MacdDif,
    MacdDea,
    MacdHist,
    Rsi,
    KdjK,
    KdjD,
    KdjJ,
    BollUpper,
    BollMid,
    BollLower,
    Atr,
    AtrPct,
    VolRatio,
    Momentum,
    HighN,
    LowN,
    PctFromHigh,
    PctFromLow,
    PrevHighClose,
    PrevLowClose,
    PlusDi,
    MinusDi,
    Adx,
    Supertrend,
    Cci,
    BollWidth,
    MaSpread,
}

/// 一个指标及其周期，例如「20 日均线」是 {id: ma, n: 20}
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IndicatorRef {
    pub id: IndicatorId,
    /// 周期；不带周期的指标忽略它，带周期的指标缺省时用目录中的默认值
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
}

/// 指标目录中的一项：给前端展示和白话描述用
#[derive(Serialize, ToSchema, Clone, Copy)]
pub struct IndicatorInfo {
    pub id: IndicatorId,
    /// 名称，带周期的用 {n} 占位，例如「{n} 日均线」
    pub name: &'static str,
    pub group: &'static str,
    /// 单位：price 价格、pct 百分数（5 表示 5%）、ratio 倍数、yuan 元、shares 股、index 指数值
    pub unit: &'static str,
    pub description: &'static str,
    /// 怎么看、要注意什么，给没有金融背景的人看
    pub usage: &'static str,
    /// 怎么算：用了哪些数据、公式和固定参数
    pub formula: &'static str,
    /// 周期的默认值和范围；不带周期时为空
    pub n_default: Option<u32>,
    pub n_min: Option<u32>,
    pub n_max: Option<u32>,
}

const fn info(
    id: IndicatorId,
    name: &'static str,
    group: &'static str,
    unit: &'static str,
    description: &'static str,
    n: Option<(u32, u32, u32)>,
) -> IndicatorInfo {
    let (n_default, n_min, n_max) = match n {
        Some((d, lo, hi)) => (Some(d), Some(lo), Some(hi)),
        None => (None, None, None),
    };
    IndicatorInfo {
        id,
        name,
        group,
        unit,
        description,
        usage: "",
        formula: "",
        n_default,
        n_min,
        n_max,
    }
}

impl IndicatorInfo {
    const fn usage(mut self, usage: &'static str) -> Self {
        self.usage = usage;
        self
    }

    const fn formula(mut self, formula: &'static str) -> Self {
        self.formula = formula;
        self
    }
}

pub const CATALOG: &[IndicatorInfo] = &[
    info(IndicatorId::Close, "收盘价", "价格", "price", "当天收盘价（前复权）", None).usage("最基本的数。条件里的价格默认都用收盘价，因为它是一天交易的最终结果。").formula("当天的收盘价，按等比前复权调整（除权除息不会造成价格断崖）"),
    info(IndicatorId::Open, "开盘价", "价格", "price", "当天开盘价（前复权）", None).usage("反映隔夜消息和开盘时的情绪。回测里的买卖都按开盘价成交。").formula("当天的开盘价，前复权"),
    info(IndicatorId::High, "最高价", "价格", "price", "当天最高价（前复权）", None).usage("当天波动的上沿。").formula("当天的最高价，前复权"),
    info(IndicatorId::Low, "最低价", "价格", "price", "当天最低价（前复权）", None).usage("当天波动的下沿。").formula("当天的最低价，前复权"),
    info(IndicatorId::ChangePct, "涨跌幅", "价格", "pct", "相对前一个交易日收盘的涨跌，5 表示涨 5%", None).usage("判断当天强弱最直接的数。不同板块的涨跌幅上限不同：主板 10%，创业板、科创板 20%。").formula("(今天收盘 ÷ 昨天收盘 − 1) × 100，用前复权收盘价"),
    info(IndicatorId::Amplitude, "振幅", "价格", "pct", "（最高 − 最低）/ 前收盘，衡量当天波动有多大", None).usage("振幅大说明当天买卖双方分歧大。").formula("(最高 − 最低) ÷ 昨天收盘 × 100，用前复权价"),
    info(IndicatorId::Amount, "成交额", "量能", "yuan", "当天成交金额（元），常用来排除成交太少的股票", None).usage("衡量交易活跃程度最常用的数。成交额太小的股票买卖都困难，策略默认排除成交额低于 5000 万元的股票。").formula("交易所公布的当天成交金额（元），不复权"),
    info(IndicatorId::Volume, "成交量", "量能", "shares", "当天成交股数", None).usage("成交的股数。不同股价的股票之间不好直接比较，比较时用成交额或量比。").formula("交易所公布的当天成交股数，不复权"),
    info(IndicatorId::VolRatio, "量比", "量能", "ratio", "当天成交量是前 {n} 天平均成交量的几倍，大于 1 表示放量", Some((5, 2, 60))).usage("大于 1 放量，小于 1 缩量；1.5 倍以上算明显放量，0.7 倍以下算明显缩量。").formula("今天成交量 ÷ 前 n 天（不含今天）平均成交量"),
    info(IndicatorId::Ma, "{n} 日均线", "均线", "price", "最近 {n} 个交易日收盘价的平均值", Some((20, 2, MAX_WINDOW))).usage("最常用的指标。5 日、10 日看短线，20 日、60 日看中期，120 日、250 日看长期。股价在均线上方，说明这段时间买入的人平均在赚钱。").formula("最近 n 天（含今天）前复权收盘价的平均值"),
    info(IndicatorId::Ema, "{n} 日指数均线", "均线", "price", "越近的价格权重越大的均线，比普通均线反应更快", Some((20, 2, MAX_WINDOW))).usage("对最近的价格反应更快，转折时比普通均线早几天，但也更容易被短期波动带偏。").formula("指数加权平均：今天的值 = α × 今天收盘 + (1 − α) × 昨天的值，α = 2 ÷ (n + 1)"),
    info(IndicatorId::MaSpread, "均线粘合度", "均线", "pct", "5、10、20、60 日均线中最高的与最低的相差多少（占收盘价的百分比），越小说明几条均线越靠拢", None).usage("小于 2 说明几条均线几乎拧在一起（粘合），常见于长时间横盘；之后一旦明显散开（大于 3），往往是选择方向的开始。").formula("(四条均线的最大值 − 最小值) ÷ 收盘价 × 100，均线是 5、10、20、60 日简单均线"),
    info(IndicatorId::MaGap, "相对 {n} 日均线偏离", "均线", "pct", "收盘价比 {n} 日均线高出（正）或低于（负）多少，2 表示高 2%", Some((20, 2, MAX_WINDOW))).usage("偏离太大时（例如高出 20 日均线 15% 以上），价格往往会向均线靠拢。").formula("(收盘价 ÷ n 日均线 − 1) × 100"),
    info(IndicatorId::MacdDif, "MACD 快线（DIF）", "MACD", "price", "12 日与 26 日指数均线之差，大于 0 表示短期强于长期", None).usage("MACD 的主线，大于 0 表示中期走势偏强。").formula("12 日指数均线 − 26 日指数均线，用前复权收盘价"),
    info(IndicatorId::MacdDea, "MACD 慢线（DEA）", "MACD", "price", "DIF 的 9 日指数均线", None).usage("DIF 的平滑线，用来判断 DIF 的交叉。").formula("DIF 的 9 日指数均线"),
    info(IndicatorId::MacdHist, "MACD 柱", "MACD", "price", "2 ×（DIF − DEA），由负转正常被看作转强", None).usage("柱子由负转正、越来越长，表示动能在增强；柱子变短说明动能减弱，常常比价格转折更早出现。").formula("2 × (DIF − DEA)"),
    info(IndicatorId::Rsi, "RSI（{n}）", "超买超卖", "index", "0 到 100，反映最近 {n} 天涨的力量占比；高于 70 常被看作超买，低于 30 看作超卖", Some((14, 2, 60))).usage("70 以上超买、30 以下超卖是常用的分界。强势股可以长期在 50 到 80 之间，弱势股长期在 20 到 50 之间。").formula("最近 n 天上涨幅度的平滑平均 ÷ 涨跌幅度（取绝对值）的平滑平均 × 100；平滑方法与通达信、同花顺一致"),
    info(IndicatorId::Cci, "CCI（{n}）", "超买超卖", "index", "顺势指标：价格偏离近期平均水平的程度，高于 100 说明明显偏强，低于 −100 明显偏弱", Some((14, 5, 60))).usage("大部分时间在 −100 到 100 之间。上穿 100 常被看作进入强势、可能有一段行情，下穿 −100 说明进入弱势。和 RSI 不同，它没有上下限。").formula("典型价 = (最高 + 最低 + 收盘) ÷ 3；CCI = (典型价 − 典型价的 n 日均值) ÷ (0.015 × 典型价的 n 日平均绝对偏差)；与通达信、同花顺一致"),
    info(IndicatorId::KdjK, "KDJ 的 K 值", "超买超卖", "index", "随机指标 K（9、3、3），0 到 100 附近，高位超买、低位超卖", None).usage("短线指标，变化很快。").formula("RSV = (收盘 − 9 日最低) ÷ (9 日最高 − 9 日最低) × 100；K 是 RSV 的 3 日平滑，初值 50"),
    info(IndicatorId::KdjD, "KDJ 的 D 值", "超买超卖", "index", "K 值的平滑", None).usage("比 K 慢，K 上穿 D 就是 KDJ 金叉。").formula("K 的 3 日平滑，初值 50"),
    info(IndicatorId::KdjJ, "KDJ 的 J 值", "超买超卖", "index", "3K − 2D，最敏感，可以超出 0 到 100", None).usage("三条线里最敏感的，常超出 0 到 100：大于 100 说明短期极强，小于 0 说明短期极弱。").formula("3K − 2D"),
    info(IndicatorId::BollUpper, "布林带上轨", "波动", "price", "20 日均线加 2 倍标准差，收盘突破上轨表示走势很强", None).usage("价格大部分时间在上下轨之间。上下轨的距离变窄说明波动在收敛，常常预示快要选择方向。").formula("20 日均线 + 2 × 最近 20 天收盘价的标准差"),
    info(IndicatorId::BollMid, "布林带中轨", "波动", "price", "20 日均线", None).usage("就是 20 日均线。").formula("20 日均线"),
    info(IndicatorId::BollLower, "布林带下轨", "波动", "price", "20 日均线减 2 倍标准差", None).usage("跌破下轨说明跌势超出平常。").formula("20 日均线 − 2 × 最近 20 天收盘价的标准差"),
    info(IndicatorId::BollWidth, "布林带宽度", "波动", "pct", "布林带上下轨相距多少（占中轨的百分比），越小说明最近波动越小", None).usage("宽度缩到近几个月最小（布林带收口）时，常常预示快要变盘；随后宽度迅速变大（开口），说明新的方向开始了。宽度本身不说明方向。").formula("(上轨 − 下轨) ÷ 中轨 × 100，即 4 倍 20 日标准差 ÷ 20 日均线 × 100"),
    info(IndicatorId::Atr, "ATR（{n}）", "波动", "price", "平均真实波幅：最近 {n} 天每天的平均波动金额", Some((14, 2, 60))).usage("估计这只股票正常一天会波动多少钱，常用来决定止损设在多远。").formula("每天的真实波幅取「最高 − 最低」「最高 − 昨收」「昨收 − 最低」中的最大值，再求最近 n 天的平均"),
    info(IndicatorId::AtrPct, "ATR 占股价（{n}）", "波动", "pct", "ATR 除以收盘价，用百分比比较不同价格的股票波动有多大", Some((14, 2, 60))).usage("可以比较不同价格的股票：3 表示正常一天波动约 3%。数值越大，波动和风险越大。").formula("ATR ÷ 收盘价 × 100"),
    info(IndicatorId::PlusDi, "+DI（{n}）", "趋势强度", "index", "上升方向的力量：最近 {n} 天创新高的幅度占波动的比例", Some((14, 2, 60))).usage("和 −DI 一起看：+DI 在 −DI 上方说明多方占优，+DI 上穿 −DI 常被看作转强。").formula("最近 n 天「上升动向」之和 ÷ 最近 n 天真实波幅之和 × 100。上升动向：今天最高 − 昨天最高，大于 0 且大于下降动向时取它，否则为 0。与通达信、同花顺的 DMI 一致"),
    info(IndicatorId::MinusDi, "−DI（{n}）", "趋势强度", "index", "下降方向的力量：最近 {n} 天创新低的幅度占波动的比例", Some((14, 2, 60))).usage("−DI 在 +DI 上方说明空方占优。").formula("最近 n 天「下降动向」之和 ÷ 最近 n 天真实波幅之和 × 100。下降动向：昨天最低 − 今天最低，大于 0 且大于上升动向时取它，否则为 0"),
    info(IndicatorId::Adx, "ADX（{n}）", "趋势强度", "index", "趋势强度：0 到 100，只说明趋势有多明确，不说明方向", Some((14, 2, 60))).usage("高于 25 说明趋势明确（不管涨还是跌），低于 20 说明在盘整。均线交叉这类趋势信号在 ADX 高时更可靠，盘整时容易反复。方向看 +DI 和 −DI 谁在上面。").formula("DX = |+DI − −DI| ÷ (+DI + −DI) × 100，ADX 是 DX 的 6 日均线；与通达信、同花顺一致"),
    info(IndicatorId::Supertrend, "Supertrend（{n}）", "趋势强度", "price", "超级趋势线：上升趋势时在股价下方，下降趋势时在股价上方，收盘越过它就翻转", Some((10, 2, 60))).usage("TradingView 上最常用的趋势指标。线在股价下方时可以把它当作移动止损线：收盘跌破就离场。横盘时会来回翻转。").formula("中价 = (最高 + 最低) ÷ 2；上下轨 = 中价 ± 3 × ATR(n)，ATR 用 Wilder 平滑；下轨只升不降、上轨只降不升，收盘越过当前的线就换到另一条；与 TradingView 一致"),
    info(IndicatorId::Momentum, "{n} 日涨跌幅", "动量", "pct", "相对 {n} 个交易日前收盘价的涨跌，20 表示涨了 20%", Some((20, 1, MAX_WINDOW))).usage("最直观的强弱：近 20 日涨得最多的就是最近最强的股票。「强者恒强」和「超跌反弹」都基于它。").formula("(今天收盘 ÷ n 天前收盘 − 1) × 100"),
    info(IndicatorId::HighN, "{n} 日最高价", "区间", "price", "最近 {n} 个交易日（含当天）的最高价", Some((60, 2, MAX_WINDOW))).usage("区间高点常被看作压力：涨到这里容易遇到之前被套的人卖出。").formula("最近 n 天（含今天）最高价中的最大值"),
    info(IndicatorId::LowN, "{n} 日最低价", "区间", "price", "最近 {n} 个交易日（含当天）的最低价", Some((60, 2, MAX_WINDOW))).usage("区间低点常被看作支撑。").formula("最近 n 天（含今天）最低价中的最小值"),
    info(IndicatorId::PctFromHigh, "距 {n} 日最高收盘", "区间", "pct", "收盘价比最近 {n} 天最高收盘价低多少，0 表示就在最高点，−10 表示低 10%", Some((60, 2, MAX_WINDOW))).usage("接近 0 说明在近期高位；−30 说明从高点跌了 30%。").formula("(收盘价 ÷ 最近 n 天最高收盘 − 1) × 100"),
    info(IndicatorId::PrevHighClose, "前 {n} 日最高收盘", "区间", "price", "当天之前 {n} 个交易日（不含当天）的最高收盘价", Some((60, 2, MAX_WINDOW))).usage("收盘价高于它，就是创 {n} 日新高；常用来写突破类信号。").formula("今天之前 n 天收盘价的最大值，不含今天"),
    info(IndicatorId::PrevLowClose, "前 {n} 日最低收盘", "区间", "price", "当天之前 {n} 个交易日（不含当天）的最低收盘价", Some((60, 2, MAX_WINDOW))).usage("收盘价低于它，就是创 {n} 日新低。").formula("今天之前 n 天收盘价的最小值，不含今天"),
    info(IndicatorId::PctFromLow, "距 {n} 日最低收盘", "区间", "pct", "收盘价比最近 {n} 天最低收盘价高多少，0 表示就在最低点", Some((60, 2, MAX_WINDOW))).usage("接近 0 说明在近期低位；50 说明比低点涨了 50%。").formula("(收盘价 ÷ 最近 n 天最低收盘 − 1) × 100"),
];

pub fn info_of(id: IndicatorId) -> &'static IndicatorInfo {
    CATALOG
        .iter()
        .find(|i| i.id == id)
        .expect("每个指标都在目录中")
}

impl IndicatorRef {
    pub const fn new(id: IndicatorId, n: Option<u32>) -> Self {
        IndicatorRef { id, n }
    }

    /// 实际使用的周期：不带周期的指标为 0
    pub fn period(&self) -> u32 {
        let i = info_of(self.id);
        match i.n_default {
            Some(d) => self.n.unwrap_or(d),
            None => 0,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let i = info_of(self.id);
        if let (Some(n), Some(lo), Some(hi)) = (self.n, i.n_min, i.n_max)
            && !(lo..=hi).contains(&n)
        {
            return Err(format!(
                "{} 的周期应在 {lo} 到 {hi} 之间",
                i.name.replace("{n}", "N")
            ));
        }
        Ok(())
    }

    /// 白话名称，例如「20 日均线」
    pub fn label(&self) -> String {
        info_of(self.id)
            .name
            .replace("{n}", &self.period().to_string())
    }

    /// 算出这个指标需要的历史日线根数（含指数均线的收敛期）
    pub fn warmup(&self) -> u32 {
        match self.id {
            IndicatorId::Ema => self.period() * 3,
            IndicatorId::MacdDif | IndicatorId::MacdDea | IndicatorId::MacdHist => 120,
            IndicatorId::Rsi | IndicatorId::Atr | IndicatorId::AtrPct => self.period() * 4,
            IndicatorId::KdjK | IndicatorId::KdjD | IndicatorId::KdjJ => 40,
            IndicatorId::BollUpper
            | IndicatorId::BollMid
            | IndicatorId::BollLower
            | IndicatorId::BollWidth => 20,
            IndicatorId::PlusDi | IndicatorId::MinusDi | IndicatorId::Adx => self.period() + 10,
            // 路径相关（翻转与否取决于之前的状态），多读一些让开头的状态不影响结果
            IndicatorId::Supertrend => self.period() * 4 + 60,
            IndicatorId::MaSpread => 61,
            _ => self.period() + 1,
        }
    }
}

/// 证券静态信息。股票和 ETF 共用分析主流程，资产差异由 asset_type 和交易扩展提供。
#[derive(Clone, Debug)]
pub struct StockMeta {
    /// main / chinext / star / bse
    pub board: String,
    /// 当前简称含 ST（没有历史 ST 状态，整段历史都按当前状态计算）
    pub is_st: bool,
    pub list_date: Option<NaiveDate>,
    pub asset_type: AssetType,
}

/// 从开始采集那天起记录的简称区间；区间内的 ST 状态用于历史涨跌停判断。
#[derive(Clone, Debug)]
pub struct StPeriod {
    pub valid_from: NaiveDate,
    pub valid_to: Option<NaiveDate>,
    pub is_st: bool,
}

/// 指标缓存的键：指标和实际周期
type IndicatorKey = (IndicatorId, u32);

/// 一只股票的日线和已经算过的指标
pub struct Frame<'a> {
    pub bars: &'a [DailyBar],
    pub meta: Option<&'a StockMeta>,
    cache: RefCell<HashMap<IndicatorKey, Rc<Vec<f64>>>>,
    /// 信号序列的缓存，键是信号和参数（由 signals 模块生成）
    bools: RefCell<HashMap<String, Rc<Vec<bool>>>>,
    limits: RefCell<Option<Rc<Limits>>>,
    /// 权威涨跌停池和这只股票的代码；没有时涨跌停全部自算
    authority: Option<(&'a LimitPool, &'a str)>,
    /// 简称历史；没有覆盖的日期回退当前简称
    st_history: Option<&'a [StPeriod]>,
}

/// 涨跌停价与当天是否涨停、跌停、炸板
pub struct Limits {
    pub up_price: Vec<f64>,
    pub down_price: Vec<f64>,
    pub limit_up: Vec<bool>,
    pub limit_down: Vec<bool>,
    /// 盘中触及涨停但收盘没封住
    pub broken: Vec<bool>,
    /// 连续第几个涨停；权威日期取供应商的连板数，否则取自算的连续天数
    pub consecutive_up: Vec<u32>,
}

impl<'a> Frame<'a> {
    pub fn new(bars: &'a [DailyBar], meta: Option<&'a StockMeta>) -> Self {
        Frame {
            bars,
            meta,
            cache: RefCell::new(HashMap::new()),
            bools: RefCell::new(HashMap::new()),
            limits: RefCell::new(None),
            authority: None,
            st_history: None,
        }
    }

    pub fn with_st_history(
        bars: &'a [DailyBar],
        meta: Option<&'a StockMeta>,
        history: &'a [StPeriod],
    ) -> Self {
        Frame {
            bars,
            meta,
            cache: RefCell::new(HashMap::new()),
            bools: RefCell::new(HashMap::new()),
            limits: RefCell::new(None),
            authority: None,
            st_history: Some(history),
        }
    }

    /// 带权威涨跌停池的 Frame：涨停、跌停、炸板以供应商的池为准，池没有覆盖的日期和北交所回退自算。
    /// pool 的加载窗口必须覆盖 bars 的全部日期，否则窗口外的日期会回退自算（见 [`LimitPool::load`] 的覆盖裁剪）
    pub fn with_authority(
        bars: &'a [DailyBar],
        meta: Option<&'a StockMeta>,
        pool: &'a LimitPool,
        code: &'a str,
    ) -> Self {
        Frame {
            bars,
            meta,
            cache: RefCell::new(HashMap::new()),
            bools: RefCell::new(HashMap::new()),
            limits: RefCell::new(None),
            authority: Some((pool, code)),
            st_history: None,
        }
    }

    pub fn with_authority_and_st_history(
        bars: &'a [DailyBar],
        meta: Option<&'a StockMeta>,
        pool: &'a LimitPool,
        code: &'a str,
        history: &'a [StPeriod],
    ) -> Self {
        Frame {
            bars,
            meta,
            cache: RefCell::new(HashMap::new()),
            bools: RefCell::new(HashMap::new()),
            limits: RefCell::new(None),
            authority: Some((pool, code)),
            st_history: Some(history),
        }
    }

    /// 布尔序列（信号）的缓存：同一只股票上同一个信号只算一次
    pub fn memo_bools(&self, key: String, compute: impl FnOnce() -> Vec<bool>) -> Rc<Vec<bool>> {
        if let Some(v) = self.bools.borrow().get(&key) {
            return v.clone();
        }
        let v = Rc::new(compute());
        self.bools.borrow_mut().insert(key, v.clone());
        v
    }

    pub fn len(&self) -> usize {
        self.bars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    /// 这根日线当天是否处于 ST 状态；历史区间没有覆盖时回退当前简称。
    pub fn is_st(&self, i: usize) -> bool {
        let date = self.bars.get(i).map(|b| b.date);
        date.map(|d| self.is_st_on(d))
            .unwrap_or_else(|| self.meta.is_some_and(|m| m.is_st))
    }

    fn is_st_on(&self, date: NaiveDate) -> bool {
        if let Some(history) = self.st_history
            && let Some(p) = history
                .iter()
                .find(|p| p.valid_from <= date && p.valid_to.is_none_or(|to| date < to))
        {
            return p.is_st;
        }
        self.meta.is_some_and(|m| m.is_st)
    }

    /// 指标序列（与日线等长，不可用的位置是 NaN）
    pub fn get(&self, r: IndicatorRef) -> Rc<Vec<f64>> {
        let key = (r.id, r.period());
        if let Some(v) = self.cache.borrow().get(&key) {
            return v.clone();
        }
        let v = Rc::new(self.compute(r.id, r.period() as usize));
        self.cache.borrow_mut().insert(key, v.clone());
        v
    }

    /// 某一天的值
    pub fn at(&self, r: IndicatorRef, i: usize) -> f64 {
        self.get(r)[i]
    }

    fn col(&self, f: impl Fn(&DailyBar) -> f64) -> Vec<f64> {
        self.bars.iter().map(f).collect()
    }

    fn compute(&self, id: IndicatorId, n: usize) -> Vec<f64> {
        use IndicatorId as I;
        let close = || self.col(|b| b.q_close);
        match id {
            I::Close => close(),
            I::Open => self.col(|b| b.q_open),
            I::High => self.col(|b| b.q_high),
            I::Low => self.col(|b| b.q_low),
            I::Amount => self.col(|b| b.amount),
            I::Volume => self.col(|b| b.volume),
            I::ChangePct => {
                let c = close();
                (0..c.len())
                    .map(|i| {
                        if i == 0 {
                            f64::NAN
                        } else {
                            (c[i] / c[i - 1] - 1.0) * 100.0
                        }
                    })
                    .collect()
            }
            I::Amplitude => {
                let b = self.bars;
                (0..b.len())
                    .map(|i| {
                        if i == 0 {
                            f64::NAN
                        } else {
                            (b[i].q_high - b[i].q_low) / b[i - 1].q_close * 100.0
                        }
                    })
                    .collect()
            }
            I::Ma => sma(&close(), n)
                .into_iter()
                .map(|v| v.unwrap_or(f64::NAN))
                .collect(),
            I::Ema => ema(&close(), n),
            I::MaGap => {
                let c = close();
                let m = self.get(IndicatorRef::new(I::Ma, Some(n as u32)));
                c.iter()
                    .zip(m.iter())
                    .map(|(c, m)| (c / m - 1.0) * 100.0)
                    .collect()
            }
            I::MacdDif => {
                let c = close();
                ema(&c, 12)
                    .iter()
                    .zip(ema(&c, 26))
                    .map(|(a, b)| a - b)
                    .collect()
            }
            I::MacdDea => ema(&self.get(IndicatorRef::new(I::MacdDif, None)), 9),
            I::MacdHist => {
                let dif = self.get(IndicatorRef::new(I::MacdDif, None));
                let dea = self.get(IndicatorRef::new(I::MacdDea, None));
                dif.iter()
                    .zip(dea.iter())
                    .map(|(a, b)| 2.0 * (a - b))
                    .collect()
            }
            I::Rsi => rsi(&close(), n),
            I::KdjK | I::KdjD | I::KdjJ => {
                let (k, d, j) = kdj(self.bars, 9);
                match id {
                    I::KdjK => k,
                    I::KdjD => d,
                    _ => j,
                }
            }
            I::BollMid => self.get(IndicatorRef::new(I::Ma, Some(20))).to_vec(),
            I::BollUpper | I::BollLower => {
                let c = close();
                let mid = self.get(IndicatorRef::new(I::Ma, Some(20)));
                let sd = rolling_std(&c, 20);
                let k = if id == I::BollUpper { 2.0 } else { -2.0 };
                mid.iter().zip(sd).map(|(m, s)| m + k * s).collect()
            }
            I::Atr => atr(self.bars, n),
            I::AtrPct => {
                let a = self.get(IndicatorRef::new(I::Atr, Some(n as u32)));
                a.iter()
                    .zip(self.bars)
                    .map(|(a, b)| a / b.q_close * 100.0)
                    .collect()
            }
            I::VolRatio => {
                let v = self.col(|b| b.volume);
                (0..v.len())
                    .map(|i| {
                        if i < n {
                            return f64::NAN;
                        }
                        let avg = v[i - n..i].iter().sum::<f64>() / n as f64;
                        if avg > 0.0 { v[i] / avg } else { f64::NAN }
                    })
                    .collect()
            }
            I::Momentum => {
                let c = close();
                (0..c.len())
                    .map(|i| {
                        if i < n {
                            f64::NAN
                        } else {
                            (c[i] / c[i - n] - 1.0) * 100.0
                        }
                    })
                    .collect()
            }
            I::HighN => rolling(&self.col(|b| b.q_high), n, f64::max),
            I::LowN => rolling(&self.col(|b| b.q_low), n, f64::min),
            I::PctFromHigh => {
                let c = close();
                let hi = rolling(&c, n, f64::max);
                c.iter()
                    .zip(hi)
                    .map(|(c, h)| (c / h - 1.0) * 100.0)
                    .collect()
            }
            I::PrevHighClose | I::PrevLowClose => {
                let c = close();
                let high = id == I::PrevHighClose;
                (0..c.len())
                    .map(|i| {
                        if i < n {
                            return f64::NAN;
                        }
                        let w = c[i - n..i].iter().copied();
                        if high {
                            w.fold(f64::MIN, f64::max)
                        } else {
                            w.fold(f64::MAX, f64::min)
                        }
                    })
                    .collect()
            }
            I::PctFromLow => {
                let c = close();
                let lo = rolling(&c, n, f64::min);
                c.iter()
                    .zip(lo)
                    .map(|(c, l)| (c / l - 1.0) * 100.0)
                    .collect()
            }
            I::PlusDi | I::MinusDi | I::Adx => {
                let (pdi, mdi, adx) = dmi(self.bars, n, 6);
                match id {
                    I::PlusDi => pdi,
                    I::MinusDi => mdi,
                    _ => adx,
                }
            }
            I::Supertrend => supertrend(self.bars, n, 3.0),
            I::Cci => cci(self.bars, n),
            I::BollWidth => {
                let (up, lo, mid) = (
                    self.get(IndicatorRef::new(I::BollUpper, None)),
                    self.get(IndicatorRef::new(I::BollLower, None)),
                    self.get(IndicatorRef::new(I::BollMid, None)),
                );
                (0..up.len())
                    .map(|i| (up[i] - lo[i]) / mid[i] * 100.0)
                    .collect()
            }
            I::MaSpread => {
                let mas: Vec<Rc<Vec<f64>>> = [5, 10, 20, 60]
                    .iter()
                    .map(|k| self.get(IndicatorRef::new(I::Ma, Some(*k))))
                    .collect();
                (0..self.len())
                    .map(|i| {
                        let v: Vec<f64> = mas.iter().map(|m| m[i]).collect();
                        if v.iter().any(|x| x.is_nan()) {
                            return f64::NAN;
                        }
                        let (hi, lo) = (
                            v.iter().copied().fold(f64::MIN, f64::max),
                            v.iter().copied().fold(f64::MAX, f64::min),
                        );
                        (hi - lo) / self.bars[i].q_close * 100.0
                    })
                    .collect()
            }
        }
    }

    // @project-doc docs/domains/strategies.md#limits
    /// 涨跌停价按交易所规则推算：涨跌幅限制按板块和 ST 确定，基准是前一交易日的收盘价（除权日按除权参考价），
    /// 四舍五入到分。上市后前 5 个交易日没有涨跌幅限制，不判断。
    ///
    /// 有权威池时，池覆盖内的日期以池里的成员为准（涨停、跌停、炸板、连板数），其他日期保留自算结果。
    /// 跌停、炸板还要求这一天出现在涨停池里：池缺一天的数据时，不会把当天所有股票都判成没有跌停
    pub fn limits(&self) -> Rc<Limits> {
        if let Some(l) = self.limits.borrow().as_ref() {
            return l.clone();
        }
        let b = self.bars;
        let n = b.len();
        let mut l = Limits {
            up_price: vec![f64::NAN; n],
            down_price: vec![f64::NAN; n],
            limit_up: vec![false; n],
            limit_down: vec![false; n],
            broken: vec![false; n],
            consecutive_up: vec![0; n],
        };
        if self
            .meta
            .is_some_and(|m| !crate::instrument::extension(m.asset_type).uses_stock_limit_pool())
        {
            let l = Rc::new(l);
            *self.limits.borrow_mut() = Some(l.clone());
            return l;
        }
        for i in 1..n {
            if self.ipo_period(i) {
                continue;
            }
            let pct = self.limit_pct(i);
            // 前复权价之比消去了除权：今天的原价 × 昨天和今天的前复权价之比 = 昨天的收盘价按除权调整后的值
            let base = b[i].close * b[i - 1].q_close / b[i].q_close;
            let up = round2(base * (1.0 + pct));
            let down = round2(base * (1.0 - pct));
            l.up_price[i] = up;
            l.down_price[i] = down;
            l.limit_up[i] = b[i].close >= up - 0.005;
            l.limit_down[i] = b[i].close <= down + 0.005;
            l.broken[i] = b[i].high >= up - 0.005 && !l.limit_up[i];
        }
        // 权威池。北交所不在池里（池里没有 %.BJ），回退自算；没有证券信息时也不知道板块，同样自算
        let authority = self
            .authority
            .filter(|_| self.meta.is_some_and(|m| m.board != "bse"));
        let empty = super::limit_pool::CodeLimits::default();
        let code_limits = authority.map(|(pool, code)| (pool, pool.code(code).unwrap_or(&empty)));
        let mut run = 0u32;
        let (mut ui, mut di, mut bi) = (0usize, 0usize, 0usize);
        for (i, bar) in b.iter().enumerate() {
            if self.ipo_period(i) {
                continue;
            }
            let date = bar.date;
            match code_limits {
                Some((pool, limits)) if pool.up.covers(date) => {
                    // 升序一次推进三个指针，不每根 K 线二分
                    while ui < limits.up.len() && limits.up[ui].0 < date {
                        ui += 1;
                    }
                    let count = limits
                        .up
                        .get(ui)
                        .filter(|(d, _)| *d == date)
                        .map_or(0, |(_, count)| *count);
                    l.limit_up[i] = count > 0;
                    l.consecutive_up[i] = count;
                    run = count;
                    if pool.down.covers(date) && pool.is_trading_day(date) {
                        while di < limits.down.len() && limits.down[di] < date {
                            di += 1;
                        }
                        l.limit_down[i] = limits.down.get(di) == Some(&date);
                    }
                    if pool.broken.covers(date) && pool.is_trading_day(date) {
                        while bi < limits.broken.len() && limits.broken[bi] < date {
                            bi += 1;
                        }
                        l.broken[i] = limits.broken.get(bi) == Some(&date);
                    }
                }
                _ => {
                    run = if l.limit_up[i] { run + 1 } else { 0 };
                    l.consecutive_up[i] = run;
                }
            }
        }
        let l = Rc::new(l);
        *self.limits.borrow_mut() = Some(l.clone());
        l
    }

    fn ipo_period(&self, i: usize) -> bool {
        match self.meta.and_then(|m| m.list_date) {
            // 本库第一根日线就是上市首日时，前 5 根没有涨跌幅限制
            Some(d) => self.bars[0].date <= d + chrono::Duration::days(3) && i < 5,
            None => false,
        }
    }

    fn limit_pct(&self, i: usize) -> f64 {
        let Some(m) = self.meta else { return 0.10 };
        let date = self.bars[i].date;
        let chinext_reform = NaiveDate::from_ymd_opt(2020, 8, 24).unwrap();
        match m.board.as_str() {
            "bse" => 0.30,
            "star" => 0.20,
            "chinext" if date >= chinext_reform => 0.20,
            _ if self.is_st_on(date) => 0.05,
            _ => 0.10,
        }
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// 指数均线，第一天的值取当天收盘
pub fn ema(values: &[f64], n: usize) -> Vec<f64> {
    let alpha = 2.0 / (n as f64 + 1.0);
    let mut out = Vec::with_capacity(values.len());
    let mut prev = f64::NAN;
    for &v in values {
        prev = if prev.is_nan() {
            v
        } else {
            alpha * v + (1.0 - alpha) * prev
        };
        out.push(prev);
    }
    out
}

/// 平滑移动平均 SMA(X, n, 1)：Y = (X + (n − 1) × Y') / n，与通达信、同花顺的 RSI、KDJ 口径一致
fn smooth(values: &[f64], n: usize, seed: f64) -> Vec<f64> {
    let mut prev = seed;
    values
        .iter()
        .map(|&v| {
            if v.is_nan() {
                return f64::NAN;
            }
            prev = if prev.is_nan() {
                v
            } else {
                (v + (n as f64 - 1.0) * prev) / n as f64
            };
            prev
        })
        .collect()
}

fn rsi(close: &[f64], n: usize) -> Vec<f64> {
    if close.is_empty() {
        return vec![];
    }
    let mut up = vec![f64::NAN];
    let mut abs = vec![f64::NAN];
    for i in 1..close.len() {
        let d = close[i] - close[i - 1];
        up.push(d.max(0.0));
        abs.push(d.abs());
    }
    let su = smooth(&up, n, f64::NAN);
    let sa = smooth(&abs, n, f64::NAN);
    su.iter()
        .zip(sa)
        .enumerate()
        .map(|(i, (u, a))| {
            if i < n {
                f64::NAN
            } else if a > 0.0 {
                u / a * 100.0
            } else {
                50.0
            }
        })
        .collect()
}

fn kdj(bars: &[DailyBar], n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let hi = rolling(
        &bars.iter().map(|b| b.q_high).collect::<Vec<_>>(),
        n,
        f64::max,
    );
    let lo = rolling(
        &bars.iter().map(|b| b.q_low).collect::<Vec<_>>(),
        n,
        f64::min,
    );
    let rsv: Vec<f64> = bars
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let range = hi[i] - lo[i];
            if hi[i].is_nan() {
                f64::NAN
            } else if range > 0.0 {
                (b.q_close - lo[i]) / range * 100.0
            } else {
                50.0
            }
        })
        .collect();
    let k = smooth(&rsv, 3, 50.0);
    let d = smooth(&k, 3, 50.0);
    let j = k
        .iter()
        .zip(d.iter())
        .map(|(k, d)| 3.0 * k - 2.0 * d)
        .collect();
    (k, d, j)
}

fn atr(bars: &[DailyBar], n: usize) -> Vec<f64> {
    sma(&true_range(bars), n)
        .into_iter()
        .map(|v| v.unwrap_or(f64::NAN))
        .collect()
}

/// 真实波幅：最高 − 最低、最高 − 昨收、昨收 − 最低中的最大值；第一天没有昨收，取最高 − 最低
fn true_range(bars: &[DailyBar]) -> Vec<f64> {
    (0..bars.len())
        .map(|i| {
            let b = &bars[i];
            if i == 0 {
                return b.q_high - b.q_low;
            }
            let pc = bars[i - 1].q_close;
            (b.q_high - b.q_low)
                .max((b.q_high - pc).abs())
                .max((b.q_low - pc).abs())
        })
        .collect()
}

/// 最近 n 个值（含当天）之和；不足 n 个或含 NaN 时为 NaN
fn rolling_sum(values: &[f64], n: usize) -> Vec<f64> {
    (0..values.len())
        .map(|i| {
            if i + 1 < n {
                f64::NAN
            } else {
                values[i + 1 - n..=i].iter().sum()
            }
        })
        .collect()
}

/// DMI，与通达信、同花顺的公式一致：
/// MTR = SUM(TR, n)；HD = 最高 − 昨最高，LD = 昨最低 − 最低；
/// DMP = SUM(HD > 0 且 HD > LD 时取 HD，否则 0, n)，DMM 同理；+DI = DMP × 100 ÷ MTR，−DI = DMM × 100 ÷ MTR；
/// ADX = MA(|−DI − +DI| ÷ (−DI + +DI) × 100, m)
fn dmi(bars: &[DailyBar], n: usize, m: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let len = bars.len();
    let mut tr = true_range(bars);
    let (mut dmp, mut dmm) = (vec![f64::NAN; len], vec![f64::NAN; len]);
    for i in 1..len {
        let hd = bars[i].q_high - bars[i - 1].q_high;
        let ld = bars[i - 1].q_low - bars[i].q_low;
        dmp[i] = if hd > 0.0 && hd > ld { hd } else { 0.0 };
        dmm[i] = if ld > 0.0 && ld > hd { ld } else { 0.0 };
    }
    // 第一天没有昨天的价格，三个序列都从第二天开始累计
    if let Some(t) = tr.first_mut() {
        *t = f64::NAN;
    }
    let (mtr, sp, sm) = (
        rolling_sum(&tr, n),
        rolling_sum(&dmp, n),
        rolling_sum(&dmm, n),
    );
    let pdi: Vec<f64> = (0..len)
        .map(|i| {
            if mtr[i] > 0.0 {
                sp[i] * 100.0 / mtr[i]
            } else {
                f64::NAN
            }
        })
        .collect();
    let mdi: Vec<f64> = (0..len)
        .map(|i| {
            if mtr[i] > 0.0 {
                sm[i] * 100.0 / mtr[i]
            } else {
                f64::NAN
            }
        })
        .collect();
    let dx: Vec<f64> = (0..len)
        .map(|i| {
            if pdi[i] + mdi[i] > 0.0 {
                (mdi[i] - pdi[i]).abs() / (mdi[i] + pdi[i]) * 100.0
            } else {
                f64::NAN
            }
        })
        .collect();
    let adx = (0..len)
        .map(|i| {
            if i + 1 < m || dx[i + 1 - m..=i].iter().any(|v| v.is_nan()) {
                f64::NAN
            } else {
                dx[i + 1 - m..=i].iter().sum::<f64>() / m as f64
            }
        })
        .collect();
    (pdi, mdi, adx)
}

/// Wilder 平滑（TradingView 的 RMA）：第一个值是前 n 个的简单平均，之后 Y = (X + (n − 1) × Y') ÷ n
fn rma(values: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; values.len()];
    if values.len() < n {
        return out;
    }
    let mut prev = values[..n].iter().sum::<f64>() / n as f64;
    out[n - 1] = prev;
    for i in n..values.len() {
        prev = (values[i] + (n as f64 - 1.0) * prev) / n as f64;
        out[i] = prev;
    }
    out
}

/// Supertrend，与 TradingView 的 ta.supertrend 一致：中价 ± mult × ATR(n)（Wilder 平滑），
/// 下轨只升不降（除非昨收跌破它）、上轨只降不升（除非昨收突破它）；下降趋势中收盘高于上轨转为上升，上升趋势中收盘低于下轨转为下降。
/// 上升趋势时取下轨，下降趋势时取上轨。开始时按下降趋势
fn supertrend(bars: &[DailyBar], n: usize, mult: f64) -> Vec<f64> {
    let atr = rma(&true_range(bars), n);
    let mut out = vec![f64::NAN; bars.len()];
    let (mut fu, mut fl, mut up) = (f64::NAN, f64::NAN, false);
    for i in 0..bars.len() {
        if atr[i].is_nan() {
            continue;
        }
        let b = &bars[i];
        let hl2 = (b.q_high + b.q_low) / 2.0;
        let (bu, bl) = (hl2 + mult * atr[i], hl2 - mult * atr[i]);
        if fu.is_nan() {
            (fu, fl) = (bu, bl);
        } else {
            let pc = bars[i - 1].q_close;
            fl = if bl > fl || pc < fl { bl } else { fl };
            fu = if bu < fu || pc > fu { bu } else { fu };
            up = if up { b.q_close >= fl } else { b.q_close > fu };
        }
        out[i] = if up { fl } else { fu };
    }
    out
}

/// CCI，与通达信、同花顺一致：(典型价 − MA(典型价, n)) ÷ (0.015 × AVEDEV(典型价, n))
fn cci(bars: &[DailyBar], n: usize) -> Vec<f64> {
    let tp: Vec<f64> = bars
        .iter()
        .map(|b| (b.q_high + b.q_low + b.q_close) / 3.0)
        .collect();
    (0..tp.len())
        .map(|i| {
            if i + 1 < n {
                return f64::NAN;
            }
            let w = &tp[i + 1 - n..=i];
            let mean = w.iter().sum::<f64>() / n as f64;
            let dev = w.iter().map(|v| (v - mean).abs()).sum::<f64>() / n as f64;
            if dev > 0.0 {
                (tp[i] - mean) / (0.015 * dev)
            } else {
                0.0
            }
        })
        .collect()
}

/// 最近 n 个值（含当天）的样本标准差
fn rolling_std(values: &[f64], n: usize) -> Vec<f64> {
    (0..values.len())
        .map(|i| {
            if i + 1 < n {
                return f64::NAN;
            }
            let w = &values[i + 1 - n..=i];
            let mean = w.iter().sum::<f64>() / n as f64;
            (w.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt()
        })
        .collect()
}

/// 最近 n 个值（含当天）按 f 归约，例如最高值、最低值
fn rolling(values: &[f64], n: usize, f: fn(f64, f64) -> f64) -> Vec<f64> {
    (0..values.len())
        .map(|i| {
            if i + 1 < n {
                f64::NAN
            } else {
                values[i + 1 - n..=i]
                    .iter()
                    .copied()
                    .reduce(f)
                    .unwrap_or(f64::NAN)
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn bars_from(closes: &[f64]) -> Vec<DailyBar> {
        let d0 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| DailyBar {
                date: d0 + chrono::Duration::days(i as i64),
                open: c,
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                q_open: c,
                q_high: c * 1.01,
                q_low: c * 0.99,
                q_close: c,
                volume: 1000.0 * (i as f64 + 1.0),
                amount: c * 1000.0 * (i as f64 + 1.0),
            })
            .collect()
    }

    #[test]
    fn moving_averages_and_momentum() {
        let b = bars_from(&[10.0, 11.0, 12.0, 13.0, 14.0]);
        let f = Frame::new(&b, None);
        let ma3 = f.get(IndicatorRef::new(IndicatorId::Ma, Some(3)));
        assert!(ma3[1].is_nan());
        assert!((ma3[4] - 13.0).abs() < 1e-12);
        assert!(
            (f.at(IndicatorRef::new(IndicatorId::Momentum, Some(2)), 4)
                - (14.0 / 12.0 - 1.0) * 100.0)
                .abs()
                < 1e-9
        );
        assert!((f.at(IndicatorRef::new(IndicatorId::ChangePct, None), 1) - 10.0).abs() < 1e-9);
        // 成交量 1000、2000、3000、4000、5000：第 5 天是前 3 天平均 3000 的 5/3 倍
        assert!(
            (f.at(IndicatorRef::new(IndicatorId::VolRatio, Some(3)), 4) - 5.0 / 3.0).abs() < 1e-9
        );
        assert!((f.at(IndicatorRef::new(IndicatorId::PctFromHigh, Some(3)), 4)).abs() < 1e-9);
    }

    #[test]
    fn rsi_is_100_when_only_rising_and_ema_starts_at_first_value() {
        let b = bars_from(&(1..=30).map(|v| v as f64).collect::<Vec<_>>());
        let f = Frame::new(&b, None);
        assert!((f.at(IndicatorRef::new(IndicatorId::Rsi, Some(14)), 29) - 100.0).abs() < 1e-9);
        assert_eq!(f.at(IndicatorRef::new(IndicatorId::Ema, Some(5)), 0), 1.0);
        assert!(
            f.at(IndicatorRef::new(IndicatorId::MacdDif, None), 29) > 0.0,
            "持续上涨时 DIF 为正"
        );
    }

    #[test]
    fn dmi_supertrend_and_cci() {
        // 先涨 40 天再跌 40 天
        let closes: Vec<f64> = (0..40)
            .map(|i| 10.0 + i as f64 * 0.2)
            .chain((0..40).map(|i| 18.0 - i as f64 * 0.2))
            .collect();
        let b = bars_from(&closes);
        let f = Frame::new(&b, None);
        let at = |id, n, i| f.at(IndicatorRef::new(id, n), i);
        // 上涨段：+DI 在 −DI 上方，趋势明确；下跌段相反
        assert!(at(IndicatorId::PlusDi, Some(14), 39) > at(IndicatorId::MinusDi, Some(14), 39));
        assert!(at(IndicatorId::MinusDi, Some(14), 79) > at(IndicatorId::PlusDi, Some(14), 79));
        assert!(
            at(IndicatorId::Adx, Some(14), 39) > 25.0 && at(IndicatorId::Adx, Some(14), 79) > 25.0
        );
        assert!(
            at(IndicatorId::PlusDi, Some(14), 13).is_nan(),
            "前 n 天算不出"
        );
        assert!(
            at(IndicatorId::Adx, Some(14), 18).is_nan()
                && !at(IndicatorId::Adx, Some(14), 19).is_nan(),
            "ADX 再晚 5 天"
        );
        // Supertrend：上涨时在股价下方，下跌时翻到上方
        assert!(at(IndicatorId::Supertrend, Some(10), 39) < closes[39]);
        assert!(at(IndicatorId::Supertrend, Some(10), 79) > closes[79]);
        // CCI：典型价匀速上涨时，最新一天在均值上方，(n − 1) / 2 个步长 ÷ (0.015 × 平均绝对偏差)
        let cci = at(IndicatorId::Cci, Some(5), 20);
        let step = 0.2;
        let mad = (2.0 + 1.0 + 0.0 + 1.0 + 2.0) * step / 5.0;
        assert!((cci - 2.0 * step / (0.015 * mad)).abs() < 1e-6, "{cci}");
        // 布林带宽度、均线粘合度
        let w = at(IndicatorId::BollWidth, None, 30);
        let (u, l, m) = (
            at(IndicatorId::BollUpper, None, 30),
            at(IndicatorId::BollLower, None, 30),
            at(IndicatorId::BollMid, None, 30),
        );
        assert!((w - (u - l) / m * 100.0).abs() < 1e-9);
        assert!(
            at(IndicatorId::MaSpread, None, 58).is_nan()
                && at(IndicatorId::MaSpread, None, 59) > 0.0
        );
    }

    #[test]
    fn limit_up_follows_board_rules() {
        let mut b = bars_from(&[10.0, 11.0, 12.1]);
        b[2].high = 12.1;
        let main = StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
            asset_type: AssetType::Stock,
        };
        let f = Frame::new(&b, Some(&main));
        let l = f.limits();
        assert!(l.limit_up[1] && l.limit_up[2], "主板涨 10% 封涨停");
        let st = StockMeta {
            board: "main".into(),
            is_st: true,
            list_date: None,
            asset_type: AssetType::Stock,
        };
        let f = Frame::new(&b, Some(&st));
        assert!((f.limits().up_price[1] - 10.5).abs() < 1e-9, "ST 涨跌幅 5%");
        let star = StockMeta {
            board: "star".into(),
            is_st: false,
            list_date: None,
            asset_type: AssetType::Stock,
        };
        let f = Frame::new(&b, Some(&star));
        assert!(!f.limits().limit_up[1], "科创板 20% 才涨停");
    }

    #[test]
    fn historical_st_period_changes_limit_rule() {
        let b = bars_from(&[10.0, 10.5, 11.55]);
        let main = StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
            asset_type: AssetType::Stock,
        };
        let history = [StPeriod {
            valid_from: b[2].date,
            valid_to: None,
            is_st: true,
        }];
        let f = Frame::with_st_history(&b, Some(&main), &history);
        let l = f.limits();
        assert!(
            (l.up_price[1] - 11.0).abs() < 1e-9,
            "历史区间外回退当前简称前仍应按主板 10%"
        );
        assert!(
            (l.up_price[2] - 11.03).abs() < 1e-9,
            "进入 ST 区间后应按 5%"
        );
    }

    fn meta(board: &str, is_st: bool) -> StockMeta {
        StockMeta {
            board: board.into(),
            is_st,
            list_date: None,
            asset_type: AssetType::Stock,
        }
    }

    /// 主板 ST：自算按 5% 判断，但 2026-07-06 起涨跌幅限制改为 10%，涨 7% 不是涨停。
    /// 权威池覆盖这一天时以池为准；没在池里就是没有涨停，不能再用自算的 5% 误判
    #[test]
    fn authority_pool_decides_main_board_st_limit_up() {
        let mut b = bars_from(&[10.0, 10.7, 11.0]);
        b[1].high = 10.7;
        b[2].high = 11.0;
        let st = meta("main", true);
        // 老的自算结果：涨 7% 被当成涨停（5% 的限制）
        assert!(
            Frame::new(&b, Some(&st)).limits().limit_up[1],
            "自算按 5% 判断，涨 7% 算涨停"
        );
        // 权威池覆盖 10-02 这一天，但这只股票不在涨停名单里
        let mut pool = LimitPool::empty();
        pool.push_up("600000.SH", b[1].date, 1);
        let f = Frame::with_authority(&b, Some(&st), &pool, "600519.SH");
        let l = f.limits();
        assert!(!l.limit_up[1], "权威名单里没有它，就不是涨停");
        assert!((l.up_price[1] - 10.5).abs() < 1e-9, "涨停价仍然输出自算值");
        // 涨 10%、在权威名单里（连板数 2）；前一天名单里没有它
        let mut pool = LimitPool::empty();
        pool.push_up("600000.SH", b[1].date, 1);
        pool.push_up("600519.SH", b[2].date, 2);
        let f = Frame::with_authority(&b, Some(&st), &pool, "600519.SH");
        let l = f.limits();
        assert!(l.limit_up[2], "ST 涨 10% 在名单里，是涨停");
        assert_eq!(l.consecutive_up[2], 2);
        assert_eq!(l.consecutive_up[1], 0, "前一天不在名单里，连板数为 0");
    }

    /// 跌停、炸板同样以名单为准；要求这一天出现在涨停池里（池缺数据时不把全部股票判成没有）
    #[test]
    fn authority_pool_decides_limit_down_and_broken() {
        // 自算：ST 跌 5% 是跌停；第 3 天盘中冲到 10.0（超过按 9.5 算出的 5% 涨停价 9.975），收盘只有 9.6，是炸板
        let mut b = bars_from(&[10.0, 9.5, 9.6]);
        b[2].high = 10.0;
        let st = meta("main", true);
        let self_limits = Frame::new(&b, Some(&st)).limits();
        assert!(
            self_limits.limit_down[1] && self_limits.broken[2],
            "自算结果"
        );
        // 权威池：只想说明这两个池覆盖了这三天（用别的股票登记），目标股票不在名单里
        let mut pool = LimitPool::empty();
        pool.push_up("600000.SH", b[1].date, 1);
        pool.push_up("600000.SH", b[2].date, 1);
        pool.push_down("600000.SH", b[1].date);
        pool.push_broken("600000.SH", b[2].date);
        let f = Frame::with_authority(&b, Some(&st), &pool, "600519.SH");
        let l = f.limits();
        assert!(
            !l.limit_down[1] && !l.broken[2],
            "权威名单里没有它，就不是跌停、炸板"
        );
        // 在名单里就是
        let mut pool = LimitPool::empty();
        pool.push_up("600519.SH", b[1].date, 1);
        pool.push_up("600519.SH", b[2].date, 1);
        pool.push_down("600519.SH", b[1].date);
        pool.push_broken("600519.SH", b[2].date);
        let f = Frame::with_authority(&b, Some(&st), &pool, "600519.SH");
        let l = f.limits();
        assert!(l.limit_down[1] && l.broken[2], "在名单里就是跌停、炸板");
    }

    /// 池没有覆盖的日期、北交所、没有证券信息时回退自算，结果与 Frame::new 逐位一致
    #[test]
    fn authority_falls_back_outside_coverage_and_for_bse() {
        let b = bars_from(&[10.0, 11.0, 12.1, 13.31]);
        let main = meta("main", false);
        // 池只覆盖第 2 天的涨停（第 1 天在 from 之前、第 3 天在 through 之后）
        let mut pool = LimitPool::empty();
        pool.push_up("600000.SH", b[1].date, 1);
        let f = Frame::with_authority(&b, Some(&main), &pool, "600519.SH");
        let self_l = Frame::new(&b, Some(&main)).limits();
        let l = f.limits();
        assert_eq!(
            (l.limit_up[0], l.limit_up[2], l.limit_up[3]),
            (self_l.limit_up[0], self_l.limit_up[2], self_l.limit_up[3]),
            "覆盖外回退自算"
        );
        assert!(!l.limit_up[1], "覆盖内、名单里没有就不是涨停");
        // 覆盖内的日期不再从自算的连续涨停累加：名单里没有，连板数为 0
        assert_eq!(l.consecutive_up[1], 0);
        // 第 3 天在 through 之后，回退自算的连续天数从 1 重新数
        assert_eq!(l.consecutive_up[2], 1);
        assert_eq!(l.consecutive_up[3], 2);
        // 北交所不在池里：即使池覆盖这一天，也回退自算（30%）
        let bse = meta("bse", false);
        let mut pool = LimitPool::empty();
        pool.push_up("600000.SH", b[2].date, 1);
        pool.push_up("920001.BJ", b[2].date, 1);
        let f = Frame::with_authority(&b, Some(&bse), &pool, "920001.BJ");
        let self_l = Frame::new(&b, Some(&bse)).limits();
        assert_eq!(f.limits().limit_up, self_l.limit_up);
        // 没有证券信息时不知道板块，也回退自算
        let f = Frame::with_authority(&b, None, &pool, "920001.BJ");
        assert_eq!(f.limits().limit_up, Frame::new(&b, None).limits().limit_up);
    }

    /// 数据起点就是第 5 板时，供应商的连板数让「连续 3 个涨停」在第一天就成立
    #[test]
    fn authority_continue_count_is_not_truncated_at_data_start() {
        use crate::analysis::signals::{self, Params, SignalId};
        let b = bars_from(&[10.0, 11.0, 12.1, 13.31, 14.64]);
        let main = meta("main", false);
        let mut pool = LimitPool::empty();
        pool.push_up("600519.SH", b[0].date, 5);
        let f = Frame::with_authority(&b, Some(&main), &pool, "600519.SH");
        let runs = f.limits().consecutive_up.clone();
        assert_eq!(runs[0], 5);
        let mut p = Params::new();
        p.insert("n".into(), 3.0);
        let series = signals::series(&f, SignalId::ConsecutiveLimitUp, &p);
        assert!(series[0], "第 5 板的第一天就满足「连续 3 个涨停」");
        // 没有池时从数据起点重新数，第一天不算
        let self_f = Frame::new(&b, Some(&main));
        assert!(!signals::series(&self_f, SignalId::ConsecutiveLimitUp, &p)[0]);
    }
}
