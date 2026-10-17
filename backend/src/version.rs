//! 版本号：按发布日期（上海时间）定，`YYYY.MM.DD`；同一天再发布时加补丁号 `YYYY.MM.DD.1`、`.2`……
//! git 标签前面带 `v`，镜像标签和程序里的版本号不带。发布步骤见 docs/operations/runbook.md#release_image。
//!
//! 以前的版本是 `X.Y.Z`（最后一个是 1.5.0），也按这里的规则解析，按数字比较时都小于日期版本。
//! 1.5.0 及更早的程序只认三段数字，所以每天第一个版本不带补丁号，老版本也能检测到并一键升级。

/// 解析后的版本号：年、月、日、补丁号（没有时为 0）；比较大小就是比较发布先后
pub type Version = (u64, u64, u64, u64);

// @project-doc docs/decisions/calendar_versioning.md
/// 正式版本号：三段或四段数字；带 `-` 的预发布和其他写法返回 None
pub fn parse(v: &str) -> Option<Version> {
    let parts: Vec<&str> = v.split('.').collect();
    if !(3..=4).contains(&parts.len()) || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let n: Vec<u64> = parts.iter().map(|p| p.parse().ok()).collect::<Option<_>>()?;
    Some((n[0], n[1], n[2], n.get(3).copied().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_versions_sort_after_old_ones_and_patches_after_the_day() {
        assert_eq!(parse("2026.10.18"), Some((2026, 10, 18, 0)));
        assert_eq!(parse("2026.10.18.2"), Some((2026, 10, 18, 2)));
        assert!(parse("1.5.0") < parse("2026.10.18"));
        assert!(parse("2026.10.18") < parse("2026.10.18.1"));
        assert!(parse("2026.10.18.9") < parse("2026.10.18.10"));
        assert!(parse("2026.10.18.1") < parse("2026.10.19"));
        for bad in ["2026.10", "2026.10.18.1.1", "2026.10.18-rc.1", "2026.10.18;id", "", "a.b.c", "2026..18", "+1.2.3"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
