//! 内置数据库的备份：每天自动一次，升级前一次。都写在部署目录的 backups/，规则见 docs/operations/runbook.md#backup_restore。
//!
//! - 在 postgres 容器里先 `pg_dump -Fc` 到临时文件，再用 `pg_restore --list` 读一遍，读得通才传出来，
//!   写到 `.tmp` 文件、检查文件头后改名：backups/ 里只会有完整、读得出来的备份。
//! - 每日备份在上海时间 BACKUP_AT 之后做（服务那时没在运行就在启动后补做）。不分每日和升级前，backups/ 里只留最新的一份：
//!   新的一份校验通过、写好之后才删旧的，避免备份把磁盘占满。
//! - 结果写进共享目录的 `backup`，niulai 在版本号弹窗里显示最近一次备份，太久没有成功时提醒管理员。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::bail;
use chrono::{NaiveDateTime, NaiveTime};
use serde_json::json;

use super::docker::Docker;

/// 每天几点（上海时间）之后做当天的备份：收盘同步和建议都做完了，夜里也没人用
const BACKUP_AT: (u32, u32) = (3, 30);
pub const DAILY_PREFIX: &str = "niulai-daily-";
pub const UPGRADE_PREFIX: &str = "niulai-before-";
/// 失败后隔多久再试
const RETRY: Duration = Duration::from_secs(3600);

/// 在 postgres 容器中导出并校验，传到 file，返回字节数
pub async fn dump(docker: &Docker, pg_id: &str, file: &Path) -> anyhow::Result<u64> {
    let tmp = file.with_extension("tmp");
    let script = "set -e; f=$(mktemp); trap 'rm -f \"$f\"' EXIT; pg_dump -U niulai -Fc niulai > \"$f\"; pg_restore --list \"$f\" > /dev/null; cat \"$f\"";
    let (code, stderr) = match docker
        .exec_to_file(pg_id, &["sh", "-c", script], &tmp)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if code != 0 {
        let _ = std::fs::remove_file(&tmp);
        bail!("导出或校验失败（退出码 {code}）：{}", stderr.trim());
    }
    // pg_dump 自定义格式的文件以 PGDMP 开头
    let head = std::fs::read(&tmp)
        .ok()
        .map(|b| b.starts_with(b"PGDMP"))
        .unwrap_or(false);
    if !head {
        let _ = std::fs::remove_file(&tmp);
        bail!("备份文件不完整");
    }
    std::fs::rename(&tmp, file)?;
    Ok(std::fs::metadata(file)?.len())
}

/// 只留最新的一份备份，每日和升级前的一起算（按修改时间：升级前备份的文件名里有版本号，按名字排不准）。
/// 在新备份写好之后调用，所以删掉的都是更旧的
pub fn keep_latest(dir: &Path) {
    let mut files = all(dir);
    files.sort_by_key(|p| {
        (
            std::fs::metadata(p).and_then(|m| m.modified()).ok(),
            p.clone(),
        )
    });
    for f in &files[..files.len().saturating_sub(1)] {
        let _ = std::fs::remove_file(f);
    }
}

/// 两类备份的全部文件
fn all(dir: &Path) -> Vec<PathBuf> {
    let mut files = list(dir, DAILY_PREFIX);
    files.extend(list(dir, UPGRADE_PREFIX));
    files
}

fn list(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    rd.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".dump"))
        })
        .collect()
}

/// 文件名末尾的时间：niulai-daily-20261018-033000.dump、niulai-before-2026.10.18-20261018-100000.dump
/// → 2026-10-18 03:30:00、2026-10-18 10:00:00
fn time_of(path: &Path) -> Option<NaiveDateTime> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".dump")?;
    let stamp = stem.get(stem.len().checked_sub(15)?..)?;
    NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok()
}

/// 最近一份备份（每日或升级前）的时间（上海时间）。升级前刚备份过，当天就不用再做每日备份
pub fn latest(dir: &Path) -> Option<NaiveDateTime> {
    newest(dir).map(|(_, t)| t)
}

fn newest(dir: &Path) -> Option<(PathBuf, NaiveDateTime)> {
    all(dir)
        .into_iter()
        .filter_map(|p| time_of(&p).map(|t| (p, t)))
        .max_by_key(|(_, t)| *t)
}

/// 现在要不要做每日备份：今天到点了还没做，或者最近一份已经超过 30 小时（服务停过、到点时在升级等）
pub fn due(now: NaiveDateTime, latest: Option<NaiveDateTime>) -> bool {
    let at = NaiveTime::from_hms_opt(BACKUP_AT.0, BACKUP_AT.1, 0).expect("合法的时间");
    match latest {
        None => true,
        Some(l) => {
            (now.time() >= at && l.date() < now.date()) || now - l > chrono::Duration::hours(30)
        }
    }
}

/// 每日备份的调度：记住上次失败的时间，失败后一小时再试
#[derive(Default)]
pub struct Daily {
    failed_at: Option<Instant>,
}

impl Daily {
    /// 到点时备份；status 是共享目录中的状态文件
    pub async fn tick(&mut self, docker: &Docker, pg_id: &str, dir: &Path, status: &Path) {
        if self.failed_at.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        let now = crate::time::now_cn();
        if !due(now, latest(dir)) {
            return;
        }
        let file = dir.join(format!(
            "{DAILY_PREFIX}{}.dump",
            now.format("%Y%m%d-%H%M%S")
        ));
        let started = Instant::now();
        let res = async {
            std::fs::create_dir_all(dir)?;
            dump(docker, pg_id, &file).await
        }
        .await;
        match res {
            Ok(size) => {
                self.failed_at = None;
                keep_latest(dir);
                tracing::info!(file = %file.display(), size, seconds = started.elapsed().as_secs(), "每日备份完成");
                write_status(dir, status, None);
            }
            Err(e) => {
                self.failed_at = Some(Instant::now());
                tracing::error!(error = %format!("{e:#}"), "每日备份失败，一小时后再试");
                write_status(dir, status, Some(&format!("{e:#}")));
            }
        }
    }
}

/// 状态文件：最近一份备份（文件名、大小、时间）和一共几份，以及最近一次失败的原因（成功后清空）。
/// niulai 看不到部署目录，只能读它（见 api::system）
pub fn write_status(dir: &Path, status: &Path, error: Option<&str>) {
    let latest = newest(dir).map(|(p, t)| {
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        json!({ "file": format!("backups/{name}"), "size": size, "at": crate::time::cn_to_utc(t).timestamp() })
    });
    let body = json!({ "latest": latest, "count": all(dir).len(), "error": error, "checked_at": chrono::Utc::now().timestamp() });
    let tmp = status.with_extension("tmp");
    let _ = std::fs::write(&tmp, body.to_string()).and_then(|_| std::fs::rename(&tmp, status));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn backs_up_once_a_day_after_the_hour() {
        assert!(due(t("2026-10-18 01:00"), None), "从没备份过：马上做");
        assert!(
            !due(t("2026-10-18 01:00"), Some(t("2026-10-17 03:30"))),
            "还没到点"
        );
        assert!(
            due(t("2026-10-18 03:31"), Some(t("2026-10-17 03:30"))),
            "到点了，今天还没做"
        );
        assert!(
            !due(t("2026-10-18 23:00"), Some(t("2026-10-18 03:31"))),
            "今天做过了"
        );
        assert!(
            due(t("2026-10-18 02:00"), Some(t("2026-10-16 18:00"))),
            "太久没做（停过机）：不等到点"
        );
    }

    #[test]
    fn keeps_only_the_newest_backup_of_either_kind() {
        let dir = std::env::temp_dir().join(format!("niulai-backup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        let write = |name: String, secs: u64| {
            let path = dir.join(name);
            std::fs::write(&path, "x").unwrap();
            let f = std::fs::File::options().write(true).open(&path).unwrap();
            f.set_modified(base + Duration::from_secs(secs)).unwrap();
        };
        for d in 10..=20 {
            write(format!("{DAILY_PREFIX}202610{d}-033000.dump"), d);
        }
        // 升级前的备份比最新的每日备份还新：它留下，每日的全删
        write(
            format!("{UPGRADE_PREFIX}2026.10.20-20261020-100000.dump"),
            30,
        );
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        keep_latest(&dir);
        assert_eq!(list(&dir, DAILY_PREFIX).len(), 0);
        assert_eq!(list(&dir, UPGRADE_PREFIX).len(), 1);
        assert!(dir.join("notes.txt").exists(), "不是备份的文件不动");
        assert_eq!(latest(&dir), Some(t("2026-10-20 10:00")));
        // 第二天的每日备份写好后，升级前的那份也删掉
        write(format!("{DAILY_PREFIX}20261021-033000.dump"), 40);
        keep_latest(&dir);
        assert_eq!(all(&dir).len(), 1);
        assert_eq!(latest(&dir), Some(t("2026-10-21 03:30")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
