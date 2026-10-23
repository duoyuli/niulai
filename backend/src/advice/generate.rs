//! 生成一份调仓建议：纯函数，输入是账户快照、账户规则、当天的因子排名、持仓股和候选股的状态、市场概况，
//! 同样的输入总是得到同样的输出。每月最后一个交易日收盘整体调仓（卖出跌出缓冲区的、买入排名靠前的），
//! 其余交易日只在没有持满时补仓。每条建议写清产生它的规则、支持的数字、反面的信息和怎么做。
//! 规则见 docs/domains/accounts.md#generation。

use std::collections::{BTreeMap, HashSet};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::account::lots::{self, Fees, round2};
use crate::account::rules::Rules;
use crate::analysis::factors;

/// 补仓时每只至少要投入目标金额的这个比例，避免用零头买一手
const MIN_FILL: f64 = 0.5;
/// 「没选上的候选」最多列几只
const MAX_SKIPPED: usize = 8;

/// 一只股票在建议交易日的状态（价格是未复权价）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Snapshot {
    pub close: f64,
    /// 当天成交额（元）
    pub amount: f64,
    /// 近 20 个交易日涨跌（%）
    pub momentum20: Option<f64>,
    /// 在近 60 个交易日价格区间中的位置（0 到 1）
    pub range60_pos: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HoldingIn {
    pub thscode: String,
    pub name: String,
    pub board: String,
    pub asset_type: String,
    pub shares: i64,
    pub cost_price: Option<f64>,
    pub is_st: bool,
    pub delisted: bool,
    /// 最近一根日线的日期；早于建议交易日表示停牌
    pub last_date: Option<NaiveDate>,
    /// 今天的因子名次（从 1 算）；不在排名里时为空
    pub rank: Option<usize>,
    /// 没有日线时为空
    pub snap: Option<Snapshot>,
}

/// 排名靠前、还没持有的股票
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CandidateIn {
    pub thscode: String,
    pub name: String,
    pub board: String,
    pub is_st: bool,
    /// 名次，从 1 算
    pub rank: usize,
    /// 合成得分（标准化后）
    pub score: f64,
    pub snap: Snapshot,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct MarketIn {
    /// 收盘站上 20 日均线的股票比例（%）
    pub above_ma20_pct: f64,
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
    /// 今天是当月最后一个交易日：整体调仓
    pub rebalance: bool,
    /// 参与排名的股票数；为 0 表示今天的排名没算出来
    pub ranked: usize,
    /// 排名靠前、还没持有的股票，按名次
    pub candidates: Vec<CandidateIn>,
    /// 之前连续几份建议都给出了同一条卖出、且没有执行：键是 item key
    pub streaks: BTreeMap<String, u32>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
#[schema(as = AdviceAction)]
pub enum Action {
    Sell,
    Buy,
    Hold,
    /// 停牌、退市、没有日线：现在做不了判断
    Wait,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Action::Sell => "sell",
            Action::Buy => "buy",
            Action::Hold => "hold",
            Action::Wait => "wait",
        }
    }

    /// 卖出、买入、继续拿、不动
    fn order(self) -> u8 {
        match self {
            Action::Sell => 0,
            Action::Buy => 1,
            Action::Hold => 2,
            Action::Wait => 3,
        }
    }
}

/// 一条理由或提醒
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
#[schema(as = AdviceNote)]
pub struct Note {
    /// rule / position / factor / price / market / risk
    pub kind: String,
    pub text: String,
}

fn note(kind: &str, text: impl Into<String>) -> Note {
    Note {
        kind: kind.into(),
        text: text.into(),
    }
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
#[schema(as = AdviceItem)]
pub struct Item {
    /// 例：sell-000858.SZ；执行、不做都按它记
    pub key: String,
    pub action: Action,
    pub thscode: String,
    pub name: String,
    /// 卖出、买入的股数；继续拿和不动时是持有的股数
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
    /// 买入时：下一个交易日开盘价不高于它才买（参考价加追高上限）
    pub limit_price: Option<f64>,
    /// 连续第几天给出这条卖出
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
    /// 一句话结论：「不用操作」或「卖出 1 只、买入 2 只」，说的是下一个交易日
    pub headline: String,
    /// 不用操作时的原因
    pub why_nothing: Option<String>,
    pub total_assets: f64,
    pub cash: f64,
    pub market_value: f64,
    /// 市场环境的一句话
    pub market: Option<String>,
    /// 卖出、买入、继续拿、不动，按这个顺序
    pub items: Vec<Item>,
    /// 排名靠前、但没买的股票和原因
    pub skipped: Vec<Skipped>,
    pub after: After,
    /// 需要用户知道的事，例如今天是不是调仓日
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

/// 参与合成的因子和方向，例如「波动率低、单季利润增速高」
pub fn factor_names(rules: &Rules) -> String {
    rules
        .factors
        .iter()
        .map(|f| {
            let info = factors::info(f.factor);
            // 权重为负时反过来用
            let higher = info.higher_is_better == (f.weight > 0.0);
            format!("{}{}", info.name, if higher { "高" } else { "低" })
        })
        .collect::<Vec<_>>()
        .join("、")
}

// @project-doc docs/domains/accounts.md#generation
pub fn generate(inp: &Input) -> Advice {
    let p = &inp.rules;
    let price_of = |h: &HoldingIn| h.snap.as_ref().map_or(0.0, |s| s.close);
    let market_value: f64 = inp
        .holdings
        .iter()
        .map(|h| h.shares as f64 * price_of(h))
        .sum();
    let total = inp.cash + market_value;
    let market_text = inp
        .market
        .map(|m| format!("全市场 {:.0}% 的股票收盘站上 20 日均线", m.above_ma20_pct));
    let top_n = p.top_n as usize;
    let keep_limit = p.keep_limit();
    let names = factor_names(p);
    let rule = format!(
        "本账户的规则：按{names}给全市场排名，每月调仓，持有前 {top_n} 只，每只约占总资产的 1/{top_n}；已持有的跌出前 {keep_limit} 名才卖"
    );
    let ready = inp.ranked > 0;
    let rebalance = ready && inp.rebalance;

    // 调仓日继续拿的：排名在缓冲区以内的持仓里最靠前的 top_n 只
    let mut ranked_held: Vec<(usize, &str)> = inp
        .holdings
        .iter()
        .filter_map(|h| h.rank.map(|r| (r, h.thscode.as_str())))
        .filter(|(r, _)| *r <= keep_limit)
        .collect();
    ranked_held.sort();
    let keep: HashSet<&str> = ranked_held.iter().take(top_n).map(|(_, c)| *c).collect();

    // 第 1 步：每只持仓下结论
    let mut items: Vec<Item> = inp
        .holdings
        .iter()
        .map(|h| judge_holding(inp, h, total, ready, rebalance, &keep, &rule))
        .collect();
    let sold: HashSet<&str> = items
        .iter()
        .filter(|i| i.action == Action::Sell)
        .map(|i| i.thscode.as_str())
        .collect();
    let proceeds: f64 = items
        .iter()
        .filter(|i| i.action == Action::Sell)
        .map(|i| i.amount - i.fee)
        .sum();
    let remaining = items.iter().filter(|i| i.action != Action::Sell).count();

    // 第 2 到 4 步：调仓日或没有持满时，按名次买入，每只投入（总资产 − 保留现金）÷ top_n
    let reserve = total * p.min_cash_pct / 100.0;
    let mut available = inp.cash + proceeds - reserve;
    let mut slots = top_n.saturating_sub(remaining);
    let target = (total - reserve).max(0.0) / top_n as f64;
    let held: HashSet<&str> = inp.holdings.iter().map(|h| h.thscode.as_str()).collect();
    let mut buys: Vec<Item> = vec![];
    let mut skipped: Vec<Skipped> = vec![];
    let mut no_money = false;
    for c in inp.candidates.iter().filter(|_| ready) {
        if slots == 0 {
            break;
        }
        if held.contains(c.thscode.as_str()) || sold.contains(c.thscode.as_str()) {
            continue;
        }
        let skip = |skipped: &mut Vec<Skipped>, reason: String| {
            skipped.push(Skipped {
                thscode: c.thscode.clone(),
                name: c.name.clone(),
                reason,
            })
        };
        if c.is_st {
            skip(&mut skipped, "ST 股不买".into());
            continue;
        }
        let budget = target.min(available);
        // 钱不够一只的目标金额的一半时，后面的候选也一样，不逐只列出
        if budget < target * MIN_FILL || budget <= 1.0 {
            no_money = true;
            break;
        }
        let close = c.snap.close;
        let lot = lots::lot_rule_for("stock", &c.board);
        let shares = lots::max_buy_shares(lot, budget, close, &inp.fees);
        if shares == 0 {
            let need = lot.min as f64 * close;
            skip(
                &mut skipped,
                format!(
                    "钱不够买最少 {} 股（约 {}，每只最多投 {}）",
                    lot.min,
                    yuan(need + inp.fees.buy_fee(need)),
                    yuan(budget)
                ),
            );
            continue;
        }
        let amount = round2(shares as f64 * close);
        let fee = inp.fees.buy_fee(amount);
        available -= amount + fee;
        slots -= 1;
        buys.push(buy_item(
            inp,
            c,
            (shares, amount, fee),
            total,
            rebalance,
            &names,
            &rule,
            market_text.as_deref(),
        ));
    }
    skipped.truncate(MAX_SKIPPED);

    // 第 5 步：汇总
    let after = after_of(inp, &items, &buys, total, proceeds);
    let n_sell = items.iter().filter(|i| i.action == Action::Sell).count();
    let n_buy = buys.len();
    let (headline, why_nothing) = if n_sell + n_buy == 0 {
        let why = if !ready {
            "今天的因子排名没算出来（当天日线太少或数据还没同步完），先不调仓。".to_string()
        } else if total <= 1.0 {
            "账户里还没有现金和持仓，先在「持仓」里填上现金，就会按排名建仓。".to_string()
        } else if remaining >= top_n {
            format!("已经持有 {remaining} 只，不是调仓日；下一次调仓在月末最后一个交易日收盘后。")
        } else if no_money {
            format!(
                "没有持满，但可用的钱不到每只目标金额（{}）的一半，不再补仓。",
                yuan(target)
            )
        } else {
            "没有要买卖的。".to_string()
        };
        ("不用操作".to_string(), Some(why))
    } else {
        let parts: Vec<String> = [(n_sell, "卖出"), (n_buy, "买入")]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, w)| format!("{w} {n} 只"))
            .collect();
        (parts.join("、"), None)
    };
    let notes = vec![if rebalance {
        format!("今天是本月最后一个交易日：按因子排名整体调仓，持有前 {top_n} 只")
    } else {
        "每月最后一个交易日收盘整体调仓；其余交易日只在没有持满时补仓".to_string()
    }];
    items.extend(buys);
    items.sort_by_key(|i| i.action.order());
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

/// 照着建议做完之后的现金和各持仓占比
fn after_of(inp: &Input, items: &[Item], buys: &[Item], total: f64, proceeds: f64) -> After {
    let buy_cost: f64 = buys.iter().map(|b| b.amount + b.fee).sum();
    let cash_after = round2(inp.cash + proceeds - buy_cost);
    let mut weights: Vec<Weight> = items
        .iter()
        .filter(|it| it.action != Action::Sell && it.shares > 0)
        .map(|it| Weight {
            thscode: it.thscode.clone(),
            name: it.name.clone(),
            weight_pct: it.shares as f64 * it.ref_price / total.max(1.0) * 100.0,
        })
        .collect();
    weights.extend(buys.iter().map(|b| Weight {
        thscode: b.thscode.clone(),
        name: b.name.clone(),
        weight_pct: b.weight_pct,
    }));
    weights.sort_by(|a, b| b.weight_pct.total_cmp(&a.weight_pct));
    After {
        cash: cash_after,
        cash_pct: cash_after / total.max(1.0) * 100.0,
        positions: weights.len(),
        weights,
    }
}

fn judge_holding(
    inp: &Input,
    h: &HoldingIn,
    total: f64,
    ready: bool,
    rebalance: bool,
    keep: &HashSet<&str>,
    rule: &str,
) -> Item {
    let p = &inp.rules;
    let keep_limit = p.keep_limit();
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
        limit_price: None,
        streak: 0,
        weight_pct: 0.0,
    };
    let Some(s) = h.snap.as_ref() else {
        return base(
            Action::Wait,
            "本地还没有这只股票的日线，暂时判断不了".into(),
            "没有行情时不给建议".into(),
        );
    };
    if h.delisted {
        return base(
            Action::Wait,
            "已经退市".into(),
            "退市的股票不能在交易所正常买卖，按券商的退市安排处理".into(),
        );
    }
    let value = h.shares as f64 * s.close;
    let weight = value / total.max(1.0) * 100.0;
    if let Some(last) = h.last_date.filter(|d| *d < inp.trade_date) {
        return Item {
            weight_pct: weight,
            ..base(
                Action::Wait,
                format!("停牌中（最后交易日 {last}），复牌后再判断"),
                "停牌时不能交易".into(),
            )
        };
    }

    let mut evidence = vec![];
    if let Some(c) = h.cost_price {
        let r = s.close / c - 1.0;
        evidence.push(note(
            "position",
            format!(
                "成本 {c:.2} 元，现价 {:.2} 元，{} {:.1}%（{}）",
                s.close,
                if r >= 0.0 { "赚" } else { "亏" },
                (r * 100.0).abs(),
                yuan(((s.close - c) * h.shares as f64).abs())
            ),
        ));
    }
    if let Some(r) = h.rank.filter(|_| ready) {
        evidence.push(note(
            "factor",
            format!("今天的因子排名第 {r}（共 {} 只）", inp.ranked),
        ));
    }
    evidence.push(note("position", format!("占总资产 {weight:.1}%")));

    let sell = |headline: String, rule: String, evidence: Vec<Note>| {
        let amount = round2(value);
        let key = format!("sell-{}", h.thscode);
        let streak = inp.streaks.get(&key).copied().unwrap_or(0) + 1;
        let mut ev = evidence;
        if streak > 1 {
            ev.insert(
                0,
                note("rule", format!("已连续 {streak} 天建议卖出，还没有执行")),
            );
        }
        Item {
            amount,
            fee: inp.fees.sell_fee(&h.asset_type, amount),
            evidence: ev,
            plan: vec![
                format!(
                    "下一个交易日（{}）开盘后卖出全部 {} 股",
                    inp.next_date, h.shares
                ),
                "开盘一字跌停卖不出时，下一份建议会继续提示".into(),
            ],
            streak,
            weight_pct: weight,
            ..base(Action::Sell, headline, rule)
        }
    };
    if h.is_st && p.sell_st {
        return sell(
            "被实施风险警示（ST）".into(),
            "本账户的规则：不持有 ST 股".into(),
            evidence,
        );
    }
    if rebalance && !keep.contains(h.thscode.as_str()) {
        let headline = match h.rank {
            None => "不在今天的因子排名里（ST、成交额不到 5,000 万元、股价低于 2 元、上市不满 60 个交易日或因子数据不全）".to_string(),
            Some(r) if r > keep_limit => format!("月度调仓：因子排名第 {r}，跌出前 {keep_limit} 名"),
            Some(r) => format!(
                "月度调仓：因子排名第 {r}，持仓里排名更靠前的已经有 {} 只",
                p.top_n
            ),
        };
        return sell(headline, rule.to_string(), evidence);
    }
    let mut cautions = vec![];
    if h.is_st {
        cautions.push(note("risk", "已被实施风险警示（ST），风险较大"));
    }
    let headline = match h.rank {
        _ if !ready => "今天的因子排名没算出来，先继续拿".to_string(),
        Some(r) if r <= keep_limit => format!("因子排名第 {r}，在前 {keep_limit} 名以内"),
        Some(r) => format!("因子排名第 {r}，月末调仓时会卖出"),
        None => "不在今天的因子排名里，月末调仓时会卖出".to_string(),
    };
    Item {
        evidence,
        cautions,
        plan: vec![format!(
            "每月最后一个交易日收盘重新排名，跌出前 {keep_limit} 名时卖出"
        )],
        weight_pct: weight,
        ..base(Action::Hold, headline, rule.to_string())
    }
}

/// 一条买入：`order` 是（股数、金额、费用）
#[allow(clippy::too_many_arguments)]
fn buy_item(
    inp: &Input,
    c: &CandidateIn,
    order: (i64, f64, f64),
    total: f64,
    rebalance: bool,
    names: &str,
    rule: &str,
    market: Option<&str>,
) -> Item {
    let (shares, amount, fee) = order;
    let p = &inp.rules;
    let close = c.snap.close;
    let chase = close * (1.0 + p.chase_limit_pct / 100.0);
    let mut evidence = vec![
        note(
            "factor",
            format!(
                "因子排名第 {}（共 {} 只），合成得分 {:.2}",
                c.rank, inp.ranked, c.score
            ),
        ),
        note("factor", format!("参与合成的因子：{names}")),
    ];
    if let Some(m) = market {
        evidence.push(note("market", m));
    }
    Item {
        key: format!("buy-{}", c.thscode),
        action: Action::Buy,
        thscode: c.thscode.clone(),
        name: c.name.clone(),
        shares,
        ref_price: close,
        amount,
        fee,
        headline: if rebalance {
            format!("月度调仓：因子排名第 {}，买入", c.rank)
        } else if inp.holdings.is_empty() {
            format!("建仓：因子排名第 {}，买入", c.rank)
        } else {
            format!("补仓：因子排名第 {}，还没有持满 {} 只", c.rank, p.top_n)
        },
        rule: rule.to_string(),
        evidence,
        cautions: buy_cautions(&c.snap),
        plan: vec![format!(
            "下一个交易日（{}）开盘后买入；开盘高于 {chase:.2} 元（比参考价高 {}%）就不要买，开盘一字涨停买不进时这条作废，之后没持满会继续补",
            inp.next_date, p.chase_limit_pct
        )],
        limit_price: Some(round2(chase)),
        streak: 0,
        weight_pct: amount / total.max(1.0) * 100.0,
    }
}

/// 买入前要知道的反面信息：接近高点、近期涨幅大、成交额小
fn buy_cautions(s: &Snapshot) -> Vec<Note> {
    let mut cautions = vec![];
    if let Some(pos) = s.range60_pos
        && pos >= 0.9
    {
        cautions.push(note(
            "price",
            format!(
                "处在近 60 个交易日价格区间的 {:.0}% 位置，接近高点",
                pos * 100.0
            ),
        ));
    }
    if let Some(m) = s.momentum20
        && m >= 20.0
    {
        cautions.push(note("price", format!("近 20 个交易日已经涨了 {m:.0}%")));
    }
    if s.amount < 1e8 {
        cautions.push(note(
            "risk",
            format!(
                "当天成交额 {:.2} 亿元，偏小，买卖可能不太顺畅",
                s.amount / 1e8
            ),
        ));
    }
    cautions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
    }

    fn snap(close: f64) -> Snapshot {
        Snapshot {
            close,
            amount: 5e8,
            ..Default::default()
        }
    }

    fn holding(code: &str, shares: i64, rank: Option<usize>) -> HoldingIn {
        HoldingIn {
            thscode: code.into(),
            name: format!("股票{code}"),
            board: "main".into(),
            asset_type: "stock".into(),
            shares,
            cost_price: Some(10.0),
            is_st: false,
            delisted: false,
            last_date: Some(day()),
            rank,
            snap: Some(snap(10.0)),
        }
    }

    fn cand(code: &str, rank: usize) -> CandidateIn {
        CandidateIn {
            thscode: code.into(),
            name: format!("候选{code}"),
            board: "main".into(),
            is_st: false,
            rank,
            score: 2.0 - rank as f64 / 100.0,
            snap: snap(10.0),
        }
    }

    fn input(
        rebalance: bool,
        cash: f64,
        holdings: Vec<HoldingIn>,
        candidates: Vec<CandidateIn>,
    ) -> Input {
        Input {
            trade_date: day(),
            next_date: day().succ_opt().unwrap(),
            rules: Rules {
                top_n: 5,
                buffer: 1.0,
                ..Rules::default()
            },
            fees: Fees {
                commission_rate: 0.00025,
                min_commission: 5.0,
            },
            cash,
            market: None,
            holdings,
            rebalance,
            ranked: 3000,
            candidates,
            streaks: BTreeMap::new(),
        }
    }

    fn act(a: &Advice, code: &str) -> Option<Action> {
        a.items.iter().find(|i| i.thscode == code).map(|i| i.action)
    }

    #[test]
    fn empty_account_builds_equal_weight_portfolio_right_away() {
        let cands = (1..=8).map(|r| cand(&format!("{r:06}.SZ"), r)).collect();
        let a = generate(&input(false, 100_000.0, vec![], cands));
        let buys: Vec<_> = a.items.iter().filter(|i| i.action == Action::Buy).collect();
        assert_eq!(buys.len(), 5, "{a:?}");
        assert_eq!(buys[0].thscode, "000001.SZ");
        // 每只约 1/5：10 万 ÷ 5 = 2 万，10 元一股 → 1,900 股（留出佣金）
        assert!(buys.iter().all(|b| b.shares == 1900), "{buys:?}");
        assert!(buys[0].headline.starts_with("建仓"));
        assert!(a.notes[0].contains("没有持满"));
    }

    #[test]
    fn month_end_sells_holdings_outside_the_buffer_and_refills() {
        // keep_limit = 5 × 2 = 10：A 第 3 名、B 第 9 名留下，C 第 11 名卖出，D 不在排名里卖出
        let hs = vec![
            holding("A.SZ", 1000, Some(3)),
            holding("B.SZ", 1000, Some(9)),
            holding("C.SZ", 1000, Some(11)),
            holding("D.SZ", 1000, None),
            holding("E.SZ", 1000, Some(1)),
        ];
        let cands = vec![cand("X.SZ", 2), cand("Y.SZ", 4), cand("Z.SZ", 5)];
        let a = generate(&input(true, 20_000.0, hs.clone(), cands.clone()));
        assert_eq!(act(&a, "A.SZ"), Some(Action::Hold));
        assert_eq!(act(&a, "B.SZ"), Some(Action::Hold));
        assert_eq!(act(&a, "C.SZ"), Some(Action::Sell));
        assert_eq!(act(&a, "D.SZ"), Some(Action::Sell));
        assert_eq!(act(&a, "X.SZ"), Some(Action::Buy));
        assert_eq!(act(&a, "Y.SZ"), Some(Action::Buy));
        assert_eq!(act(&a, "Z.SZ"), None, "只空出 2 个位置");

        // 不是月末：持满了就不动，跌出缓冲区的也等到月末
        let a = generate(&input(false, 20_000.0, hs, cands));
        assert_eq!(a.headline, "不用操作");
        assert!(a.items.iter().all(|i| i.action == Action::Hold));
    }

    #[test]
    fn st_holdings_are_sold_and_streaks_counted() {
        let mut h = holding("A.SZ", 1000, Some(1));
        h.is_st = true;
        let mut inp = input(false, 0.0, vec![h], vec![]);
        inp.streaks.insert("sell-A.SZ".into(), 2);
        let a = generate(&inp);
        let it = &a.items[0];
        assert_eq!(it.action, Action::Sell);
        assert_eq!(it.streak, 3);
        assert!(it.evidence[0].text.contains("已连续 3 天"));
    }

    #[test]
    fn missing_ranking_holds_everything() {
        let mut inp = input(true, 50_000.0, vec![holding("A.SZ", 100, None)], vec![]);
        inp.ranked = 0;
        let a = generate(&inp);
        assert_eq!(a.headline, "不用操作");
        assert!(a.why_nothing.unwrap().contains("没算出来"));
        assert_eq!(a.items[0].action, Action::Hold);
    }

    #[test]
    fn empty_cash_explains_instead_of_listing_every_candidate() {
        let cands = (1..=8).map(|r| cand(&format!("{r:06}.SZ"), r)).collect();
        let a = generate(&input(false, 0.0, vec![], cands));
        assert!(a.skipped.is_empty());
        assert!(a.why_nothing.unwrap().contains("还没有现金"));
    }

    #[test]
    fn suspended_and_delisted_holdings_wait() {
        let mut s = holding("A.SZ", 100, Some(1));
        s.last_date = Some(day().pred_opt().unwrap());
        let mut d = holding("B.SZ", 100, None);
        d.delisted = true;
        let a = generate(&input(true, 0.0, vec![s, d], vec![]));
        assert_eq!(act(&a, "A.SZ"), Some(Action::Wait));
        assert_eq!(act(&a, "B.SZ"), Some(Action::Wait));
    }
}
