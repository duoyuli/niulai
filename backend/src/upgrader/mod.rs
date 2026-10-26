//! 一键升级助手：`niulai upgrader`，在 Docker 部署的 upgrader 容器中运行（与 niulai 同一个镜像）。
//! 它通过 Docker 的控制接口升级同一个 Compose 项目中的 niulai 容器；niulai 本身没有这个权限，
//! 只能在共享目录中写一个版本号。双方交换的文件见 `api::system`，规则见 docs/operations/runbook.md#one_click_upgrade。

mod backup;
pub mod docker;
mod migrate;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
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
    std::os::unix::fs::chown(&opts.upgrade_dir, Some(NIULAI_UID), Some(NIULAI_UID))
        .context("共享目录改属主失败")?;
    let docker = Docker::new(&opts.socket)?;
    let me = docker
        .inspect_container(&own_id()?)
        .await
        .context("找不到 upgrader 自己的容器")?;
    let project = label(&me, "com.docker.compose.project")
        .context("upgrader 不在 Compose 项目中")?
        .to_string();
    cleanup_previous(&docker, &me, &project).await;

    // 心跳单独运行：升级要几分钟，期间页面仍能知道 upgrader 活着
    let dir = opts.upgrade_dir.clone();
    tokio::spawn(async move {
        loop {
            let _ = std::fs::write(
                dir.join("heartbeat"),
                chrono::Utc::now().timestamp().to_string(),
            );
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    tracing::info!(project = %project, "一键升级助手已启动");

    let files = Files {
        dir: opts.upgrade_dir.clone(),
    };
    let up = Upgrade {
        docker: &docker,
        files: &files,
        deploy: &opts.deploy_dir,
        project: &project,
        me: &me,
    };
    // 上一个升级助手把升级交给了我（它先把自己换成了新版本）：接着做剩下的步骤
    if let Some(h) = files.take_handoff() {
        let p = Progress::resume(&files, &h.target, h.steps);
        p.done("self", "已换成新版本");
        up.finish(&p, &h.target).await;
    } else if up.migrations_pending() {
        // 不是升级（例如升级助手刚被手动更新）也检查一遍部署迁移
        let p = Progress::new(&files, "", &[("migrate", "部署迁移")]);
        match up.run_deploy_migrations(&p, false).await {
            Ok(()) => p.finish_ok("部署迁移已完成"),
            Err(e) => p.finish_err(&format!("{e:#}")),
        }
    }
    // 每日备份：只备份内置数据库；状态先写一次，页面马上能看到已有的备份
    let backups = opts.deploy_dir.join("backups");
    let backup_status = opts.upgrade_dir.join("backup");
    backup::write_status(&backups, &backup_status, None);
    let mut daily = backup::Daily::default();
    let mut last_check: Option<std::time::Instant> = None;
    loop {
        if last_check.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            last_check = Some(std::time::Instant::now());
            match up.find("niulai").await {
                Ok(n) if builtin_db(&n) => match up.find("postgres").await {
                    Ok(pg) => {
                        daily
                            .tick(
                                &docker,
                                pg["Id"].as_str().unwrap_or_default(),
                                &backups,
                                &backup_status,
                            )
                            .await
                    }
                    Err(e) => {
                        tracing::warn!(error = %format!("{e:#}"), "找不到内置数据库容器，跳过每日备份")
                    }
                },
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "找不到 niulai 容器，跳过每日备份")
                }
            }
        }
        if let Some(v) = files.take_request() {
            if valid_version(&v) {
                up.begin(&v).await;
            } else {
                Progress::new(&files, "", &[]).finish_err("版本号不合法");
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
    c["Name"]
        .as_str()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string()
}

/// 只接受正式版本号（`YYYY.MM.DD`、`YYYY.MM.DD.N`），版本号会拼进镜像标签
pub fn valid_version(v: &str) -> bool {
    crate::version::parse(v).is_some()
}

/// 自己升级后留下的旧 upgrader 容器（名字以 -old 结尾）
async fn cleanup_previous(docker: &Docker, me: &Value, project: &str) {
    let Some(service) = label(me, "com.docker.compose.service") else {
        return;
    };
    let my_id = me["Id"].as_str().unwrap_or_default();
    for c in docker
        .service_containers(project, service)
        .await
        .unwrap_or_default()
    {
        let id = c["Id"].as_str().unwrap_or_default();
        let old = c["Names"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|n| n.as_str().is_some_and(|n| n.ends_with("-old")));
        if id != my_id && old {
            let _ = docker.remove(id).await;
            tracing::info!(container = %id, "已删除旧的 upgrader 容器");
        }
    }
}

// @project-doc docs/operations/runbook.md#one_click_upgrade
/// 升级的一个步骤：写在状态文件里，页面据此显示进度条和步骤列表
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Step {
    pub key: String,
    pub label: String,
    /// pending / running / done / failed / skipped
    pub state: String,
    #[serde(default)]
    pub detail: String,
}

/// 上一个升级助手交给新版本升级助手的进度
#[derive(Serialize, Deserialize)]
struct Handoff {
    target: String,
    steps: Vec<Step>,
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

    fn take_handoff(&self) -> Option<Handoff> {
        let path = self.dir.join("handoff");
        let v = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        serde_json::from_str(&v).ok()
    }

    fn write_handoff(&self, h: &Handoff) -> anyhow::Result<()> {
        Ok(std::fs::write(
            self.dir.join("handoff"),
            serde_json::to_string(h)?,
        )?)
    }

    /// 状态文件：页面读它显示进度。message 是当前步骤或结果的一句话，失败时页面显示它
    fn status(&self, state: &str, target: &str, message: &str, steps: &[Step]) {
        let body = json!({ "state": state, "target": target, "message": message, "at": chrono::Utc::now().timestamp(), "steps": steps });
        let tmp = self.dir.join("status.tmp");
        let _ = std::fs::write(&tmp, body.to_string())
            .and_then(|_| std::fs::rename(&tmp, self.dir.join("status")));
        tracing::info!(state, target, message, "升级状态");
    }

    fn log(&self, line: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("log"))
        {
            let _ = writeln!(f, "{} {line}", chrono::Utc::now().format("%F %T"));
        }
    }

    fn reset_log(&self) {
        let _ = std::fs::write(self.dir.join("log"), "");
    }
}

/// 一次升级（或部署迁移）的进度：每个步骤的状态和说明，每次变化都写进状态文件
struct Progress<'a> {
    files: &'a Files,
    target: String,
    steps: RefCell<Vec<Step>>,
}

impl<'a> Progress<'a> {
    fn new(files: &'a Files, target: &str, plan: &[(&str, &str)]) -> Self {
        let steps = plan
            .iter()
            .map(|(k, l)| Step {
                key: k.to_string(),
                label: l.to_string(),
                state: "pending".into(),
                detail: String::new(),
            })
            .collect();
        Progress {
            files,
            target: target.into(),
            steps: RefCell::new(steps),
        }
    }

    fn resume(files: &'a Files, target: &str, steps: Vec<Step>) -> Self {
        Progress {
            files,
            target: target.into(),
            steps: RefCell::new(steps),
        }
    }

    fn set(&self, key: &str, state: &str, detail: &str) {
        let mut steps = self.steps.borrow_mut();
        if let Some(s) = steps.iter_mut().find(|s| s.key == key) {
            s.state = state.into();
            s.detail = detail.into();
        }
        let label = steps
            .iter()
            .find(|s| s.key == key)
            .map_or(key.to_string(), |s| s.label.clone());
        let message = if detail.is_empty() {
            label
        } else {
            format!("{label}：{detail}")
        };
        self.files.status("running", &self.target, &message, &steps);
    }

    fn running(&self, key: &str, detail: &str) {
        self.set(key, "running", detail);
    }

    fn done(&self, key: &str, detail: &str) {
        self.set(key, "done", detail);
        self.files.log(&format!("{key} 完成 {detail}"));
    }

    fn skip(&self, key: &str, detail: &str) {
        self.set(key, "skipped", detail);
        self.files.log(&format!("{key} 跳过：{detail}"));
    }

    fn snapshot(&self) -> Vec<Step> {
        self.steps.borrow().clone()
    }

    fn finish_ok(&self, message: &str) {
        self.files.log(message);
        self.files
            .status("done", &self.target, message, &self.steps.borrow());
    }

    /// 失败：正在进行的步骤记为失败，后面的保持未开始
    fn finish_err(&self, message: &str) {
        let mut steps = self.steps.borrow_mut();
        for s in steps.iter_mut().filter(|s| s.state == "running") {
            s.state = "failed".into();
        }
        self.files.log(message);
        self.files.status("failed", &self.target, message, &steps);
    }
}

/// 升级的步骤。先把升级助手换成新版本，由新版本的升级助手做部署迁移和后面的步骤：
/// 新版本需要的迁移只有新版本的升级助手知道，迁移要在新版本的 niulai 启动之前做完
const PLAN: [(&str, &str); 5] = [
    ("pull", "下载新版本"),
    ("self", "更新升级助手"),
    ("backup", "备份数据库"),
    ("migrate", "部署迁移"),
    ("restart", "启动新版本"),
];

struct Upgrade<'a> {
    docker: &'a Docker,
    files: &'a Files,
    deploy: &'a Path,
    project: &'a str,
    me: &'a Value,
}

impl Upgrade<'_> {
    /// 收到升级请求：下载新版本，把升级助手自己换成新版本，交给它做剩下的步骤
    async fn begin(&self, v: &str) {
        self.files.reset_log();
        let p = Progress::new(self.files, v, &PLAN);
        match self.download(&p, v).await {
            Ok(()) => {}
            Err(e) => return p.finish_err(&format!("{e:#}")),
        }
        p.running("self", "");
        match self.handoff(&p, v).await {
            // 新的升级助手已经启动，由它继续；这个旧容器会被它删掉
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => p.skip("self", &format!("没有换成：{e:#}，由当前的升级助手继续")),
        }
        self.finish(&p, v).await;
    }

    async fn download(&self, p: &Progress<'_>, v: &str) -> anyhow::Result<()> {
        let niulai = self.find("niulai").await?;
        let current = niulai["Config"]["Image"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let repo = release_repo(&current).with_context(|| {
            format!("niulai 用的镜像是 {current}，不是发布的镜像，不能一键升级")
        })?;
        let target = format!("{repo}:{v}");
        self.files.log(&format!("从 {current} 升级到 {target}"));
        p.running("pull", "开始下载");
        let mut last = std::time::Instant::now();
        let mut on_progress = |done: u64, total: u64| {
            // 每秒最多写一次状态文件
            if total > 0 && last.elapsed() >= Duration::from_secs(1) {
                last = std::time::Instant::now();
                p.running(
                    "pull",
                    &format!(
                        "{}%（{} / {} MB）",
                        done * 100 / total,
                        done >> 20,
                        total >> 20
                    ),
                );
            }
        };
        let mut pulled = String::new();
        if let Err(e) = self
            .docker
            .pull(repo, v, &mut pulled, &mut on_progress)
            .await
        {
            // 本机已经有这个镜像时（例如离线环境预先导入）照常升级
            if !self.docker.image_exists(&target).await {
                return Err(e.context("下载新版本失败，没有做任何改动"));
            }
            self.files.log(&format!("{e:#}；本机已有 {target}，继续"));
        }
        if !pulled.trim().is_empty() {
            self.files.log(pulled.trim_end());
        }
        p.done("pull", "");
        Ok(())
    }

    /// 把升级助手换成新版本。返回 true 表示新的升级助手已经接手
    async fn handoff(&self, p: &Progress<'_>, v: &str) -> anyhow::Result<bool> {
        let Some(repo) = self.me["Config"]["Image"].as_str().and_then(release_repo) else {
            p.skip("self", "升级助手用的不是发布的镜像");
            return Ok(false);
        };
        let image = format!("{repo}:{v}");
        if self.me["Config"]["Image"].as_str() == Some(image.as_str()) {
            p.skip("self", "已经是新版本");
            return Ok(false);
        }
        p.running("self", "换成新版本，由它接着升级");
        self.files.write_handoff(&Handoff {
            target: v.into(),
            steps: p.snapshot(),
        })?;
        if let Err(e) = self.recreate(self.me, &image, false).await {
            let _ = self.files.take_handoff();
            return Err(e);
        }
        Ok(true)
    }

    /// 升级的后半段：备份、部署迁移、用新版本重建 niulai
    async fn finish(&self, p: &Progress<'_>, v: &str) {
        match self.finish_steps(p, v).await {
            Ok(()) => p.finish_ok(&format!("已升级到 {v}")),
            Err(e) => p.finish_err(&format!("{e:#}")),
        }
    }

    async fn finish_steps(&self, p: &Progress<'_>, v: &str) -> anyhow::Result<()> {
        let niulai = self.find("niulai").await?;
        let current = niulai["Config"]["Image"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let repo = release_repo(&current).with_context(|| {
            format!("niulai 用的镜像是 {current}，不是发布的镜像，不能一键升级")
        })?;
        let target = format!("{repo}:{v}");
        let backed_up = if builtin_db(&niulai) {
            p.running("backup", "pg_dump");
            let file = self.backup(v).await.context("备份数据库失败，没有升级")?;
            p.done("backup", &file);
            true
        } else {
            p.skip("backup", "用的不是内置数据库");
            false
        };
        self.run_deploy_migrations(p, backed_up)
            .await
            .context("部署迁移失败，没有升级；迁移前的状态已经恢复")?;

        p.running(
            "restart",
            "停止旧版本（最多等 2 分钟，让正在执行的同步任务结束）",
        );
        let env_path = self.deploy.join(".env");
        let env_before = std::fs::read_to_string(&env_path).ok();
        if let Some(text) = &env_before {
            write_in_place(&env_path, &set_env_line(text, "NIULAI_IMAGE", &target))?;
        }
        let niulai = self.find("niulai").await?;
        let mut body = create_body_for(self.docker, &niulai, &target).await?;
        // 旧容器由更新前的 Compose 创建时，补上部署目录中的共享缓存配置。
        if let Some(redis_url) = read_env_value(&env_path, "REDIS_URL") {
            add_container_env(&mut body, "REDIS_URL", &redis_url);
        }
        let tick = |secs: u64| {
            p.running(
                "restart",
                &format!(
                    "等新版本启动，已等 {secs} 秒（含数据库迁移，最多 {} 分钟）",
                    HEALTHY_TIMEOUT.as_secs() / 60
                ),
            )
        };
        if let Err(e) = self.recreate_with(&niulai, body, true, &tick).await {
            if let Some(text) = &env_before {
                let _ = write_in_place(&env_path, text);
            }
            return Err(e.context("新版本没有正常启动，已换回原来的版本；数据库迁移可能已经执行，必要时用 backups/ 中的备份恢复"));
        }
        p.done("restart", "");
        Ok(())
    }

    async fn find(&self, service: &str) -> anyhow::Result<Value> {
        let list = self
            .docker
            .service_containers(self.project, service)
            .await?;
        let first = list
            .iter()
            .find(|c| !c["Names"].to_string().contains("-old"))
            .with_context(|| format!("项目 {} 中没有 {service} 容器", self.project))?;
        self.docker
            .inspect_container(first["Id"].as_str().unwrap_or_default())
            .await
    }

    /// 备份内置数据库到部署目录的 backups/，返回文件名。写好之后删掉更旧的备份，只留这一份
    async fn backup(&self, label: &str) -> anyhow::Result<String> {
        let pg = self.find("postgres").await?;
        let dir = self.deploy.join("backups");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!(
            "{}{label}-{}.dump",
            backup::UPGRADE_PREFIX,
            crate::time::now_cn().format("%Y%m%d-%H%M%S")
        ));
        backup::dump(self.docker, pg["Id"].as_str().unwrap_or_default(), &file).await?;
        backup::keep_latest(&dir);
        let name = format!(
            "backups/{}",
            file.file_name().unwrap_or_default().to_string_lossy()
        );
        self.files.log(&format!("已备份到 {name}"));
        Ok(name)
    }

    /// 用新镜像重建容器，保留原来的配置、卷和网络
    async fn recreate(&self, old: &Value, image: &str, wait: bool) -> anyhow::Result<()> {
        let body = create_body_for(self.docker, old, image).await?;
        self.recreate_with(old, body, wait, &|_| {}).await
    }

    /// 按给定的参数重建容器。wait 时先停旧容器、等新容器 healthy（每 3 秒调用一次 tick），失败则换回旧容器
    async fn recreate_with(
        &self,
        old: &Value,
        body: Value,
        wait: bool,
        tick: &dyn Fn(u64),
    ) -> anyhow::Result<()> {
        let old_id = old["Id"].as_str().unwrap_or_default().to_string();
        let name = name(old);
        let parked = format!("{name}-old");
        if wait {
            self.docker.stop(&old_id, STOP_TIMEOUT_SECS).await?;
        }
        self.docker.rename(&old_id, &parked).await?;
        let new_id = match self.docker.create(&name, &body).await {
            Ok(id) => id,
            Err(e) => return Err(self.restore(&old_id, &name, None, wait, e).await),
        };
        if let Err(e) = self.docker.start(&new_id).await {
            return Err(self.restore(&old_id, &name, Some(&new_id), wait, e).await);
        }
        if !wait {
            return Ok(());
        }
        if let Err(e) = self.wait_healthy(&new_id, tick).await {
            return Err(self.restore(&old_id, &name, Some(&new_id), wait, e).await);
        }
        self.docker.remove(&old_id).await?;
        Ok(())
    }

    /// 换回旧容器，返回带上换回结果的错误。wait 时旧容器之前停过，要等它重新 healthy 才算换回成功：
    /// 新版本的数据库迁移已经执行时，旧版本有可能起不来，这时如实报告，不说「已换回」
    async fn restore(
        &self,
        old_id: &str,
        name: &str,
        new_id: Option<&str>,
        wait: bool,
        err: anyhow::Error,
    ) -> anyhow::Error {
        if let Some(n) = new_id {
            let _ = self.docker.remove(n).await;
        }
        let _ = self.docker.rename(old_id, name).await;
        let started = self.docker.start(old_id).await;
        let back = match started {
            Err(e) => Err(e),
            Ok(()) if wait => self.wait_healthy(old_id, &|_| {}).await,
            Ok(()) => Ok(()),
        };
        match back {
            Ok(()) => {
                self.files.log(&format!("已换回原来的容器 {name}"));
                err.context(format!("已换回原来的版本（{name}）"))
            }
            Err(e) => {
                self.files
                    .log(&format!("换回原来的容器 {name} 后它没能正常运行：{e:#}"));
                err.context(format!(
                    "换回的原版本（{name}）也没能正常运行：{e:#}。请按运维手册从升级前的备份恢复"
                ))
            }
        }
    }

    async fn wait_healthy(&self, id: &str, tick: &dyn Fn(u64)) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        loop {
            tick(started.elapsed().as_secs());
            let c = self.docker.inspect_container(id).await?;
            let state = &c["State"];
            match state["Health"]["Status"].as_str() {
                Some("healthy") => return Ok(()),
                // 没有健康检查的容器：运行 10 秒没退出就算正常
                None if state["Running"].as_bool() == Some(true)
                    && started.elapsed() > Duration::from_secs(10) =>
                {
                    return Ok(());
                }
                _ => {}
            }
            // 容器有自动重启策略：启动后退出会被不断重启，看到重启或退出就算失败，不用等到超时
            if c["RestartCount"].as_i64().unwrap_or(0) > 0
                || state["Restarting"].as_bool() == Some(true)
            {
                bail!("新容器启动后退出了（退出码 {}）", state["ExitCode"]);
            }
            if state["Running"].as_bool() == Some(false)
                && started.elapsed() > Duration::from_secs(5)
            {
                bail!("新容器退出了（退出码 {}）", state["ExitCode"]);
            }
            if started.elapsed() > HEALTHY_TIMEOUT {
                bail!("新容器 {} 秒内没有恢复 healthy", HEALTHY_TIMEOUT.as_secs());
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
}

/// niulai 连的是不是同一个 compose 中的内置数据库
fn builtin_db(niulai: &Value) -> bool {
    env_var(niulai, "DATABASE_URL").is_some_and(|u| u.contains("@postgres:"))
}

async fn create_body_for(docker: &Docker, old: &Value, image: &str) -> anyhow::Result<Value> {
    let old_image = docker
        .inspect_image(old["Image"].as_str().unwrap_or_default())
        .await?;
    Ok(create_body(old, &old_image, image))
}

/// 发布的镜像（ghcr.io/<所有者>/niulai:<标签>）的仓库部分；其他镜像返回 None
pub fn release_repo(image: &str) -> Option<&str> {
    let (repo, tag) = image.rsplit_once(':')?;
    let parts: Vec<&str> = repo.split('/').collect();
    (!tag.contains('/') && parts.len() == 3 && parts[0] == "ghcr.io" && parts[2] == "niulai")
        .then_some(repo)
}

fn env_var<'a>(c: &'a Value, key: &str) -> Option<&'a str> {
    c["Config"]["Env"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|e| e.strip_prefix(key)?.strip_prefix('='))
}

fn read_env_value(path: &Path, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
}

fn add_container_env(body: &mut Value, key: &str, value: &str) {
    let Some(env) = body["Env"].as_array_mut() else {
        return;
    };
    let prefix = format!("{key}=");
    env.retain(|entry| entry.as_str().is_none_or(|line| !line.starts_with(&prefix)));
    env.push(Value::String(format!("{prefix}{value}")));
}

/// 由旧容器的配置生成新容器的创建参数：换镜像，保留 Compose 写入的命令、环境变量、卷、端口、网络和标签。
/// 来自旧镜像的默认环境变量和标签去掉，让新镜像的默认值生效
pub fn create_body(old: &Value, old_image: &Value, image: &str) -> Value {
    let mut cfg = old["Config"].as_object().cloned().unwrap_or_default();
    cfg.insert("Image".into(), json!(image));
    cfg.remove("Hostname");
    let image_env: Vec<&str> = old_image["Config"]["Env"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if let Some(env) = cfg.get_mut("Env").and_then(Value::as_array_mut) {
        env.retain(|e| e.as_str().is_none_or(|e| !image_env.contains(&e)));
    }
    if let (Some(labels), Some(image_labels)) = (
        cfg.get_mut("Labels").and_then(Value::as_object_mut),
        old_image["Config"]["Labels"].as_object(),
    ) {
        labels.retain(|k, v| image_labels.get(k) != Some(v));
    }
    let mut endpoints = Map::new();
    for (net, ep) in old["NetworkSettings"]["Networks"]
        .as_object()
        .into_iter()
        .flatten()
    {
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
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("写 {} 失败", path.display()))?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_release_versions_and_images() {
        assert!(valid_version("2026.10.18") && valid_version("2026.10.18.1"));
        for bad in [
            "1.2.0",
            "1.2",
            "2026.10.18-rc.1",
            "2026.10.18;id",
            "",
            "a.b.c",
            "2026.10.18.1.1",
            "2026.10.18 x",
        ] {
            assert!(!valid_version(bad), "{bad}");
        }
        assert_eq!(
            release_repo("ghcr.io/duoyuli/niulai:2026.10.18"),
            Some("ghcr.io/duoyuli/niulai")
        );
        assert_eq!(release_repo("niulai:local"), None);
        assert_eq!(release_repo("ghcr.io/duoyuli/other:1.0.0"), None);
    }

    #[test]
    fn edits_env_line() {
        let t = "A=1\nNIULAI_IMAGE=ghcr.io/x/niulai:2026.10.17\n# 注释\n";
        assert_eq!(
            set_env_line(t, "NIULAI_IMAGE", "ghcr.io/x/niulai:2026.10.18"),
            "A=1\nNIULAI_IMAGE=ghcr.io/x/niulai:2026.10.18\n# 注释\n"
        );
        assert_eq!(
            set_env_line("A=1", "NIULAI_IMAGE", "i:2"),
            "A=1\nNIULAI_IMAGE=i:2\n"
        );
    }

    #[test]
    fn new_container_keeps_compose_config_but_not_old_image_defaults() {
        let old = json!({
            "Config": {
                "Hostname": "abc123",
                "Image": "ghcr.io/x/niulai:2026.10.17",
                "Cmd": ["niulai", "serve"],
                "Env": ["DATABASE_URL=postgres://u:p@postgres:5432/niulai", "NIULAI_WEB_DIR=/app/web", "PATH=/usr/bin"],
                "Labels": { "com.docker.compose.project": "niulai", "org.opencontainers.image.version": "2026.10.17" }
            },
            "HostConfig": { "Binds": ["niulai_upgrade:/upgrade"], "PortBindings": { "8080/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "8080" }] } },
            "NetworkSettings": { "Networks": { "niulai_default": { "Aliases": ["niulai"], "NetworkID": "n1", "IPAddress": "172.0.0.2", "IPAMConfig": null } } }
        });
        let old_image = json!({ "Config": { "Env": ["NIULAI_WEB_DIR=/app/web", "PATH=/usr/bin"], "Labels": { "org.opencontainers.image.version": "2026.10.17" } } });
        let b = create_body(&old, &old_image, "ghcr.io/x/niulai:2026.10.18");
        assert_eq!(b["Image"], "ghcr.io/x/niulai:2026.10.18");
        assert!(b.get("Hostname").is_none());
        assert_eq!(
            b["Env"],
            json!(["DATABASE_URL=postgres://u:p@postgres:5432/niulai"])
        );
        assert_eq!(
            b["Labels"],
            json!({ "com.docker.compose.project": "niulai" })
        );
        assert_eq!(b["Cmd"], json!(["niulai", "serve"]));
        assert_eq!(
            b["HostConfig"]["PortBindings"]["8080/tcp"][0]["HostPort"],
            "8080"
        );
        assert_eq!(
            b["NetworkingConfig"]["EndpointsConfig"],
            json!({ "niulai_default": { "Aliases": ["niulai"] } })
        );
    }

    #[test]
    fn container_env_overlay_replaces_existing_value() {
        let mut body = json!({"Env": ["DATABASE_URL=redacted", "REDIS_URL=old"]});
        add_container_env(&mut body, "REDIS_URL", "redis://redis:6379");
        assert_eq!(
            body["Env"].as_array().unwrap(),
            &vec![
                Value::String("DATABASE_URL=redacted".into()),
                Value::String("REDIS_URL=redis://redis:6379".into())
            ]
        );
    }
}
