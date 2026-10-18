//! 从数据库组装生成建议的输入：所有账户共用的部分（交易日、市场、巡检结果、策略表现）读一次，
//! 每个账户再读自己的持仓股的日线和诊断。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate, Weekday};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::generate::{
    CandidateIn, HoldingIn, Input, MarketIn, Pick, Snapshot, StrategyIn, StrategyRef,
};
use crate::account::history::{HistoryReturn, HistorySeries};
use crate::account::rules::{HistoryWindow, Rules};
use crate::account::{self, Account};
use crate::analysis::diagnosis::{self, Verdict};
use crate::analysis::indicators::{Frame, IndicatorId, IndicatorRef, StockMeta};
use crate::analysis::limit_pool::LimitPool;
use crate::analysis::market::{self, Securities};
use crate::analysis::prices::{self, DailyBar};
use crate::analysis::screener;

/// 读多少根日线：覆盖诊断和 120 日均线
const BARS: i64 = diagnosis::BARS_NEEDED;
/// 每个账户最多细看多少只候选（按选中的策略数和成交额预先排好）
const MAX_CANDIDATES: usize = 40;

/// (策略, 股票, 命中条件, 成交额)
type HitRow = (i64, String, sqlx::types::Json<Vec<String>>, Option<f64>);

/// 所有账户共用的输入
pub struct Shared {
    pub trade_date: NaiveDate,
    pub next_date: NaiveDate,
    pub market: Option<MarketIn>,
    sec: Securities,
    /// 参加巡检的策略（候选股的来源），按编号
    screening: Vec<(i64, String)>,
    names: HashMap<i64, String>,
    definitions: HashMap<i64, serde_json::Value>,
    /// 当天买点的权威涨跌停池（区间覆盖每只股票要读的日线）
    limits: LimitPool,
    /// 当天的买点：(策略, 股票, 命中条件, 成交额)
    hits: Vec<(i64, String, Vec<String>, f64)>,
    delisted: HashMap<String, bool>,
    history: HashMap<(i64, String), Vec<HistoryReturn>>,
}

/// 参加巡检的策略最近一次巡检的交易日：建议跟着它走
pub async fn latest_screen_date(pool: &PgPool) -> sqlx::Result<Option<NaiveDate>> {
    sqlx::query_scalar("SELECT max(r.trade_date) FROM app.screen_run r JOIN app.strategy s USING (strategy_id) WHERE s.screening").fetch_one(pool).await
}

/// Whether every currently enabled screening strategy has a result for a date.
pub async fn screening_complete(pool: &PgPool, date: NaiveDate) -> sqlx::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM app.strategy WHERE screening)
                AND NOT EXISTS (
                    SELECT 1 FROM app.strategy s
                    WHERE s.screening AND NOT EXISTS (
                        SELECT 1 FROM app.screen_run r WHERE r.strategy_id = s.strategy_id AND r.trade_date = $1
                    )
                )",
    )
    .bind(date)
    .fetch_one(pool)
    .await
}

/// 交易日之后的下一个交易日：本地日历有就用，没有时取下一个工作日
pub async fn next_trading_day(pool: &PgPool, d: NaiveDate) -> sqlx::Result<NaiveDate> {
    let next: Option<NaiveDate> =
        sqlx::query_scalar("SELECT min(cal_date) FROM ref.trading_day WHERE cal_date > $1")
            .bind(d)
            .fetch_one(pool)
            .await?;
    Ok(next.unwrap_or_else(|| {
        let mut n = d.succ_opt().unwrap_or(d);
        while matches!(n.weekday(), Weekday::Sat | Weekday::Sun) {
            n = n.succ_opt().unwrap_or(n);
        }
        n
    }))
}

/// 还没有内置策略的巡检结果时为空
pub async fn shared(pool: &PgPool) -> anyhow::Result<Option<Shared>> {
    // 核心数据的首次回补还没完成：日线不全，算出来的建议不可信，先不生成
    if crate::sync::audit::core_backfill_left(pool).await? > 0 {
        return Ok(None);
    }
    let Some(trade_date) = latest_screen_date(pool).await? else {
        return Ok(None);
    };
    if !screening_complete(pool, trade_date).await? {
        return Ok(None);
    }
    let next_date = next_trading_day(pool, trade_date).await?;
    let market = market::overview(pool, Some(trade_date))
        .await?
        .map(|o| MarketIn {
            above_ma20_pct: o.above_ma20 * 100.0,
            stocks: o.stocks,
        });
    let sec = market::securities(pool).await?;
    // 权威涨跌停池：持仓股和候选股的诊断、卖出条件里的涨跌停信号共用，整个读取窗口加载一次
    let lo = trade_date - chrono::Duration::days(market::calendar_days_for(BARS as u32));
    let limits = LimitPool::load(pool, lo, trade_date).await?;
    let names: HashMap<i64, String> =
        sqlx::query_as::<_, (i64, String)>("SELECT strategy_id, name FROM app.strategy")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    let definitions: HashMap<i64, serde_json::Value> =
        sqlx::query_as::<_, (i64, serde_json::Value)>(
            "SELECT strategy_id, definition FROM app.strategy",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    let screening: Vec<(i64, String)> = sqlx::query_as(
        "SELECT strategy_id, name FROM app.strategy WHERE screening ORDER BY sort_order, strategy_id",
    )
    .fetch_all(pool)
    .await?;
    let ids: Vec<i64> = screening.iter().map(|(id, _)| *id).collect();
    let rows: Vec<HitRow> = sqlx::query_as(
        "SELECT r.strategy_id, h.thscode, h.reasons, (h.metrics->>'amount')::float8 FROM app.screen_hit h JOIN app.screen_run r USING (run_id)
         JOIN app.strategy s USING (strategy_id)
         WHERE r.trade_date = $1 AND h.kind = 'buy' AND r.strategy_id = ANY($2)
         ORDER BY s.sort_order, r.strategy_id, h.thscode",
    )
    .bind(trade_date)
    .bind(&ids)
    .fetch_all(pool)
    .await?;
    let hits: Vec<_> = rows
        .into_iter()
        .map(|(id, code, r, amount)| (id, code, r.0, amount.unwrap_or(0.0)))
        .collect();
    // 只有启用回看的账户所选策略的候选需要长历史；同一股票只读取一次。
    let account_rules: Vec<sqlx::types::Json<Rules>> =
        sqlx::query_scalar("SELECT rules FROM app.account")
            .fetch_all(pool)
            .await?;
    let defs = screener::screening_strategies(pool).await?;
    let checked: Vec<_> = defs
        .iter()
        .filter(|(id, _)| {
            account_rules
                .iter()
                .any(|r| r.history_check.is_some() && r.uses(*id))
        })
        .collect();
    let mut history = HashMap::new();
    if !checked.is_empty() {
        let warmup = checked.iter().map(|(_, d)| d.warmup()).max().unwrap_or(30) + 5;
        let start = HistoryWindow::FiveYears.start(trade_date)
            - chrono::Duration::days(market::calendar_days_for(warmup));
        let history_limits = LimitPool::load(pool, start, trade_date).await?;
        let codes: std::collections::BTreeSet<_> = hits
            .iter()
            .filter(|(id, _, _, _)| checked.iter().any(|(sid, _)| sid == id))
            .map(|(_, code, _, _)| code)
            .collect();
        for code in codes {
            let Some(meta) = sec.meta.get(code) else {
                continue;
            };
            let bars = prices::stock_bars_between(pool, code, start, trade_date).await?;
            let events = prices::corp_events(pool, code, Some(trade_date)).await?;
            let st = sec.st_history.get(code).map(Vec::as_slice).unwrap_or(&[]);
            prices::run_cpu(|| {
                let frame = Frame::with_authority_and_st_history(
                    &bars,
                    Some(meta),
                    &history_limits,
                    code,
                    st,
                );
                for (id, def) in &checked {
                    if hits.iter().any(|(sid, c, _, _)| sid == id && c == code) {
                        let series = HistorySeries::new(&bars, &frame, meta, def);
                        history.insert(
                            (*id, code.clone()),
                            series.evaluate(&bars, &events, &HistoryWindow::ALL, trade_date),
                        );
                    }
                }
            });
        }
    }
    let delisted: HashMap<String, bool> = sqlx::query_as::<_, (String, bool)>(
        "SELECT thscode, delisted_on IS NOT NULL FROM ref.security",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    Ok(Some(Shared {
        trade_date,
        next_date,
        market,
        sec,
        screening,
        names,
        definitions,
        limits,
        hits,
        delisted,
        history,
    }))
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// 一只股票在最后一根日线上的状态；价格换算成未复权价（最新一段的前复权价等于原价，这里按比例换算以防万一）
fn snapshot(
    bars: &[DailyBar],
    meta: &StockMeta,
    trend_ma: u32,
    authority: Option<(&LimitPool, &str)>,
) -> Option<Snapshot> {
    let last = bars.last()?;
    let d = diagnosis::diagnose(bars, meta, &[], authority)?;
    let f = match authority {
        Some((pool, code)) => Frame::with_authority(bars, Some(meta), pool, code),
        None => Frame::new(bars, Some(meta)),
    };
    let i = bars.len() - 1;
    let v = |id, n| finite(f.at(IndicatorRef::new(id, n), i));
    let to_raw = if last.q_close > 0.0 {
        last.close / last.q_close
    } else {
        1.0
    };
    let (hi, lo) = (
        v(IndicatorId::HighN, Some(60)),
        v(IndicatorId::LowN, Some(60)),
    );
    let range60_pos = match (hi, lo) {
        (Some(h), Some(l)) if h > l => Some((last.q_close - l) / (h - l)),
        _ => None,
    };
    let trend = &d.dimensions[0];
    let volume = &d.dimensions[2];
    Some(Snapshot {
        close: last.close,
        change_pct: v(IndicatorId::ChangePct, None),
        amount: last.amount,
        ma_trend: v(IndicatorId::Ma, Some(trend_ma)).map(|m| m * to_raw),
        rsi: v(IndicatorId::Rsi, Some(14)),
        momentum20: v(IndicatorId::Momentum, Some(20)),
        range60_pos,
        trend_text: trend.headline.clone(),
        trend_strong: trend.verdict == Verdict::Strong,
        trend_weak: trend.verdict == Verdict::Weak,
        volume_text: if volume.detail.is_empty() {
            volume.headline.clone()
        } else {
            format!("{}（{}）", volume.headline, volume.detail)
        },
    })
}

/// 之前的建议中连续给出、还没执行的卖出和减仓：key → 连续天数
async fn streaks(
    pool: &PgPool,
    account_id: i64,
    trade_date: NaiveDate,
) -> anyhow::Result<BTreeMap<String, u32>> {
    let prev: Option<(i64, serde_json::Value)> = sqlx::query_as(
        "SELECT advice_id, result FROM app.advice WHERE account_id = $1 AND trade_date < $2 AND superseded_by IS NULL ORDER BY trade_date DESC LIMIT 1",
    )
    .bind(account_id)
    .bind(trade_date)
    .fetch_optional(pool)
    .await?;
    let Some((advice_id, result)) = prev else {
        return Ok(BTreeMap::new());
    };
    let executed: Vec<String> = sqlx::query_scalar(
        "SELECT item_key FROM app.advice_item_state WHERE advice_id = $1 AND state = 'executed'",
    )
    .bind(advice_id)
    .fetch_all(pool)
    .await?;
    let advice: super::generate::Advice = serde_json::from_value(result)?;
    Ok(advice
        .items
        .iter()
        .filter(|i| i.streak > 0 && !executed.contains(&i.key))
        .map(|i| (i.key.clone(), i.streak))
        .collect())
}

pub async fn input_for(pool: &PgPool, sh: &Shared, acc: &Account) -> anyhow::Result<Input> {
    let rules = acc.rules.clone();
    let positions = account::positions(pool, acc.account_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut holdings = vec![];
    for p in &positions {
        let meta = sh.sec.meta.get(&p.thscode).cloned().unwrap_or(StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
        });
        let bars = prices::recent_bars_until(pool, &p.thscode, sh.trade_date, BARS).await?;
        let snap = snapshot(&bars, &meta, rules.trend_ma, Some((&sh.limits, &p.thscode)));
        let bars_held = match p.opened_on {
            Some(d) => Some(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mkt.stock_bar_1d WHERE thscode = $1 AND trade_date >= $2 AND trade_date <= $3")
                    .bind(&p.thscode)
                    .bind(d)
                    .bind(sh.trade_date)
                    .fetch_one(pool)
                    .await?,
            ),
            None => None,
        };
        let strategy = p.strategy_id.map(|id| StrategyRef {
            strategy_id: id,
            name: sh
                .names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("策略 {id}")),
        });
        let strategy_exit = match (p.strategy_id, bars.is_empty()) {
            (Some(id), false) => match screener::load_strategy(pool, id).await? {
                Some(def) => {
                    let f = Frame::with_authority(&bars, Some(&meta), &sh.limits, &p.thscode);
                    let i = bars.len() - 1;
                    if def.exit_signal(&f, i) {
                        def.exit_reason_text(&f, i)
                            .or_else(|| Some("卖出条件".into()))
                    } else {
                        None
                    }
                }
                None => None,
            },
            _ => None,
        };
        holdings.push(HoldingIn {
            thscode: p.thscode.clone(),
            name: sh
                .sec
                .names
                .get(&p.thscode)
                .cloned()
                .unwrap_or_else(|| p.thscode.clone()),
            board: meta.board.clone(),
            shares: p.shares,
            cost_price: p.cost_price,
            opened_on: p.opened_on,
            bars_held,
            strategy,
            strategy_exit,
            is_st: meta.is_st,
            delisted: sh.delisted.get(&p.thscode).copied().unwrap_or(false),
            last_date: bars.last().map(|b| b.date),
            snap,
        });
    }

    // 候选：参考的策略今天的买点，按股票合并；先按和生成时相同的优先级预选，只细看前面的
    let ids: Vec<i64> = sh
        .screening
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| rules.uses(*id))
        .collect();
    let mut by_code: HashMap<&str, (Vec<Pick>, f64)> = HashMap::new();
    for (id, code, reasons, amount) in &sh.hits {
        if ids.contains(id) {
            let e = by_code
                .entry(code.as_str())
                .or_insert_with(|| (vec![], *amount));
            e.0.push(Pick {
                strategy_id: *id,
                reasons: reasons.clone(),
                history: sh
                    .history
                    .get(&(*id, code.clone()))
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    }
    let mut pre: Vec<(&str, Vec<Pick>, f64)> = by_code
        .into_iter()
        .map(|(c, (picks, a))| (c, picks, a))
        .collect();
    let mut history_rejected = 0;
    pre.retain_mut(|(code, picks, _)| {
        if positions.iter().any(|p| p.thscode == *code) {
            return false;
        }
        let before = picks.len();
        picks.retain(|p| p.eligible(&rules, sh.trade_date));
        let enough = picks.len() >= rules.min_picks as usize;
        if !enough && before >= rules.min_picks as usize && picks.len() < before {
            history_rejected += 1;
        }
        enough
    });
    pre.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| b.2.total_cmp(&a.2))
            .then_with(|| a.0.cmp(b.0))
    });
    let mut candidates = vec![];
    for (code, picks, _) in pre
        .into_iter()
        .filter(|(c, _, _)| !positions.iter().any(|p| p.thscode == *c))
        .take(MAX_CANDIDATES)
    {
        let Some(meta) = sh.sec.meta.get(code) else {
            continue;
        };
        let bars = prices::recent_bars_until(pool, code, sh.trade_date, BARS).await?;
        let Some(snap) = snapshot(&bars, meta, rules.trend_ma, Some((&sh.limits, code))) else {
            continue;
        };
        candidates.push(CandidateIn {
            thscode: code.to_string(),
            name: sh.sec.names.get(code).cloned().unwrap_or_default(),
            board: meta.board.clone(),
            is_st: meta.is_st,
            picks,
            snap,
        });
    }

    let strategies = sh
        .screening
        .iter()
        .filter(|(id, _)| rules.uses(*id))
        .map(|(id, name)| StrategyIn {
            strategy_id: *id,
            name: name.clone(),
            definition_hash: sh
                .definitions
                .get(id)
                .map(definition_hash)
                .unwrap_or_default(),
        })
        .collect();
    Ok(Input {
        trade_date: sh.trade_date,
        next_date: sh.next_date,
        rules,
        fees: acc.fees,
        cash: acc.cash,
        market: sh.market,
        holdings,
        candidates,
        history_rejected,
        strategies,
        streaks: streaks(pool, acc.account_id, sh.trade_date).await?,
    })
}

fn definition_hash(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    let encoded: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{encoded}")
}
