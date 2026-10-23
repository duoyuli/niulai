//! 自选股：当前用户收藏的股票（每个用户各自一份），以及最新价和最近 1 日、20 日的涨跌。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use super::auth::CurrentUser;
use crate::analysis::prices;
use crate::error::{ApiError, ApiResult};
use crate::instrument::AssetType;

/// 算 20 日涨跌需要的日线根数
const LOOKBACK: i64 = 21;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_watchlist))
        .routes(routes!(add_watch, remove_watch))
}

#[derive(Serialize, ToSchema)]
pub struct WatchItem {
    pub thscode: String,
    pub name: String,
    /// main / chinext / star / bse
    pub board: String,
    pub asset_type: AssetType,
    pub delisted_on: Option<NaiveDate>,
    pub added_at: DateTime<Utc>,
    /// 本地最新一根日线（未复权）的日期和收盘价；还没有日线时为空
    pub last_date: Option<NaiveDate>,
    pub close: Option<String>,
    /// 最近 1 个交易日的涨跌幅（按前复权价，0.01 表示 1%）
    pub change_1d: Option<f64>,
    /// 最近 20 个交易日的涨跌幅
    pub change_20d: Option<f64>,
}

/// 自选股列表，按收藏时间倒序
#[utoipa::path(get, path = "/watchlist", tag = "watchlist",
    responses((status = 200, description = "成功", body = [WatchItem])))]
async fn list_watchlist(
    State(s): State<AppState>,
    me: CurrentUser,
) -> ApiResult<Json<Vec<WatchItem>>> {
    /// (代码, 名称, 板块, 资产类型, 退市日, 收藏时间)
    type BaseRow = (
        String,
        String,
        String,
        AssetType,
        Option<NaiveDate>,
        DateTime<Utc>,
    );
    let base: Vec<BaseRow> = sqlx::query_as(
        "SELECT w.thscode, s.name, s.board, s.asset_type, s.delisted_on, w.added_at
         FROM app.watchlist w JOIN ref.instrument s USING (thscode)
         WHERE w.user_id = $1
         ORDER BY w.added_at DESC, w.thscode",
    )
    .bind(me.user_id)
    .fetch_all(&s.pool)
    .await?;
    let codes: Vec<String> = base.iter().map(|r| r.0.clone()).collect();
    let mut bars = prices::recent_bars_batch(&s.pool, &codes, None, LOOKBACK).await?;
    Ok(Json(
        base.into_iter()
            .map(
                |(thscode, name, board, asset_type, delisted_on, added_at)| {
                    let bars = bars.remove(&thscode).unwrap_or_default();
                    let closes: Vec<f64> = bars.iter().map(|b| b.q_close).collect();
                    let last = bars.last();
                    WatchItem {
                        change_1d: change(&closes, 1),
                        change_20d: change(&closes, 20),
                        last_date: last.map(|b| b.date),
                        close: last.map(|b| b.close.to_string()),
                        thscode,
                        name,
                        board,
                        asset_type,
                        delisted_on,
                        added_at,
                    }
                },
            )
            .collect(),
    ))
}

/// 最后一根相对 k 根之前的涨跌；日线不够时为空
fn change(closes: &[f64], k: usize) -> Option<f64> {
    let last = *closes.last()?;
    let before = closes.get(closes.len().checked_sub(k + 1)?)?;
    Some(last / before - 1.0).filter(|v| v.is_finite())
}

/// 收藏一只股票（重复收藏不报错）
#[utoipa::path(put, path = "/watchlist/{thscode}", tag = "watchlist", params(("thscode" = String, Path, description = "例：600519.SH")),
    responses((status = 204, description = "已收藏"), (status = 404, description = "证券不存在", body = crate::error::ErrorBody)))]
async fn add_watch(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(code): Path<String>,
) -> ApiResult<StatusCode> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ref.instrument WHERE thscode = $1)")
            .bind(&code)
            .fetch_one(&s.pool)
            .await?;
    if !exists {
        return Err(ApiError::NotFound(format!("证券不存在：{code}")));
    }
    sqlx::query(
        "INSERT INTO app.watchlist (user_id, thscode) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(me.user_id)
    .bind(&code)
    .execute(&s.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 取消收藏（本来没有收藏也不报错）
#[utoipa::path(delete, path = "/watchlist/{thscode}", tag = "watchlist", params(("thscode" = String, Path, description = "例：600519.SH")),
    responses((status = 204, description = "已取消收藏")))]
async fn remove_watch(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(code): Path<String>,
) -> ApiResult<StatusCode> {
    sqlx::query("DELETE FROM app.watchlist WHERE user_id = $1 AND thscode = $2")
        .bind(me.user_id)
        .bind(&code)
        .execute(&s.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_need_enough_history() {
        let closes = [10.0, 11.0, 12.1];
        assert!((change(&closes, 1).unwrap() - 0.1).abs() < 1e-12);
        assert!((change(&closes, 2).unwrap() - 0.21).abs() < 1e-12);
        assert!(change(&closes, 3).is_none());
        assert!(change(&[], 1).is_none());
    }
}
