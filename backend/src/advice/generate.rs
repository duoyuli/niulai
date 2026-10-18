//! 生成一份调仓建议：纯函数，输入是账户快照、账户规则、当天的巡检结果、策略过去表现、持仓股和候选股的状态、市场概况，
//! 同样的输入总是得到同样的输出。每条建议写清产生它的规则、支持的数字、反面的信息和怎么做。
//! 规则见 docs/domains/accounts.md#generation。

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::stats::StrategyStat;
use crate::account::lots::{self, Fees, round2};
use crate::account::rules::{Rules, WeakMarket};

/// 生成规则的版本：规则改了就加一，历史建议记着当时的版本
pub const RULES_VERSION: i32 = 1;
/// 「没选上的候选」最多列几只
const MAX_SKIPPED: usize = 8;

/// 一只股票在建议交易日的状态（价格是未复权价；最新一段的前复权价等于原价，均线可以直接比较）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Snapshot {
    pub close: f64,
    pub change_pct: Option<f64>,
    /// 当天成交额（元）
    pub amount: f64,
    /// 规则的趋势均线（例如 20 日均线）；日线不够时为空
    pub ma_trend: Option<f64>,
    pub rsi: Option<f64>,
    /// 近 20 个交易日涨跌（%）
    pub momentum20: Option<f64>,
    /// 在近 60 个交易日价格区间中的位置（0 到 1）
    pub range60_pos: Option<f64>,
    /// 诊断的趋势和量能结论（一句话），以及趋势是否偏强 / 偏弱
    pub trend_text: String,
    pub trend_strong: bool,
    pub trend_weak: bool,
    pub volume_text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StrategyRef {
    pub strategy_id: i64,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HoldingIn {
    pub thscode: String,
    pub name: String,
    pub board: String,
    pub shares: i64,
    pub cost_price: Option<f64>,
    pub opened_on: Option<NaiveDate>,
    /// 从买入日到建议交易日的交易日数；不知道买入日时为空
    pub bars_held: Option<i64>,
    /// 按哪个策略的建议买的
    pub strategy: Option<StrategyRef>,
    /// 买入依据的策略今天满足了卖出条件：条件的白话
    pub strategy_exit: Option<String>,
    pub is_st: bool,
    pub delisted: bool,
    /// 最近一根日线的日期；早于建议交易日表示停牌
    pub last_date: Option<NaiveDate>,
    /// 没有日线时为空
    pub snap: Option<Snapshot>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pick {
    pub strategy_id: i64,
    /// 命中条件的白话
    pub reasons: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CandidateIn {
    pub thscode: String,
    pub name: String,
    pub board: String,
    pub is_st: bool,
    /// 今天选中它的策略（账户参考的那些）
    pub picks: Vec<Pick>,
    pub snap: Snapshot,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StrategyIn {
    pub strategy_id: i64,
    pub name: String,
    pub stat: Option<StrategyStat>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct MarketIn {
    /// 收盘站上 20 日均线的股票比例（%）
    pub above_ma20_pct: f64,
    pub stocks: usize,
}

/// 生成一份建议需要的全部输入；原样保存在 `app.advice.input` 中
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Input {
    pub trade_date: NaiveDate,
    /// 操作日：下一个交易日
    pub next_date: NaiveDate,
    pub rules: Rules,
    pub fees: Fees,
    pub cash: f64,
    pub market: Option<MarketIn>,
    pub holdings: Vec<HoldingIn>,
    pub candidates: Vec<CandidateIn>,
    /// 参考的策略（全部参加巡检的，或账户勾选的）和它们的过去表现
    pub strategies: Vec<StrategyIn>,
    /// 之前连续几份建议都给出了同一条卖出或减仓、且没有执行：键是 item key
    pub streaks: BTreeMap<String, u32>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
#[schema(as = AdviceAction)]
pub enum Action {
    Sell,
    Reduce,
    Buy,
    Hold,
    /// 停牌、退市、没有日线：现在做不了判断
    Wait,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Action::Sell => "sell",
            Action::Reduce => "reduce",
            Action::Buy => "buy",
            Action::Hold => "hold",
            Action::Wait => "wait",
        }
    }
}

/// 一条理由或提醒
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[schema(as = AdviceNote)]
pub struct Note {
    /// price / position / diagnosis / strategy / stat / market / risk
    pub kind: String,
    pub text: String,
}

fn note(kind: &str, text: impl Into<String>) -> Note {
    Note { kind: kind.into(), text: text.into() }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[schema(as = AdviceItem)]
pub struct Item {
    /// 例：sell-000858.SZ；执行、不做都按它记
    pub key: String,
    pub action: Action,
    pub thscode: String,
    pub name: String,
    /// 卖出、减仓、买入的股数；继续拿和不动时是持有的股数
    pub shares: i64,
    /// 参考价：建议交易日的收盘价
    pub ref_price: f64,
    /// 参考金额 = 股数 × 参考价
    pub amount: f64,
    /// 估算的费用（佣金、印花税）
    pub fee: f64,
    /// 一句话结论
    pub headline: String,
    /// 产生这个动作的规则原文
    pub rule: String,
    /// 支持这个结论的数字
    pub evidence: Vec<Note>,
    /// 反面的信息：有就必须显示
    pub cautions: Vec<Note>,
    /// 怎么做、什么时候结论会变
    pub plan: Vec<String>,
    /// 买入时：选中它的策略（排第一的）；执行后记在持仓上，以后按它的卖出条件判断
    pub strategy_id: Option<i64>,
    pub stop_price: Option<f64>,
    /// 连续第几天给出这条卖出或减仓
    pub streak: u32,
    /// 现在占总资产的百分比（买入时是买入后的占比）
    pub weight_pct: f64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[schema(as = SkippedCandidate)]
pub struct Skipped {
    pub thscode: String,
    pub name: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[schema(as = AdviceWeight)]
pub struct Weight {
    pub thscode: String,
    pub name: String,
    pub weight_pct: f64,
}

/// 照着建议做完之后的预计
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[schema(as = AdviceAfter)]
pub struct After {
    pub cash: f64,
    pub cash_pct: f64,
    pub positions: usize,
    pub weights: Vec<Weight>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct Advice {
    pub trade_date: NaiveDate,
    pub next_date: NaiveDate,
    /// 一句话结论：「今天不用操作」或「卖出 1 只、买入 2 只」
    pub headline: String,
    /// 不用操作时的原因
    pub why_nothing: Option<String>,
    pub total_assets: f64,
    pub cash: f64,
    pub market_value: f64,
    /// 市场环境的一句话
    pub market: Option<String>,
    /// 卖出、减仓、买入、继续拿、不动，按这个顺序
    pub items: Vec<Item>,
    /// 今天有买点、但没进建议的候选和原因
    pub skipped: Vec<Skipped>,
    pub after: After,
    /// 需要用户知道的事，例如有持仓没填成本价
    pub notes: Vec<String>,
}

pub fn yuan(v: f64) -> String {
    let neg = v < 0.0;
    let n = v.abs().round() as i64;
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}¥{out}", if neg { "-" } else { "" })
}

// @project-doc docs/domains/accounts.md#generation
pub fn generate(inp: &Input) -> Advice {
    let p = &inp.rules;
    let price_of = |h: &HoldingIn| h.snap.as_ref().map(|s| s.close);
    let market_value: f64 = inp.holdings.iter().map(|h| h.shares as f64 * price_of(h).unwrap_or(0.0)).sum();
    let total = inp.cash + market_value;
    let weak = inp.market.is_some_and(|m| m.above_ma20_pct < p.weak_threshold_pct);
    let market_text = inp.market.map(|m| {
        let verdict = if weak { format!("低于 {:.0}%，算弱市", p.weak_threshold_pct) } else { format!("不低于 {:.0}%，不算弱市", p.weak_threshold_pct) };
        format!("全市场 {:.0}% 的股票收盘站上 20 日均线，{verdict}", m.above_ma20_pct)
    });

    // 第 1 步：每只持仓下结论
    let mut items: Vec<Item> = inp.holdings.iter().map(|h| judge_holding(inp, h, total)).collect();
    let sold: HashSet<&str> = items.iter().filter(|i| i.action == Action::Sell).map(|i| i.thscode.as_str()).collect();
    let proceeds: f64 = items.iter().filter(|i| matches!(i.action, Action::Sell | Action::Reduce)).map(|i| i.amount - i.fee).sum();
    let remaining = items.iter().filter(|i| i.action != Action::Sell).count();

    // 第 2 步：明天能用的钱；第 3、4 步：挑候选、分配
    let reserve = total * p.min_cash_pct / 100.0;
    let mut available = inp.cash + proceeds - reserve;
    let mut slots = p.max_positions as i64 - remaining as i64;
    let held: HashSet<&str> = inp.holdings.iter().map(|h| h.thscode.as_str()).collect();
    let stats: HashMap<i64, &StrategyIn> = inp.strategies.iter().map(|s| (s.strategy_id, s)).collect();
    let excess = |id: i64| stats.get(&id).and_then(|s| s.stat.as_ref()).and_then(|s| s.excess20_pct());
    // 有效策略：过去 3 年买点之后 20 日平均比随便买多涨，超过账户的门槛；还没有统计的先算有效
    let effective = |id: i64| p.effective(excess(id));
    let name_of = |id: i64| stats.get(&id).map_or_else(|| format!("策略 {id}"), |s| s.name.clone());

    let mut skipped: Vec<Skipped> = vec![];
    let skip = |skipped: &mut Vec<Skipped>, c: &CandidateIn, reason: String| skipped.push(Skipped { thscode: c.thscode.clone(), name: c.name.clone(), reason });
    // (候选, 采纳的策略, 被排除的策略)
    let mut ranked: Vec<(&CandidateIn, Vec<&Pick>, Vec<&Pick>)> = vec![];
    for c in &inp.candidates {
        if held.contains(c.thscode.as_str()) || sold.contains(c.thscode.as_str()) {
            continue;
        }
        if c.is_st {
            skip(&mut skipped, c, "ST 股不买".into());
            continue;
        }
        let (kept, dropped): (Vec<&Pick>, Vec<&Pick>) = c.picks.iter().filter(|k| p.uses(k.strategy_id)).partition(|k| effective(k.strategy_id));
        let names = |l: &[&Pick]| l.iter().map(|k| format!("「{}」", name_of(k.strategy_id))).collect::<Vec<_>>().join("、");
        if kept.is_empty() {
            if !dropped.is_empty() {
                skip(&mut skipped, c, format!("只被不够有效门槛的策略选中（{}，门槛是比随便买多涨 {}%）", names(&dropped), p.min_edge_pct));
            }
            continue;
        }
        if (kept.len() as u32) < p.min_picks {
            skip(&mut skipped, c, format!("只有 {} 个有效策略选中（{}），本账户要求至少 {} 个", kept.len(), names(&kept), p.min_picks));
            continue;
        }
        ranked.push((c, kept, dropped));
    }
    // 优先级：同时选中的有效策略数 → 它们的超额之和 → 成交额
    let edge_sum = |picks: &[&Pick]| picks.iter().filter_map(|k| excess(k.strategy_id)).sum::<f64>();
    ranked.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| edge_sum(&b.1).total_cmp(&edge_sum(&a.1)))
            .then_with(|| b.0.snap.amount.total_cmp(&a.0.snap.amount))
            .then_with(|| a.0.thscode.cmp(&b.0.thscode))
    });

    let pause = weak && p.weak_market == WeakMarket::Pause;
    let half = weak && p.weak_market == WeakMarket::Half;
    // 弱市减半：能用于买新股票的钱整体减半，每只分到的金额仍然差不多
    if half {
        available *= 0.5;
    }
    let mut buys: Vec<Item> = vec![];
    let (mut full, mut no_money) = (false, false);
    for (c, kept, dropped) in &ranked {
        if pause {
            skip(&mut skipped, c, "弱市：本账户的规则是暂停买入新股票".into());
            continue;
        }
        if slots <= 0 {
            full = true;
            skip(&mut skipped, c, format!("仓位已满（本账户最多持有 {} 只）", p.max_positions));
            continue;
        }
        if available <= 1.0 {
            no_money = true;
            skip(&mut skipped, c, format!("可用的钱用完了（要保留总资产的 {:.0}% 作为现金）", p.min_cash_pct));
            continue;
        }
        let target = (available / slots as f64).min(total * p.max_weight_pct / 100.0);
        let close = c.snap.close;
        let rule = lots::lot_rule(&c.board);
        let shares = lots::max_buy_shares(rule, target, close, &inp.fees);
        if shares == 0 {
            no_money = true;
            let need = rule.min as f64 * close;
            skip(&mut skipped, c, format!("钱不够买最少 {} 股（约 {}，这只最多能投 {}）", rule.min, yuan(need + inp.fees.buy_fee(need)), yuan(target)));
            continue;
        }
        let amount = round2(shares as f64 * close);
        let fee = inp.fees.buy_fee(amount);
        available -= amount + fee;
        slots -= 1;
        buys.push(buy_item(inp, c, kept, dropped, shares, amount, fee, buys.len() + 1, total, weak, half, &name_of, &excess, market_text.as_deref()));
    }
    skipped.truncate(MAX_SKIPPED);

    // 第 5 步：汇总
    let buy_cost: f64 = buys.iter().map(|b| b.amount + b.fee).sum();
    let cash_after = round2(inp.cash + proceeds - buy_cost);
    let mut weights: Vec<Weight> = vec![];
    for it in &items {
        let left = match it.action {
            Action::Sell => 0,
            Action::Reduce => inp.holdings.iter().find(|h| h.thscode == it.thscode).map_or(0, |h| h.shares - it.shares),
            _ => it.shares,
        };
        if left > 0 {
            weights.push(Weight { thscode: it.thscode.clone(), name: it.name.clone(), weight_pct: left as f64 * it.ref_price / total.max(1.0) * 100.0 });
        }
    }
    weights.extend(buys.iter().map(|b| Weight { thscode: b.thscode.clone(), name: b.name.clone(), weight_pct: b.weight_pct }));
    weights.sort_by(|a, b| b.weight_pct.total_cmp(&a.weight_pct));
    let after = After { cash: cash_after, cash_pct: cash_after / total.max(1.0) * 100.0, positions: weights.len(), weights };

    let n_sell = items.iter().filter(|i| i.action == Action::Sell).count();
    let n_reduce = items.iter().filter(|i| i.action == Action::Reduce).count();
    let n_buy = buys.len();
    let (headline, why_nothing) = if n_sell + n_reduce + n_buy == 0 {
        let holding_part = if inp.holdings.is_empty() {
            "现在空仓".to_string()
        } else if items.iter().any(|i| i.action == Action::Wait) {
            "持仓都没有触发卖出规则（停牌的复牌后再判断）".to_string()
        } else {
            format!("{} 只持仓都还没有触发卖出规则", inp.holdings.len())
        };
        let buy_part = if pause && !ranked.is_empty() {
            "全市场偏弱，本账户的规则是暂停买入".to_string()
        } else if ranked.is_empty() {
            "今天没有符合本账户规则的新买点".to_string()
        } else if full {
            format!("仓位已满（最多 {} 只）", p.max_positions)
        } else if no_money {
            "可用的钱不够买入新的股票".to_string()
        } else {
            "没有要买入的".to_string()
        };
        ("今天不用操作".to_string(), Some(format!("{holding_part}；{buy_part}。")))
    } else {
        let parts: Vec<String> = [(n_sell, "卖出"), (n_reduce, "减仓"), (n_buy, "买入")].iter().filter(|(n, _)| *n > 0).map(|(n, w)| format!("{w} {n} 只")).collect();
        (parts.join("、"), None)
    };

    let used: Vec<&StrategyIn> = inp.strategies.iter().filter(|s| p.uses(s.strategy_id)).collect();
    let good = used.iter().filter(|s| effective(s.strategy_id)).count();
    let mut notes = vec![format!(
        "参考 {} 个策略，其中 {good} 个过去 3 年买点之后 20 个交易日平均比随便买多涨 {}% 以上（有效策略），只有它们选中的股票才考虑买入",
        used.len(),
        p.min_edge_pct
    )];
    let missing_cost = inp.holdings.iter().filter(|h| h.cost_price.is_none()).count();
    if missing_cost > 0 {
        notes.push(format!("{missing_cost} 只持仓没填成本价，不能按止损线判断，可以在持仓页补上"));
    }
    if inp.strategies.iter().any(|s| s.stat.is_none()) {
        notes.push("部分策略的过去表现还在统计中（每周统计一次）；统计出来之前先按有效策略算".into());
    }

    // 卖出、减仓、买入、继续拿、不动
    let order = |a: Action| match a {
        Action::Sell => 0,
        Action::Reduce => 1,
        Action::Buy => 2,
        Action::Hold => 3,
        Action::Wait => 4,
    };
    items.extend(buys);
    items.sort_by_key(|i| order(i.action));
    Advice {
        trade_date: inp.trade_date,
        next_date: inp.next_date,
        headline,
        why_nothing,
        total_assets: round2(total),
        cash: inp.cash,
        market_value: round2(market_value),
        market: market_text,
        items,
        skipped,
        after,
        notes,
    }
}

fn judge_holding(inp: &Input, h: &HoldingIn, total: f64) -> Item {
    let p = &inp.rules;
    let base = |action: Action, headline: String, rule: String| Item {
        key: format!("{}-{}", action.key(), h.thscode),
        action,
        thscode: h.thscode.clone(),
        name: h.name.clone(),
        shares: h.shares,
        ref_price: h.snap.as_ref().map_or(0.0, |s| s.close),
        amount: 0.0,
        fee: 0.0,
        headline,
        rule,
        evidence: vec![],
        cautions: vec![],
        plan: vec![],
        strategy_id: h.strategy.as_ref().map(|s| s.strategy_id),
        stop_price: None,
        streak: 0,
        weight_pct: 0.0,
    };
    let Some(s) = h.snap.as_ref() else {
        return base(Action::Wait, "本地还没有这只股票的日线，暂时判断不了".into(), "没有行情时不给建议".into());
    };
    if h.delisted {
        return base(Action::Wait, "已经退市".into(), "退市的股票不能在交易所正常买卖，按券商的退市安排处理".into());
    }
    if h.last_date.is_some_and(|d| d < inp.trade_date) {
        let mut it = base(Action::Wait, format!("停牌中（最后交易日 {}），复牌后再判断", h.last_date.unwrap_or(inp.trade_date)), "停牌时不能交易".into());
        it.weight_pct = h.shares as f64 * s.close / total.max(1.0) * 100.0;
        return it;
    }
    let close = s.close;
    let value = h.shares as f64 * close;
    let weight = value / total.max(1.0) * 100.0;
    let ret = h.cost_price.map(|c| close / c - 1.0);
    let stop_price = h.cost_price.map(|c| c * (1.0 - p.stop_loss_pct / 100.0));

    let mut evidence = vec![];
    if let (Some(c), Some(r)) = (h.cost_price, ret) {
        let pnl = (close - c) * h.shares as f64;
        let held = h.bars_held.map(|n| format!("，已持有 {n} 个交易日")).unwrap_or_default();
        evidence.push(note("position", format!("成本 {c:.2} 元，现价 {close:.2} 元，{} {:.1}%（{}）{held}", if r >= 0.0 { "赚" } else { "亏" }, (r * 100.0).abs(), yuan(pnl.abs()))));
    }
    if let Some(ma) = s.ma_trend {
        let gap = (close / ma - 1.0) * 100.0;
        evidence.push(note("price", format!("收盘 {close:.2} 元，{} 日均线 {ma:.2} 元，{}{:.1}%", p.trend_ma, if gap >= 0.0 { "高出 " } else { "低了 " }, gap.abs())));
    }
    if !s.trend_text.is_empty() {
        evidence.push(note("diagnosis", format!("趋势：{}", s.trend_text)));
    }
    evidence.push(note("position", format!("占总资产 {weight:.1}%")));
    let mut cautions = vec![];
    if let Some(rsi) = s.rsi
        && rsi <= 30.0
    {
        cautions.push(note("diagnosis", format!("RSI(14) 为 {rsi:.0}，已经超卖，短期可能反弹")));
    }

    let sell = |headline: String, rule: String, evidence: Vec<Note>, cautions: Vec<Note>| {
        let amount = round2(value);
        let key = format!("sell-{}", h.thscode);
        let streak = inp.streaks.get(&key).copied().unwrap_or(0) + 1;
        let mut ev = evidence;
        if streak > 1 {
            ev.insert(0, note("rule", format!("已连续 {streak} 天建议卖出，还没有执行")));
        }
        Item {
            amount,
            fee: inp.fees.sell_fee(amount),
            evidence: ev,
            cautions,
            plan: vec![format!("下一个交易日（{}）开盘后卖出全部 {} 股", inp.next_date, h.shares), "开盘一字跌停卖不出时，下一份建议会继续提示".into()],
            stop_price,
            streak,
            weight_pct: weight,
            ..base(Action::Sell, headline, rule)
        }
    };

    if let (Some(r), Some(c)) = (ret, h.cost_price)
        && r <= -p.stop_loss_pct / 100.0
    {
        return sell(
            format!("亏损 {:.1}%，超过止损线 {}%", -r * 100.0, p.stop_loss_pct),
            format!("本账户的规则：比成本价亏 {}% 就卖出（止损价 {:.2} 元）", p.stop_loss_pct, c * (1.0 - p.stop_loss_pct / 100.0)),
            evidence,
            cautions,
        );
    }
    if let (Some(text), Some(st)) = (&h.strategy_exit, &h.strategy) {
        return sell(format!("买入依据「{}」的卖出条件满足：{text}", st.name), format!("按「{}」的买入建议买的，就按它的卖出条件卖出：{text}", st.name), evidence, cautions);
    }
    if let Some(ma) = s.ma_trend
        && close < ma
    {
        return sell(
            format!("收盘 {close:.2} 元跌破 {} 日均线 {ma:.2} 元", p.trend_ma),
            format!("本账户的规则：收盘跌破 {} 日均线就卖出", p.trend_ma),
            evidence,
            cautions,
        );
    }
    if h.is_st && p.sell_st {
        return sell("被实施风险警示（ST）".into(), "本账户的规则：不持有 ST 股".into(), evidence, cautions);
    }
    if weight > p.max_weight_pct + 0.5 {
        let target = total * p.max_weight_pct / 100.0;
        let want = ((value - target) / close).ceil() as i64;
        let n = lots::reduce_shares(lots::lot_rule(&h.board), h.shares, want);
        if n > 0 {
            let amount = round2(n as f64 * close);
            let key = format!("reduce-{}", h.thscode);
            let streak = inp.streaks.get(&key).copied().unwrap_or(0) + 1;
            let after_pct = (h.shares - n) as f64 * close / total.max(1.0) * 100.0;
            let mut ev = evidence;
            if streak > 1 {
                ev.insert(0, note("rule", format!("已连续 {streak} 天建议减仓，还没有执行")));
            }
            return Item {
                shares: n,
                amount,
                fee: inp.fees.sell_fee(amount),
                evidence: ev,
                cautions,
                plan: vec![format!("下一个交易日（{}）卖出 {n} 股，剩下 {} 股，约占总资产 {after_pct:.0}%", inp.next_date, h.shares - n)],
                stop_price,
                streak,
                weight_pct: weight,
                ..base(
                    Action::Reduce,
                    format!("占总资产 {weight:.0}%，超过单只上限 {}%，减到约 {after_pct:.0}%", p.max_weight_pct),
                    format!("本账户的规则：单只最多占总资产的 {}%，涨多了要减回去，避免一只股票拖累整个账户", p.max_weight_pct),
                )
            };
        }
    }

    // 继续拿：写出会让结论变成卖出的价格
    let mut triggers = vec![];
    if let Some(ma) = s.ma_trend {
        triggers.push(format!("收盘跌破 {ma:.2} 元（{} 日均线）", p.trend_ma));
    }
    if let Some(sp) = stop_price {
        triggers.push(format!("亏到 {sp:.2} 元（止损价）"));
    }
    let headline = match s.ma_trend {
        Some(ma) => format!("收盘仍在 {} 日均线上方 {:.1}%", p.trend_ma, (close / ma - 1.0) * 100.0),
        None => "还没有触发卖出规则".into(),
    };
    if h.is_st {
        cautions.push(note("risk", "已被实施风险警示（ST），风险较大"));
    }
    let mut plan = vec![];
    if !triggers.is_empty() {
        plan.push(format!("{}时会建议卖出", triggers.join("，或")));
    }
    if let Some(st) = &h.strategy {
        plan.push(format!("买入依据「{}」的卖出条件满足时也会建议卖出", st.name));
    }
    Item {
        evidence,
        cautions,
        plan,
        stop_price,
        weight_pct: weight,
        ..base(Action::Hold, headline, "本账户的卖出规则（止损、趋势、ST、单只上限）一条都没有触发".into())
    }
}

#[allow(clippy::too_many_arguments)]
fn buy_item(
    inp: &Input,
    c: &CandidateIn,
    kept: &[&Pick],
    dropped: &[&Pick],
    shares: i64,
    amount: f64,
    fee: f64,
    rank: usize,
    total: f64,
    weak: bool,
    half: bool,
    name_of: &dyn Fn(i64) -> String,
    excess: &dyn Fn(i64) -> Option<f64>,
    market: Option<&str>,
) -> Item {
    let p = &inp.rules;
    let s = &c.snap;
    let close = s.close;
    let names: Vec<String> = kept.iter().map(|k| format!("「{}」", name_of(k.strategy_id))).collect();
    let edges: Vec<f64> = kept.iter().filter_map(|k| excess(k.strategy_id)).collect();
    let edge = if edges.is_empty() { String::new() } else { format!("，过去 3 年比随便买合计多涨 {:.1}%", edges.iter().sum::<f64>()) };
    let together = if kept.len() > 1 { format!("，{} 个有效策略同时选中", kept.len()) } else { "选中".to_string() };
    let headline = format!("{}{together}{edge}，排第 {rank}", names.join(" + "));

    let mut evidence: Vec<Note> = kept.iter().map(|k| note("strategy", format!("{}：{}", format_name(&name_of(k.strategy_id)), k.reasons.join("，")))).collect();
    let mut pending = vec![];
    for k in kept {
        let st = inp.strategies.iter().find(|x| x.strategy_id == k.strategy_id);
        match st.and_then(|x| x.stat.as_ref()) {
            Some(stat) => evidence.push(note("stat", stat.describe(&name_of(k.strategy_id)))),
            None => pending.push(format_name(&name_of(k.strategy_id))),
        }
    }
    if !pending.is_empty() {
        evidence.push(note("stat", format!("{}的过去表现还在统计中", pending.join("、"))));
    }
    if !s.trend_text.is_empty() {
        evidence.push(note("diagnosis", format!("趋势：{}", s.trend_text)));
    }
    if !s.volume_text.is_empty() {
        evidence.push(note("diagnosis", format!("量能：{}", s.volume_text)));
    }
    if let Some(m) = market {
        evidence.push(note("market", m));
    }

    let mut cautions = vec![];
    if let Some(rsi) = s.rsi
        && rsi >= 70.0
    {
        cautions.push(note("diagnosis", format!("RSI(14) 为 {rsi:.0}，短期涨得较多（超买）")));
    }
    if let Some(pos) = s.range60_pos
        && pos >= 0.9
    {
        cautions.push(note("diagnosis", format!("处在近 60 个交易日价格区间的 {:.0}% 位置，接近高点", pos * 100.0)));
    }
    if let Some(m) = s.momentum20
        && m >= 20.0
    {
        cautions.push(note("diagnosis", format!("近 20 个交易日已经涨了 {m:.0}%")));
    }
    if s.trend_weak {
        cautions.push(note("diagnosis", "诊断的趋势偏弱（均线空头排列），属于逆势买入"));
    }
    if s.amount < 1e8 {
        cautions.push(note("risk", format!("当天成交额 {:.2} 亿元，偏小，买卖可能不太顺畅", s.amount / 1e8)));
    }
    if half {
        cautions.push(note("market", "弱市：按本账户的规则，用于买新股票的钱减半"));
    } else if weak {
        cautions.push(note("market", "全市场偏弱，买入后更容易遇到下跌"));
    }
    for k in dropped {
        cautions.push(note("stat", format!("{}也选中了它，但不够有效门槛，没有算进来", format_name(&name_of(k.strategy_id)))));
    }

    let stop_price = close * (1.0 - p.stop_loss_pct / 100.0);
    let loss = shares as f64 * (close - stop_price) + fee + inp.fees.sell_fee(shares as f64 * stop_price);
    let chase = close * (1.0 + p.chase_limit_pct / 100.0);
    let mut plan = vec![
        format!("下一个交易日（{}）开盘后买入；开盘高于 {chase:.2} 元（比参考价高 {}%）就不要买，开盘一字涨停买不进时这条作废", inp.next_date, p.chase_limit_pct),
        format!("止损价 {stop_price:.2} 元：跌到这里会建议卖出，这笔大约亏 {}，占总资产 {:.1}%", yuan(loss), loss / total.max(1.0) * 100.0),
    ];
    if let Some(ma) = s.ma_trend {
        plan.push(format!("收盘跌破 {} 日均线（现在 {ma:.2} 元）也会建议卖出", p.trend_ma));
    }
    let scope = match &p.strategies {
        None => "全部参加巡检的策略".to_string(),
        Some(l) => format!("勾选的 {} 个策略", l.len()),
    };
    let rule = format!(
        "本账户的规则：参考{scope}中过去 3 年比随便买多涨 {}% 以上的（有效策略），至少 {} 个同时选中才买；按「同时选中的有效策略数 → 它们的超额之和 → 成交额」排序；每只投入「可用的钱 ÷ 空余仓位」，不超过总资产的 {}%{}",
        p.min_edge_pct,
        p.min_picks,
        p.max_weight_pct,
        if p.min_cash_pct > 0.0 { format!("，并保留 {}% 现金", p.min_cash_pct) } else { String::new() }
    );
    Item {
        key: format!("buy-{}", c.thscode),
        action: Action::Buy,
        thscode: c.thscode.clone(),
        name: c.name.clone(),
        shares,
        ref_price: close,
        amount,
        fee,
        headline,
        rule,
        evidence,
        cautions,
        plan,
        strategy_id: kept.first().map(|k| k.strategy_id),
        stop_price: Some(stop_price),
        streak: 0,
        weight_pct: amount / total.max(1.0) * 100.0,
    }
}

fn format_name(n: &str) -> String {
    format!("「{n}」")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::rules::Preset;

    fn snap(close: f64, ma: f64) -> Snapshot {
        Snapshot { close, change_pct: Some(1.0), amount: 5e8, ma_trend: Some(ma), rsi: Some(55.0), momentum20: Some(5.0), range60_pos: Some(0.6), trend_text: "上升趋势".into(), trend_strong: true, ..Default::default() }
    }

    fn holding(code: &str, shares: i64, cost: Option<f64>, s: Snapshot) -> HoldingIn {
        HoldingIn {
            thscode: code.into(),
            name: format!("股票{code}"),
            board: "main".into(),
            shares,
            cost_price: cost,
            opened_on: None,
            bars_held: None,
            strategy: None,
            strategy_exit: None,
            is_st: false,
            delisted: false,
            last_date: Some(day()),
            snap: Some(s),
        }
    }

    fn cand(code: &str, close: f64, picks: &[i64], amount: f64) -> CandidateIn {
        CandidateIn {
            thscode: code.into(),
            name: format!("候选{code}"),
            board: "main".into(),
            is_st: false,
            picks: picks.iter().map(|id| Pick { strategy_id: *id, reasons: vec!["当天出现「放量突破」".into()] }).collect(),
            snap: Snapshot { amount, ..snap(close, close * 0.95) },
        }
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()
    }

    fn stat(excess: f64) -> StrategyStat {
        StrategyStat { start: day(), end: day(), signals: 1000, closed: 900, win_rate: Some(0.55), avg_return_pct: Some(1.2), avg_bars_held: Some(9.0), after20_avg_pct: Some(0.6 + excess), base20_avg_pct: Some(0.6), after20_up_ratio: Some(0.52) }
    }

    fn input(preset: Preset, cash: f64, holdings: Vec<HoldingIn>, candidates: Vec<CandidateIn>) -> Input {
        Input {
            trade_date: day(),
            next_date: day().succ_opt().unwrap(),
            rules: preset.rules(),
            fees: Fees { commission_rate: 0.00025, min_commission: 5.0 },
            cash,
            market: Some(MarketIn { above_ma20_pct: 55.0, stocks: 5000 }),
            holdings,
            candidates,
            strategies: vec![
                StrategyIn { strategy_id: 1, name: "放量突破".into(), stat: Some(stat(1.8)) },
                StrategyIn { strategy_id: 2, name: "MACD 金叉".into(), stat: Some(stat(1.2)) },
                StrategyIn { strategy_id: 3, name: "回踩 20 日均线".into(), stat: Some(stat(-0.4)) },
            ],
            streaks: BTreeMap::new(),
        }
    }

    fn find<'a>(a: &'a Advice, code: &str) -> &'a Item {
        a.items.iter().find(|i| i.thscode == code).unwrap()
    }

    #[test]
    fn sells_on_stop_loss_then_trend_and_holds_otherwise() {
        let hs = vec![
            holding("A", 1000, Some(15.10), snap(13.80, 13.0)),  // 亏 8.6%，止损（稳健 8%）
            holding("B", 200, Some(142.5), snap(138.0, 141.3)),  // 亏 3.2%，跌破 20 日均线
            holding("C", 500, Some(10.0), snap(12.0, 11.6)),    // 赚，在均线上方
        ];
        let a = generate(&input(Preset::Balanced, 50_000.0, hs, vec![]));
        let (ia, ib, ic) = (find(&a, "A"), find(&a, "B"), find(&a, "C"));
        assert_eq!(ia.action, Action::Sell);
        assert!(ia.headline.contains("亏损 8.6%") && ia.rule.contains("止损价 13.89 元"), "{} / {}", ia.headline, ia.rule);
        assert_eq!(ib.action, Action::Sell);
        assert!(ib.headline.contains("跌破 20 日均线 141.30 元"), "{}", ib.headline);
        assert!(ib.evidence.iter().any(|n| n.text.contains("亏 3.2%（¥900）")), "{:?}", ib.evidence);
        assert_eq!(ic.action, Action::Hold);
        assert!(ic.plan[0].contains("收盘跌破 11.60 元（20 日均线），或亏到 9.20 元（止损价）"), "{:?}", ic.plan);
        assert_eq!(a.headline, "卖出 2 只");
        assert_eq!(a.items[0].action, Action::Sell, "卖出排在前面");
        // 卖出全部；印花税和佣金按卖出金额估算
        assert_eq!(ia.shares, 1000);
        assert_eq!(ia.fee, 11.9, "佣金不足 5 元按 5 元，印花税 6.9 元");
    }

    #[test]
    fn strategy_exit_and_streak_and_overweight() {
        let mut h = holding("A", 1000, Some(10.0), snap(12.0, 11.0));
        h.strategy = Some(StrategyRef { strategy_id: 1, name: "放量突破".into() });
        h.strategy_exit = Some("收盘价下穿 20 日均线".into());
        let mut inp = input(Preset::Balanced, 10_000.0, vec![h, holding("B", 3000, Some(10.0), snap(20.0, 18.0))], vec![]);
        inp.streaks.insert("sell-A".into(), 1);
        let a = generate(&inp);
        let ia = find(&a, "A");
        assert_eq!(ia.action, Action::Sell);
        assert!(ia.headline.contains("「放量突破」的卖出条件满足"));
        assert_eq!(ia.streak, 2);
        assert!(ia.evidence[0].text.contains("已连续 2 天建议卖出"));
        // B 占 60,000 / 82,000 = 73%，超过 25%：减到 25% 左右，按整百取整
        let ib = find(&a, "B");
        assert_eq!(ib.action, Action::Reduce);
        assert_eq!(ib.shares % 100, 0);
        let left = (3000 - ib.shares) as f64 * 20.0 / 82_000.0 * 100.0;
        assert!((20.0..=26.0).contains(&left), "剩 {left}%");
    }

    #[test]
    fn buys_ranked_by_agreement_edge_and_split_across_free_slots() {
        let cands = vec![
            cand("X", 20.0, &[1], 9e8),     // 一个策略
            cand("Y", 30.0, &[1, 2], 2e8),  // 两个策略同时选中：排第一
            cand("Z", 10.0, &[3], 9e9),     // 只被过去表现不如随便买的策略选中：不采纳
            cand("W", 5000.0, &[2], 3e8),   // 太贵：一手要 50 万
        ];
        let a = generate(&input(Preset::Balanced, 100_000.0, vec![], cands));
        let buys: Vec<&Item> = a.items.iter().filter(|i| i.action == Action::Buy).collect();
        assert_eq!(buys.iter().map(|b| b.thscode.as_str()).collect::<Vec<_>>(), ["Y", "X"]);
        assert!(buys[0].headline.contains("2 个有效策略同时选中，过去 3 年比随便买合计多涨 3.0%，排第 1"), "{}", buys[0].headline);
        // 稳健：保留 10% 现金，6 个空位平分 90,000，每只不超过总资产的 25%
        assert_eq!(buys[0].shares, 400, "15,000 元：500 股加佣金超了，买 400 股");
        assert_eq!(buys[1].shares, 700, "剩下 77,995 元分给 5 个空位，每只 15,599 元");
        assert!(buys[0].plan[0].contains("31.50 元"), "追高上限 5%：{}", buys[0].plan[0]);
        assert!(buys[0].plan[1].contains("止损价 27.60 元"), "{}", buys[0].plan[1]);
        assert!(buys[0].evidence.iter().any(|n| n.kind == "stat" && n.text.contains("随便买")));
        let reasons: HashMap<&str, &str> = a.skipped.iter().map(|s| (s.thscode.as_str(), s.reason.as_str())).collect();
        assert!(reasons["Z"].contains("不够有效门槛"), "{reasons:?}");
        assert!(reasons["W"].contains("钱不够买最少 100 股"), "{reasons:?}");
        assert_eq!(a.headline, "买入 2 只");
        assert!(a.after.cash < 100_000.0 && a.after.positions == 2);
        assert!(a.notes[0].contains("参考 3 个策略，其中 2 个"), "{:?}", a.notes);

        // 保守型要求至少 2 个有效策略同时选中：X 只有 1 个，跳过
        let mut inp = input(Preset::Conservative, 100_000.0, vec![], vec![cand("X", 20.0, &[1], 9e8), cand("Y", 30.0, &[1, 2], 2e8)]);
        inp.market = Some(MarketIn { above_ma20_pct: 55.0, stocks: 5000 });
        let a = generate(&inp);
        let buys: Vec<&str> = a.items.iter().filter(|i| i.action == Action::Buy).map(|i| i.thscode.as_str()).collect();
        assert_eq!(buys, ["Y"]);
        assert!(a.skipped[0].reason.contains("本账户要求至少 2 个"), "{:?}", a.skipped);

        // 只勾选了策略 2：只按它选中的算
        let mut inp = input(Preset::Balanced, 100_000.0, vec![], vec![cand("X", 20.0, &[1], 9e8), cand("Y", 30.0, &[1, 2], 2e8)]);
        inp.rules.strategies = Some(vec![2]);
        let a = generate(&inp);
        let y = a.items.iter().find(|i| i.thscode == "Y").unwrap();
        assert!(y.headline.starts_with("「MACD 金叉」选中"), "{}", y.headline);
        assert!(!a.items.iter().any(|i| i.thscode == "X"));
    }

    #[test]
    fn weak_market_pauses_conservative_and_halves_balanced() {
        let cands = || vec![cand("Y", 30.0, &[1, 2], 2e8)];
        let mut inp = input(Preset::Conservative, 100_000.0, vec![], cands());
        inp.strategies.iter_mut().for_each(|s| s.stat = Some(stat(1.5)));
        inp.market = Some(MarketIn { above_ma20_pct: 22.0, stocks: 5000 });
        let a = generate(&inp);
        assert_eq!(a.headline, "今天不用操作");
        assert!(a.why_nothing.as_deref().unwrap().contains("暂停买入"), "{:?}", a.why_nothing);
        assert!(a.market.as_deref().unwrap().contains("算弱市"));

        let mut inp = input(Preset::Balanced, 100_000.0, vec![], cands());
        inp.market = Some(MarketIn { above_ma20_pct: 22.0, stocks: 5000 });
        let a = generate(&inp);
        let b = a.items.iter().find(|i| i.action == Action::Buy).unwrap();
        assert_eq!(b.shares, 200, "可用的 90,000 元减半，6 个空位每只 7,500 元，买 200 股");
        assert!(b.cautions.iter().any(|n| n.text.contains("减半")));
    }

    #[test]
    fn nothing_to_do_explains_why() {
        let a = generate(&input(Preset::Balanced, 100_000.0, vec![holding("C", 500, None, snap(12.0, 11.6))], vec![]));
        assert_eq!(a.headline, "今天不用操作");
        assert_eq!(a.why_nothing.as_deref(), Some("1 只持仓都还没有触发卖出规则；今天没有符合本账户规则的新买点。"));
        assert!(a.notes[1].contains("没填成本价"), "{:?}", a.notes);
        let mut h = holding("D", 100, Some(10.0), snap(9.0, 9.5));
        h.last_date = Some(day().pred_opt().unwrap());
        let a = generate(&input(Preset::Balanced, 0.0, vec![h], vec![]));
        assert_eq!(a.items[0].action, Action::Wait, "停牌不判断止损和趋势");
        assert_eq!(yuan(1_234_567.4), "¥1,234,567");
        assert_eq!(yuan(-950.0), "-¥950");
    }
}
