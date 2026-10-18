//! 部署迁移：新版本对部署方式（compose.yml、.env、容器的挂载）有不兼容的改动时，由新版本的升级助手在启动新版本之前做完。
//! 升级助手只改 niulai 的镜像，服务器上的 compose.yml 和 .env 不会跟着仓库变，所以这类改动要写成迁移。
//!
//! 每个迁移只做一次，结果记在部署目录的 `.niulai-migrations`（一行一个：编号、结果、时间、说明）。
//! 迁移先检查前提，不满足就跳过并说明原因，不冒险；做到一半失败时恢复原样。规则见 docs/operations/runbook.md#deploy_migrations

use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};

use super::{Progress, STOP_TIMEOUT_SECS, Upgrade, builtin_db, create_body_for, set_env_line, write_in_place};

/// 已经做过的部署迁移
const STATE_FILE: &str = ".niulai-migrations";

/// 部署迁移，按顺序执行。
///
/// 只做「改部署文件或挂载、能安全回退、改之前能认出用户没有动过」的迁移，例如数据目录。
/// 资源上限和容器的安全选项不在这里做：升级助手重建容器时沿用原容器的启动参数（见 `create_body`），
/// 只改 compose.yml 不会让运行中的容器生效，要用户执行 `docker compose up -d` 重建；内存上限还取决于服务器大小，
/// 猜错会让数据库被强制杀掉。手动步骤见 docs/operations/runbook.md#resource_limits。
const MIGRATIONS: &[(&str, &str)] = &[(DATA_DIR, "数据库数据从 Docker 卷移到部署目录 data/postgres")];

const DATA_DIR: &str = "2026-10-17-postgres-data-dir";
/// 数据库容器里数据目录的挂载点（postgres 18 镜像）
const PG_TARGET: &str = "/var/lib/postgresql";
/// 改动前 compose.yml 中数据库挂载的写法，和改动后的写法
const OLD_COMPOSE_LINE: &str = "volumes: [pgdata:/var/lib/postgresql]";
const NEW_COMPOSE_LINE: &str = "volumes: [\"${POSTGRES_DATA:-./data/postgres}:/var/lib/postgresql\"]";

/// 一个迁移的结果
enum Outcome {
    Done(String),
    /// 不需要或不能自动做，附原因
    Skipped(String),
}

impl Upgrade<'_> {
    fn applied(&self) -> Vec<String> {
        std::fs::read_to_string(self.deploy.join(STATE_FILE))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_whitespace().next().map(str::to_string))
            .collect()
    }

    fn record(&self, id: &str, result: &str, note: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.deploy.join(STATE_FILE)) {
            let _ = writeln!(f, "{id} {result} {} {}", crate::time::now_cn().format("%Y-%m-%dT%H:%M:%S"), note.replace('\n', " "));
        }
    }

    pub(super) fn migrations_pending(&self) -> bool {
        let applied = self.applied();
        MIGRATIONS.iter().any(|(id, _)| !applied.iter().any(|a| a == id))
    }

    /// 做完还没做过的部署迁移，进度写在 migrate 这一步。backed_up：这次升级已经备份过数据库
    pub(super) async fn run_deploy_migrations(&self, p: &Progress<'_>, backed_up: bool) -> anyhow::Result<()> {
        let applied = self.applied();
        let todo: Vec<_> = MIGRATIONS.iter().filter(|(id, _)| !applied.iter().any(|a| a == id)).collect();
        if todo.is_empty() {
            p.skip("migrate", "没有需要做的");
            return Ok(());
        }
        let mut notes = vec![];
        for (id, title) in todo {
            p.running("migrate", title);
            let outcome = match *id {
                DATA_DIR => self.migrate_data_dir(p, title, backed_up).await,
                _ => unreachable!("每个迁移都有实现"),
            };
            match outcome {
                Ok(Outcome::Done(note)) => {
                    self.record(id, "done", &note);
                    notes.push(format!("{title}：{note}"));
                }
                Ok(Outcome::Skipped(why)) => {
                    self.record(id, "skipped", &why);
                    notes.push(format!("{title}：跳过，{why}"));
                }
                Err(e) => {
                    // 失败的不记下来，下次升级时再试；已经恢复原样
                    return Err(e.context(format!("{title}失败")));
                }
            }
        }
        p.done("migrate", &notes.join("；"));
        Ok(())
    }

    // @project-doc docs/operations/runbook.md#deploy_migrations
    /// 数据库数据从 Docker 卷 pgdata 移到部署目录下的 data/postgres：删除容器、docker compose down -v 都不会再丢数据
    async fn migrate_data_dir(&self, p: &Progress<'_>, title: &str, backed_up: bool) -> anyhow::Result<Outcome> {
        let step = |detail: &str| p.running("migrate", &format!("{title}：{detail}"));
        let niulai = self.find("niulai").await?;
        if !builtin_db(&niulai) {
            return Ok(Outcome::Skipped("用的不是内置数据库".into()));
        }
        let pg = self.find("postgres").await?;
        let Some(mount) = pg["Mounts"].as_array().into_iter().flatten().find(|m| m["Destination"] == PG_TARGET) else {
            return Ok(Outcome::Skipped("数据库容器没有挂载数据目录".into()));
        };
        if mount["Type"] != "volume" {
            return Ok(Outcome::Skipped("数据已经在宿主机目录中".into()));
        }
        let volume = mount["Name"].as_str().context("数据卷没有名字")?.to_string();
        let compose_path = self.deploy.join("compose.yml");
        let compose = std::fs::read_to_string(&compose_path).context("读不到 compose.yml")?;
        let Some(new_compose) = rewrite_compose(&compose) else {
            return Ok(Outcome::Skipped(format!("compose.yml 中数据库的挂载改过（找不到「{OLD_COMPOSE_LINE}」），请按运维手册手动迁移")));
        };
        let host_deploy = host_deploy_dir(self.me).context("找不到部署目录在宿主机上的路径")?;
        let host_data = format!("{}/data/postgres", host_deploy.trim_end_matches('/'));
        let local_data = self.deploy.join("data/postgres");
        if local_data.read_dir().is_ok_and(|mut d| d.next().is_some()) {
            return Ok(Outcome::Skipped("部署目录的 data/postgres 已经有文件，没有覆盖".into()));
        }

        if !backed_up {
            step("备份数据库");
            self.backup("data-dir").await.context("备份数据库失败，没有迁移")?;
        }
        step("停止服务");
        let niulai_id = niulai["Id"].as_str().unwrap_or_default().to_string();
        let pg_id = pg["Id"].as_str().unwrap_or_default().to_string();
        self.docker.stop(&niulai_id, STOP_TIMEOUT_SECS).await?;
        self.docker.stop(&pg_id, 60).await?;

        step("复制数据（几 GB 要一两分钟）");
        let image = pg["Config"]["Image"].as_str().unwrap_or("postgres:18.6").to_string();
        if let Err(e) = self.copy_volume(&image, &volume, &host_data).await {
            let _ = std::fs::remove_dir_all(&local_data);
            self.restart_both(&pg_id, &niulai_id).await;
            return Err(e);
        }

        step("改 compose.yml 和 .env");
        let env_path = self.deploy.join(".env");
        let env_before = std::fs::read_to_string(&env_path).ok();
        write_in_place(&compose_path, &new_compose)?;
        if let Some(text) = &env_before {
            write_in_place(&env_path, &set_env_line(text, "POSTGRES_DATA", "./data/postgres"))?;
        }

        step("用新的数据目录启动数据库");
        let body = with_data_bind(create_body_for(self.docker, &pg, &image).await?, &host_data);
        if let Err(e) = self.recreate_with(&pg, body, true, &|_| {}).await {
            // recreate_with 已经换回原来的数据库容器（还用 Docker 卷）；配置改回去，复制出来的目录留着排查
            let _ = write_in_place(&compose_path, &compose);
            if let Some(text) = &env_before {
                let _ = write_in_place(&env_path, text);
            }
            let failed = self.deploy.join(format!("data/postgres.failed-{}", crate::time::now_cn().format("%Y%m%d%H%M%S")));
            let _ = std::fs::rename(&local_data, failed);
            let _ = self.docker.start(&niulai_id).await;
            return Err(e.context("数据库用新的数据目录没有启动，已换回原来的 Docker 卷"));
        }
        step("启动 niulai");
        self.docker.start(&niulai_id).await?;
        Ok(Outcome::Done(format!("原来的 Docker 卷 {volume} 保留未删，确认数据没问题后可以 docker volume rm {volume}")))
    }

    async fn restart_both(&self, pg: &str, niulai: &str) {
        let _ = self.docker.start(pg).await;
        // 等数据库起来再启动 niulai，否则它连不上会退出
        tokio::time::sleep(Duration::from_secs(5)).await;
        let _ = self.docker.start(niulai).await;
    }

    /// 用数据库镜像起一个临时容器，把卷里的文件原样（含属主和权限）复制到宿主机目录，再核对文件数和总大小
    async fn copy_volume(&self, image: &str, volume: &str, host_dir: &str) -> anyhow::Result<()> {
        // 先看空间够不够：要放下一整份数据，再留 1 GB 余量；不够时什么都不改
        let script = "set -e; need=$(du -sk /from | cut -f1); free=$(df -Pk /to | awk 'NR==2 {print $4}'); \
            [ \"$free\" -gt $((need + 1048576)) ] || { echo \"部署目录所在的磁盘空间不够：数据约 $((need / 1024)) MB，剩余 $((free / 1024)) MB，至少要多留 1 GB\" >&2; exit 5; }; \
            cp -a /from/. /to/; \
            a=$(cd /from && find . | wc -l); b=$(cd /to && find . | wc -l); \
            [ \"$a\" = \"$b\" ] || { echo \"文件数不一致：$a 和 $b\" >&2; exit 3; }; \
            sa=$(du -s --apparent-size -B1 /from | cut -f1); sb=$(du -s --apparent-size -B1 /to | cut -f1); \
            [ \"$sa\" = \"$sb\" ] || { echo \"总大小不一致：$sa 和 $sb\" >&2; exit 4; }; \
            echo \"$a 个文件，$sa 字节\"";
        let body = json!({
            "Image": image,
            "User": "root",
            "Entrypoint": ["sh", "-c"],
            "Cmd": [script],
            "HostConfig": { "Binds": [format!("{volume}:/from:ro"), format!("{host_dir}:/to")] },
        });
        let id = self.docker.create(&format!("{}-migrate-data", self.project), &body).await?;
        let result = async {
            self.docker.start(&id).await?;
            let code = self.docker.wait(&id).await?;
            let (out, err) = self.docker.logs(&id).await.unwrap_or_default();
            if code != 0 {
                bail!("复制数据失败（退出码 {code}）：{}", err.trim());
            }
            self.files.log(&format!("已复制数据：{}", out.trim()));
            Ok(())
        }
        .await;
        let _ = self.docker.remove(&id).await;
        result
    }
}

/// 部署目录（upgrader 里的 /deploy）在宿主机上的路径
fn host_deploy_dir(me: &Value) -> Option<String> {
    me["Mounts"].as_array()?.iter().find(|m| m["Destination"] == "/deploy")?["Source"].as_str().map(str::to_string)
}

/// compose.yml 中数据库的挂载换成可配置的数据目录；找不到原来的写法（改过）时返回 None
pub fn rewrite_compose(text: &str) -> Option<String> {
    let mut found = false;
    let out: Vec<String> = text
        .lines()
        .map(|l| {
            if !found && l.trim() == OLD_COMPOSE_LINE {
                found = true;
                let indent = &l[..l.len() - l.trim_start().len()];
                format!("{indent}{NEW_COMPOSE_LINE}")
            } else {
                l.to_string()
            }
        })
        .collect();
    found.then(|| out.join("\n") + if text.ends_with('\n') { "\n" } else { "" })
}

/// 新数据库容器的挂载：去掉原来挂在数据目录上的卷，换成宿主机目录
pub fn with_data_bind(mut body: Value, host_dir: &str) -> Value {
    let hc = &mut body["HostConfig"];
    if let Some(m) = hc["Mounts"].as_array_mut() {
        m.retain(|m| m["Target"] != PG_TARGET);
    }
    let mut binds: Vec<Value> = hc["Binds"].as_array().cloned().unwrap_or_default();
    binds.retain(|b| b.as_str().is_none_or(|b| b.split(':').nth(1) != Some(PG_TARGET)));
    binds.push(json!(format!("{host_dir}:{PG_TARGET}")));
    hc["Binds"] = json!(binds);
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真的用 Docker 做一遍迁移：按改动前的写法起一个 Compose 项目（数据在卷里），写入数据，迁移，
    /// 检查数据还在、compose.yml 和 .env 改好了、之后 docker compose up -d 仍然用新目录。
    /// 需要本机有 Docker：cargo test --lib -- --ignored real_docker_data_dir_migration
    #[tokio::test]
    #[ignore]
    async fn real_docker_data_dir_migration() {
        use std::process::Command;
        let project = format!("niulaimig{}", std::process::id());
        let dir = std::fs::canonicalize(std::env::temp_dir()).unwrap().join(&project);
        std::fs::create_dir_all(&dir).unwrap();
        let compose = "services:\n  postgres:\n    image: postgres:18.6\n    environment: {POSTGRES_USER: niulai, POSTGRES_DB: niulai, POSTGRES_PASSWORD: niulai}\n    volumes: [pgdata:/var/lib/postgresql]\n    healthcheck: {test: [CMD, pg_isready, -U, niulai, -d, niulai], interval: 2s, retries: 30}\n  niulai:\n    image: postgres:18.6\n    init: true\n    entrypoint: [sleep, infinity]\n    env_file: .env\n    depends_on: {postgres: {condition: service_healthy}}\nvolumes:\n  pgdata:\n";
        std::fs::write(dir.join("compose.yml"), compose).unwrap();
        std::fs::write(dir.join(".env"), "DATABASE_URL=postgres://niulai:niulai@postgres:5432/niulai\n").unwrap();
        let dc = |args: &[&str]| {
            let out = Command::new("docker").args(["compose", "-p", &project]).args(args).current_dir(&dir).output().unwrap();
            assert!(out.status.success(), "docker compose {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        let psql = |sql: &str| dc(&["exec", "-T", "postgres", "psql", "-U", "niulai", "-d", "niulai", "-Atc", sql]);
        dc(&["up", "-d", "--wait"]);
        psql("CREATE TABLE t (v text); INSERT INTO t VALUES ('还在')");

        let socket = std::env::var("DOCKER_SOCK").unwrap_or_else(|_| "/var/run/docker.sock".into());
        let docker = super::super::docker::Docker::new(std::path::Path::new(&socket)).unwrap();
        let upgrade_dir = dir.join("upgrade");
        std::fs::create_dir_all(&upgrade_dir).unwrap();
        let files = super::super::Files { dir: upgrade_dir };
        let me = json!({ "Mounts": [{ "Destination": "/deploy", "Source": dir.to_string_lossy() }] });
        let up = Upgrade { docker: &docker, files: &files, deploy: &dir, project: &project, me: &me };
        assert!(up.migrations_pending());
        let p = Progress::new(&files, "", &[("migrate", "部署迁移")]);
        let result = up.run_deploy_migrations(&p, false).await;

        let check = || -> Result<(), String> {
            result.as_ref().map_err(|e| format!("{e:#}"))?;
            let row = psql("SELECT v FROM t");
            if row.trim() != "还在" { return Err(format!("数据不对：{row}")) }
            let c = std::fs::read_to_string(dir.join("compose.yml")).unwrap();
            if !c.contains(NEW_COMPOSE_LINE) { return Err("compose.yml 没改".into()) }
            if !std::fs::read_to_string(dir.join(".env")).unwrap().contains("POSTGRES_DATA=./data/postgres") { return Err(".env 没改".into()) }
            if !dir.join("data/postgres/18").exists() { return Err("data/postgres 里没有数据".into()) }
            if !std::fs::read_to_string(dir.join(STATE_FILE)).unwrap().starts_with(DATA_DIR) { return Err("没记下迁移".into()) }
            if !std::fs::read_dir(dir.join("backups")).map(|d| d.count() == 1).unwrap_or(false) { return Err("没有备份".into()) }
            // 之后按新的 compose.yml 重建，还是这份数据；再迁移一次不做任何事
            dc(&["up", "-d", "--wait"]);
            let row = psql("SELECT v FROM t");
            if row.trim() != "还在" { return Err(format!("compose up 之后数据不对：{row}")) }
            if up.migrations_pending() { return Err("迁移应该只做一次".into()) }
            Ok(())
        };
        let outcome = check();
        let _ = Command::new("docker").args(["compose", "-p", &project, "down", "-v"]).current_dir(&dir).output();
        // 数据目录里的文件属于容器里的 postgres 用户，用容器删
        let _ = Command::new("docker").args(["run", "--rm", "-v", &format!("{}:/d", dir.display()), "postgres:18.6", "rm", "-rf", "/d/data"]).output();
        let _ = std::fs::remove_dir_all(&dir);
        outcome.unwrap();
    }

    #[test]
    fn compose_line_is_rewritten_only_when_unchanged() {
        let old = "services:\n  postgres:\n    volumes: [pgdata:/var/lib/postgresql]\n    restart: unless-stopped\n";
        let new = rewrite_compose(old).unwrap();
        assert!(new.contains("    volumes: [\"${POSTGRES_DATA:-./data/postgres}:/var/lib/postgresql\"]\n"), "{new}");
        assert!(new.ends_with("restart: unless-stopped\n"));
        assert_eq!(rewrite_compose("    volumes: [/srv/pg:/var/lib/postgresql]\n"), None, "改过的不动");
        assert_eq!(rewrite_compose(&new), None, "已经迁移过的不再改");
    }

    #[test]
    fn data_mount_becomes_a_host_directory() {
        let body = json!({ "HostConfig": {
            "Mounts": [{ "Type": "volume", "Source": "niulai_pgdata", "Target": "/var/lib/postgresql" }, { "Type": "volume", "Source": "x", "Target": "/other" }],
            "Binds": ["/a:/b"]
        }});
        let b = with_data_bind(body, "/opt/niulai/data/postgres");
        assert_eq!(b["HostConfig"]["Mounts"], json!([{ "Type": "volume", "Source": "x", "Target": "/other" }]));
        assert_eq!(b["HostConfig"]["Binds"], json!(["/a:/b", "/opt/niulai/data/postgres:/var/lib/postgresql"]));
        assert_eq!(host_deploy_dir(&json!({ "Mounts": [{ "Destination": "/deploy", "Source": "/opt/niulai" }] })), Some("/opt/niulai".into()));
    }
}
