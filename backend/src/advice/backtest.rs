//! 规则回测：假设过去一段时间每天都按一组规则操作，账户总值怎么变。只在用户主动发起时用账户自己的规则现算，
//! 结果不参与账户建议。和每日建议用同一套选股优先级和风控规则：
//! - 候选：参考的策略当天的买点（和巡检相同的买卖点规则）；启用 `history_check` 时，按信号日以前的数据检查各个回看周期；
//! - 未启用 `history_check` 时，所有勾选策略都可提供候选；启用后只有通过回看的策略才计入 `min_picks`；
//! - 排序：同时选中的策略数 → 成交额，至少 `min_picks` 个；
//! - 卖出：买入依据策略的完整卖出规则；最多持几只、单只上限、保留现金、弱市、追高上限。
//!
//! 近似：价格用前复权价（分红送转已在价格里），允许零股，佣金和印花税照扣；ST 按本地历史简称判断，
//! 没有记录时回退当前简称，启用 `sell_st` 时持有中被 ST 会在下一个交易日卖出；不做单只超过上限时的减仓。
//! 规则见 docs/domains/accounts.md#rules_backtest。

use std::collections::{BTreeSet, HashMap};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::account::history::{self, HistorySeries};
use crate::account::lots::{Fees, round2};
use crate::account::rules::{Rules, WeakMarket};
use crate::analysis::backtest::{Costs, Side, TRAILING_ATR_PERIOD};
use crate::analysis::indicators::{Frame, IndicatorId, IndicatorRef};
use crate::analysis::market::{self, calendar_days_for};
use crate::analysis::points;
use crate::analysis::prices;
use crate::analysis::strategy::Definition;

/// 买卖点从回测开始日之前几年空仓起步推算，开始日那天的持有状态才和巡检一致；只取开始日之后的买点
pub const YEARS: i64 = 3;

/// 回测区间最长几年
pub const MAX_YEARS: i64 = 5;

/// 回测区间：开始日期为空时是结束日期那年的 1 月 1 日（今年以来），结束日期为空或晚于最新交易日时取最新交易日
pub fn resolve_range(
    latest: NaiveDate,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
) -> Result<(NaiveDate, NaiveDate), String> {
    let end = end.map_or(latest, |e| e.min(latest));
    let start = start.unwrap_or_else(|| NaiveDate::from_ymd_opt(end.year(), 1, 1).unwrap_or(end));
    if start >= end {
        return Err("开始日期要早于结束日期（结束日期最晚是最新交易日）".into());
    }
    if end - start > chrono::Duration::days(365 * MAX_YEARS + 2) {
        return Err(format!("回测区间最长 {MAX_YEARS} 年"));
    }
    Ok((start, end))
}
/// 一次最多参考多少个策略（按位记录）
pub const MAX_STRATEGIES: usize = 64;
/// 本金（只影响金额，不影响收益率）
const CAPITAL: f64 = 1_000_000.0;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RulesBacktestSummary {
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// 区间涨跌（0.1 表示 10%）
    pub total_return: f64,
    pub annual_return: Option<f64>,
    /// 最大回撤（0.2 表示从最高点跌了 20%）
    pub max_drawdown: f64,
    pub risk: crate::analysis::research::RiskMetrics,
    pub benchmark_name: Option<String>,
    pub benchmark_return: Option<f64>,
    /// 已卖出的交易笔数
    pub trades: usize,
    pub win_rate: Option<f64>,
    /// 平均每天持有几只
    pub avg_positions: f64,
    /// 平均现金占比
    pub avg_cash_ratio: f64,
    /// 参考了几个策略，以及回测期间平均有几个算作有效
    pub strategies: usize,
    pub avg_effective: f64,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RulesEquityPoint {
    pub date: NaiveDate,
    /// 账户总值相对本金的倍数（1.1 表示赚了 10%）
    pub value: f64,
    /// 基准指数相对第一天的倍数
    pub benchmark: Option<f64>,
}

/// 回测中的一笔交易
#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RulesTrade {
    pub thscode: String,
    pub name: String,
    /// 出现买点的那天（收盘后知道）
    pub signal_date: NaiveDate,
    /// 下一个交易日开盘买入；价格是未复权开盘价
    pub entry_date: NaiveDate,
    pub entry_price: f64,
    /// 卖出日和未复权开盘价；回测结束时还持有为空
    pub exit_date: Option<NaiveDate>,
    pub exit_price: Option<f64>,
    /// 为什么卖：策略卖出规则；还持有为空
    pub exit_reason: Option<String>,
    /// 持有的交易日数（还持有时算到回测最后一天）
    pub days_held: u32,
    /// 投入的金额（本金 100 万元时）
    pub invest: f64,
    /// 盈亏金额和收益率（%，前复权，扣佣金和印花税；还持有的按最后收盘价，不扣卖出费用）
    pub profit: f64,
    pub return_pct: f64,
    /// 当时选中它的有效策略
    pub strategies: Vec<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug)]
pub struct RulesBacktest {
    pub summary: RulesBacktestSummary,
    pub manifest: crate::analysis::research::BacktestManifest,
    /// 每周一个点
    pub equity: Vec<RulesEquityPoint>,
    /// 全部交易，买入日新的在前；还持有的也在里面
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trades: Vec<RulesTrade>,
}

/// 一只股票在回测区间内每天的数据（前复权价）
struct Stock {
    code: String,
    name: String,
    asset_type: String,
    /// 第 k 天对应全局日历的下标
    day: Vec<u32>,
    /// 未复权开盘价：交易记录里展示用
    open: Vec<f32>,
    q_open: Vec<f32>,
    q_high: Vec<f32>,
    q_close: Vec<f32>,
    /// ATR(14)，用于买入依据策略的吊灯止损
    atr: Vec<f32>,
    one_price: Vec<bool>,
    /// 第 k 天哪些策略出现买点，按位
    buy: Vec<u64>,
    /// 第 k 天哪些策略的卖出条件满足，按位
    exit: Vec<u64>,
    /// 第 k 天是否处于 ST 状态。
    st: Vec<bool>,
    amount: Vec<f32>,
}

impl Stock {
    /// 全局第 d 天在这只股票里的下标；停牌时为空
    fn at(&self, d: u32) -> Option<usize> {
        self.day.binary_search(&d).ok()
    }
}

/// 某一年允许参与买入的策略（按位）。账户回测不再根据过去表现筛掉策略。
#[derive(Clone, Default, Debug)]
struct YearEdge {
    mask: u64,
}

// @project-doc docs/domains/accounts.md#rules_backtest
/// 回测 [start, end]（用 resolve_range 定好的区间）
pub async fn run(
    pool: &PgPool,
    rules: &Rules,
    all: &[(i64, Definition)],
    start: NaiveDate,
    end: NaiveDate,
    fees: Fees,
) -> anyhow::Result<Option<RulesBacktest>> {
    let used: Vec<&(i64, Definition)> = all
        .iter()
        .filter(|(id, _)| rules.uses(*id))
        .take(MAX_STRATEGIES)
        .collect();
    if used.is_empty() {
        return Ok(None);
    }
    let defs: Vec<&Definition> = used.iter().map(|(_, d)| d).collect();
    let names: HashMap<i64, String> =
        sqlx::query_as::<_, (i64, String)>("SELECT strategy_id, name FROM app.strategy")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    let strategy_names: Vec<String> = used
        .iter()
        .map(|(id, _)| {
            names
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("策略 {id}"))
        })
        .collect();
    let edge_from = start - chrono::Duration::days(365 * YEARS);
    let sec = market::securities(pool).await?;
    let asset_types: Vec<String> = defs
        .iter()
        .flat_map(|d| d.universe.asset_types.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let base_def = Definition::any_assets(asset_types.clone());
    let warmup = defs.iter().map(|d| d.warmup()).max().unwrap_or(30) + 5;
    let history_start = rules
        .history_check
        .as_ref()
        .and_then(|c| c.windows.iter().map(|w| w.start(start)).min())
        .unwrap_or(edge_from);
    let lo = edge_from.min(history_start) - chrono::Duration::days(calendar_days_for(warmup));
    // 权威涨跌停池：规则回测里的买卖点同样以供应商的名单为准
    let limits = crate::analysis::limit_pool::LimitPool::load(pool, lo, end).await?;
    let events = if rules.history_check.is_some() {
        prices::corp_events_for_window(pool, lo, end).await?
    } else {
        HashMap::new()
    };
    let mut raw: Vec<(Vec<NaiveDate>, Stock)> = vec![];
    let mut ew = crate::analysis::portfolio::EqualWeight::default();
    // 每天：收盘站上 20 日均线的股票数、当天有日线的股票数
    let mut breadth: HashMap<NaiveDate, (u32, u32)> = HashMap::new();
    prices::for_each_assets(
        pool,
        lo,
        end,
        asset_types.iter().any(|a| a == "stock"),
        asset_types.iter().any(|a| a == "etf"),
        |code, bars| {
            let Some(meta) = sec.meta.get(code) else {
                return;
            };
            let Some(first_edge) = bars.iter().position(|b| b.date >= edge_from) else {
                return;
            };
            let history = sec.st_history.get(code).map(Vec::as_slice).unwrap_or(&[]);
            let f = Frame::with_authority_and_st_history(bars, Some(meta), &limits, code, history);
            let first = bars.iter().position(|b| b.date >= start);
            let n = first.map_or(0, |f0| bars.len() - f0);
            let mut buy = vec![0u64; n];
            for (k, d) in defs.iter().enumerate() {
                let history_series = rules
                    .history_check
                    .as_ref()
                    .map(|_| HistorySeries::new(bars, &f, meta, d));
                for t in points::run(bars, &f, meta, d, first_edge).trades {
                    let Ok(s) = bars.binary_search_by_key(&t.signal_date, |b| b.date) else {
                        continue;
                    };
                    if let Some(f0) = first
                        && s >= f0
                        && !f.is_st(s)
                    {
                        if let (Some(check), Some(series)) = (&rules.history_check, &history_series)
                        {
                            let returns = series.evaluate(
                                bars,
                                events.get(code).map(Vec::as_slice).unwrap_or(&[]),
                                &check.windows,
                                t.signal_date,
                            );
                            if !history::passes(check, &returns, t.signal_date) {
                                continue;
                            }
                        }
                        buy[s - f0] |= 1 << k;
                    }
                }
            }
            let Some(f0) = first else { return };
            ew.add(&base_def, bars, &f, meta, f0);
            let ma20 = f.get(IndicatorRef::new(IndicatorId::Ma, Some(20)));
            let atr = f.get(IndicatorRef::new(
                IndicatorId::Atr,
                Some(TRAILING_ATR_PERIOD),
            ));
            let exits: Vec<Vec<bool>> = defs.iter().map(|d| d.exit_series(&f)).collect();
            let mut s = Stock {
                code: code.to_string(),
                name: sec.names.get(code).cloned().unwrap_or_default(),
                asset_type: if meta.asset_type == crate::instrument::AssetType::Etf {
                    "etf".into()
                } else {
                    "stock".into()
                },
                day: vec![],
                open: Vec::with_capacity(n),
                q_open: Vec::with_capacity(n),
                q_high: Vec::with_capacity(n),
                q_close: Vec::with_capacity(n),
                atr: Vec::with_capacity(n),
                one_price: Vec::with_capacity(n),
                buy,
                exit: Vec::with_capacity(n),
                st: Vec::with_capacity(n),
                amount: Vec::with_capacity(n),
            };
            let mut dates = Vec::with_capacity(n);
            for i in f0..bars.len() {
                let b = &bars[i];
                let e = breadth.entry(b.date).or_default();
                e.1 += 1;
                e.0 += (!ma20[i].is_nan() && b.q_close > ma20[i]) as u32;
                let mut em = 0u64;
                for (k, x) in exits.iter().enumerate() {
                    if x[i] {
                        em |= 1 << k;
                    }
                }
                dates.push(b.date);
                s.open.push(b.open as f32);
                s.q_open.push(b.q_open as f32);
                s.q_high.push(b.q_high as f32);
                s.q_close.push(b.q_close as f32);
                s.atr.push(atr[i] as f32);
                s.one_price.push(b.high == b.low);
                s.exit.push(em);
                s.st.push(f.is_st(i));
                s.amount.push(b.amount as f32);
            }
            raw.push((dates, s));
        },
    )
    .await?;
    let calendar: Vec<NaiveDate> = raw
        .iter()
        .flat_map(|(d, _)| d.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if calendar.len() < 20 {
        return Ok(None);
    }
    let index: HashMap<NaiveDate, u32> = calendar
        .iter()
        .enumerate()
        .map(|(i, d)| (*d, i as u32))
        .collect();
    let stocks: Vec<Stock> = raw
        .into_iter()
        .map(|(dates, mut s)| {
            s.day = dates.iter().map(|d| index[d]).collect();
            s
        })
        .collect();
    let mut signals: Vec<Vec<(usize, usize)>> = vec![vec![]; calendar.len()];
    for (si, s) in stocks.iter().enumerate() {
        for (k, &m) in s.buy.iter().enumerate() {
            if m != 0 {
                signals[s.day[k] as usize].push((si, k));
            }
        }
    }
    let all_mask = if defs.len() >= 64 {
        u64::MAX
    } else {
        (1u64 << defs.len()) - 1
    };
    let edges: HashMap<i32, YearEdge> = (calendar[0].year()..=calendar[calendar.len() - 1].year())
        .map(|y| (y, YearEdge { mask: all_mask }))
        .collect();
    let bench = ew.series(calendar[0], *calendar.last().expect("日历不为空"));
    let mut r = simulate(
        rules,
        &stocks,
        &signals,
        &calendar,
        &breadth,
        &edges,
        bench,
        &strategy_names,
        &defs,
        fees,
    );
    r.summary.strategies = defs.len();
    let versions: std::collections::BTreeMap<String, String> =
        sqlx::query_as("SELECT name, updated_at::text FROM ops.derived_version ORDER BY name")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    let manifest = crate::analysis::research::BacktestManifest {
        strategy_definition_hash: crate::analysis::research::definition_hash(&defs),
        data_start: r.summary.start,
        data_end: r.summary.end,
        price_basis: "forward_adjusted_account_approx".into(),
        execution_model: "account_rules;daily_open;full_fill;trailing_stop_atr".into(),
        point_in_time_universe: true,
        derived_versions: versions,
        trial_count: 1,
        validation_kind: "single_run_unvalidated".into(),
        coverage: crate::analysis::research::Coverage::default(),
        delisted_policy: "exclude_after_point_in_time_delisted_on;unknown_st_fallback_current"
            .into(),
    };
    r.manifest = manifest;
    Ok(Some(r))
}

struct Held {
    stock: usize,
    units: f64,
    cost: f64,
    entry_q: f64,
    /// 买入依据的策略（位）
    origin: u64,
    last_close: f64,
    /// 持有期间的最高前复权价，用于 ATR 吊灯止损
    peak: f64,
    /// 信号日、买入日（全局日历下标）、未复权买入价、当时选中它的有效策略（位）
    signal_day: u32,
    entry_day: u32,
    entry_raw: f64,
    picks: u64,
}

/// 明天开盘要卖的一只，以及为什么
struct PendingSell {
    stock: usize,
    reason: String,
}

/// 明天开盘要买的一只
struct PendingBuy {
    stock: usize,
    budget: f64,
    origin: u64,
    picks: u64,
    ref_close: f64,
    signal_day: u32,
}

/// 按位列出策略名
fn names_of(bits: u64, names: &[String]) -> Vec<String> {
    (0..names.len())
        .filter(|k| bits & (1 << k) != 0)
        .map(|k| names[k].clone())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn simulate(
    p: &Rules,
    stocks: &[Stock],
    signals: &[Vec<(usize, usize)>],
    calendar: &[NaiveDate],
    breadth: &HashMap<NaiveDate, (u32, u32)>,
    edges: &HashMap<i32, YearEdge>,
    bench: Option<(String, HashMap<NaiveDate, f64>)>,
    strategy_names: &[String],
    defs: &[&Definition],
    fees: Fees,
) -> RulesBacktest {
    let execution_costs = Costs {
        commission_rate: fees.commission_rate,
        min_commission: fees.min_commission,
        stamp_tax_rate: 0.0005,
        ..Costs::default()
    };
    let execution = execution_costs.execution();
    let mut cash = CAPITAL;
    let mut held: Vec<Held> = vec![];
    let mut pending_sell: Vec<PendingSell> = vec![];
    let mut pending_buy: Vec<PendingBuy> = vec![];
    let mut trades: Vec<RulesTrade> = vec![];
    let trade = |h: &Held, exit: Option<(u32, f64, String)>, net: f64, end_day: u32| {
        let s = &stocks[h.stock];
        RulesTrade {
            thscode: s.code.clone(),
            name: s.name.clone(),
            signal_date: calendar[h.signal_day as usize],
            entry_date: calendar[h.entry_day as usize],
            entry_price: round2(h.entry_raw),
            exit_date: exit.as_ref().map(|e| calendar[e.0 as usize]),
            exit_price: exit.as_ref().map(|e| round2(e.1)),
            days_held: exit.as_ref().map_or(end_day, |e| e.0) - h.entry_day,
            exit_reason: exit.map(|e| e.2),
            invest: round2(h.cost),
            profit: round2(net - h.cost),
            return_pct: (net / h.cost - 1.0) * 100.0,
            strategies: names_of(h.picks, strategy_names),
        }
    };
    let (mut peak, mut max_dd) = (CAPITAL, 0.0f64);
    let (mut closed, mut wins) = (0usize, 0usize);
    let (mut pos_sum, mut cash_ratio_sum, mut eff_sum) = (0.0, 0.0, 0.0);
    let mut equity: Vec<RulesEquityPoint> = vec![];
    // 风险指标按日收益计算；响应里的 equity 仍每周一个点
    let mut daily_equity: Vec<f64> = Vec::with_capacity(calendar.len());
    let mut last_week = None;
    let none = YearEdge::default();
    for (d, date) in calendar.iter().enumerate() {
        let d32 = d as u32;
        let year = edges.get(&date.year()).unwrap_or(&none);
        eff_sum += year.mask.count_ones() as f64;
        // 开盘：卖出（一字跌停或停牌顺延），再买入（停牌、一字涨停、高开超过追高上限时放弃）
        let mut still = vec![];
        for ps in pending_sell.drain(..) {
            let s = &stocks[ps.stock];
            let Some(k) = s.at(d32) else {
                still.push(ps);
                continue;
            };
            if s.one_price[k] && k > 0 && s.q_open[k] < s.q_close[k - 1] {
                still.push(ps);
                continue;
            }
            if let Some(pos) = held.iter().position(|h| h.stock == ps.stock) {
                let h = held.swap_remove(pos);
                let bar = crate::analysis::prices::DailyBar {
                    date: *date,
                    open: s.q_open[k] as f64,
                    high: s.q_open[k] as f64,
                    low: s.q_open[k] as f64,
                    close: s.q_close[k] as f64,
                    q_open: s.q_open[k] as f64,
                    q_high: s.q_open[k] as f64,
                    q_low: s.q_open[k] as f64,
                    q_close: s.q_close[k] as f64,
                    volume: f64::INFINITY,
                    amount: 0.0,
                };
                let Some(fill) = execution.fill(
                    &bar,
                    Side::Sell,
                    h.units,
                    &execution_costs,
                    if s.asset_type == "etf" { 0.0 } else { 0.0005 },
                ) else {
                    still.push(ps);
                    continue;
                };
                let gross = fill.filled * fill.price;
                let net = gross - fill.commission - fill.transfer_fee - fill.stamp_tax;
                cash += net;
                closed += 1;
                wins += (net > h.cost) as usize;
                trades.push(trade(
                    &h,
                    Some((d32, s.open[k] as f64, ps.reason)),
                    net,
                    d32,
                ));
            }
        }
        pending_sell = still;
        for b in pending_buy.drain(..) {
            let s = &stocks[b.stock];
            let Some(k) = s.at(d32) else { continue };
            let open = s.q_open[k] as f64;
            if (s.one_price[k] && open > b.ref_close)
                || open / b.ref_close - 1.0 > p.chase_limit_pct / 100.0
            {
                continue;
            }
            let budget = b.budget.min(cash);
            let bar = crate::analysis::prices::DailyBar {
                date: *date,
                open,
                high: open,
                low: open,
                close: s.q_close[k] as f64,
                q_open: open,
                q_high: open,
                q_low: open,
                q_close: s.q_close[k] as f64,
                volume: f64::INFINITY,
                amount: 0.0,
            };
            let Some(price) = execution.estimate_price(&bar, Side::Buy) else {
                continue;
            };
            // 先留出佣金和过户费，最后一份预算也能真正买进
            let affordable = execution_costs.affordable_amount(budget);
            if affordable < price {
                continue;
            }
            let Some(fill) =
                execution.fill(&bar, Side::Buy, affordable / price, &execution_costs, 0.0)
            else {
                continue;
            };
            let fee = fill.commission + fill.transfer_fee;
            let total_cost = fill.filled * fill.price + fee;
            if fill.filled <= 0.0 || total_cost <= fee || total_cost > cash + 1e-8 {
                continue;
            }
            cash -= total_cost;
            held.push(Held {
                stock: b.stock,
                units: fill.filled,
                cost: total_cost,
                entry_q: open,
                origin: b.origin,
                last_close: open,
                peak: s.q_high[k] as f64,
                signal_day: b.signal_day,
                entry_day: d32,
                entry_raw: s.open[k] as f64,
                picks: b.picks,
            });
        }

        // 收盘：市值和持有期最高价
        for h in held.iter_mut() {
            if let Some(k) = stocks[h.stock].at(d32) {
                h.last_close = stocks[h.stock].q_close[k] as f64;
                h.peak = h.peak.max(stocks[h.stock].q_high[k] as f64);
            }
        }
        let value = cash + held.iter().map(|h| h.units * h.last_close).sum::<f64>();
        peak = peak.max(value);
        max_dd = max_dd.max(1.0 - value / peak);
        daily_equity.push(value / CAPITAL);
        pos_sum += held.len() as f64;
        cash_ratio_sum += cash / value.max(1.0);
        let week = date.format("%G-%V").to_string();
        if last_week.as_ref() != Some(&week) || d + 1 == calendar.len() {
            last_week = Some(week);
            equity.push(RulesEquityPoint {
                date: *date,
                value: value / CAPITAL,
                benchmark: bench.as_ref().and_then(|(_, b)| b.get(date).copied()),
            });
        }

        // 收盘后：决定明天卖什么
        for h in &held {
            if pending_sell.iter().any(|x| x.stock == h.stock) {
                continue;
            }
            let s = &stocks[h.stock];
            let Some(k) = s.at(d32) else { continue };
            let close = s.q_close[k] as f64;
            let ret = close / h.entry_q - 1.0;
            let strategy_index = h.origin.trailing_zeros() as usize;
            let strategy = defs.get(strategy_index).copied();
            // 与单股回测、每日建议一致：策略卖出条件 → 止损 → 止盈 → ATR 吊灯 → 最长持有，
            // 策略规则都没有触发时才看账户的 sell_st。
            let reason = if s.exit[k] & h.origin != 0 {
                Some(format!(
                    "「{}」的卖出条件",
                    names_of(h.origin, strategy_names).join("")
                ))
            } else if let Some(loss) = strategy
                .and_then(|d| d.exit.stop_loss_pct)
                .filter(|loss| ret <= -*loss / 100.0)
            {
                Some(format!(
                    "策略止损：亏 {:.1}%（阈值 {}%）",
                    -ret * 100.0,
                    loss
                ))
            } else if let Some(profit) = strategy
                .and_then(|d| d.exit.take_profit_pct)
                .filter(|profit| ret >= *profit / 100.0)
            {
                Some(format!(
                    "策略止盈：赚 {:.1}%（阈值 {}%）",
                    ret * 100.0,
                    profit
                ))
            } else if let Some(multiple) = strategy
                .and_then(|d| d.exit.trailing_stop_atr)
                .filter(|m| m.is_finite() && *m > 0.0)
            {
                let atr = s.atr[k] as f64;
                let hit = d32 > h.entry_day
                    && atr.is_finite()
                    && atr > 0.0
                    && close < h.peak - multiple * atr;
                hit.then(|| format!("策略回撤止损：收盘价跌破 {multiple} 倍 ATR(14) 的吊灯线"))
            } else if let Some(days) = strategy
                .and_then(|d| d.exit.max_hold_days)
                .filter(|days| d32 + 1 - h.entry_day >= *days)
            {
                Some(format!("策略持有满 {days} 个交易日"))
            } else if p.sell_st && s.st[k] {
                Some("账户规则：当天被实施风险警示（ST）".to_string())
            } else {
                None
            };
            if let Some(reason) = reason {
                pending_sell.push(PendingSell {
                    stock: h.stock,
                    reason,
                });
            }
        }
        // 再决定明天买什么
        let (above, total) = breadth.get(date).copied().unwrap_or((0, 0));
        let weak = total > 0 && (above as f64 / total as f64 * 100.0) < p.weak_threshold_pct;
        if weak && p.weak_market == WeakMarket::Pause {
            continue;
        }
        let mut slots =
            p.max_positions as i64 - held.len().saturating_sub(pending_sell.len()) as i64;
        let proceeds: f64 = held
            .iter()
            .filter(|h| pending_sell.iter().any(|x| x.stock == h.stock))
            .map(|h| h.units * h.last_close)
            .sum();
        let mut available = cash + proceeds - value * p.min_cash_pct / 100.0;
        if weak && p.weak_market == WeakMarket::Half {
            available *= 0.5;
        }
        let mut list: Vec<(usize, usize, u64)> = signals[d]
            .iter()
            .map(|&(si, k)| (si, k, stocks[si].buy[k] & year.mask))
            .filter(|(si, _, m)| {
                m.count_ones() >= p.min_picks && !held.iter().any(|h| h.stock == *si)
            })
            .collect();
        list.sort_by(|a, b| {
            b.2.count_ones()
                .cmp(&a.2.count_ones())
                .then_with(|| stocks[b.0].amount[b.1].total_cmp(&stocks[a.0].amount[a.1]))
        });
        for (si, k, m) in list {
            if slots <= 0 || available <= 0.0 {
                break;
            }
            let target = (available / slots as f64).min(value * p.max_weight_pct / 100.0);
            pending_buy.push(PendingBuy {
                stock: si,
                budget: target,
                origin: m.isolate_lowest_one(),
                picks: m,
                ref_close: stocks[si].q_close[k] as f64,
                signal_day: d32,
            });
            available -= target;
            slots -= 1;
        }
    }
    let n = calendar.len();
    // 回测结束时还持有的：按最后收盘价算，不扣卖出费用
    for h in &held {
        trades.push(trade(h, None, h.units * h.last_close, n as u32 - 1));
    }
    trades.sort_by(|a, b| {
        b.entry_date
            .cmp(&a.entry_date)
            .then_with(|| a.thscode.cmp(&b.thscode))
    });
    let last = equity.last().map_or(1.0, |e| e.value);
    let days = (calendar[n - 1] - calendar[0]).num_days();
    let annual_return = (days >= 90).then(|| last.powf(365.25 / days as f64) - 1.0);
    let risk = crate::analysis::research::risk_metrics(&daily_equity, annual_return, max_dd);
    RulesBacktest {
        summary: RulesBacktestSummary {
            start: calendar[0],
            end: calendar[n - 1],
            total_return: last - 1.0,
            annual_return,
            max_drawdown: max_dd,
            risk,
            benchmark_name: bench.as_ref().map(|(name, _)| name.clone()),
            benchmark_return: equity
                .iter()
                .rev()
                .find_map(|e| e.benchmark)
                .map(|b| b - 1.0),
            trades: closed,
            win_rate: (closed > 0).then(|| wins as f64 / closed as f64),
            avg_positions: pos_sum / n as f64,
            avg_cash_ratio: cash_ratio_sum / n as f64,
            strategies: 0,
            avg_effective: eff_sum / n as f64,
        },
        manifest: crate::analysis::research::BacktestManifest {
            strategy_definition_hash: String::new(),
            data_start: calendar[0],
            data_end: calendar[n - 1],
            price_basis: String::new(),
            execution_model: String::new(),
            point_in_time_universe: false,
            derived_versions: std::collections::BTreeMap::new(),
            trial_count: 0,
            validation_kind: String::new(),
            coverage: crate::analysis::research::Coverage::default(),
            delisted_policy: String::new(),
        },
        equity,
        trades,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::rules::Preset;

    /// 两只股票、10 天：A 只被策略 1 选中、一路上涨；B 被两个策略选中、之后跌破止损。开盘价等于前一天收盘价
    fn stocks() -> (Vec<Stock>, Vec<NaiveDate>) {
        let cal: Vec<NaiveDate> = (0..10)
            .map(|i| NaiveDate::from_ymd_opt(2026, 9, 1).unwrap() + chrono::Duration::days(i))
            .collect();
        let mk = |closes: Vec<f32>, buy_days: &[usize], bits: u64| Stock {
            code: "T".into(),
            name: "测试".into(),
            asset_type: "stock".into(),
            day: (0..10).collect(),
            open: (0..10).map(|i| closes[i.max(1) - 1]).collect(),
            q_open: (0..10).map(|i| closes[i.max(1) - 1]).collect(),
            q_high: closes.clone(),
            q_close: closes.clone(),
            atr: vec![1.0; 10],
            one_price: vec![false; 10],
            buy: (0..10)
                .map(|i| if buy_days.contains(&i) { bits } else { 0 })
                .collect(),
            exit: vec![0; 10],
            st: vec![false; 10],
            amount: vec![1e9; 10],
        };
        let a = mk(
            (0..10).map(|i| 10.0 + i as f32).collect(),
            &[0, 1, 2, 3],
            0b01,
        );
        let mut b = mk(
            vec![10.0, 10.0, 10.0, 9.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0],
            &[1],
            0b11,
        );
        b.exit[3] = 0b11;
        (vec![a, b], cal)
    }

    fn signals(stocks: &[Stock]) -> Vec<Vec<(usize, usize)>> {
        let mut out: Vec<Vec<(usize, usize)>> = vec![vec![]; 10];
        for (si, s) in stocks.iter().enumerate() {
            for (k, m) in s.buy.iter().enumerate() {
                if *m != 0 {
                    out[k].push((si, k));
                }
            }
        }
        out
    }

    #[test]
    fn selected_strategies_count_and_stops_out() {
        let (stocks, cal) = stocks();
        let breadth: HashMap<NaiveDate, (u32, u32)> = cal.iter().map(|d| (*d, (60, 100))).collect();
        let p = Preset::Aggressive.rules();
        let names = vec!["策略一".to_string(), "策略二".to_string()];
        let defs_owned = [
            Definition::any_assets(vec!["stock".into()]),
            Definition::any_assets(vec!["stock".into()]),
        ];
        let defs: Vec<&Definition> = defs_owned.iter().collect();
        // 两个策略都有效：A 第 1 天买入一路涨；B 第 2 天买入，第 3 天命中策略卖出条件，第 4 天卖出
        let both: HashMap<i32, YearEdge> = HashMap::from([(2026, YearEdge { mask: 0b11 })]);
        let r = simulate(
            &p,
            &stocks,
            &signals(&stocks),
            &cal,
            &breadth,
            &both,
            None,
            &names,
            &defs,
            Fees::default(),
        );
        assert_eq!(r.summary.trades, 1, "{:?}", r.summary);
        assert_eq!(r.summary.win_rate, Some(0.0));
        assert!(r.summary.total_return > 0.0, "{:?}", r.summary);
        assert!(r.equity.len() >= 2 && r.equity[0].date == cal[0]);
        // 交易记录：B 命中策略卖出条件，A 期末还持有；买入日新的在前
        assert_eq!(r.trades.len(), 2, "{:?}", r.trades);
        let b = r.trades.iter().find(|t| t.exit_date.is_some()).unwrap();
        // 第 3 天收盘命中策略卖出条件，下一个交易日开盘卖出
        assert_eq!(
            (b.signal_date, b.entry_date, b.exit_date),
            (cal[1], cal[2], Some(cal[4]))
        );
        assert_eq!(b.exit_reason.as_deref(), Some("「策略一」的卖出条件"));
        assert_eq!(b.strategies, ["策略一", "策略二"]);
        assert!(b.return_pct < 0.0 && b.profit < 0.0 && b.days_held == 2);
        let a = r.trades.iter().find(|t| t.exit_date.is_none()).unwrap();
        assert!(a.return_pct > 0.0 && a.exit_reason.is_none());
        assert!(r.trades[0].entry_date >= r.trades[1].entry_date);
        // 至少 2 个勾选策略同时选中时，只买 B
        let strict = Rules { min_picks: 2, ..p };
        let r = simulate(
            &strict,
            &stocks,
            &signals(&stocks),
            &cal,
            &breadth,
            &both,
            None,
            &names,
            &defs,
            Fees::default(),
        );
        assert_eq!(r.summary.trades, 1);
        assert!(r.summary.total_return < 0.0);
    }

    #[test]
    fn trailing_stop_uses_atr_and_precedes_sell_st() {
        let cal: Vec<NaiveDate> = (0..6)
            .map(|i| NaiveDate::from_ymd_opt(2026, 9, 1).unwrap() + chrono::Duration::days(i))
            .collect();
        let closes = [10.0f32, 10.0, 10.0, 10.0, 8.0, 8.0];
        let stock = Stock {
            code: "T".into(),
            name: "测试".into(),
            asset_type: "stock".into(),
            day: (0..6).collect(),
            open: vec![10.0, 10.0, 10.0, 10.0, 8.0, 8.0],
            q_open: vec![10.0, 10.0, 10.0, 10.0, 8.0, 8.0],
            q_high: closes.to_vec(),
            q_close: closes.to_vec(),
            atr: vec![1.0; 6],
            one_price: vec![false; 6],
            buy: vec![1, 0, 0, 0, 0, 0],
            exit: vec![0; 6],
            // 第 4 天同时被 ST：策略吊灯止损应先触发
            st: vec![false, false, false, false, true, true],
            amount: vec![1e9; 6],
        };
        let stocks = vec![stock];
        let breadth: HashMap<NaiveDate, (u32, u32)> = cal.iter().map(|d| (*d, (60, 100))).collect();
        let mut strategy = Definition::any_assets(vec!["stock".into()]);
        strategy.exit.trailing_stop_atr = Some(1.0);
        let defs = [&strategy];
        let names = vec!["策略一".to_string()];
        let edges: HashMap<i32, YearEdge> = HashMap::from([(2026, YearEdge { mask: 1 })]);
        let rules = Preset::Balanced.rules();
        assert!(rules.sell_st);
        let r = simulate(
            &rules,
            &stocks,
            &signals(&stocks),
            &cal,
            &breadth,
            &edges,
            None,
            &names,
            &defs,
            Fees::default(),
        );
        let t = &r.trades[0];
        assert_eq!(t.entry_date, cal[1]);
        assert_eq!(t.exit_date, Some(cal[5]), "第 4 天收盘触发，次一交易日卖出");
        assert!(
            t.exit_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("回撤止损")),
            "{:?}",
            t.exit_reason
        );
    }

    #[test]
    fn ranges_default_to_this_year_and_stop_at_latest_day() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        let latest = d(2026, 10, 16);
        assert_eq!(
            resolve_range(latest, None, None),
            Ok((d(2026, 1, 1), latest)),
            "默认今年以来"
        );
        assert_eq!(
            resolve_range(latest, Some(d(2026, 7, 16)), Some(d(2026, 12, 31))),
            Ok((d(2026, 7, 16), latest)),
            "结束日期不晚于最新交易日"
        );
        assert!(resolve_range(latest, Some(d(2026, 10, 16)), None).is_err());
        assert!(
            resolve_range(latest, Some(d(2020, 1, 1)), None).is_err(),
            "最长 5 年"
        );
    }
}
