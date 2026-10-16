//! 一键升级助手：`niulai upgrader`，在 Docker 部署的 upgrader 容器中运行（与 niulai 同一个镜像）。
//! 它通过 Docker 的控制接口升级同一个 Compose 项目中的 niulai 容器；niulai 本身没有这个权限，
//! 只能在共享目录中写一个版本号。双方交换的文件见 `api::system`，规则见 docs/operations/runbook.md#one_click_upgrade。

pub mod docker;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};

use docker::Docker;

/// niulai 容器以 nobody 运行，要能在共享目录中写请求
const NIULAI_UID: u32 = 65534;
/// 新版本最多等这么久恢复 healthy：停机等待 2 分钟，加上启动和数据库迁移
const HEALTHY_TIMEOUT: Duration = Duration::from_secs(300);
/// 停止 niulai 时的等待秒数，与 compose.yml 的 stop_grace_period 一致
const STOP_TIMEOUT_SECS: u64 = 120;

pub struct Options {
    /// 与 niulai 共享的目录
    pub upgrade_dir: PathBuf,
    /// compose.yml 和 .env 所在目录，备份也写在这里的 backups/
    pub deploy_dir: PathBuf,
    pub socket: PathBuf,
}

pub async fn run(opts: Options) -> anyhow::Result<()> {
    std::fs::create_dir_all(&opts.upgrade_dir)?;
    std::os::unix::fs::chown(&opts.upgrade_dir, Some(NIULAI_UID), Some(NIULAI_UID)).context("共享目录改属主失败")?;
    let docker = Docker::new(&opts.socket)?;
    let me = docker.inspect_container(&own_id()?).await.context("找不到 upgrader 自己的容器")?;
    let project = label(&me, "com.docker.compose.project").context("upgrader 不在 Compose 项目中")?.to_string();
    cleanup_previous(&docker, &me, &project).await;

    // 心跳单独运行：升级要几分钟，期间页面仍能知道 upgrader 活着
    let dir = opts.upgrade_dir.clone();
    tokio::spawn(async move {
        loop {
            let _ = std::fs::write(dir.join("heartbeat"), chrono::Utc::now().timestamp().to_string());
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    tracing::info!(project = %project, "一键升级助手已启动");

    let files = Files { dir: opts.upgrade_dir.clone() };
    loop {
        if let Some(v) = files.take_request() {
            if valid_version(&v) {
                let up = Upgrade { docker: &docker, files: &files, deploy: &opts.deploy_dir, project: &project, me: &me };
                if up.run(&v).await {
                    up.replace_self(&v).await;
                }
            } else {
                files.status("failed", "", "版本号不合法");
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Docker 默认把容器 ID 的前 12 位作为主机名
fn own_id() -> anyhow::Result<String> {
    if let Ok(h) = std::env::var("HOSTNAME") {
        return Ok(h);
    }
    Ok(std::fs::read_to_string("/etc/hostname")?.trim().to_string())
}

fn label<'a>(c: &'a Value, key: &str) -> Option<&'a str> {
    c["Config"]["Labels"][key].as_str()
}

fn name(c: &Value) -> String {
    c["Name"].as_str().unwrap_or_default().trim_start_matches('/').to_string()
}

pub fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// 自己升级后留下的旧 upgrader 容器（名字以 -old 结尾）
async fn cleanup_previous(docker: &Docker, me: &Value, project: &str) {
    let Some(service) = label(me, "com.docker.compose.service") else { return };
    let my_id = me["Id"].as_str().unwrap_or_default();
    for c in docker.service_containers(project, service).await.unwrap_or_default() {
        let id = c["Id"].as_str().unwrap_or_default();
        let old = c["Names"].as_array().into_iter().flatten().any(|n| n.as_str().is_some_and(|n| n.ends_with("-old")));
        if id != my_id && old {
            let _ = docker.remove(id).await;
            tracing::info!(container = %id, "已删除旧的 upgrader 容器");
        }
    }
}

struct Files {
    dir: PathBuf,
}

impl Files {
    fn take_request(&self) -> Option<String> {
        let path = self.dir.join("request");
        let v = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        Some(v.trim().to_string())
    }

    fn status(&self, state: &str, target: &str, message: &str) {
        let body = json!({ "state": state, "target": target, "message": message, "at": chrono::Utc::now().timestamp() });
        let tmp = self.dir.join("status.tmp");
        let _ = std::fs::write(&tmp, body.to_string()).and_then(|_| std::fs::rename(&tmp, self.dir.join("status")));
        tracing::info!(state, target, message, "升级状态");
    }

    fn log(&self, line: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("log")) {
            let _ = writeln!(f, "{} {line}", chrono::Utc::now().format("%F %T"));
        }
    }

    fn reset_log(&self) {
        let _ = std::fs::write(self.dir.join("log"), "");
    }
}

struct Upgrade<'a> {
    docker: &'a Docker,
    files: &'a Files,
    deploy: &'a Path,
    project: &'a str,
    me: &'a Value,
}

impl Upgrade<'_> {
    /// 升级 niulai 到 v，返回是否成功。失败时恢复原来的镜像和 .env，并写好失败原因
    async fn run(&self, v: &str) -> bool {
        self.files.reset_log();
        match self.try_run(v).await {
            Ok(()) => {
                self.files.log("完成");
                self.files.status("done", v, &format!("已升级到 {v}"));
                true
            }
            Err(e) => {
                let msg = format!("{e:#}");
                self.files.log(&msg);
                self.files.status("failed", v, &msg);
                false
            }
        }
    }

    async fn try_run(&self, v: &str) -> anyhow::Result<()> {
        let niulai = self.find("niulai").await?;
        let current = niulai["Config"]["Image"].as_str().unwrap_or_default().to_string();
        let repo = release_repo(&current).with_context(|| format!("niulai 用的镜像是 {current}，不是发布的镜像，不能一键升级"))?;
        let target = format!("{repo}:{v}");
        self.files.log(&format!("从 {current} 升级到 {target}"));

        self.files.status("running", v, "下载新版本");
        let mut pulled = String::new();
        if let Err(e) = self.docker.pull(repo, v, &mut pulled).await {
            // 本机已经有这个镜像时（例如离线环境预先导入）照常升级
            if !self.docker.image_exists(&target).await {
                return Err(e.context("下载新版本失败，没有做任何改动"));
            }
            self.files.log(&format!("{e:#}；本机已有 {target}，继续"));
        }
        if !pulled.trim().is_empty() {
            self.files.log(pulled.trim_end());
        }

        if env_var(&niulai, "DATABASE_URL").is_some_and(|u| u.contains("@postgres:")) {
            self.files.status("running", v, "备份数据库");
            self.backup(v).await.context("备份数据库失败，没有升级")?;
        }

        self.files.status("running", v, "重启服务");
        let env_path = self.deploy.join(".env");
        let env_before = std::fs::read_to_string(&env_path).ok();
        if let Some(text) = &env_before {
            write_in_place(&env_path, &set_env_line(text, "NIULAI_IMAGE", &target))?;
        }
        if let Err(e) = self.recreate(&niulai, &target, true).await {
            if let Some(text) = &env_before {
                let _ = write_in_place(&env_path, text);
            }
            return Err(e.context("新版本没有正常启动，已换回原来的版本；数据库迁移可能已经执行，必要时用 backups/ 中的备份恢复"));
        }
        Ok(())
    }

    async fn find(&self, service: &str) -> anyhow::Result<Value> {
        let list = self.docker.service_containers(self.project, service).await?;
        let first = list.iter().find(|c| !c["Names"].to_string().contains("-old")).with_context(|| format!("项目 {} 中没有 {service} 容器", self.project))?;
        self.docker.inspect_container(first["Id"].as_str().unwrap_or_default()).await
    }

    async fn backup(&self, v: &str) -> anyhow::Result<()> {
        let pg = self.find("postgres").await?;
        let dir = self.deploy.join("backups");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("niulai-before-{v}-{}.dump", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        let (code, stderr) = self.docker.exec_to_file(pg["Id"].as_str().unwrap_or_default(), &["pg_dump", "-U", "niulai", "-Fc", "niulai"], &file).await?;
        if code != 0 {
            let _ = std::fs::remove_file(&file);
            bail!("pg_dump 退出码 {code}：{}", stderr.trim());
        }
        self.files.log(&format!("已备份到 backups/{}", file.file_name().unwrap_or_default().to_string_lossy()));
        Ok(())
    }

    /// 用新镜像重建容器，保留原来的配置、卷和网络。wait 时等新容器 healthy，失败则换回旧容器
    async fn recreate(&self, old: &Value, image: &str, wait: bool) -> anyhow::Result<()> {
        let old_id = old["Id"].as_str().unwrap_or_default().to_string();
        let old_image = self.docker.inspect_image(old["Image"].as_str().unwrap_or_default()).await?;
        let body = create_body(old, &old_image, image);
        let name = name(old);
        let parked = format!("{name}-old");
        if wait {
            self.docker.stop(&old_id, STOP_TIMEOUT_SECS).await?;
        }
        self.docker.rename(&old_id, &parked).await?;
        let new_id = match self.docker.create(&name, &body).await {
            Ok(id) => id,
            Err(e) => {
                self.restore(&old_id, &name, None).await;
                return Err(e);
            }
        };
        if let Err(e) = self.docker.start(&new_id).await {
            self.restore(&old_id, &name, Some(&new_id)).await;
            return Err(e);
        }
        if !wait {
            return Ok(());
        }
        if let Err(e) = self.wait_healthy(&new_id).await {
            self.restore(&old_id, &name, Some(&new_id)).await;
            return Err(e);
        }
        self.docker.remove(&old_id).await?;
        Ok(())
    }

    async fn restore(&self, old_id: &str, name: &str, new_id: Option<&str>) {
        if let Some(n) = new_id {
            let _ = self.docker.remove(n).await;
        }
        let _ = self.docker.rename(old_id, name).await;
        let _ = self.docker.start(old_id).await;
        self.files.log(&format!("已换回原来的容器 {name}"));
    }

    async fn wait_healthy(&self, id: &str) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        loop {
            let c = self.docker.inspect_container(id).await?;
            let state = &c["State"];
            match state["Health"]["Status"].as_str() {
                Some("healthy") => return Ok(()),
                // 没有健康检查的容器：运行 10 秒没退出就算正常
                None if state["Running"].as_bool() == Some(true) && started.elapsed() > Duration::from_secs(10) => return Ok(()),
                _ => {}
            }
            // 容器有自动重启策略：启动后退出会被不断重启，看到重启或退出就算失败，不用等到超时
            if c["RestartCount"].as_i64().unwrap_or(0) > 0 || state["Restarting"].as_bool() == Some(true) {
                bail!("新容器启动后退出了（退出码 {}）", state["ExitCode"]);
            }
            if state["Running"].as_bool() == Some(false) && started.elapsed() > Duration::from_secs(5) {
                bail!("新容器退出了（退出码 {}）", state["ExitCode"]);
            }
            if started.elapsed() > HEALTHY_TIMEOUT {
                bail!("新容器 {} 秒内没有恢复 healthy", HEALTHY_TIMEOUT.as_secs());
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }

    /// niulai 升级成功后，upgrader 自己也换成新镜像：新容器启动时删掉这个旧容器
    async fn replace_self(&self, v: &str) {
        let Some(repo) = self.me["Config"]["Image"].as_str().and_then(release_repo) else { return };
        let image = format!("{repo}:{v}");
        if let Err(e) = self.recreate(self.me, &image, false).await {
            tracing::warn!(error = %format!("{e:#}"), "upgrader 换成新版本失败，下次 docker compose up 时会更新");
        }
    }
}

/// 发布的镜像（ghcr.io/<所有者>/niulai:<标签>）的仓库部分；其他镜像返回 None
pub fn release_repo(image: &str) -> Option<&str> {
    let (repo, tag) = image.rsplit_once(':')?;
    let parts: Vec<&str> = repo.split('/').collect();
    (!tag.contains('/') && parts.len() == 3 && parts[0] == "ghcr.io" && parts[2] == "niulai").then_some(repo)
}

fn env_var<'a>(c: &'a Value, key: &str) -> Option<&'a str> {
    c["Config"]["Env"].as_array()?.iter().filter_map(Value::as_str).find_map(|e| e.strip_prefix(key)?.strip_prefix('='))
}

/// 由旧容器的配置生成新容器的创建参数：换镜像，保留 Compose 写入的命令、环境变量、卷、端口、网络和标签。
/// 来自旧镜像的默认环境变量和标签去掉，让新镜像的默认值生效
pub fn create_body(old: &Value, old_image: &Value, image: &str) -> Value {
    let mut cfg = old["Config"].as_object().cloned().unwrap_or_default();
    cfg.insert("Image".into(), json!(image));
    cfg.remove("Hostname");
    let image_env: Vec<&str> = old_image["Config"]["Env"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if let Some(env) = cfg.get_mut("Env").and_then(Value::as_array_mut) {
        env.retain(|e| e.as_str().is_none_or(|e| !image_env.contains(&e)));
    }
    if let (Some(labels), Some(image_labels)) = (cfg.get_mut("Labels").and_then(Value::as_object_mut), old_image["Config"]["Labels"].as_object()) {
        labels.retain(|k, v| image_labels.get(k) != Some(v));
    }
    let mut endpoints = Map::new();
    for (net, ep) in old["NetworkSettings"]["Networks"].as_object().into_iter().flatten() {
        let mut keep = Map::new();
        for k in ["Aliases", "IPAMConfig", "Links", "DriverOpts"] {
            if let Some(v) = ep.get(k).filter(|v| !v.is_null()) {
                keep.insert(k.into(), v.clone());
            }
        }
        endpoints.insert(net.clone(), Value::Object(keep));
    }
    let mut body = Value::Object(cfg);
    body["HostConfig"] = old["HostConfig"].clone();
    body["NetworkingConfig"] = json!({ "EndpointsConfig": endpoints });
    body
}

/// 把 .env 中的 key=... 改成新值；没有这一行时追加
pub fn set_env_line(text: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with(&prefix) {
                found = true;
                format!("{prefix}{value}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        out.push(format!("{prefix}{value}"));
    }
    out.join("\n") + "\n"
}

/// 写回原文件而不是替换它：保留属主和权限，单文件挂载时也有效
fn write_in_place(path: &Path, text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().write(true).truncate(true).open(path).with_context(|| format!("写 {} 失败", path.display()))?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_release_versions_and_images() {
        assert!(valid_version("1.2.0"));
        for bad in ["1.2", "1.2.0-rc.1", "1.0;id", "", "a.b.c"] {
            assert!(!valid_version(bad), "{bad}");
        }
        assert_eq!(release_repo("ghcr.io/duoyuli/niulai:1.1.0"), Some("ghcr.io/duoyuli/niulai"));
        assert_eq!(release_repo("niulai:local"), None);
        assert_eq!(release_repo("ghcr.io/duoyuli/other:1.0.0"), None);
    }

    #[test]
    fn edits_env_line() {
        let t = "A=1\nNIULAI_IMAGE=ghcr.io/x/niulai:1.0.0\n# 注释\n";
        assert_eq!(set_env_line(t, "NIULAI_IMAGE", "ghcr.io/x/niulai:1.1.0"), "A=1\nNIULAI_IMAGE=ghcr.io/x/niulai:1.1.0\n# 注释\n");
        assert_eq!(set_env_line("A=1", "NIULAI_IMAGE", "i:2"), "A=1\nNIULAI_IMAGE=i:2\n");
    }

    #[test]
    fn new_container_keeps_compose_config_but_not_old_image_defaults() {
        let old = json!({
            "Config": {
                "Hostname": "abc123",
                "Image": "ghcr.io/x/niulai:1.0.0",
                "Cmd": ["niulai", "serve"],
                "Env": ["DATABASE_URL=postgres://u:p@postgres:5432/niulai", "NIULAI_WEB_DIR=/app/web", "PATH=/usr/bin"],
                "Labels": { "com.docker.compose.project": "niulai", "org.opencontainers.image.version": "1.0.0" }
            },
            "HostConfig": { "Binds": ["niulai_upgrade:/upgrade"], "PortBindings": { "8080/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "8080" }] } },
            "NetworkSettings": { "Networks": { "niulai_default": { "Aliases": ["niulai"], "NetworkID": "n1", "IPAddress": "172.0.0.2", "IPAMConfig": null } } }
        });
        let old_image = json!({ "Config": { "Env": ["NIULAI_WEB_DIR=/app/web", "PATH=/usr/bin"], "Labels": { "org.opencontainers.image.version": "1.0.0" } } });
        let b = create_body(&old, &old_image, "ghcr.io/x/niulai:1.1.0");
        assert_eq!(b["Image"], "ghcr.io/x/niulai:1.1.0");
        assert!(b.get("Hostname").is_none());
        assert_eq!(b["Env"], json!(["DATABASE_URL=postgres://u:p@postgres:5432/niulai"]));
        assert_eq!(b["Labels"], json!({ "com.docker.compose.project": "niulai" }));
        assert_eq!(b["Cmd"], json!(["niulai", "serve"]));
        assert_eq!(b["HostConfig"]["PortBindings"]["8080/tcp"][0]["HostPort"], "8080");
        assert_eq!(b["NetworkingConfig"]["EndpointsConfig"], json!({ "niulai_default": { "Aliases": ["niulai"] } }));
    }
}
