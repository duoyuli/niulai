//! 运行配置。来源（后者覆盖前者）：内置默认值 → `backend/.env` → 进程环境变量。
//!
//! 标准变量：`DATABASE_URL`、`HITHINK_FINANCE_API_KEY`（可选：不设置时在网页的首次登录引导中配置，保存在数据库）。
//! 其余变量统一加 `NIULAI_` 前缀，例如 `NIULAI_API_BIND=0.0.0.0:8080`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;

// @project-doc docs/interfaces/configuration.md#settings
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub database_url: String,
    /// 设置后优先于网页中保存的 Key，网页上不能修改
    #[serde(default)]
    pub hithink_finance_api_key: Option<String>,

    #[serde(default = "default_base_url")]
    pub vendor_base_url: String,
    /// 数据集注册表路径；为空时依次查找 `config/datasets.yaml`（在 backend/ 下运行）、`backend/config/datasets.yaml`（在仓库根目录运行）
    #[serde(default)]
    pub datasets_path: Option<PathBuf>,
    #[serde(default = "default_api_bind")]
    pub api_bind: String,
    #[serde(default = "default_db_max_connections")]
    pub db_max_connections: u32,
    /// 单个进程同时执行的同步任务数
    #[serde(default = "default_worker_concurrency")]
    pub worker_concurrency: usize,
    /// 供应商接口每秒请求数上限。同一时刻只有一个进程执行同步任务，所以这就是全局上限
    #[serde(default = "default_rate_limit_rps")]
    pub rate_limit_rps: u32,
    /// 请求超时（秒）
    #[serde(default = "default_http_timeout_s")]
    pub http_timeout_s: u64,
    /// 是否在 worker 中启用定时调度
    #[serde(default = "default_true")]
    pub scheduler_enabled: bool,
    /// 前端构建产物目录（frontend/dist）；配置后 `niulai serve` 在 API 之外同时提供前端页面
    #[serde(default)]
    pub web_dir: Option<PathBuf>,
    /// 第一个管理员的账号和密码，没有配置时都是 niulai。只在第一个管理员还没有密码时使用一次（见 auth::init_first_admin），之后在网页上改
    #[serde(default = "default_admin_user")]
    pub admin_user: String,
    #[serde(default = "default_admin_password")]
    pub admin_password: String,
    /// 与 upgrader 容器共享的目录（Docker 部署中是 /upgrade）；为空时不能一键升级
    #[serde(default)]
    pub upgrade_dir: Option<PathBuf>,
    /// 到哪个 GitHub 仓库查新版本；设为空字符串时不检查
    #[serde(default = "default_update_repo")]
    pub update_repo: String,
}

fn default_update_repo() -> String {
    "duoyuli/niulai".into()
}

pub const DEFAULT_ADMIN_USER: &str = "niulai";
pub const DEFAULT_ADMIN_PASSWORD: &str = "niulai";

fn default_admin_user() -> String {
    DEFAULT_ADMIN_USER.into()
}
fn default_admin_password() -> String {
    DEFAULT_ADMIN_PASSWORD.into()
}

fn default_base_url() -> String {
    "https://fuyao.aicubes.cn".into()
}
fn default_api_bind() -> String {
    "127.0.0.1:8080".into()
}
fn default_db_max_connections() -> u32 {
    10
}
fn default_worker_concurrency() -> usize {
    3
}
fn default_rate_limit_rps() -> u32 {
    2
}
fn default_http_timeout_s() -> u64 {
    60
}
fn default_true() -> bool {
    true
}

impl Settings {
    pub fn load() -> anyhow::Result<Self> {
        let mut vars = find_dotenv()
            .map(|p| read_dotenv(&p))
            .transpose()?
            .unwrap_or_default();
        vars.extend(std::env::vars());

        let mut settings: Settings = config::Config::builder()
            .add_source(
                config::Environment::default()
                    .source(Some(vars.clone()))
                    .try_parsing(true),
            )
            .add_source(
                config::Environment::with_prefix("NIULAI")
                    .source(Some(vars))
                    .try_parsing(true),
            )
            .build()?
            .try_deserialize()
            .context("读取配置失败：需要 DATABASE_URL")?;
        // 空值（例如 .env 中写了 HITHINK_FINANCE_API_KEY= 但没填）按未配置处理
        settings.hithink_finance_api_key = settings
            .hithink_finance_api_key
            .take()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.worker_concurrency == 0 || self.rate_limit_rps == 0 {
            bail!("NIULAI_WORKER_CONCURRENCY 和 NIULAI_RATE_LIMIT_RPS 必须大于 0");
        }
        Ok(())
    }

    pub fn datasets_file(&self) -> anyhow::Result<PathBuf> {
        if let Some(p) = &self.datasets_path {
            return Ok(p.clone());
        }
        ["config/datasets.yaml", "backend/config/datasets.yaml"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .context("找不到 config/datasets.yaml，可用 NIULAI_DATASETS_PATH 指定")
    }
}

/// 从当前目录向上查找 `.env` 或 `backend/.env`（最多 3 层），兼容在仓库根目录或 backend/ 下运行。
fn find_dotenv() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    for _ in 0..3 {
        for p in [dir.join(".env"), dir.join("backend/.env")] {
            if p.is_file() {
                return Some(p);
            }
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// 解析 `KEY=VALUE` 格式；忽略空行和 `#` 注释，去掉值两侧的引号。
fn read_dotenv(path: &Path) -> anyhow::Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("读取 {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v);
            (k.trim().to_string(), v.to_string())
        })
        .collect())
}
