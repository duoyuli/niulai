//! 版本检测和一键升级（只用于 Docker 部署）。
//! niulai 自己不能操作 Docker：升级由 compose.yml 中的 upgrader 容器执行，双方通过共享目录 `NIULAI_UPGRADE_DIR` 交换文件：
//! niulai 写 `request`（要升级到的版本号），upgrader 写 `heartbeat`（还在运行）、`status`（进度）和 `log`。
//! 规则见 docs/operations/runbook.md#one_click_upgrade。

use std::path::Path;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::AppState;
use crate::version::parse;
use crate::error::{ApiError, ApiResult};

/// 构建时写入的版本号（CI 用 git 标签，例如 v2026.10.18）；本机编译的是 dev
pub fn current_version() -> &'static str {
    option_env!("NIULAI_VERSION").map(|v| v.trim_start_matches('v')).filter(|v| !v.is_empty()).unwrap_or("dev")
}

/// 多久查一次 GitHub 上的最新版本
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// 查询失败后多久再试
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
/// upgrader 每 3 秒写一次心跳；超过这个时间没更新，认为它没有运行
const HEARTBEAT_FRESH_SECS: i64 = 30;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(version)).routes(routes!(upgrade))
}

/// 缓存的最新版本
#[derive(Default)]
pub struct LatestCache {
    inner: tokio::sync::Mutex<Option<Checked>>,
}

#[derive(Clone)]
struct Checked {
    at: Instant,
    checked_at: chrono::DateTime<chrono::Utc>,
    latest: Option<String>,
    error: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct VersionInfo {
    /// 正在运行的版本，例如 2026.10.18；本机编译的是 dev
    pub current: String,
    /// GitHub 上最新的正式版本；还没查到或查询失败时为空
    pub latest: Option<String>,
    pub has_update: bool,
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 查询失败的原因
    pub check_error: Option<String>,
    /// 两个版本之间改了什么（GitHub 对比页面）
    pub changes_url: Option<String>,
    pub upgrade: UpgradeStatus,
}

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeState {
    /// 没有进行中的升级
    Idle,
    /// 已提交，等 upgrader 接手
    Requested,
    Running,
    Done,
    Failed,
}

#[derive(Serialize, ToSchema)]
pub struct UpgradeStatus {
    /// upgrader 容器在运行，可以一键升级
    pub available: bool,
    pub state: UpgradeState,
    /// 正在升级或最近一次升级的目标版本
    pub target: Option<String>,
    /// 当前步骤或结果，例如「拉取镜像」「已升级到 2026.10.18」
    pub message: Option<String>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 失败时 upgrader 日志的最后几行
    pub log_tail: Option<String>,
    /// 每一步的进度（下载、更新升级助手、备份、部署迁移、启动新版本）；升级助手还没接手时为空
    pub steps: Vec<UpgradeStep>,
}

/// 升级的一步
#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct UpgradeStep {
    pub key: String,
    /// 例如「下载新版本」
    pub label: String,
    /// pending / running / done / failed / skipped
    pub state: String,
    /// 例如「45%（120 / 260 MB）」「已等 35 秒」
    #[serde(default)]
    pub detail: String,
}

#[derive(Deserialize, IntoParams)]
pub struct VersionQuery {
    /// 为 true 时不用缓存，立即查询 GitHub
    #[serde(default)]
    pub refresh: bool,
}

/// 当前版本、最新版本和一键升级的状态
#[utoipa::path(get, path = "/system/version", tag = "system", params(VersionQuery),
    responses((status = 200, description = "成功", body = VersionInfo)))]
async fn version(State(s): State<AppState>, Query(q): Query<VersionQuery>) -> Json<VersionInfo> {
    let checked = latest(&s, q.refresh).await;
    let current = current_version();
    let latest = checked.as_ref().and_then(|c| c.latest.clone());
    let has_update = matches!((parse(current), latest.as_deref().and_then(parse)), (Some(a), Some(b)) if b > a);
    let repo = &s.engine.settings.update_repo;
    let changes_url = latest.as_ref().filter(|_| has_update).map(|l| format!("https://github.com/{repo}/compare/v{current}...v{l}"));
    Json(VersionInfo {
        current: current.into(),
        latest,
        has_update,
        checked_at: checked.as_ref().map(|c| c.checked_at),
        check_error: checked.and_then(|c| c.error),
        changes_url,
        upgrade: upgrade_status(s.engine.settings.upgrade_dir.as_deref()),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct UpgradeInput {
    /// 要升级到的版本，例如 2026.10.18；为空时升级到最新版本
    pub version: Option<String>,
}

/// 一键升级：交给 upgrader 容器执行。它会先备份内置数据库，再拉取新镜像并重建 niulai 容器，期间服务中断约一分钟
#[utoipa::path(post, path = "/system/upgrade", tag = "system", request_body = UpgradeInput,
    responses((status = 202, description = "已提交", body = UpgradeStatus),
              (status = 400, description = "版本号不合法", body = crate::error::ErrorBody),
              (status = 409, description = "不能升级：没有运行 upgrader、已是最新或正在升级", body = crate::error::ErrorBody)))]
async fn upgrade(State(s): State<AppState>, Json(input): Json<UpgradeInput>) -> ApiResult<(StatusCode, Json<UpgradeStatus>)> {
    let Some(dir) = s.engine.settings.upgrade_dir.as_deref() else {
        return Err(ApiError::Conflict("这个部署没有启用一键升级".into()));
    };
    let status = upgrade_status(Some(dir));
    if !status.available {
        return Err(ApiError::Conflict("升级助手（upgrader 容器）没有运行，见运维手册的「一键升级」".into()));
    }
    if matches!(status.state, UpgradeState::Requested | UpgradeState::Running) {
        return Err(ApiError::Conflict("正在升级，请等它完成".into()));
    }
    let target = match input.version.map(|v| v.trim().trim_start_matches('v').to_string()).filter(|v| !v.is_empty()) {
        Some(v) => v,
        None => latest(&s, true).await.and_then(|c| c.latest).ok_or_else(|| ApiError::Conflict("查不到最新版本".into()))?,
    };
    let Some(want) = parse(&target) else {
        return Err(ApiError::BadRequest(format!("版本号不合法：{target}")));
    };
    if parse(current_version()).is_some_and(|cur| want <= cur) {
        return Err(ApiError::Conflict(format!("当前已是 {}，不需要升级到 {target}", current_version())));
    }
    // 先写临时文件再改名，upgrader 不会读到写了一半的内容
    let tmp = dir.join("request.tmp");
    std::fs::write(&tmp, &target).and_then(|_| std::fs::rename(&tmp, dir.join("request"))).map_err(anyhow::Error::from)?;
    tracing::info!(target = %target, "已提交一键升级");
    Ok((StatusCode::ACCEPTED, Json(upgrade_status(Some(dir)))))
}

/// 读缓存，过期时查 GitHub；查询失败时保留上一次的结果
async fn latest(s: &AppState, refresh: bool) -> Option<Checked> {
    let repo = s.engine.settings.update_repo.trim();
    if repo.is_empty() {
        return None;
    }
    let mut cache = s.latest.inner.lock().await;
    let fresh = cache.as_ref().is_some_and(|c| c.at.elapsed() < if c.error.is_some() { RETRY_AFTER } else { CHECK_EVERY });
    if fresh && !refresh {
        return cache.clone();
    }
    let checked_at = chrono::Utc::now();
    let next = match fetch_latest(repo).await {
        Ok(latest) => Checked { at: Instant::now(), checked_at, latest, error: None },
        Err(e) => {
            tracing::warn!(error = %e, "查询最新版本失败");
            let previous = cache.as_ref().and_then(|c| c.latest.clone());
            Checked { at: Instant::now(), checked_at, latest: previous, error: Some(format!("查询 GitHub 失败：{e}")) }
        }
    };
    *cache = Some(next.clone());
    Some(next)
}

#[derive(Deserialize)]
struct Tag {
    name: String,
}

/// 可以升级到的最新正式版本（vYYYY.MM.DD[.N]，见 [`crate::version`]）：GitHub 上的标签，并且镜像已经构建好、能从 GHCR 拉取。
/// 推送标签后 CI 要几分钟才把镜像推上去，这期间不提示新版本，避免一键升级拉取镜像失败
async fn fetch_latest(repo: &str) -> anyhow::Result<Option<String>> {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).user_agent(concat!("niulai/", env!("CARGO_PKG_VERSION"))).build()?;
    let tags: Vec<Tag> = http
        .get(format!("https://api.github.com/repos/{repo}/tags?per_page=100"))
        .header("accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    // 从新到旧最多看 3 个：最新的还在构建时，退回上一个
    for v in versions_desc(tags.iter().map(|t| t.name.as_str())).into_iter().take(3) {
        if image_published(&http, repo, &v).await? {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

/// GHCR 上有没有这个版本的镜像（匿名拉取令牌 + 查询清单）
async fn image_published(http: &reqwest::Client, repo: &str, version: &str) -> anyhow::Result<bool> {
    let repo = repo.to_lowercase();
    #[derive(Deserialize)]
    struct Token {
        token: String,
    }
    let token: Token = http.get(format!("https://ghcr.io/token?scope=repository:{repo}:pull")).send().await?.error_for_status()?.json().await?;
    let resp = http
        .head(format!("https://ghcr.io/v2/{repo}/manifests/{version}"))
        .bearer_auth(token.token)
        .header(
            "accept",
            "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json",
        )
        .send()
        .await?;
    match resp.status() {
        s if s.is_success() => Ok(true),
        reqwest::StatusCode::NOT_FOUND => Ok(false),
        s => anyhow::bail!("查询镜像失败：HTTP {s}"),
    }
}

/// 正式版本号，从新到旧
fn versions_desc<'a>(tags: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut v: Vec<_> = tags.filter_map(|t| t.strip_prefix('v')).filter_map(|v| parse(v).map(|p| (p, v.to_string()))).collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.0));
    v.into_iter().map(|(_, v)| v).collect()
}


#[derive(Deserialize)]
struct StatusFile {
    state: UpgradeState,
    #[serde(default)]
    target: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    at: i64,
    #[serde(default)]
    steps: Vec<UpgradeStep>,
}

fn upgrade_status(dir: Option<&Path>) -> UpgradeStatus {
    let none = UpgradeStatus { available: false, state: UpgradeState::Idle, target: None, message: None, updated_at: None, log_tail: None, steps: vec![] };
    let Some(dir) = dir else { return none };
    let now = chrono::Utc::now().timestamp();
    let heartbeat = std::fs::read_to_string(dir.join("heartbeat")).ok().and_then(|s| s.trim().parse::<i64>().ok());
    let available = heartbeat.is_some_and(|t| (now - t).abs() <= HEARTBEAT_FRESH_SECS);
    let mut st = UpgradeStatus { available, ..none };
    if let Ok(v) = std::fs::read_to_string(dir.join("request")) {
        st.state = UpgradeState::Requested;
        st.target = Some(v.trim().to_string());
        st.message = Some("等待升级助手接手".into());
        return st;
    }
    let Some(f) = std::fs::read_to_string(dir.join("status")).ok().and_then(|s| serde_json::from_str::<StatusFile>(&s).ok()) else {
        return st;
    };
    st.state = f.state;
    st.target = Some(f.target).filter(|t| !t.is_empty());
    st.message = Some(f.message).filter(|m| !m.is_empty());
    st.updated_at = chrono::DateTime::from_timestamp(f.at, 0);
    st.steps = f.steps;
    if f.state == UpgradeState::Failed {
        st.log_tail = std::fs::read_to_string(dir.join("log")).ok().map(|l| tail(&l, 20));
    }
    st
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_newest_stable_tag() {
        let tags = ["v1.5.0", "v2026.10.18", "v2026.10.18.1", "v2026.10.09", "v2026.10.18-rc.1", "latest", "v1.2"];
        assert_eq!(versions_desc(tags.into_iter()), vec!["2026.10.18.1".to_string(), "2026.10.18".into(), "2026.10.09".into()]);
        assert_eq!(parse("dev"), None);
    }

    #[test]
    fn reads_upgrade_files() {
        let dir = std::env::temp_dir().join(format!("niulai_upgrade_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!upgrade_status(Some(&dir)).available, "没有心跳");
        std::fs::write(dir.join("heartbeat"), chrono::Utc::now().timestamp().to_string()).unwrap();
        let st = upgrade_status(Some(&dir));
        assert!(st.available && st.state == UpgradeState::Idle);
        std::fs::write(dir.join("status"), r#"{"state":"failed","target":"2026.10.18","message":"拉取镜像失败","at":1}"#).unwrap();
        std::fs::write(dir.join("log"), (1..=30).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
        let st = upgrade_status(Some(&dir));
        assert_eq!((st.state, st.target.as_deref()), (UpgradeState::Failed, Some("2026.10.18")));
        assert!(st.log_tail.unwrap().starts_with("line 11"));
        assert!(st.steps.is_empty(), "状态里没有步骤时为空");
        // 新版本的升级助手写出每一步的进度
        std::fs::write(
            dir.join("status"),
            r#"{"state":"running","target":"2026.10.18","message":"下载新版本：45%","at":1,"steps":[{"key":"pull","label":"下载新版本","state":"running","detail":"45%"},{"key":"self","label":"更新升级助手","state":"pending"}]}"#,
        )
        .unwrap();
        let st = upgrade_status(Some(&dir));
        assert_eq!(st.steps.len(), 2);
        assert_eq!((st.steps[0].state.as_str(), st.steps[0].detail.as_str(), st.steps[1].detail.as_str()), ("running", "45%", ""));
        std::fs::write(dir.join("request"), "2026.10.19").unwrap();
        assert_eq!(upgrade_status(Some(&dir)).state, UpgradeState::Requested);
        std::fs::write(dir.join("heartbeat"), "1").unwrap();
        assert!(!upgrade_status(Some(&dir)).available, "心跳过期");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
