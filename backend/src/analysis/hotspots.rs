//! 今日热点：权威涨停、跌停、炸板池和封板率统计，按连板高度分组。
//!
//! 数据集 ID 和「涨停池从 2020-07-22 起」的规则与市场概况共用 [`super::limit_pool`]；供应商 payload 的字段解析
//! 也只在这里。HTTP 参数和缓存装配在 `api::market`，首页预热在 `crate::precompute`。口径见 docs/domains/hotspots.md。

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use utoipa::ToSchema;

use super::limit_pool::{BREAK, DOWN, STATS, UP};
use super::market;

/// 每个列表最多列多少只股票；全部家数在 total 字段里另给
const STOCK_LIMIT: usize = 50;

#[derive(Deserialize, Serialize, ToSchema)]
pub struct HotMarket {
    /// 数据对应的交易日；本地还没有任何行情数据时为空
    pub trade_date: Option<NaiveDate>,
    /// 这一天的权威涨跌停数据（涨停、跌停、炸板池）是否已经同步
    pub available: bool,
    /// 没有数据时的一句白话说明，例如「今天的涨停数据还没同步，稍后再看」
    pub note: Option<String>,
    /// 涨停家数（不只是下面列出的那些）
    pub limit_up_total: i64,
    /// 按连板高度分组，从高到低；1 表示首板
    pub limit_up_groups: Vec<HotLimitUpGroup>,
    /// 最高连板数，没有涨停时为 0
    pub max_board: i32,
    /// 跌停家数
    pub limit_down_total: i64,
    /// 跌停股，最多列 50 只
    pub limit_down_stocks: Vec<HotStock>,
    /// 炸板家数（盘中涨停、收盘没封住）
    pub broken_total: i64,
    /// 炸板股，最多列 50 只
    pub broken_stocks: Vec<HotStock>,
    /// 当天的封板率等统计，没同步时为空
    pub stats: Option<HotStats>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct HotLimitUpGroup {
    /// 连板高度，1 表示首板
    pub board_num: i32,
    /// 「首板」「2 板」「3 板」……
    pub label: String,
    /// 这一档一共多少只（不只是下面列出的那些）
    pub count: i64,
    pub stocks: Vec<HotLimitUpStock>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct HotLimitUpStock {
    pub thscode: String,
    pub name: String,
    /// 供应商的连板说明，例如「首板」「2 连板」
    pub continue_day_text: Option<String>,
    /// 涨停原因，例如「低空经济+无人机物流+海上油气」
    pub limit_up_reason: Option<String>,
    /// 最新价（字符串，保留供应商给的精度）
    pub last_price: Option<String>,
    /// 收盘时的封单额（元，字符串）
    pub seal_money: Option<String>,
    /// 首次涨停时间，例如「10:55」
    pub limit_up_time: Option<String>,
    /// 是否 ST 股
    pub is_st: bool,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct HotStock {
    pub thscode: String,
    pub name: String,
    /// 最新价（字符串，保留供应商给的精度）
    pub last_price: Option<String>,
    /// 当天涨跌幅（%）
    pub price_change_pct: Option<f64>,
    /// 成交额（元，字符串）
    pub turnover: Option<String>,
    /// 换手率（%）
    pub turnover_ratio_pct: Option<f64>,
    /// 炸板次数，只有炸板股有
    pub open_times: Option<i64>,
    /// 首次跌停时间，只有跌停股有
    pub first_limit_time: Option<String>,
    /// 最后一次跌停时间，只有跌停股有
    pub last_limit_time: Option<String>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct HotStats {
    /// 封板率：收盘还封在涨停的占当天触及涨停的比例（0.55 表示 55%）
    pub seal_rate: Option<f64>,
    /// 收盘涨停家数
    pub limit_up_count: Option<i64>,
    /// 盘中触及过涨停的家数
    pub touched_limit_up_count: Option<i64>,
    /// 盘中涨停后打开、收盘没封住的家数
    pub open_count: Option<i64>,
}

// @project-doc docs/domains/hotspots.md#rules
/// 热点对应的交易日：和 `/market/overview` 一样取本地日线（股票和 ETF）里不晚于请求日期的最近一天；
/// 日线还没同步时退回请求的日期，或涨停池自己的最新日期，先把已经同步的涨停池显示出来
pub async fn trade_date(
    pool: &PgPool,
    requested: Option<NaiveDate>,
) -> sqlx::Result<Option<NaiveDate>> {
    Ok(
        match (market::resolve_date(pool, requested).await?, requested) {
            (Some(d), _) => Some(d),
            (None, Some(d)) => Some(d),
            (None, None) => {
                sqlx::query_scalar("SELECT max(obs_date) FROM raw.record WHERE dataset_id = $1")
                    .bind(UP)
                    .fetch_one(pool)
                    .await?
            }
        },
    )
}

/// 本地还没有任何行情数据时的结果
pub fn no_market_data() -> HotMarket {
    empty(
        None,
        "本地还没有行情数据，等第一次同步完成后再看".to_string(),
    )
}

/// 一个交易日的热点
pub async fn compute(pool: &PgPool, trade_date: NaiveDate) -> sqlx::Result<HotMarket> {
    let ids: Vec<String> = [UP, DOWN, BREAK, STATS]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    // 只取这 4 个数据集当天的记录；供应商偶尔有一行的 thscode 为空，用 ticker 到证券表里补
    let rows: Vec<(String, Value, Option<String>)> = sqlx::query_as(
        "SELECT r.dataset_id, r.payload,
                coalesce(nullif(r.payload->>'thscode', ''),
                         (SELECT sec.thscode FROM ref.security sec WHERE sec.ticker = r.payload->>'ticker' LIMIT 1))
         FROM raw.record r
         WHERE r.dataset_id = ANY($1) AND r.obs_date = $2",
    )
    .bind(&ids)
    .bind(trade_date)
    .fetch_all(pool)
    .await?;

    let (mut up_rows, mut down_rows, mut break_rows, mut stats) =
        (Vec::new(), Vec::new(), Vec::new(), None);
    for (dataset, payload, thscode) in rows {
        match dataset.as_str() {
            UP => up_rows.push((payload, thscode)),
            DOWN => down_rows.push((payload, thscode)),
            BREAK => break_rows.push((payload, thscode)),
            STATS => stats = Some(hot_stats(&payload)),
            _ => {}
        }
    }
    let available =
        !up_rows.is_empty() || !down_rows.is_empty() || !break_rows.is_empty() || stats.is_some();
    if !available {
        // 涨停池从 2020-07-22 才有；比这更早的日期不是没同步，而是那时还没数据
        let earliest = NaiveDate::from_ymd_opt(2020, 7, 22).expect("固定日期");
        let note = if trade_date < earliest {
            format!(
                "{} 太早了：涨停数据从 2020 年 7 月 22 日才开始有",
                trade_date.format("%Y-%m-%d")
            )
        } else if trade_date == crate::time::today_cn() {
            "今天的涨停数据还没同步，稍后再看".to_string()
        } else {
            format!(
                "{} 的涨停、跌停、炸板数据还没同步，稍后再看",
                trade_date.format("%Y-%m-%d")
            )
        };
        return Ok(empty(Some(trade_date), note));
    }

    // 涨停按连板高度分组，组内封单额大的排在前面（封单越大，说明想买的人越多）
    let mut by_board: BTreeMap<i32, Vec<(f64, HotLimitUpStock)>> = BTreeMap::new();
    for (payload, thscode) in up_rows {
        let seal = json_num(&payload, "seal_money").unwrap_or(0.0);
        by_board
            .entry(board_num_of(&payload))
            .or_default()
            .push((seal, limit_up_stock(&payload, thscode)));
    }
    let limit_up_total = by_board.values().map(|v| v.len()).sum::<usize>() as i64;
    let max_board = by_board.keys().next_back().copied().unwrap_or(0);
    let limit_up_groups = by_board
        .into_iter()
        .rev()
        .map(|(board_num, mut stocks)| {
            stocks.sort_by(|a, b| {
                b.0.total_cmp(&a.0)
                    .then_with(|| a.1.thscode.cmp(&b.1.thscode))
            });
            let count = stocks.len() as i64;
            let stocks = stocks
                .into_iter()
                .take(STOCK_LIMIT)
                .map(|(_, s)| s)
                .collect();
            HotLimitUpGroup {
                board_num,
                label: board_label(board_num),
                count,
                stocks,
            }
        })
        .collect();

    // 跌停按跌幅从大到小；炸板按炸板次数从多到少。都只列前 50 只，总数另给
    let limit_down_total = down_rows.len() as i64;
    let mut limit_down_stocks: Vec<HotStock> = down_rows
        .into_iter()
        .map(|(payload, thscode)| hot_stock(&payload, thscode))
        .collect();
    limit_down_stocks.sort_by(|a, b| {
        a.price_change_pct
            .unwrap_or(0.0)
            .total_cmp(&b.price_change_pct.unwrap_or(0.0))
            .then_with(|| a.thscode.cmp(&b.thscode))
    });
    limit_down_stocks.truncate(STOCK_LIMIT);

    let broken_total = break_rows.len() as i64;
    let mut broken_stocks: Vec<HotStock> = break_rows
        .into_iter()
        .map(|(payload, thscode)| hot_stock(&payload, thscode))
        .collect();
    broken_stocks.sort_by(|a, b| {
        b.open_times
            .unwrap_or(0)
            .cmp(&a.open_times.unwrap_or(0))
            .then_with(|| a.thscode.cmp(&b.thscode))
    });
    broken_stocks.truncate(STOCK_LIMIT);

    Ok(HotMarket {
        trade_date: Some(trade_date),
        available: true,
        note: None,
        limit_up_total,
        limit_up_groups,
        max_board,
        limit_down_total,
        limit_down_stocks,
        broken_total,
        broken_stocks,
        stats,
    })
}

// @project-doc docs/domains/hotspots.md#empty
fn empty(trade_date: Option<NaiveDate>, note: String) -> HotMarket {
    HotMarket {
        trade_date,
        available: false,
        note: Some(note),
        limit_up_total: 0,
        limit_up_groups: Vec::new(),
        max_board: 0,
        limit_down_total: 0,
        limit_down_stocks: Vec::new(),
        broken_total: 0,
        broken_stocks: Vec::new(),
        stats: None,
    }
}

/// 连板高度：`continue_day_cnt` 为 1 表示首板；缺字段时按首板算
fn board_num_of(payload: &Value) -> i32 {
    payload
        .get("continue_day_cnt")
        .and_then(Value::as_i64)
        .or_else(|| payload.get("board_num").and_then(Value::as_i64))
        .map(|n| n.clamp(1, 100) as i32)
        .unwrap_or(1)
}

fn board_label(board_num: i32) -> String {
    if board_num <= 1 {
        "首板".to_string()
    } else {
        format!("{board_num} 板")
    }
}

fn limit_up_stock(payload: &Value, thscode: Option<String>) -> HotLimitUpStock {
    HotLimitUpStock {
        thscode: stock_code(payload, thscode),
        name: json_str(payload, "name").unwrap_or_default(),
        continue_day_text: json_str(payload, "continue_day_text"),
        limit_up_reason: json_str(payload, "limit_up_reason"),
        last_price: json_str(payload, "last_price"),
        seal_money: json_str(payload, "seal_money"),
        limit_up_time: json_str(payload, "limit_up_time"),
        is_st: payload
            .get("is_st")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn hot_stock(payload: &Value, thscode: Option<String>) -> HotStock {
    HotStock {
        thscode: stock_code(payload, thscode),
        name: json_str(payload, "name").unwrap_or_default(),
        last_price: json_str(payload, "last_price"),
        price_change_pct: json_num(payload, "price_change_ratio_pct"),
        turnover: json_str(payload, "turnover"),
        turnover_ratio_pct: json_num(payload, "turnover_ratio_pct"),
        open_times: payload.get("open_times").and_then(Value::as_i64),
        first_limit_time: json_str(payload, "first_limit_time"),
        last_limit_time: json_str(payload, "last_limit_time"),
    }
}

fn hot_stats(payload: &Value) -> HotStats {
    HotStats {
        seal_rate: json_num(payload, "seal_rate"),
        limit_up_count: payload.get("limit_up_stock_count").and_then(Value::as_i64),
        touched_limit_up_count: payload
            .get("touched_limit_up_stock_count")
            .and_then(Value::as_i64),
        open_count: payload.get("open_stock_count").and_then(Value::as_i64),
    }
}

/// 记录的证券代码：优先用 payload 的 thscode（数据库里已经用 ticker 在证券表里补过一次），
/// 都没有时按 6 位代码段补交易所后缀，保证页面上的链接可用
fn stock_code(payload: &Value, resolved: Option<String>) -> String {
    resolved
        .or_else(|| {
            payload
                .get("thscode")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .get("ticker")
                .and_then(Value::as_str)
                .and_then(ticker_with_exchange)
        })
        .unwrap_or_default()
}

fn ticker_with_exchange(ticker: &str) -> Option<String> {
    let exchange = match ticker.chars().next()? {
        '6' => "SH",
        '0' | '3' => "SZ",
        '4' | '8' | '9' => "BJ",
        _ => return None,
    };
    Some(format!("{ticker}.{exchange}"))
}

fn json_num(payload: &Value, key: &str) -> Option<f64> {
    payload.get(key).and_then(Value::as_f64)
}

/// 取字符串字段；供应商的数值字段（价格、金额）也转成十进制字符串返回
fn json_str(payload: &Value, key: &str) -> Option<String> {
    match payload.get(key)? {
        Value::String(s) => (!s.is_empty()).then(|| s.clone()),
        Value::Number(n) => Some(decimal_text(n)),
        _ => None,
    }
}

/// JSON 数字转文本：整数不带小数点，小数保留原样（例如 172300270 → "172300270"、61.44 → "61.44"）
fn decimal_text(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(v) if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 => format!("{}", v as i64),
        _ => n.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_labels_and_numbers() {
        assert_eq!(ticker_with_exchange("600519").as_deref(), Some("600519.SH"));
        assert_eq!(ticker_with_exchange("920022").as_deref(), Some("920022.BJ"));
        assert_eq!(board_label(1), "首板");
        assert_eq!(board_label(3), "3 板");
        assert_eq!(board_num_of(&serde_json::json!({"continue_day_cnt": 0})), 1);
        assert_eq!(
            json_str(&serde_json::json!({"x": 172300270}), "x").as_deref(),
            Some("172300270")
        );
        assert_eq!(
            json_str(&serde_json::json!({"x": 61.44}), "x").as_deref(),
            Some("61.44")
        );
    }
}
