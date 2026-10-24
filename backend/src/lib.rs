//! niulai：A 股数据工作台后端。
//!
//! - `sync`：通用同步框架。读取 `config/datasets.yaml`，规划任务、执行请求、写入 PostgreSQL。
//! - `vendor`：同花顺金融数据服务（fuyao）客户端与全局限速。
//! - `analysis`：证券信息、价格读取、市场环境和因子研究。
//! - `account`、`advice`：账户、持仓、流水和按因子排名的每日调仓建议。
//! - `api`：对外的 REST API（Axum + utoipa）。
//! - `server`：常驻服务，在一个进程中运行 API、任务执行、定时调度、应用事件、建议重算和页面预热。
//! - `bootstrap`：应用初始化和同步领域事件的编排；`precompute`：首页结果预热。
//! - `setup`：首次登录引导，在网页上配置 API Key。
//! - `upgrader`：Docker 部署的一键升级助手（`niulai upgrader`）。

pub mod account;
pub mod advice;
pub mod analysis;
pub mod api;
pub mod auth;
pub mod bootstrap;
pub mod cache;
pub mod config;
pub mod db;
pub mod error;
pub mod instrument;
pub mod operation_log;
pub mod precompute;
pub(crate) mod readiness;
pub(crate) mod scheduling;
pub mod server;
pub mod setup;
pub mod sync;
pub mod telemetry;
pub mod time;
pub mod upgrader;
pub mod vendor;
pub mod version;

/// 架构适应度函数：把模块注释里声明的依赖方向和 AGENTS.md 中的 SQL 审计约定变成测试，违反时构建失败。
/// 规则见 docs/project_overview.md#dependency_rules。
#[cfg(test)]
mod architecture {
    use std::path::{Path, PathBuf};

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        if dir.is_file() {
            out.push(dir.to_path_buf());
            return;
        }
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    fn src() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// 去掉注释后的代码行：注释里提到别的模块（例如「预热在 `crate::precompute`」）不算依赖
    fn code_lines(path: &Path) -> Vec<(usize, String)> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l.split("//").next().unwrap_or("").to_string()))
            .collect()
    }

    #[test]
    fn modules_only_depend_downward() {
        // (模块, 不允许引用的模块)：同步层只发布事实，分析不知道账户，账户不知道建议，业务层不依赖接口和进程编排
        let business: &[&str] = &[
            "auth",
            "sync",
            "analysis",
            "account",
            "advice",
            "api",
            "bootstrap",
            "server",
        ];
        let rules: [(&str, &[&str]); 11] = [
            // 基础模块不依赖任何业务模块
            ("error.rs", business),
            ("cache.rs", business),
            ("instrument.rs", business),
            ("readiness.rs", business),
            ("scheduling.rs", business),
            ("time.rs", business),
            ("config.rs", business),
            (
                "sync",
                &[
                    "analysis",
                    "account",
                    "advice",
                    "api",
                    "bootstrap",
                    "server",
                    "cache",
                    "precompute",
                ],
            ),
            (
                "analysis",
                &[
                    "sync",
                    "account",
                    "advice",
                    "api",
                    "bootstrap",
                    "server",
                    "precompute",
                ],
            ),
            ("account", &["advice", "sync", "api", "bootstrap", "server"]),
            (
                "advice",
                &["sync", "api", "bootstrap", "server", "precompute"],
            ),
        ];
        let mut violations = vec![];
        for (module, banned) in rules {
            let mut files = vec![];
            rust_files(&src().join(module), &mut files);
            for file in files {
                for (n, line) in code_lines(&file) {
                    for b in banned {
                        if line.contains(&format!("crate::{b}::"))
                            || line.contains(&format!("crate::{b};"))
                        {
                            violations.push(format!("{}:{n} 引用了 crate::{b}", file.display()));
                        }
                    }
                }
            }
        }
        assert!(
            violations.is_empty(),
            "依赖方向不对：\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn sync_state_tables_are_read_only_through_sync_and_readiness() {
        // 运行、任务和数据集注册表属于同步层：其他模块通过 sync 的函数或 readiness 读取，
        // 不各自写 SQL。api/sync.rs 是同步管理页的读取接口，属于同步的一部分
        let allowed = ["sync", "readiness.rs", "api/sync.rs", "bin", "lib.rs"];
        let mut files = vec![];
        rust_files(&src(), &mut files);
        let mut violations = vec![];
        for file in files {
            let rel = file
                .strip_prefix(src())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if allowed
                .iter()
                .any(|a| rel == *a || rel.starts_with(&format!("{a}/")))
            {
                continue;
            }
            for (n, line) in code_lines(&file) {
                for table in ["ops.task", "ops.run ", "ops.dataset"] {
                    if line.contains(table) {
                        violations.push(format!("{rel}:{n} 直接查询 {}", table.trim()));
                    }
                }
            }
        }
        assert!(
            violations.is_empty(),
            "同步状态表只由同步层和 readiness 读取：\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn dynamic_sql_has_an_audit_comment() {
        let mut files = vec![];
        rust_files(&src(), &mut files);
        let mut missing = vec![];
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                // 跳过字符串里的这个词（本测试自己）
                let wrapped = code.contains("AssertSqlSafe(") && !code.contains("\"AssertSqlSafe(");
                if wrapped && i > 0 && !lines[i - 1].contains("审计：") {
                    missing.push(format!("{}:{}", file.display(), i + 1));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "动态拼接的 SQL 上一行要用「审计：」说明为什么安全：\n{}",
            missing.join("\n")
        );
    }
}
