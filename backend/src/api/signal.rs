//! 指标和信号：目录、自定义信号的增删改、今天出现的股票、过去表现、在一只股票 K 线上的位置。
//! 信号就是「起了名字的条件」，内置信号能写成条件的都给出条件定义，见 docs/domains/strategies.md#signals。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::analysis::backtest::ExitReason;
use crate::analysis::indicators::{self, Frame, IndicatorId, IndicatorInfo, IndicatorRef};
use crate::analysis::limit_pool::{CacheVersions, LimitPool};
use crate::analysis::points::{self, PointTrade, Today};
use crate::analysis::screener::{self, Hit};
use crate::analysis::signal_stats::{self, SignalStats};
use crate::analysis::signals::{self, ParamInfo, SignalId, SignalKind, Tone};
use crate::analysis::strategy::{
    Condition, CustomSignal, Definition, Op, SignalDef, SignalLib, Sort, Universe, cmp, cmp_ind,
};
use crate::analysis::{market, prices, signal_lib};
use crate::error::{ApiError, ApiResult};

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_signals, create_signal))
        .routes(routes!(get_signal))
        .routes(routes!(update_signal, delete_signal))
        .routes(routes!(signal_today))
        .routes(routes!(signal_stats))
        .routes(routes!(signal_days))
        .routes(routes!(list_indicators))
        .routes(routes!(indicator_series))
        .routes(routes!(strategy_points))
}

/// 自定义信号的键前缀：内置信号用 SignalId 的名字（例如 ma_cross_up），自定义信号用 my-<编号>
const MY: &str = "my-";

#[derive(Serialize, ToSchema, Clone)]
pub struct UsedBy {
    pub strategy_id: i64,
    pub name: String,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct IndicatorUse {
    pub id: IndicatorId,
    /// 带周期的名称，例如「20 日均线」
    pub label: String,
}

/// 信号目录中的一项：内置或自定义
#[derive(Serialize, ToSchema)]
pub struct SignalEntry {
    /// 内置信号是 SignalId，例如 ma_cross_up；自定义信号是 my-<编号>
    pub key: String,
    pub builtin: bool,
    /// 自定义信号的编号
    pub signal_id: Option<i64>,
    /// 名称，内置信号代入默认参数，例如「5 日均线上穿 20 日均线」
    pub name: String,
    /// 内置信号的分组；自定义信号是「我的信号」
    pub group: String,
    pub tone: Tone,
    pub kind: SignalKind,
    /// 是什么
    pub description: String,
    /// 怎么看、要注意什么
    pub usage: String,
    /// 内置信号的可调参数
    pub params: Vec<ParamInfo>,
    /// 条件定义（内置信号代入默认参数）；写不成条件的特殊信号为空
    pub definition: Option<SignalDef>,
    /// 定义的白话：每条条件一句；特殊信号是它的判断规则
    pub explained: Vec<String>,
    /// 写不成条件、逻辑在代码里的特殊信号
    pub special: bool,
    /// 用到的指标
    pub indicators: Vec<IndicatorUse>,
    /// 哪些策略在用
    pub used_by: Vec<UsedBy>,
    /// 自定义信号从哪个内置信号复制来
    pub based_on: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// 全部策略的 (编号, 名称, 定义)，用来算「被谁用」
async fn strategies(s: &AppState, lib: &SignalLib) -> ApiResult<Vec<(i64, String, Definition)>> {
    let rows: Vec<(i64, String, serde_json::Value)> = sqlx::query_as(
        "SELECT strategy_id, name, definition FROM app.strategy ORDER BY strategy_id",
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, name, v)| {
            serde_json::from_value::<Definition>(v)
                .ok()
                .map(|d| (id, name, d.with_lib(lib.clone())))
        })
        .collect())
}

fn uses(def: &SignalDef) -> Vec<IndicatorUse> {
    def.indicators()
        .into_iter()
        .map(|r| IndicatorUse {
            id: r.id,
            label: r.label(),
        })
        .collect()
}

fn builtin_entry(id: SignalId, all: &[(i64, String, Definition)], lib: &SignalLib) -> SignalEntry {
    let info = signals::info_of(id);
    let params = signals::Params::new();
    let def = signals::definition(id, &params);
    let explained = match (&def, signals::special_rule(id)) {
        (Some(d), _) => d.describe(lib),
        (None, Some(rule)) => vec![rule.to_string()],
        (None, None) => vec![],
    };
    // 直接用这个信号的策略，以及通过自定义信号间接用到的
    let via: Vec<i64> = lib
        .iter()
        .filter(|(_, c)| {
            c.def
                .conditions
                .iter()
                .any(|x| matches!(x, Condition::Signal { signal, .. } if *signal == id))
        })
        .map(|(k, _)| *k)
        .collect();
    let used_by = all
        .iter()
        .filter(|(_, _, d)| {
            d.builtin_signals().contains(&id) || d.my_signals().iter().any(|m| via.contains(m))
        })
        .map(|(sid, name, _)| UsedBy {
            strategy_id: *sid,
            name: name.clone(),
        })
        .collect();
    let indicators = def.as_ref().map(uses).unwrap_or_default();
    let desc = signals::label(id, &params);
    let fill = |t: &str| {
        let mut out = t.to_string();
        for p in info.params {
            let v = signals::param(id, &params, p.key);
            out = out.replace(
                &format!("{{{}}}", p.key),
                &if v.fract() == 0.0 {
                    format!("{}", v as i64)
                } else {
                    format!("{v}")
                },
            );
        }
        out
    };
    SignalEntry {
        key: serde_json::to_value(id)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        builtin: true,
        signal_id: None,
        name: desc,
        group: info.group.into(),
        tone: info.tone,
        kind: info.kind,
        description: fill(info.description),
        usage: fill(info.usage),
        params: info.params.to_vec(),
        special: def.is_none(),
        definition: def,
        explained,
        indicators,
        used_by,
        based_on: None,
        updated_at: None,
    }
}

type SignalRow = (
    i64,
    String,
    String,
    bool,
    serde_json::Value,
    Option<String>,
    DateTime<Utc>,
);

async fn custom_rows(s: &AppState, id: Option<i64>) -> ApiResult<Vec<SignalRow>> {
    Ok(sqlx::query_as(
        "SELECT signal_id, name, description, bullish, definition, based_on, updated_at FROM app.signal
         WHERE ($1::bigint IS NULL OR signal_id = $1) ORDER BY signal_id",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await?)
}

fn custom_entry(
    row: SignalRow,
    all: &[(i64, String, Definition)],
    lib: &SignalLib,
) -> Option<SignalEntry> {
    let (id, name, description, bullish, v, based_on, updated_at) = row;
    let def: SignalDef = serde_json::from_value(v).ok()?;
    let used_by = all
        .iter()
        .filter(|(_, _, d)| d.my_signals().contains(&id))
        .map(|(sid, n, _)| UsedBy {
            strategy_id: *sid,
            name: n.clone(),
        })
        .collect();
    Some(SignalEntry {
        key: format!("{MY}{id}"),
        builtin: false,
        signal_id: Some(id),
        name,
        group: "我的信号".into(),
        tone: if bullish {
            Tone::Bullish
        } else {
            Tone::Bearish
        },
        kind: if def.is_event() {
            SignalKind::Event
        } else {
            SignalKind::State
        },
        usage: String::new(),
        description,
        params: vec![],
        explained: def.describe(lib),
        indicators: uses(&def),
        special: false,
        definition: Some(def),
        used_by,
        based_on,
        updated_at: Some(updated_at),
    })
}

/// 全部信号：内置的按分组，自定义的在最后
#[utoipa::path(get, path = "/signals", tag = "signal", responses((status = 200, description = "成功", body = [SignalEntry])))]
async fn list_signals(State(s): State<AppState>) -> ApiResult<Json<Vec<SignalEntry>>> {
    let lib = signal_lib::load(&s.pool).await?;
    let all = strategies(&s, &lib).await?;
    let mut out: Vec<SignalEntry> = signals::CATALOG
        .iter()
        .map(|i| builtin_entry(i.id, &all, &lib))
        .collect();
    out.extend(
        custom_rows(&s, None)
            .await?
            .into_iter()
            .filter_map(|r| custom_entry(r, &all, &lib)),
    );
    Ok(Json(out))
}

/// 一个信号
#[utoipa::path(get, path = "/signals/{key}", tag = "signal", params(("key" = String, Path, description = "内置信号的名字（例如 ma_cross_up）或 my-<编号>")),
    responses((status = 200, description = "成功", body = SignalEntry), (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn get_signal(
    State(s): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<SignalEntry>> {
    let lib = signal_lib::load(&s.pool).await?;
    let all = strategies(&s, &lib).await?;
    if let Some(id) = key.strip_prefix(MY).and_then(|v| v.parse::<i64>().ok()) {
        let row = custom_rows(&s, Some(id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::NotFound(format!("信号不存在：{key}")))?;
        return custom_entry(row, &all, &lib)
            .map(Json)
            .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("信号定义无法解析：{key}")));
    }
    let id: SignalId = serde_json::from_value(serde_json::Value::String(key.clone()))
        .map_err(|_| ApiError::NotFound(format!("信号不存在：{key}")))?;
    Ok(Json(builtin_entry(id, &all, &lib)))
}

#[derive(Deserialize, ToSchema)]
pub struct SignalInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 偏多（true）或偏空
    #[serde(default = "yes")]
    pub bullish: bool,
    pub definition: SignalDef,
    /// 从哪个内置信号复制来（只作记录）
    #[serde(default)]
    pub based_on: Option<String>,
}

fn yes() -> bool {
    true
}

async fn check(s: &AppState, input: &SignalInput) -> ApiResult<()> {
    let n = input.name.trim().chars().count();
    if n == 0 || n > 30 {
        return Err(ApiError::BadRequest("信号名称应为 1 到 30 个字".into()));
    }
    input
        .definition
        .validate(&signal_lib::load(&s.pool).await?)
        .map_err(ApiError::BadRequest)
}

/// 新建自定义信号
#[utoipa::path(post, path = "/signals", tag = "signal", request_body = SignalInput,
    responses((status = 201, description = "已创建", body = SignalEntry), (status = 400, description = "定义不合法", body = crate::error::ErrorBody)))]
async fn create_signal(
    State(s): State<AppState>,
    Json(input): Json<SignalInput>,
) -> ApiResult<(StatusCode, Json<SignalEntry>)> {
    check(&s, &input).await?;
    let id: i64 = sqlx::query_scalar("INSERT INTO app.signal (name, description, bullish, definition, based_on) VALUES ($1, $2, $3, $4, $5) RETURNING signal_id")
        .bind(input.name.trim())
        .bind(input.description.trim())
        .bind(input.bullish)
        .bind(serde_json::to_value(&input.definition).map_err(anyhow::Error::from)?)
        .bind(&input.based_on)
        .fetch_one(&s.pool)
        .await?;
    let Json(entry) = get_signal(State(s), Path(format!("{MY}{id}"))).await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

/// 修改自定义信号。用到它的策略下一次巡检起按新定义
#[utoipa::path(put, path = "/signals/my/{signal_id}", tag = "signal", params(("signal_id" = i64, Path)), request_body = SignalInput,
    responses((status = 200, description = "已保存", body = SignalEntry),
              (status = 400, description = "定义不合法", body = crate::error::ErrorBody),
              (status = 404, description = "不存在", body = crate::error::ErrorBody)))]
async fn update_signal(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<SignalInput>,
) -> ApiResult<Json<SignalEntry>> {
    check(&s, &input).await?;
    let n = sqlx::query("UPDATE app.signal SET name = $2, description = $3, bullish = $4, definition = $5, updated_at = now() WHERE signal_id = $1")
        .bind(id)
        .bind(input.name.trim())
        .bind(input.description.trim())
        .bind(input.bullish)
        .bind(serde_json::to_value(&input.definition).map_err(anyhow::Error::from)?)
        .execute(&s.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::NotFound(format!("信号不存在：{id}")));
    }
    get_signal(State(s), Path(format!("{MY}{id}"))).await
}

/// 删除自定义信号；有策略在用时拒绝，并列出这些策略
#[utoipa::path(delete, path = "/signals/my/{signal_id}", tag = "signal", params(("signal_id" = i64, Path)),
    responses((status = 204, description = "已删除"), (status = 409, description = "有策略在用", body = crate::error::ErrorBody)))]
async fn delete_signal(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let lib = signal_lib::load(&s.pool).await?;
    let users: Vec<String> = strategies(&s, &lib)
        .await?
        .into_iter()
        .filter(|(_, _, d)| d.my_signals().contains(&id))
        .map(|(_, n, _)| format!("「{n}」"))
        .collect();
    if !users.is_empty() {
        return Err(ApiError::Conflict(format!(
            "策略 {} 在用这个信号，先从策略里去掉再删除",
            users.join("、")
        )));
    }
    sqlx::query("DELETE FROM app.signal WHERE signal_id = $1")
        .bind(id)
        .execute(&s.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 要看的信号：一个条件（例如「当天出现 ma_cross_up」或某个自定义信号），或者还没保存的草稿定义
#[derive(Deserialize, ToSchema, Clone, Serialize)]
pub struct SignalTarget {
    pub condition: Option<Condition>,
    pub draft: Option<SignalDef>,
}

/// 草稿用这个编号临时放进信号库
const DRAFT_ID: i64 = -1;

/// 解析成「一个条件 + 信号库」
async fn resolve(s: &AppState, t: &SignalTarget) -> ApiResult<(Condition, SignalLib)> {
    let lib = signal_lib::load(&s.pool).await?;
    let (cond, lib) = match (&t.condition, &t.draft) {
        (_, Some(def)) => {
            def.validate(&lib).map_err(ApiError::BadRequest)?;
            let mut map = (*lib).clone();
            map.insert(
                DRAFT_ID,
                CustomSignal {
                    name: "草稿".into(),
                    bullish: true,
                    def: def.clone(),
                },
            );
            (
                Condition::MySignal {
                    signal_id: DRAFT_ID,
                    within_days: 1,
                    streak_days: 1,
                },
                Arc::new(map),
            )
        }
        (Some(c), None) => (c.clone(), lib),
        (None, None) => return Err(ApiError::BadRequest("要给出 condition 或 draft".into())),
    };
    cond.validate(&lib).map_err(ApiError::BadRequest)?;
    Ok((cond, lib))
}

#[derive(Deserialize, ToSchema)]
pub struct TodayInput {
    pub target: SignalTarget,
    /// 交易日，默认最新
    #[serde(default)]
    pub date: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema)]
pub struct SignalToday {
    pub trade_date: NaiveDate,
    /// 默认股票池中，今天满足的股票数
    pub matched: usize,
    /// 按成交额排在前面的 50 只
    pub hits: Vec<Hit>,
    /// 统计时用的股票池（白话）
    pub universe: Vec<String>,
}

/// 今天哪些股票出现了这个信号（默认股票池，按成交额排序）
#[utoipa::path(post, path = "/signals/today", tag = "signal", request_body = TodayInput,
    responses((status = 200, description = "成功", body = SignalToday), (status = 400, description = "定义不合法或没有日线", body = crate::error::ErrorBody)))]
async fn signal_today(
    State(s): State<AppState>,
    Json(input): Json<TodayInput>,
) -> ApiResult<Json<SignalToday>> {
    let (cond, lib) = resolve(&s, &input.target).await?;
    let d = Definition {
        universe: Universe::default(),
        all: vec![cond],
        any: vec![],
        exit: Default::default(),
        sort: Sort::default(),
        limit: 50,
        lib,
    };
    let out = screener::screen(&s.pool, &[&d], input.date, screener::Mode::Preview)
        .await?
        .ok_or_else(|| ApiError::BadRequest("本地还没有日线".into()))?;
    let r = out.results.into_iter().next().expect("一个策略一个结果");
    Ok(Json(SignalToday {
        trade_date: out.trade_date,
        matched: r.matched,
        hits: r.hits,
        universe: d.describe_universe(),
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct StatsInput {
    pub target: SignalTarget,
    /// 统计最近几年，默认 3，范围 1 到 5
    #[serde(default)]
    pub years: Option<u32>,
}

/// 统计结果的缓存：全市场几年的日线逐只算一遍要几秒，数据不变时直接复用
/// (键, 数据版本, 结果)
type CachedStats = (String, CacheVersions, SignalStats);

#[derive(Default)]
pub struct StatsCache {
    entries: Mutex<VecDeque<CachedStats>>,
}

/// 过去表现：最近几年默认股票池中每次出现之后 5、10、20 个交易日的涨跌，和同期所有股票对比。全市场计算约几秒，结果缓存到有新数据为止
#[utoipa::path(post, path = "/signals/stats", tag = "signal", request_body = StatsInput,
    responses((status = 200, description = "成功", body = SignalStats), (status = 400, description = "定义不合法或没有日线", body = crate::error::ErrorBody)))]
async fn signal_stats(
    State(s): State<AppState>,
    Json(input): Json<StatsInput>,
) -> ApiResult<Json<SignalStats>> {
    let years = input.years.unwrap_or(3);
    if !(1..=5).contains(&years) {
        return Err(ApiError::BadRequest("years 应在 1 到 5 之间".into()));
    }
    let (cond, lib) = resolve(&s, &input.target).await?;
    // 键包含条件本身和草稿或自定义信号的定义：改了定义就不会命中旧结果
    let defs: Vec<(i64, String)> = lib
        .iter()
        .map(|(k, v)| (*k, serde_json::to_string(&v.def).unwrap_or_default()))
        .collect();
    let key = format!(
        "{}|{years}|{defs:?}",
        serde_json::to_string(&cond).unwrap_or_default()
    );
    // 复权价或权威涨跌停池有变化都会让统计结果失效
    let version = CacheVersions::load(&s.pool).await?;
    if let Some(hit) = s
        .stats
        .entries
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, v, _)| *k == key && *v == version)
    {
        return Ok(Json(hit.2.clone()));
    }
    let stats = signal_stats::compute(&s.pool, &cond, &lib, years)
        .await?
        .ok_or_else(|| ApiError::BadRequest("本地还没有日线".into()))?;
    let mut entries = s.stats.entries.lock().unwrap_or_else(|e| e.into_inner());
    entries.push_back((key, version, stats.clone()));
    while entries.len() > 64 {
        entries.pop_front();
    }
    Ok(Json(stats))
}

#[derive(Deserialize, ToSchema)]
pub struct DaysInput {
    pub target: SignalTarget,
    #[serde(default)]
    pub start: Option<NaiveDate>,
    #[serde(default)]
    pub end: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema)]
pub struct SignalDays {
    /// 出现的日子：连续几天都满足只给第一天
    pub dates: Vec<NaiveDate>,
}

async fn stock(
    s: &AppState,
    code: &str,
) -> ApiResult<(
    Vec<prices::DailyBar>,
    indicators::StockMeta,
    Vec<indicators::StPeriod>,
)> {
    let sec = market::securities(&s.pool).await?;
    let meta = sec
        .meta
        .get(code)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("证券不存在：{code}")))?;
    let history = sec.st_history.get(code).cloned().unwrap_or_default();
    Ok((
        prices::stock_bars(&s.pool, code, None).await?,
        meta,
        history,
    ))
}

/// 这只股票在日线窗口内的权威涨跌停记录：K 线上的涨跌停、炸板、连板信号以供应商的名单为准
async fn load_limits(s: &AppState, code: &str, bars: &[prices::DailyBar]) -> ApiResult<LimitPool> {
    let codes = [code.to_string()];
    Ok(match (bars.first(), bars.last()) {
        (Some(first), Some(last)) => {
            LimitPool::load_for(&s.pool, &codes, first.date, last.date).await?
        }
        _ => LimitPool::empty(),
    })
}

/// 这只股票上，信号在哪些交易日出现（K 线上画标记用）
#[utoipa::path(post, path = "/securities/{thscode}/signal-days", tag = "signal", params(("thscode" = String, Path)), request_body = DaysInput,
    responses((status = 200, description = "成功", body = SignalDays), (status = 400, description = "定义不合法", body = crate::error::ErrorBody), (status = 404, description = "证券不存在", body = crate::error::ErrorBody)))]
async fn signal_days(
    State(s): State<AppState>,
    Path(code): Path<String>,
    Json(input): Json<DaysInput>,
) -> ApiResult<Json<SignalDays>> {
    let (cond, lib) = resolve(&s, &input.target).await?;
    let (bars, meta, history) = stock(&s, &code).await?;
    let limits = load_limits(&s, &code, &bars).await?;
    let f = Frame::with_authority_and_st_history(&bars, Some(&meta), &limits, &code, &history);
    let series = cond.series(&f, &lib);
    let dates = (0..bars.len())
        .filter(|&i| series[i] && (i == 0 || !series[i - 1]))
        .map(|i| bars[i].date)
        .filter(|d| input.start.is_none_or(|x| *d >= x) && input.end.is_none_or(|x| *d <= x))
        .collect();
    Ok(Json(SignalDays { dates }))
}

/// 一个常用写法：可以直接拿去新建信号
#[derive(Serialize, ToSchema)]
pub struct IndicatorExample {
    pub condition: Condition,
    /// 白话
    pub text: String,
}

#[derive(Serialize, ToSchema)]
pub struct IndicatorEntry {
    #[serde(flatten)]
    pub info: IndicatorInfo,
    pub examples: Vec<IndicatorExample>,
    /// 哪些信号用到它（key 和名称）
    pub signals: Vec<SignalRefItem>,
    /// 哪些策略用到它（条件或排序）
    pub strategies: Vec<UsedBy>,
}

#[derive(Serialize, ToSchema)]
pub struct SignalRefItem {
    pub key: String,
    pub name: String,
}

/// 每个指标的常用写法：页面上点一下就能拿去新建信号
fn examples(id: IndicatorId) -> Vec<Condition> {
    use IndicatorId as I;
    use Op::*;
    match id {
        I::Close => vec![
            cmp_ind(I::Close, None, CrossesAbove, I::Ma, Some(20)),
            cmp_ind(I::Close, None, Gt, I::Ma, Some(60)),
        ],
        I::Open => vec![cmp_ind(I::Close, None, Gt, I::Open, None)],
        I::ChangePct => vec![
            cmp(I::ChangePct, None, Ge, 5.0),
            cmp(I::ChangePct, None, Le, -5.0),
        ],
        I::Amplitude => vec![cmp(I::Amplitude, None, Gt, 8.0)],
        I::Amount => vec![cmp(I::Amount, None, Ge, 5e8)],
        I::VolRatio => vec![
            cmp(I::VolRatio, Some(5), Ge, 2.0),
            cmp(I::VolRatio, Some(5), Le, 0.7),
        ],
        I::Ma => vec![
            cmp_ind(I::Ma, Some(5), CrossesAbove, I::Ma, Some(20)),
            cmp_ind(I::Ma, Some(20), Gt, I::Ma, Some(60)),
        ],
        I::Ema => vec![cmp_ind(I::Close, None, CrossesAbove, I::Ema, Some(20))],
        I::MaGap => vec![
            cmp(I::MaGap, Some(20), Le, -10.0),
            cmp(I::MaGap, Some(20), Ge, 15.0),
        ],
        I::MacdDif | I::MacdDea => vec![
            cmp_ind(I::MacdDif, None, CrossesAbove, I::MacdDea, None),
            cmp(I::MacdDif, None, Gt, 0.0),
        ],
        I::MacdHist => vec![cmp(I::MacdHist, None, CrossesAbove, 0.0)],
        I::Rsi => vec![
            cmp(I::Rsi, Some(14), Lt, 30.0),
            cmp(I::Rsi, Some(14), Gt, 70.0),
            cmp(I::Rsi, Some(6), CrossesAbove, 20.0),
        ],
        I::KdjK => vec![cmp_ind(I::KdjK, None, CrossesAbove, I::KdjD, None)],
        I::KdjD => vec![cmp(I::KdjD, None, Lt, 20.0)],
        I::KdjJ => vec![cmp(I::KdjJ, None, Lt, 0.0)],
        I::BollUpper => vec![cmp_ind(I::Close, None, CrossesAbove, I::BollUpper, None)],
        I::BollMid => vec![cmp_ind(I::Close, None, Gt, I::BollMid, None)],
        I::BollLower => vec![cmp_ind(I::Close, None, CrossesBelow, I::BollLower, None)],
        I::AtrPct => vec![cmp(I::AtrPct, Some(14), Lt, 3.0)],
        I::Momentum => vec![
            cmp(I::Momentum, Some(20), Ge, 20.0),
            cmp(I::Momentum, Some(20), Le, -20.0),
        ],
        I::PctFromHigh => vec![
            cmp(I::PctFromHigh, Some(60), Ge, -5.0),
            cmp(I::PctFromHigh, Some(60), Le, -30.0),
        ],
        I::PctFromLow => vec![cmp(I::PctFromLow, Some(60), Le, 10.0)],
        I::PrevHighClose => vec![cmp_ind(I::Close, None, Gt, I::PrevHighClose, Some(60))],
        I::PrevLowClose => vec![cmp_ind(I::Close, None, Lt, I::PrevLowClose, Some(60))],
        I::PlusDi | I::MinusDi => vec![
            cmp_ind(I::PlusDi, Some(14), CrossesAbove, I::MinusDi, Some(14)),
            cmp_ind(I::PlusDi, Some(14), Gt, I::MinusDi, Some(14)),
        ],
        I::Adx => vec![
            cmp(I::Adx, Some(14), Gt, 25.0),
            cmp(I::Adx, Some(14), Lt, 20.0),
        ],
        I::Supertrend => vec![
            cmp_ind(I::Close, None, CrossesAbove, I::Supertrend, Some(10)),
            cmp_ind(I::Close, None, Gt, I::Supertrend, Some(10)),
        ],
        I::Cci => vec![
            cmp(I::Cci, Some(14), CrossesAbove, 100.0),
            cmp(I::Cci, Some(14), CrossesBelow, -100.0),
        ],
        I::BollWidth => vec![cmp(I::BollWidth, None, Lt, 10.0)],
        I::MaSpread => vec![cmp(I::MaSpread, None, Le, 2.0)],
        I::High | I::Low | I::Volume | I::Atr | I::HighN | I::LowN => vec![],
    }
}

/// 全部指标：说明、公式、常用写法、被哪些信号和策略用到
#[utoipa::path(get, path = "/indicators", tag = "signal", responses((status = 200, description = "成功", body = [IndicatorEntry])))]
async fn list_indicators(State(s): State<AppState>) -> ApiResult<Json<Vec<IndicatorEntry>>> {
    let lib = signal_lib::load(&s.pool).await?;
    let all = strategies(&s, &lib).await?;
    let mut signal_uses: HashMap<IndicatorId, Vec<SignalRefItem>> = HashMap::new();
    let none = signals::Params::new();
    for info in signals::CATALOG {
        if let Some(d) = signals::definition(info.id, &none) {
            for r in d.indicators() {
                let key = serde_json::to_value(info.id)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                signal_uses.entry(r.id).or_default().push(SignalRefItem {
                    key,
                    name: signals::label(info.id, &none),
                });
            }
        }
    }
    for (id, c) in lib.iter() {
        for r in c.def.indicators() {
            signal_uses.entry(r.id).or_default().push(SignalRefItem {
                key: format!("{MY}{id}"),
                name: c.name.clone(),
            });
        }
    }
    let out = indicators::CATALOG
        .iter()
        .map(|info| {
            let mut sigs = signal_uses.remove(&info.id).unwrap_or_default();
            sigs.dedup_by(|a, b| a.key == b.key);
            IndicatorEntry {
                info: *info,
                examples: examples(info.id)
                    .into_iter()
                    .map(|c| IndicatorExample {
                        text: c.describe(&lib),
                        condition: c,
                    })
                    .collect(),
                signals: sigs,
                strategies: all
                    .iter()
                    .filter(|(_, _, d)| d.indicator_ids().contains(&info.id))
                    .map(|(sid, n, _)| UsedBy {
                        strategy_id: *sid,
                        name: n.clone(),
                    })
                    .collect(),
            }
        })
        .collect();
    Ok(Json(out))
}

#[derive(Deserialize, IntoParams)]
pub struct IndicatorQuery {
    /// 指标，例如 rsi
    pub id: IndicatorId,
    /// 周期，不带周期的指标忽略
    pub n: Option<u32>,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema)]
pub struct IndicatorPoint {
    pub date: NaiveDate,
    pub value: f64,
}

#[derive(Serialize, ToSchema)]
pub struct IndicatorSeries {
    /// 带周期的名称
    pub label: String,
    /// 单位，同指标目录
    pub unit: String,
    pub points: Vec<IndicatorPoint>,
}

/// 这只股票上一个指标每天的值（K 线下方画图用）
#[utoipa::path(get, path = "/securities/{thscode}/indicator", tag = "signal", params(("thscode" = String, Path), IndicatorQuery),
    responses((status = 200, description = "成功", body = IndicatorSeries), (status = 400, description = "周期不合法", body = crate::error::ErrorBody), (status = 404, description = "证券不存在", body = crate::error::ErrorBody)))]
async fn indicator_series(
    State(s): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<IndicatorQuery>,
) -> ApiResult<Json<IndicatorSeries>> {
    let r = IndicatorRef::new(q.id, q.n);
    r.validate().map_err(ApiError::BadRequest)?;
    let (bars, meta, history) = stock(&s, &code).await?;
    let f = Frame::with_st_history(&bars, Some(&meta), &history);
    let v = f.get(r);
    let points = bars
        .iter()
        .zip(v.iter())
        .filter(|(b, v)| {
            v.is_finite()
                && q.start.is_none_or(|x| b.date >= x)
                && q.end.is_none_or(|x| b.date <= x)
        })
        .map(|(b, v)| IndicatorPoint {
            date: b.date,
            value: *v,
        })
        .collect();
    Ok(Json(IndicatorSeries {
        label: r.label(),
        unit: indicators::info_of(q.id).unit.into(),
        points,
    }))
}

#[derive(Deserialize, IntoParams)]
pub struct PointsQuery {
    pub strategy_id: i64,
    /// 从哪天开始算（空仓起步），默认 3 年前
    pub start: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema)]
pub struct StrategyPoints {
    pub strategy_id: i64,
    pub strategy_name: String,
    /// 历次买卖，按买入时间排序
    pub trades: Vec<PointTrade>,
    /// 最后一个交易日：buy 今天是买点、sell 今天是卖点、holding 持有中、flat 空仓
    pub today: String,
    /// 今天是卖点时的原因
    pub today_reason: Option<String>,
    pub today_exit: Option<ExitReason>,
}

/// 一个策略在这只股票上的买卖点（K 线上画标记用），和每日巡检同一套规则
#[utoipa::path(get, path = "/securities/{thscode}/strategy-points", tag = "signal", params(("thscode" = String, Path), PointsQuery),
    responses((status = 200, description = "成功", body = StrategyPoints), (status = 404, description = "证券或策略不存在", body = crate::error::ErrorBody)))]
async fn strategy_points(
    State(s): State<AppState>,
    Path(code): Path<String>,
    Query(q): Query<PointsQuery>,
) -> ApiResult<Json<StrategyPoints>> {
    let row: Option<(String, serde_json::Value)> =
        sqlx::query_as("SELECT name, definition FROM app.strategy WHERE strategy_id = $1")
            .bind(q.strategy_id)
            .fetch_optional(&s.pool)
            .await?;
    let (name, v) =
        row.ok_or_else(|| ApiError::NotFound(format!("策略不存在：{}", q.strategy_id)))?;
    let d = serde_json::from_value::<Definition>(v)
        .map_err(anyhow::Error::from)?
        .with_lib(signal_lib::load(&s.pool).await?);
    let (bars, meta, history) = stock(&s, &code).await?;
    let limits = load_limits(&s, &code, &bars).await?;
    let f = Frame::with_authority_and_st_history(&bars, Some(&meta), &limits, &code, &history);
    let start = q.start.unwrap_or_else(|| {
        bars.last().map_or(crate::time::today_cn(), |b| b.date) - chrono::Duration::days(365 * 3)
    });
    let from = bars
        .iter()
        .position(|b| b.date >= start)
        .unwrap_or(bars.len());
    let p = points::run(&bars, &f, &meta, &d, from);
    let (today, today_reason, today_exit) = match p.today {
        Today::Buy => ("buy", None, None),
        Today::Sell { reason, text } => ("sell", Some(text), Some(reason)),
        Today::Holding => ("holding", None, None),
        Today::Flat => ("flat", None, None),
    };
    Ok(Json(StrategyPoints {
        strategy_id: q.strategy_id,
        strategy_name: name,
        trades: p.trades,
        today: today.into(),
        today_reason,
        today_exit,
    }))
}
