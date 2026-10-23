//! 每日净值：按交易日记录账户的总资产（现金 + 持仓按当天收盘价）。
//!
//! 口径：
//!
//! - 每个交易日的股数 = 这一天（含）之前全部有效流水的股数变化之和，撤销的流水不算；
//! - 现金 = 这一天（含）之前全部有效流水的现金变化之和（转入转出、买卖、分红、校准、配股都在内）；
//! - 持仓市值 = 股数 × 当天收盘价；当天没有日线（停牌、退市后）时用之前最近的收盘价，不引入实时行情；
//! - 当日盈亏 = 今天总资产 − 昨天总资产 − 今天转入转出的净额（转入转出不是赚亏）；
//! - 区间收益率 =（期末利润 − 期初利润）÷ 期初总资产，利润 = 总资产 − 累计净转入。这是把转入转出
//!   当外部现金流剔除的简单口径，没有考虑资金在区间中间进出的时点，只用来大致看账户做得好不好。
//!
//! 快照保存在 `app.account_equity`：每天生成建议前（`corp::apply`）写一次当天快照，打开曲线时再从建账户
//! （或第一笔流水）开始重算整段并 upsert，所以补记、撤销流水或校准之后，历史曲线也会跟着修正。
//! 规则见 docs/domains/accounts.md#equity。

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::lots::round2;
use super::{AccountError, Result};

/// 曲线上的一个交易日
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct AccountEquityPoint {
    pub trade_date: NaiveDate,
    /// 现金（元）
    pub cash: f64,
    /// 持仓市值（元）
    pub market_value: f64,
    /// 总资产（元）
    pub total_assets: f64,
    /// 当日盈亏（元），已经剔除当天的转入转出
    pub day_pnl: f64,
}

/// 一段时间的净值曲线
#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct EquityCurve {
    /// 按交易日从早到晚；最多请求的 `days` 个交易日
    pub points: Vec<AccountEquityPoint>,
    /// 区间收益率（%）：（期末利润 − 期初利润）÷ 期初总资产；期初总资产为 0 时为空
    pub return_pct: Option<f64>,
    /// 区间内的转入减转出（元）
    pub net_transfer: f64,
    /// 从哪一天开始有数据（建账户或第一笔流水）
    pub since: Option<NaiveDate>,
}

/// 有效流水里和净值有关的部分：(成交日, 股票, 股数变化, 现金变化, 类型)
type TradeRow = (NaiveDate, Option<String>, Option<i64>, f64, String);

// @project-doc docs/domains/accounts.md#equity
/// 重算 `account_id` 从建账户以来的每日净值并 upsert 快照，返回最近 `window` 个交易日的曲线。
/// `through` 有值时只算到这一天（每日维护传当天的交易日），为空时算到本地最新的收盘日。
pub async fn refresh(
    pool: &PgPool,
    account_id: i64,
    window: i64,
    through: Option<NaiveDate>,
) -> Result<EquityCurve> {
    let created: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT created_at FROM app.account WHERE account_id = $1")
            .bind(account_id)
            .fetch_optional(pool)
            .await?;
    let Some(created) = created else {
        return Err(AccountError::NotFound);
    };
    let created = (created + Duration::hours(8)).date_naive();
    let end = crate::analysis::market::resolve_date(pool, through).await?;
    let Some(end) = end else {
        return Ok(empty_curve(Some(created)));
    };
    let first_trade: Option<NaiveDate> =
        sqlx::query_scalar("SELECT min(trade_date) FROM app.account_trade WHERE account_id = $1 AND reversed_at IS NULL AND trade_date <= $2")
            .bind(account_id)
            .bind(end)
            .fetch_one(pool)
            .await?;
    // 补记的流水可能早于建账户那天，从更早的那天开始算
    let start = first_trade.map_or(created, |t| created.min(t));
    if start > end {
        return Ok(empty_curve(Some(start)));
    }
    let rows: Vec<TradeRow> = sqlx::query_as(
        "SELECT trade_date, thscode, shares, amount::float8, kind FROM app.account_trade
         WHERE account_id = $1 AND reversed_at IS NULL AND trade_date <= $2
         ORDER BY trade_date, trade_id",
    )
    .bind(account_id)
    .bind(end)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(empty_curve(Some(start)));
    }

    // 按成交日归并：现金变化、转入转出、股数变化
    let mut cash_events: BTreeMap<NaiveDate, (f64, f64)> = BTreeMap::new();
    let mut share_events: BTreeMap<NaiveDate, Vec<(String, i64)>> = BTreeMap::new();
    let mut codes: Vec<String> = Vec::new();
    for (date, code, shares, amount, kind) in rows {
        let e = cash_events.entry(date).or_insert((0.0, 0.0));
        e.0 += amount;
        if kind == "cash_in" || kind == "cash_out" {
            e.1 += amount;
        }
        if let (Some(code), Some(n)) = (code, shares)
            && n != 0
        {
            codes.push(code.clone());
            share_events.entry(date).or_default().push((code, n));
        }
    }
    let held: Vec<String> =
        sqlx::query_scalar("SELECT thscode FROM app.position WHERE account_id = $1")
            .bind(account_id)
            .fetch_all(pool)
            .await?;
    codes.extend(held);
    codes.sort();
    codes.dedup();

    // 每只股票的收盘价（到 end 为止），停牌时沿用最近一根
    let bars: Vec<(String, NaiveDate, f64)> = if codes.is_empty() {
        vec![]
    } else {
        sqlx::query_as(
            "SELECT thscode, trade_date, close FROM mkt.v_analysis_bar
             WHERE thscode = ANY($1) AND trade_date <= $2
             ORDER BY thscode, trade_date",
        )
        .bind(&codes)
        .bind(end)
        .fetch_all(pool)
        .await?
    };
    let mut series: HashMap<String, Vec<(NaiveDate, f64)>> = HashMap::new();
    for (code, date, close) in bars {
        series.entry(code).or_default().push((date, close));
    }

    // 交易日：优先用日历（ref.trading_day）；测试库或日历还没同步时用日线里出现过的日期
    let mut days: Vec<NaiveDate> = sqlx::query_scalar(
        "SELECT cal_date FROM ref.trading_day WHERE cal_date BETWEEN $1 AND $2 ORDER BY cal_date",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;
    if days.is_empty() {
        days = sqlx::query_scalar(
            "SELECT DISTINCT trade_date FROM mkt.v_bar_date WHERE trade_date BETWEEN $1 AND $2 ORDER BY trade_date",
        )
        .bind(start)
        .bind(end)
        .fetch_all(pool)
        .await?;
    }
    if days.is_empty() || days.last() != Some(&end) {
        days.push(end);
    }

    let code_pos: HashMap<&str, usize> = codes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.as_str(), i))
        .collect();
    let mut share_lists: Vec<Vec<(NaiveDate, i64)>> = vec![vec![]; codes.len()];
    for (date, list) in &share_events {
        for (code, n) in list {
            if let Some(&i) = code_pos.get(code.as_str()) {
                share_lists[i].push((*date, *n));
            }
        }
    }
    let mut share_iters: Vec<std::iter::Peekable<std::vec::IntoIter<(NaiveDate, i64)>>> =
        share_lists
            .into_iter()
            .map(|v| v.into_iter().peekable())
            .collect();
    let mut shares = vec![0_i64; codes.len()];
    let mut bar_next = vec![0_usize; codes.len()];

    let (mut cash, mut transfer, mut pending_transfer) = (0.0, 0.0, 0.0);
    let mut cash_iter = cash_events.into_iter().peekable();
    let mut prev_total: Option<f64> = None;
    let mut points: Vec<AccountEquityPoint> = Vec::with_capacity(days.len());
    let mut transfers: Vec<f64> = Vec::with_capacity(days.len());
    for day in &days {
        while let Some((date, (delta_cash, delta_transfer))) = cash_iter.peek() {
            if *date > *day {
                break;
            }
            cash += delta_cash;
            transfer += delta_transfer;
            pending_transfer += delta_transfer;
            cash_iter.next();
        }
        for i in 0..codes.len() {
            while let Some((date, delta)) = share_iters[i].peek() {
                if *date > *day {
                    break;
                }
                shares[i] += delta;
                share_iters[i].next();
            }
        }
        let mut market_value = 0.0;
        for i in 0..codes.len() {
            if shares[i] == 0 {
                continue;
            }
            let Some(bars) = series.get(codes[i].as_str()) else {
                continue;
            };
            while bar_next[i] < bars.len() && bars[bar_next[i]].0 <= *day {
                bar_next[i] += 1;
            }
            if bar_next[i] == 0 {
                continue;
            }
            market_value += shares[i] as f64 * bars[bar_next[i] - 1].1;
        }
        let total_assets = round2(cash + market_value);
        let day_pnl = match prev_total {
            Some(prev) => round2(total_assets - prev - pending_transfer),
            // 第一个点没有前一天：拿它和累计净转入比，通常正好是 0（刚建账户）
            None => round2(total_assets - transfer),
        };
        points.push(AccountEquityPoint {
            trade_date: *day,
            cash: round2(cash),
            market_value: round2(market_value),
            total_assets,
            day_pnl,
        });
        transfers.push(round2(transfer));
        prev_total = Some(total_assets);
        pending_transfer = 0.0;
    }

    // 快照：整段重算后 upsert，补记或撤销流水后历史也跟着修正
    let dates: Vec<NaiveDate> = points.iter().map(|p| p.trade_date).collect();
    let cash_v: Vec<f64> = points.iter().map(|p| p.cash).collect();
    let mv_v: Vec<f64> = points.iter().map(|p| p.market_value).collect();
    let total_v: Vec<f64> = points.iter().map(|p| p.total_assets).collect();
    let pnl_v: Vec<f64> = points.iter().map(|p| p.day_pnl).collect();
    sqlx::query(
        "INSERT INTO app.account_equity (account_id, trade_date, cash, market_value, total_assets, day_pnl)
         SELECT $1, d, round(c::numeric, 2), round(m::numeric, 2), round(t::numeric, 2), round(p::numeric, 2)
         FROM unnest($2::date[], $3::float8[], $4::float8[], $5::float8[], $6::float8[]) AS x(d, c, m, t, p)
         ON CONFLICT (account_id, trade_date) DO UPDATE SET cash = EXCLUDED.cash, market_value = EXCLUDED.market_value,
           total_assets = EXCLUDED.total_assets, day_pnl = EXCLUDED.day_pnl, updated_at = now()",
    )
    .bind(account_id)
    .bind(&dates)
    .bind(&cash_v)
    .bind(&mv_v)
    .bind(&total_v)
    .bind(&pnl_v)
    .execute(pool)
    .await?;

    let keep = window.clamp(1, 5_000) as usize;
    let start_ix = points.len().saturating_sub(keep);
    let shown = points.split_off(start_ix);
    let since = shown.first().map(|p| p.trade_date);
    let (first, last) = (shown[0].total_assets, shown[shown.len() - 1].total_assets);
    let profit_first = first - transfers[start_ix];
    let profit_last = last - transfers[transfers.len() - 1];
    let return_pct = (first >= 0.01).then(|| round2((profit_last - profit_first) / first * 100.0));
    let net_transfer = round2(transfers[transfers.len() - 1] - transfers[start_ix]);
    Ok(EquityCurve {
        points: shown,
        return_pct,
        net_transfer,
        since,
    })
}

/// 每天生成建议前给所有账户写一次截至 `through` 的净值快照。打开曲线时会整段重算，这里只负责让快照不断天。
pub async fn refresh_all(pool: &PgPool, through: NaiveDate) -> anyhow::Result<usize> {
    let mut n = 0;
    for acc in super::all(pool).await? {
        match refresh(pool, acc.account_id, 1, Some(through)).await {
            Ok(_) => n += 1,
            Err(e) => tracing::warn!(account_id = acc.account_id, error = %e, "写每日净值快照失败"),
        }
    }
    Ok(n)
}

fn empty_curve(since: Option<NaiveDate>) -> EquityCurve {
    EquityCurve {
        points: vec![],
        return_pct: None,
        net_transfer: 0.0,
        since,
    }
}
