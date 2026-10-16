//! 首次登录引导：在网页上配置同花顺金融数据服务的 API Key，第一次配置成功后规划全量回补。
//!
//! Key 的来源按优先级：环境变量 `HITHINK_FINANCE_API_KEY`（设置后网页上不能修改）→ 网页中保存的 `app.setting`。
//! 两者都没有时，执行循环和定时调度暂停，等用户在网页上配置。见 docs/decisions/api_key_setup.md。

use crate::sync::Engine;
use crate::sync::planner::PlanOptions;
use crate::sync::run::RunKind;
use crate::vendor::CallError;

/// 生成和管理 API Key 的页面
pub const API_KEY_URL: &str = "https://fuyao.aicubes.cn/admin/";
/// `app.setting` 中保存 Key 的行
const SETTING: &str = "hithink_api_key";
/// 验证 Key 用的接口：一次请求，响应很小
const VERIFY_ENDPOINT: &str = "/api/a-share/calendar/trading-days";
/// 第一次配置 Key 后规划回补的优先级，与运维手册中的首次回补一致
const INITIAL_PRIORITIES: [&str; 2] = ["P0", "P1"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Env,
    Database,
}

/// 网页中保存的 Key
pub async fn stored_api_key(pool: &sqlx::PgPool) -> sqlx::Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM app.setting WHERE name = $1").bind(SETTING).fetch_optional(pool).await
}

/// 当前生效的 Key 的来源，以及只露出末尾 4 位的提示
pub async fn key_status(engine: &Engine) -> sqlx::Result<Option<(KeySource, String)>> {
    let (source, key) = match &engine.settings.hithink_finance_api_key {
        Some(k) => (KeySource::Env, k.clone()),
        None => match stored_api_key(&engine.pool).await? {
            Some(k) => (KeySource::Database, k),
            None => return Ok(None),
        },
    };
    Ok(Some((source, hint(&key))))
}

fn hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
    format!("••••{tail}")
}

/// 是否已经规划过回补（任何一次）
pub async fn backfill_planned(pool: &sqlx::PgPool) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM ops.run WHERE kind = 'backfill')").fetch_one(pool).await
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("API Key 由环境变量 HITHINK_FINANCE_API_KEY 提供，要更换请修改部署配置（deploy/.env）后重启")]
    FromEnv,
    #[error("{0}")]
    Invalid(String),
    #[error("暂时连不上同花顺接口，稍后再试：{0}")]
    Unreachable(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<sqlx::Error> for SetupError {
    fn from(e: sqlx::Error) -> Self {
        SetupError::Other(e.into())
    }
}

// @project-doc docs/interfaces/rest_api.md#setup
/// 验证并保存 Key，立即生效。第一次配置（还没有任何回补运行）时规划全量回补，返回第一个波次的运行 ID
pub async fn configure_api_key(engine: &Engine, key: &str) -> Result<Option<i64>, SetupError> {
    if engine.settings.hithink_finance_api_key.is_some() {
        return Err(SetupError::FromEnv);
    }
    let key = key.trim();
    if key.is_empty() {
        return Err(SetupError::Invalid("API Key 不能为空".into()));
    }
    match engine.client.verify_key(key, VERIFY_ENDPOINT).await {
        Ok(_) => {}
        Err(CallError::Retryable { message, .. }) => return Err(SetupError::Unreachable(message)),
        Err(CallError::Fatal { code, message, .. }) => {
            let why = match code {
                Some(2001..=2003) => "同花顺接口拒绝了这个 Key：确认复制完整、没有多余的空格，并且 Key 没有被删除或停用".to_string(),
                _ => format!("验证失败（code={}）：{message}", code.map_or("—".into(), |c| c.to_string())),
            };
            return Err(SetupError::Invalid(why));
        }
    }
    sqlx::query(
        "INSERT INTO app.setting (name, value) VALUES ($1, $2)
         ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(SETTING)
    .bind(key)
    .execute(&engine.pool)
    .await?;
    engine.client.set_api_key(Some(key.to_string()));
    tracing::info!("API Key 已在网页上配置");

    if backfill_planned(&engine.pool).await? {
        return Ok(None);
    }
    let priorities: Vec<String> = INITIAL_PRIORITIES.iter().map(|p| p.to_string()).collect();
    let specs = engine.select(&[], &priorities)?;
    let (run_id, _) = engine.planner().plan_run(RunKind::Backfill, &specs, PlanOptions::default()).await?;
    tracing::info!(run_id, "已规划首次全量回补");
    Ok(Some(run_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_shows_only_last_four() {
        assert_eq!(hint("abcdef123456"), "••••3456");
        assert_eq!(hint("ab"), "••••ab");
    }
}
