//! 自选股：当前用户收藏的股票（每个用户各自一份），以及对它们的快速诊断（最新价、涨跌、均线状态、最近一次交叉）。
//!
//! 均线是等比前复权收盘价的简单移动平均，交叉规则与 `analysis::ma` 相同。

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::auth::CurrentUser;
use super::market::windows;
use crate::analysis::ma::{CrossSignal, ma_cross, state_of};
use crate::analysis::diagnosis::{SignalEvent, recent_signals};
use crate::analysis::indicators::Frame;
use crate::analysis::{market, prices};
use crate::error::{ApiError, ApiResult};

/// 每只股票取最近多少根前复权日线做诊断：约两年，足够算出 250 日均线并找到最近一次交叉
const LOOKBACK: i64 = 500;
/// 计算最近 5 天信号需要的日线根数：覆盖每个信号的预热（最多的是 MACD 背离，约 190 根）
const RECENT_BARS: i64 = 200;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_watchlist)).routes(routes!(add_watch, remove_watch))
}

#[derive(Deserialize, IntoParams)]
pub struct WatchQuery {
    /// 快线周期，默认 15
    pub fast: Option<i32>,
    /// 慢线周期，默认 30
    pub slow: Option<i32>,
}

#[derive(Serialize, ToSchema)]
pub struct Watchlist {
    pub fast: i32,
    pub slow: i32,
    /// 按收藏时间倒序
    pub items: Vec<WatchItem>,
}

#[derive(Serialize, ToSchema)]
pub struct WatchItem {
    pub thscode: String,
    pub name: String,
    /// main / chinext / star / bse
    pub board: String,
    pub delisted_on: Option<NaiveDate>,
    pub added_at: DateTime<Utc>,
    /// 本地最新一根日线（未复权）的日期和收盘价；还没有日线时为空
    pub last_date: Option<NaiveDate>,
    pub close: Option<String>,
    /// 按前复权价计算的诊断；还没有日线时为空
    pub diagnosis: Option<WatchDiagnosis>,
    /// 最近 5 个交易日出现过的信号，新的在前
    pub recent_signals: Vec<SignalEvent>,
}

#[derive(Serialize, ToSchema)]
pub struct WatchDiagnosis {
    /// 最近 1 个交易日的涨跌幅（0.01 表示 1%）
    pub change_1d: Option<f64>,
    /// 最近 20 个交易日的涨跌幅
    pub change_20d: Option<f64>,
    /// 日线不足 slow 根时为空
    pub ma: Option<MaState>,
}

#[derive(Serialize, ToSchema)]
pub struct MaState {
    pub ma_fast: f64,
    pub ma_slow: f64,
    pub trend: MaTrend,
    /// 最近 LOOKBACK 根日线内的最后一次交叉
    pub last_cross: Option<LastCross>,
}

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MaTrend {
    /// 快线在慢线上方
    Above,
    /// 快线在慢线下方
    Below,
    /// 两线相等（在容差内）
    Equal,
}

#[derive(Serialize, ToSchema)]
pub struct LastCross {
    pub signal: CrossSignal,
    pub trade_date: NaiveDate,
    /// 距今几个交易日，0 表示就在诊断日当天
    pub bars_ago: i64,
}

/// 自选股列表及诊断
#[utoipa::path(get, path = "/watchlist", tag = "watchlist", params(WatchQuery),
    responses((status = 200, description = "成功", body = Watchlist), (status = 400, description = "参数不合法", body = crate::error::ErrorBody)))]
async fn list_watchlist(State(s): State<AppState>, me: CurrentUser, Query(q): Query<WatchQuery>) -> ApiResult<Json<Watchlist>> {
    let (fast, slow) = windows(q.fast, q.slow)?;
    /// (代码, 名称, 板块, 退市日, 收藏时间, 最新日期, 最新收盘)
    type BaseRow = (String, String, String, Option<NaiveDate>, DateTime<Utc>, Option<NaiveDate>, Option<String>);
    let base: Vec<BaseRow> = sqlx::query_as(
        "SELECT w.thscode, s.name, s.board, s.delisted_on, w.added_at, l.trade_date, l.close::text
         FROM app.watchlist w JOIN ref.security s USING (thscode)
         LEFT JOIN LATERAL (SELECT trade_date, close FROM mkt.stock_bar_1d b WHERE b.thscode = w.thscode
                            ORDER BY trade_date DESC LIMIT 1) l ON true
         WHERE w.user_id = $1
         ORDER BY w.added_at DESC, w.thscode",
    )
    .bind(me.user_id)
    .fetch_all(&s.pool)
    .await?;

    let codes: Vec<String> = base.iter().map(|r| r.0.clone()).collect();
    let series = prices::recent_closes(&s.pool, &codes, LOOKBACK).await?;

    let sec = market::securities(&s.pool).await?;
    let mut items = vec![];
    for (thscode, name, board, delisted_on, added_at, last_date, close) in base {
        let diagnosis = series.get(&thscode).and_then(|pts| diagnose(pts, fast, slow));
        let bars = prices::recent_bars(&s.pool, &thscode, RECENT_BARS).await?;
        let recent_signals = {
            let f = Frame::new(&bars, sec.meta.get(&thscode));
            recent_signals(&f, 5)
        };
        items.push(WatchItem { thscode, name, board, delisted_on, added_at, last_date, close, diagnosis, recent_signals });
    }
    Ok(Json(Watchlist { fast: fast as i32, slow: slow as i32, items }))
}

// @project-doc docs/domains/ma_signals.md#watchlist
/// pts 是按日期升序的前复权收盘价
fn diagnose(pts: &[(NaiveDate, f64)], fast: usize, slow: usize) -> Option<WatchDiagnosis> {
    let &(_, close) = pts.last()?;
    let change = |k: usize| pts.len().checked_sub(k + 1).map(|i| close / pts[i].1 - 1.0);
    let closes: Vec<f64> = pts.iter().map(|p| p.1).collect();
    let rows = ma_cross(&closes, fast, slow);
    let ma = rows.last().copied().flatten().map(|last| {
        let last_cross = rows.iter().enumerate().rev().find_map(|(i, r)| {
            r.and_then(|r| r.signal).map(|signal| LastCross { signal, trade_date: pts[i].0, bars_ago: (pts.len() - 1 - i) as i64 })
        });
        let trend = match state_of(last.ma_fast, last.ma_slow) {
            1 => MaTrend::Above,
            -1 => MaTrend::Below,
            _ => MaTrend::Equal,
        };
        MaState { ma_fast: last.ma_fast, ma_slow: last.ma_slow, trend, last_cross }
    });
    Some(WatchDiagnosis { change_1d: change(1), change_20d: change(20), ma })
}

/// 收藏一只股票（重复收藏不报错）
#[utoipa::path(put, path = "/watchlist/{thscode}", tag = "watchlist", params(("thscode" = String, Path, description = "例：600519.SH")),
    responses((status = 204, description = "已收藏"), (status = 404, description = "证券不存在", body = crate::error::ErrorBody)))]
async fn add_watch(State(s): State<AppState>, me: CurrentUser, Path(code): Path<String>) -> ApiResult<StatusCode> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ref.security WHERE thscode = $1)").bind(&code).fetch_one(&s.pool).await?;
    if !exists {
        return Err(ApiError::NotFound(format!("证券不存在：{code}")));
    }
    sqlx::query("INSERT INTO app.watchlist (user_id, thscode) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(me.user_id).bind(&code).execute(&s.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 取消收藏（本来没有收藏也不报错）
#[utoipa::path(delete, path = "/watchlist/{thscode}", tag = "watchlist", params(("thscode" = String, Path, description = "例：600519.SH")),
    responses((status = 204, description = "已取消收藏")))]
async fn remove_watch(State(s): State<AppState>, me: CurrentUser, Path(code): Path<String>) -> ApiResult<StatusCode> {
    sqlx::query("DELETE FROM app.watchlist WHERE user_id = $1 AND thscode = $2").bind(me.user_id).bind(&code).execute(&s.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enough_history_for_recent_signals() {
        use crate::analysis::signals;
        let most = signals::CATALOG.iter().map(|s| signals::warmup(s.id, &signals::Params::new())).max().unwrap();
        assert!(most as i64 + 5 <= RECENT_BARS);
    }

    fn pts(closes: &[f64]) -> Vec<(NaiveDate, f64)> {
        let d0 = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        closes.iter().enumerate().map(|(i, &c)| (d0 + chrono::Duration::days(i as i64), c)).collect()
    }

    #[test]
    fn diagnosis_finds_last_cross_and_changes() {
        // 快线 3.5 2.5 2.5 4 6.5，慢线 4 3 2.67 3.33 5.33：下标 5 金叉
        let p = pts(&[5.0, 4.0, 3.0, 2.0, 3.0, 5.0, 8.0]);
        let d = diagnose(&p, 2, 3).unwrap();
        assert!((d.change_1d.unwrap() - 0.6).abs() < 1e-12);
        assert!(d.change_20d.is_none(), "不足 21 根时不算 20 日涨跌");
        let ma = d.ma.unwrap();
        assert_eq!(ma.trend, MaTrend::Above);
        let c = ma.last_cross.unwrap();
        assert_eq!((c.signal, c.trade_date, c.bars_ago), (CrossSignal::Golden, p[5].0, 1));
    }

    #[test]
    fn diagnosis_without_full_window_has_no_ma() {
        let d = diagnose(&pts(&[10.0]), 15, 30).unwrap();
        assert!(d.change_1d.is_none() && d.ma.is_none());
    }
}
