//! 版本号：按发布日期（上海时间）定，`YYYY.MM.DD`；同一天再发布时加补丁号 `YYYY.MM.DD.1`、`.2`……
//! git 标签前面带 `v`，镜像标签和程序里的版本号不带。发布步骤见 docs/operations/runbook.md#release_image。

/// 解析后的版本号：年、月、日、补丁号（没有时为 0）；比较大小就是比较发布先后
pub type Version = (u64, u64, u64, u64);

// @project-doc docs/decisions/calendar_versioning.md
/// 正式版本号：四位年、两位月、两位日，可选补丁号；其他写法（例如带 `-` 的预发布）返回 None
pub fn parse(v: &str) -> Option<Version> {
    let parts: Vec<&str> = v.split('.').collect();
    let widths_ok = matches!(parts.as_slice(), [y, m, d] | [y, m, d, _] if y.len() == 4 && m.len() == 2 && d.len() == 2);
    if !widths_ok || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let n: Vec<u64> = parts.iter().map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let date_ok = (1..=12).contains(&n[1]) && (1..=31).contains(&n[2]);
    date_ok.then(|| (n[0], n[1], n[2], n.get(3).copied().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_versions_and_patches_sort_by_release() {
        assert_eq!(parse("2026.10.18"), Some((2026, 10, 18, 0)));
        assert_eq!(parse("2026.10.18.2"), Some((2026, 10, 18, 2)));
        assert!(parse("2026.10.18") < parse("2026.10.18.1"));
        assert!(parse("2026.10.18.9") < parse("2026.10.18.10"));
        assert!(parse("2026.10.18.1") < parse("2026.10.19"));
        for bad in ["1.5.0", "2026.1.8", "2026.13.01", "2026.10", "2026.10.18.1.1", "2026.10.18-rc.1", "2026.10.18;id", "", "a.b.c", "+202.10.18"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
