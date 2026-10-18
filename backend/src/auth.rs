//! 用户与登录凭证：用户由管理员添加（不开放注册），密码用 Argon2id 哈希保存。
//!
//! 第一个管理员由迁移创建、还没有密码；`init_first_admin` 在启动时用配置里的账号密码设置它（只在它还没有密码时），
//! 之后账号密码只在网页上改。登录状态是签名 Cookie，签名覆盖用户编号和密码哈希：改密码后旧的登录状态都失效。
//! 角色和接口权限见 docs/interfaces/rest_api.md#auth。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::config::{DEFAULT_ADMIN_PASSWORD, Settings};

/// 新密码至少几个字符
pub const MIN_PASSWORD_LEN: usize = 8;
/// 系统生成的随机密码长度
const GENERATED_PASSWORD_LEN: usize = 12;
/// 同一用户名连续失败几次后暂停登录
const MAX_FAILURES: u32 = 5;
/// 暂停多久
const LOCK_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// 管理员：添加用户、数据同步、数据源设置、升级、修改策略和信号
    Admin,
    /// 普通用户：自己的自选股（以后还有账户），其余只能查看
    Member,
}

impl Role {
    pub fn key(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "admin" => Ok(Role::Admin),
            "member" => Ok(Role::Member),
            _ => anyhow::bail!("未知的角色：{s}"),
        }
    }
}

#[derive(Serialize, ToSchema, Clone, Debug)]
pub struct User {
    pub user_id: i64,
    pub username: String,
    pub role: Role,
    pub disabled: bool,
    /// 密码是管理员设置的或默认的，本人登录后页面提醒修改
    pub must_change_password: bool,
    pub created_at: DateTime<Utc>,
    #[serde(skip)]
    pub password_hash: String,
}

type UserRow = (i64, String, String, bool, bool, DateTime<Utc>, String);
const USER_COLS: &str =
    "user_id, username, role, disabled, must_change_password, created_at, password_hash";

impl TryFrom<UserRow> for User {
    type Error = anyhow::Error;
    fn try_from(
        (user_id, username, role, disabled, must_change_password, created_at, password_hash): UserRow,
    ) -> anyhow::Result<Self> {
        Ok(User {
            user_id,
            username,
            role: Role::parse(&role)?,
            disabled,
            must_change_password,
            created_at,
            password_hash,
        })
    }
}

/// 用户名：去掉首尾空白后 1 到 32 个字符，不能有空白和控制字符
pub fn validate_username(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 32 {
        return Err("用户名要 1 到 32 个字符".into());
    }
    if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("用户名不能有空格".into());
    }
    Ok(name.to_string())
}

pub fn validate_password(pw: &str) -> Result<(), String> {
    let n = pw.chars().count();
    if n < MIN_PASSWORD_LEN {
        return Err(format!("密码至少 {MIN_PASSWORD_LEN} 个字符"));
    }
    if n > 128 {
        return Err("密码最多 128 个字符".into());
    }
    Ok(())
}

/// 随机密码：去掉容易看错的字符（0 O 1 l I）
pub fn generate_password() -> anyhow::Result<String> {
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; GENERATED_PASSWORD_LEN];
    getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("取随机数失败：{e}"))?;
    // 256 不是字母表长度的整数倍，取模有极小的偏差，对 12 位密码可以忽略
    Ok(buf
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect())
}

/// Argon2id 哈希要几十毫秒的 CPU，放到阻塞线程上
pub async fn hash_password(pw: &str) -> anyhow::Result<String> {
    let pw = pw.to_string();
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(pw.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| anyhow::anyhow!("密码哈希失败：{e}"))
    })
    .await?
}

/// 密码是否正确。哈希为空（还没有设置密码）时总是不正确，但仍花同样的时间，不让人从耗时看出用户存不存在
pub async fn verify_password(hash: &str, pw: &str) -> bool {
    let (hash, pw) = (hash.to_string(), pw.to_string());
    tokio::task::spawn_blocking(move || {
        let real = PasswordHash::new(&hash).ok();
        let target = real.clone().unwrap_or_else(|| dummy_hash().clone());
        let ok = Argon2::default()
            .verify_password(pw.as_bytes(), &target)
            .is_ok();
        ok && real.is_some()
    })
    .await
    .unwrap_or(false)
}

fn dummy_hash() -> &'static PasswordHash {
    static H: std::sync::OnceLock<PasswordHash> = std::sync::OnceLock::new();
    H.get_or_init(|| {
        Argon2::default()
            .hash_password(b"niulai-dummy")
            .expect("固定输入的哈希不会失败")
    })
}

/// 还没有密码的第一个管理员：用配置里的账号密码设置。默认密码登录后页面提醒修改
pub async fn init_first_admin(pool: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    let pending: Option<i64> =
        sqlx::query_scalar("SELECT user_id FROM app.user WHERE role = 'admin' AND password_hash = '' ORDER BY user_id LIMIT 1").fetch_optional(pool).await?;
    let Some(user_id) = pending else {
        return Ok(());
    };
    let username = validate_username(&settings.admin_user)
        .map_err(|e| anyhow::anyhow!("NIULAI_ADMIN_USER 不合法：{e}"))?;
    let hash = hash_password(&settings.admin_password).await?;
    let is_default = settings.admin_password == DEFAULT_ADMIN_PASSWORD;
    sqlx::query("UPDATE app.user SET username = $2, password_hash = $3, must_change_password = $4, updated_at = now() WHERE user_id = $1 AND password_hash = ''")
        .bind(user_id)
        .bind(&username)
        .bind(&hash)
        .bind(is_default)
        .execute(pool)
        .await?;
    if is_default {
        tracing::warn!(
            username,
            "已创建第一个管理员，密码是默认的 niulai，请登录后立即修改"
        );
    } else {
        tracing::info!(username, "已用配置的账号密码创建第一个管理员");
    }
    Ok(())
}

pub async fn find(pool: &PgPool, user_id: i64) -> anyhow::Result<Option<User>> {
    // 审计：USER_COLS 是本文件中的常量列名
    let row: Option<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS} FROM app.user WHERE user_id = $1"
    )))
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    row.map(User::try_from).transpose()
}

pub async fn find_by_name(pool: &PgPool, username: &str) -> anyhow::Result<Option<User>> {
    // 审计：USER_COLS 是本文件中的常量列名
    let row: Option<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS} FROM app.user WHERE lower(username) = lower($1)"
    )))
    .bind(username.trim())
    .fetch_optional(pool)
    .await?;
    row.map(User::try_from).transpose()
}

pub async fn list(pool: &PgPool) -> anyhow::Result<Vec<User>> {
    // 审计：USER_COLS 是本文件中的常量列名
    let rows: Vec<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {USER_COLS} FROM app.user ORDER BY user_id"
    )))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(User::try_from).collect()
}

#[derive(Debug, thiserror::Error)]
pub enum UserError {
    #[error("{0}")]
    Invalid(String),
    #[error("用户名「{0}」已经有人用了")]
    Taken(String),
    #[error("用户不存在")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for UserError {
    fn from(e: sqlx::Error) -> Self {
        UserError::Internal(e.into())
    }
}

/// 添加用户。密码是管理员设置的，本人登录后提醒修改
pub async fn create(
    pool: &PgPool,
    username: &str,
    password: &str,
    role: Role,
) -> Result<User, UserError> {
    let username = validate_username(username).map_err(UserError::Invalid)?;
    validate_password(password).map_err(UserError::Invalid)?;
    let hash = hash_password(password).await?;
    // 审计：USER_COLS 是本文件中的常量列名
    let row: Option<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO app.user (username, password_hash, role, must_change_password) VALUES ($1, $2, $3, true)
         ON CONFLICT ((lower(username))) DO NOTHING RETURNING {USER_COLS}"
    )))
    .bind(&username)
    .bind(&hash)
    .bind(role.key())
    .fetch_optional(pool)
    .await?;
    row.map(User::try_from)
        .transpose()?
        .ok_or(UserError::Taken(username))
}

/// 设置密码。`by_self` 为 false 表示管理员重置，本人登录后提醒修改
pub async fn set_password(
    pool: &PgPool,
    user_id: i64,
    password: &str,
    by_self: bool,
) -> Result<(), UserError> {
    validate_password(password).map_err(UserError::Invalid)?;
    let hash = hash_password(password).await?;
    let n = sqlx::query("UPDATE app.user SET password_hash = $2, must_change_password = $3, updated_at = now() WHERE user_id = $1")
        .bind(user_id)
        .bind(&hash)
        .bind(!by_self)
        .execute(pool)
        .await?
        .rows_affected();
    if n == 0 {
        Err(UserError::NotFound)
    } else {
        Ok(())
    }
}

/// 修改角色或停用。至少要留一个没有停用的管理员，避免谁都进不了管理功能
pub async fn update(
    pool: &PgPool,
    user_id: i64,
    role: Role,
    disabled: bool,
) -> Result<User, UserError> {
    let mut tx = pool.begin().await?;
    // 锁住全部管理员，两个请求同时降级两个管理员时也能判断对
    let admins: Vec<(i64, bool)> =
        sqlx::query_as("SELECT user_id, disabled FROM app.user WHERE role = 'admin' FOR UPDATE")
            .fetch_all(&mut *tx)
            .await?;
    let others_active = admins.iter().any(|(id, off)| *id != user_id && !off);
    let stays_admin = role == Role::Admin && !disabled;
    if admins.iter().any(|(id, _)| *id == user_id) && !stays_admin && !others_active {
        return Err(UserError::Invalid("至少要留一个可以登录的管理员".into()));
    }
    // 审计：USER_COLS 是本文件中的常量列名
    let row: Option<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE app.user SET role = $2, disabled = $3, updated_at = now() WHERE user_id = $1 RETURNING {USER_COLS}"
    )))
    .bind(user_id)
    .bind(role.key())
    .bind(disabled)
    .fetch_optional(&mut *tx)
    .await?;
    let user = row
        .map(User::try_from)
        .transpose()?
        .ok_or(UserError::NotFound)?;
    tx.commit().await?;
    Ok(user)
}

/// 登录状态的签名：用户编号、过期时间、密码哈希。密码哈希含随机盐，改密码（哪怕改成一样的）后旧签名都对不上
pub fn sign_session(secret: &[u8], user: &User, expires: i64) -> String {
    format!(
        "{}.{expires}.{}",
        user.user_id,
        to_hex(
            &session_mac(secret, user.user_id, &user.password_hash, expires)
                .finalize()
                .into_bytes()
        )
    )
}

/// 解析 Cookie 中的登录状态，返回用户编号和过期时间；签名要拿到用户之后用 `session_valid` 核对
pub fn parse_session(token: &str) -> Option<(i64, i64, Vec<u8>)> {
    let mut parts = token.splitn(3, '.');
    let user_id = parts.next()?.parse().ok()?;
    let expires = parts.next()?.parse().ok()?;
    let sig = from_hex(parts.next()?)?;
    Some((user_id, expires, sig))
}

pub fn session_valid(secret: &[u8], user: &User, expires: i64, sig: &[u8]) -> bool {
    !user.disabled
        && !user.password_hash.is_empty()
        && expires > Utc::now().timestamp()
        && session_mac(secret, user.user_id, &user.password_hash, expires)
            .verify_slice(sig)
            .is_ok()
}

fn session_mac(secret: &[u8], user_id: i64, password_hash: &str, expires: i64) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC 接受任意长度的密钥");
    m.update(format!("session\n{user_id}\n{password_hash}\n{expires}").as_bytes());
    m
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// 登录失败计数：同一用户名连续失败 `MAX_FAILURES` 次后暂停 `LOCK_FOR`，放慢猜密码。只在进程内（只有一个 serve 进程）
#[derive(Default)]
pub struct LoginGuard {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

impl LoginGuard {
    /// 还要等多久才能再试；None 表示现在可以登录
    pub fn locked_for(&self, username: &str) -> Option<Duration> {
        let map = self.failures.lock().expect("登录计数的锁不会中毒");
        let (n, last) = map.get(&username.trim().to_lowercase())?;
        let left = LOCK_FOR.checked_sub(last.elapsed())?;
        (*n >= MAX_FAILURES).then_some(left)
    }

    pub fn failed(&self, username: &str) {
        let mut map = self.failures.lock().expect("登录计数的锁不会中毒");
        // 用户名由请求方决定：过期的记录随手清掉，不让表无限变大
        map.retain(|_, (_, last)| last.elapsed() < LOCK_FOR);
        let e = map
            .entry(username.trim().to_lowercase())
            .or_insert((0, Instant::now()));
        *e = (e.0 + 1, Instant::now());
    }

    pub fn succeeded(&self, username: &str) {
        self.failures
            .lock()
            .expect("登录计数的锁不会中毒")
            .remove(&username.trim().to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: i64, hash: &str) -> User {
        User {
            user_id: id,
            username: "a".into(),
            role: Role::Member,
            disabled: false,
            must_change_password: false,
            created_at: Utc::now(),
            password_hash: hash.into(),
        }
    }

    #[test]
    fn session_depends_on_user_password_and_expiry() {
        let k = b"secret";
        let exp = Utc::now().timestamp() + 60;
        let u = user(7, "$argon2id$v=19$hash-a");
        let token = sign_session(k, &u, exp);
        let (id, e, sig) = parse_session(&token).unwrap();
        assert_eq!((id, e), (7, exp));
        assert!(session_valid(k, &u, e, &sig));
        assert!(
            !session_valid(k, &user(7, "$argon2id$v=19$hash-b"), e, &sig),
            "改了密码，旧的登录状态失效"
        );
        assert!(
            !session_valid(k, &user(8, "$argon2id$v=19$hash-a"), e, &sig),
            "换个用户编号不行"
        );
        assert!(!session_valid(k, &u, e + 1, &sig));
        assert!(
            !session_valid(
                k,
                &User {
                    disabled: true,
                    ..u.clone()
                },
                e,
                &sig
            ),
            "停用后不能登录"
        );
        assert!(!session_valid(
            k,
            &u,
            Utc::now().timestamp() - 1,
            &sign_session(k, &u, Utc::now().timestamp() - 1)
                .rsplit('.')
                .next()
                .and_then(from_hex)
                .unwrap()
        ));
        assert!(
            parse_session("7.abc.00").is_none()
                && parse_session("x").is_none()
                && parse_session("7.1.zz").is_none()
        );
    }

    #[tokio::test]
    async fn passwords_hash_and_verify() {
        let h = hash_password("correct horse").await.unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password(&h, "correct horse").await);
        assert!(!verify_password(&h, "wrong").await);
        assert!(
            !verify_password("", "").await,
            "还没有设置密码的用户不能登录"
        );
    }

    #[test]
    fn validates_names_passwords_and_generates() {
        assert_eq!(validate_username("  妈妈 "), Ok("妈妈".into()));
        assert!(
            validate_username("").is_err()
                && validate_username("a b").is_err()
                && validate_username(&"x".repeat(33)).is_err()
        );
        assert!(validate_password("1234567").is_err() && validate_password("12345678").is_ok());
        let p = generate_password().unwrap();
        assert_eq!(p.len(), GENERATED_PASSWORD_LEN);
        assert!(validate_password(&p).is_ok() && !p.contains(['0', 'O', '1', 'l', 'I']));
    }

    #[test]
    fn locks_after_repeated_failures() {
        let g = LoginGuard::default();
        for _ in 0..MAX_FAILURES - 1 {
            g.failed("Mama");
        }
        assert!(g.locked_for("mama").is_none());
        g.failed("mama ");
        assert!(g.locked_for("MAMA").is_some(), "用户名不区分大小写");
        assert!(g.locked_for("other").is_none());
        g.succeeded("mama");
        assert!(g.locked_for("mama").is_none());
    }
}
