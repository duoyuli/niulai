//! 多因子组合账户的调仓建议：账户规则里有 `factor` 时不看勾选策略的买点，按因子合成得分给全市场排名。
//! 每月最后一个交易日收盘整体调仓（卖出跌出缓冲区的、买入排名靠前的），其余交易日只在没有持满时补仓。
//! 排名用因子研究的同一套计算（`analysis::factor_lab`），规则见 docs/domains/accounts.md#factor_plan。

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::generate::{
    Action, Advice, CandidateIn, Input, Item, Skipped, after_of, buy_cautions, holding_item,
    judge_holding, note, order, sell_all, yuan,
};
use crate::account::lots::{self, round2};
use crate::account::rules::FactorPlan;
use crate::analysis::factor_lab::{self, Prep, Spec};
use crate::analysis::factors;
use crate::analysis::strategy::Universe;

/// 补仓时每只至少要投入目标金额的这个比例，避免用零头买一手
const MIN_FILL: f64 = 0.5;
/// 「没选上的候选」最多列几只
const MAX_SKIPPED: usize = 8;
/// 除了填满空位需要的只数，再多看几只候选（ST、买不起时顺延）
pub const EXTRA_CANDIDATES: usize = 20;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FactorCandidate {
    /// 名次，从 1 算
    pub rank: usize,
    /// 合成得分（标准化后）
    pub score: f64,
    pub stock: CandidateIn,
}

/// 多因子账户的输入：原样保存在 `app.advice.input` 中
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FactorIn {
    /// 今天是当月最后一个交易日：整体调仓
    pub rebalance: bool,
    /// 参与排名的股票数；为 0 表示今天的排名没算出来
    pub ranked: usize,
    /// 持仓的名次（从 1 算）；不在排名里的没有
    pub holding_ranks: BTreeMap<String, usize>,
    /// 排名靠前、还没持有的股票，按名次
    pub candidates: Vec<FactorCandidate>,
}

/// 交易日是当月最后一个交易日（下一个交易日在下个月）
pub fn month_end(trade_date: NaiveDate, next_date: NaiveDate) -> bool {
    (trade_date.year(), trade_date.month()) != (next_date.year(), next_date.month())
}

type Ranking = Arc<Vec<(String, f64)>>;

/// 同一天、同一组因子的排名只算一次：改持仓、改设置后重新生成建议时直接复用
fn cache() -> &'static Mutex<Vec<(String, Ranking)>> {
    static CACHE: OnceLock<Mutex<Vec<(String, Ranking)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(vec![]))
}

// @project-doc docs/domains/accounts.md#factor_plan
/// 交易日收盘的全市场排名（代码、得分），从高到低。股票池、去极值、行业和规模中性化与因子研究的默认值相同
pub async fn ranking(pool: &PgPool, date: NaiveDate, plan: &FactorPlan) -> anyhow::Result<Ranking> {
    let key = format!("{date}|{}", serde_json::to_string(&plan.factors)?);
    if let Some((_, r)) = cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("因子排名缓存不可用"))?
        .iter()
        .find(|(k, _)| *k == key)
    {
        return Ok(r.clone());
    }
    let spec = Spec {
        start: date,
        end: date,
        factors: plan.factors.iter().map(|f| f.factor).collect(),
        universe: Universe::default(),
        step: 1,
    };
    let weights: Vec<f64> = plan.factors.iter().map(|f| f.weight).collect();
    let prep = Prep {
        neutralize: true,
        exclude_small_pct: 0.0,
    };
    let ranked = match factor_lab::load(pool, &spec).await? {
        Some(panel) if panel.calendar.last() == Some(&date) => {
            crate::analysis::prices::run_cpu(|| factor_lab::ranking(&panel, prep, &weights))
        }
        _ => vec![],
    };
    let ranked = Arc::new(ranked);
    let mut c = cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("因子排名缓存不可用"))?;
    c.retain(|(k, _)| *k != key);
    c.push((key, ranked.clone()));
    if c.len() > 4 {
        c.remove(0);
    }
    Ok(ranked)
}

fn factor_names(plan: &FactorPlan) -> String {
    plan.factors
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

/// 多因子账户的一份建议。和按策略选股的账户一样是纯函数
pub fn generate(inp: &Input, plan: &FactorPlan, f: Option<&FactorIn>) -> Advice {
    let p = &inp.rules;
    let market_value: f64 = inp
        .holdings
        .iter()
        .map(|h| h.shares as f64 * h.snap.as_ref().map_or(0.0, |s| s.close))
        .sum();
    let total = inp.cash + market_value;
    let market_text = inp.market.map(|m| {
        format!(
            "全市场 {:.0}% 的股票收盘站上 20 日均线（多因子账户不按弱市调整）",
            m.above_ma20_pct
        )
    });
    let top_n = plan.top_n as usize;
    let keep_limit = plan.keep_limit();
    let names = factor_names(plan);
    let rule = format!(
        "本账户的规则：多因子月度调仓（{names}），持有排名前 {top_n} 只，每只约占总资产的 1/{top_n}；已持有的跌出前 {keep_limit} 名才卖"
    );
    let ready = f.filter(|f| f.ranked > 0);
    let rebalance = ready.is_some_and(|f| f.rebalance);
    let rank_of = |code: &str| ready.and_then(|f| f.holding_ranks.get(code).copied());

    // 调仓日继续拿的：排名在缓冲区以内的持仓里最靠前的 top_n 只
    let mut ranked_held: Vec<(usize, &str)> = inp
        .holdings
        .iter()
        .filter_map(|h| rank_of(&h.thscode).map(|r| (r, h.thscode.as_str())))
        .filter(|(r, _)| *r <= keep_limit)
        .collect();
    ranked_held.sort();
    let keep: HashSet<&str> = ranked_held.iter().take(top_n).map(|(_, c)| *c).collect();

    // 第 1 步：每只持仓下结论
    let mut items: Vec<Item> = vec![];
    for h in &inp.holdings {
        let judged = judge_holding(inp, h, total);
        if judged.action == Action::Wait {
            items.push(judged);
            continue;
        }
        let close = h.snap.as_ref().map_or(0.0, |s| s.close);
        let weight = h.shares as f64 * close / total.max(1.0) * 100.0;
        let mut evidence = vec![];
        if let (Some(c), Some(s)) = (h.cost_price, h.snap.as_ref()) {
            let r = s.close / c - 1.0;
            evidence.push(note(
                "position",
                format!(
                    "成本 {c:.2} 元，现价 {:.2} 元，{} {:.1}%",
                    s.close,
                    if r >= 0.0 { "赚" } else { "亏" },
                    (r * 100.0).abs()
                ),
            ));
        }
        let rank = rank_of(&h.thscode);
        if let (Some(r), Some(f)) = (rank, ready) {
            evidence.push(note(
                "strategy",
                format!("今天的因子排名第 {r}（共 {} 只）", f.ranked),
            ));
        }
        evidence.push(note("position", format!("占总资产 {weight:.1}%")));
        if h.is_st && p.sell_st {
            items.push(sell_all(
                inp,
                h,
                total,
                "被实施风险警示（ST）".into(),
                "本账户的规则：不持有 ST 股".into(),
                evidence,
                vec![],
            ));
            continue;
        }
        if rebalance && !keep.contains(h.thscode.as_str()) {
            let headline = match rank {
                None => "不在今天的因子排名里（ST、成交额不到 5,000 万元、股价低于 2 元、上市不满 60 个交易日或因子数据不全）".to_string(),
                Some(r) if r > keep_limit => format!("月度调仓：因子排名第 {r}，跌出前 {keep_limit} 名"),
                Some(r) => format!("月度调仓：因子排名第 {r}，持仓里排名更靠前的已经有 {top_n} 只"),
            };
            items.push(sell_all(
                inp,
                h,
                total,
                headline,
                rule.clone(),
                evidence,
                vec![],
            ));
            continue;
        }
        let mut cautions = vec![];
        if h.is_st {
            cautions.push(note("risk", "已被实施风险警示（ST），风险较大"));
        }
        let headline = match (rank, ready) {
            (_, None) => "今天的因子排名没算出来，先继续拿".to_string(),
            (Some(r), _) if r <= keep_limit => format!("因子排名第 {r}，在前 {keep_limit} 名以内"),
            (Some(r), _) => format!("因子排名第 {r}，月末调仓时会卖出"),
            (None, _) => "不在今天的因子排名里，月末调仓时会卖出".to_string(),
        };
        items.push(Item {
            evidence,
            cautions,
            plan: vec![format!(
                "每月最后一个交易日收盘重新排名，跌出前 {keep_limit} 名时卖出"
            )],
            weight_pct: weight,
            ..holding_item(h, Action::Hold, headline, rule.clone())
        });
    }
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

    // 第 2 到 4 步：调仓日或没有持满时，按名次买入，每只投入总资产的 1/top_n
    let reserve = total * p.min_cash_pct / 100.0;
    let mut available = inp.cash + proceeds - reserve;
    let mut slots = top_n.saturating_sub(remaining);
    let target = (total - reserve).max(0.0) / top_n as f64;
    let held: HashSet<&str> = inp.holdings.iter().map(|h| h.thscode.as_str()).collect();
    let mut buys: Vec<Item> = vec![];
    let mut skipped: Vec<Skipped> = vec![];
    let mut no_money = false;
    if let Some(f) = ready {
        for c in &f.candidates {
            if slots == 0 {
                break;
            }
            let s = &c.stock;
            if held.contains(s.thscode.as_str()) || sold.contains(s.thscode.as_str()) {
                continue;
            }
            let skip = |skipped: &mut Vec<Skipped>, reason: String| {
                skipped.push(Skipped {
                    thscode: s.thscode.clone(),
                    name: s.name.clone(),
                    reason,
                })
            };
            if s.is_st {
                skip(&mut skipped, "ST 股不买".into());
                continue;
            }
            let budget = target.min(available);
            // 钱不够一只的目标金额的一半时，后面的候选也一样，不逐只列出
            if budget < target * MIN_FILL || budget <= 1.0 {
                no_money = true;
                break;
            }
            let close = s.snap.close;
            let lot = lots::lot_rule_for(&s.asset_type, &s.board);
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
            let chase = close * (1.0 + p.chase_limit_pct / 100.0);
            let mut evidence = vec![
                note(
                    "strategy",
                    format!(
                        "因子排名第 {}（共 {} 只），合成得分 {:.2}",
                        c.rank, f.ranked, c.score
                    ),
                ),
                note("strategy", format!("参与合成的因子：{names}")),
            ];
            if !s.snap.trend_text.is_empty() {
                evidence.push(note("diagnosis", format!("趋势：{}", s.snap.trend_text)));
            }
            if let Some(m) = &market_text {
                evidence.push(note("market", m.clone()));
            }
            buys.push(Item {
                key: format!("buy-{}", s.thscode),
                action: Action::Buy,
                thscode: s.thscode.clone(),
                name: s.name.clone(),
                shares,
                ref_price: close,
                amount,
                fee,
                headline: if rebalance {
                    format!("月度调仓：因子排名第 {}，买入", c.rank)
                } else if inp.holdings.is_empty() {
                    format!("建仓：因子排名第 {}，买入", c.rank)
                } else {
                    format!("补仓：因子排名第 {}，还没有持满 {top_n} 只", c.rank)
                },
                rule: rule.clone(),
                evidence,
                cautions: buy_cautions(&s.snap),
                plan: vec![format!(
                    "下一个交易日（{}）开盘后买入；开盘高于 {chase:.2} 元（比参考价高 {}%）就不要买，开盘一字涨停买不进时这条作废，之后没持满会继续补",
                    inp.next_date, p.chase_limit_pct
                )],
                strategy_id: None,
                limit_price: Some(round2(chase)),
                streak: 0,
                weight_pct: amount / total.max(1.0) * 100.0,
            });
        }
    }
    skipped.truncate(MAX_SKIPPED);

    // 第 5 步：汇总
    let after = after_of(inp, &items, &buys, total, proceeds);
    let n_sell = items.iter().filter(|i| i.action == Action::Sell).count();
    let n_buy = buys.len();
    let (headline, why_nothing) = if n_sell + n_buy == 0 {
        let why = if ready.is_none() {
            "今天的因子排名没算出来（当天日线太少或数据还没同步完），先不调仓。".to_string()
        } else if remaining >= top_n {
            format!("已经持有 {remaining} 只，不是调仓日；下一次调仓在月末最后一个交易日收盘后。")
        } else if total <= 1.0 {
            "账户里还没有现金和持仓，先在「持仓」里填上现金，就会按排名建仓。".to_string()
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
    let mut notes = vec![if rebalance {
        format!("今天是本月最后一个交易日：按因子排名整体调仓，持有前 {top_n} 只")
    } else {
        "每月最后一个交易日收盘整体调仓；其余交易日只在没有持满时补仓".to_string()
    }];
    notes.push("多因子账户不看勾选的策略、至少几个策略同时看好、最多持有只数和弱市规则".into());
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
        settings_hint: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::lots::Fees;
    use crate::account::rules::Preset;
    use crate::advice::generate::{self, HoldingIn, Snapshot};

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

    fn holding(code: &str, shares: i64) -> HoldingIn {
        HoldingIn {
            thscode: code.into(),
            name: format!("股票{code}"),
            board: "main".into(),
            asset_type: "stock".into(),
            shares,
            cost_price: Some(10.0),
            opened_on: None,
            bars_held: None,
            strategy: None,
            strategy_exit: None,
            is_st: false,
            delisted: false,
            last_date: Some(day()),
            snap: Some(snap(10.0)),
        }
    }

    fn cand(code: &str, rank: usize) -> FactorCandidate {
        FactorCandidate {
            rank,
            score: 2.0 - rank as f64 / 100.0,
            stock: CandidateIn {
                thscode: code.into(),
                name: format!("候选{code}"),
                board: "main".into(),
                asset_type: "stock".into(),
                is_st: false,
                picks: vec![],
                snap: snap(10.0),
            },
        }
    }

    fn input(
        next: NaiveDate,
        cash: f64,
        holdings: Vec<HoldingIn>,
        ranks: &[(&str, usize)],
        cands: Vec<FactorCandidate>,
    ) -> Input {
        let rules = crate::account::rules::Rules {
            factor: Some(FactorPlan {
                top_n: 5,
                buffer: 1.0,
                ..FactorPlan::combo_e()
            }),
            ..Preset::Factor.rules()
        };
        Input {
            trade_date: day(),
            next_date: next,
            rules,
            fees: Fees {
                commission_rate: 0.00025,
                min_commission: 5.0,
            },
            cash,
            market: None,
            holdings,
            candidates: vec![],
            history_rejected: 0,
            strategies: vec![],
            streaks: BTreeMap::new(),
            factor: Some(FactorIn {
                rebalance: month_end(day(), next),
                ranked: 3000,
                holding_ranks: ranks.iter().map(|(c, r)| (c.to_string(), *r)).collect(),
                candidates: cands,
            }),
        }
    }

    fn month_end_next() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()
    }

    #[test]
    fn empty_account_builds_equal_weight_portfolio_right_away() {
        let cands = (1..=8).map(|r| cand(&format!("{r:06}.SZ"), r)).collect();
        let a = generate::generate(&input(
            day().succ_opt().unwrap(),
            100_000.0,
            vec![],
            &[],
            cands,
        ));
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
        let hs = vec![
            holding("A.SZ", 1000),
            holding("B.SZ", 1000),
            holding("C.SZ", 1000),
            holding("D.SZ", 1000),
            holding("E.SZ", 1000),
        ];
        // keep_limit = 5 × 2 = 10：A 第 3 名、B 第 9 名留下，C 第 11 名卖出，D 不在排名里卖出
        let ranks = [("A.SZ", 3), ("B.SZ", 9), ("C.SZ", 11), ("E.SZ", 1)];
        let cands = vec![cand("X.SZ", 2), cand("Y.SZ", 4), cand("Z.SZ", 5)];
        let a = generate::generate(&input(
            month_end_next(),
            20_000.0,
            hs.clone(),
            &ranks,
            cands.clone(),
        ));
        let act = |c: &str| a.items.iter().find(|i| i.thscode == c).map(|i| i.action);
        assert_eq!(act("A.SZ"), Some(Action::Hold));
        assert_eq!(act("B.SZ"), Some(Action::Hold));
        assert_eq!(act("C.SZ"), Some(Action::Sell));
        assert_eq!(act("D.SZ"), Some(Action::Sell));
        assert_eq!(act("X.SZ"), Some(Action::Buy));
        assert_eq!(act("Y.SZ"), Some(Action::Buy));
        assert_eq!(act("Z.SZ"), None, "只空出 2 个位置");

        // 不是月末：持满了就不动，跌出缓冲区的也等到月末
        let a = generate::generate(&input(
            day().succ_opt().unwrap(),
            20_000.0,
            hs,
            &ranks,
            cands,
        ));
        assert_eq!(a.headline, "不用操作");
        assert!(a.items.iter().all(|i| i.action == Action::Hold));
    }

    #[test]
    fn missing_ranking_holds_everything() {
        let mut inp = input(
            month_end_next(),
            50_000.0,
            vec![holding("A.SZ", 100)],
            &[],
            vec![],
        );
        if let Some(f) = inp.factor.as_mut() {
            f.ranked = 0;
        }
        let a = generate::generate(&inp);
        assert_eq!(a.headline, "不用操作");
        assert!(a.why_nothing.unwrap().contains("没算出来"));
        assert_eq!(a.items[0].action, Action::Hold);
    }

    #[test]
    fn empty_cash_explains_instead_of_listing_every_candidate() {
        let cands = (1..=8).map(|r| cand(&format!("{r:06}.SZ"), r)).collect();
        let a = generate::generate(&input(day().succ_opt().unwrap(), 0.0, vec![], &[], cands));
        assert!(a.skipped.is_empty());
        assert!(a.why_nothing.unwrap().contains("还没有现金"));
    }

    #[test]
    fn month_end_detection() {
        assert!(month_end(day(), month_end_next()));
        assert!(!month_end(day(), day().succ_opt().unwrap()));
    }
}
