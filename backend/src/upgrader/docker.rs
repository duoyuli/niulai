//! 最小的 Docker Engine API 客户端：通过 Unix socket 调用一键升级用到的几个接口。

use std::path::Path;

use anyhow::{Context, bail};
use serde_json::{Value, json};

pub struct Docker {
    http: reqwest::Client,
}

/// 写到文件时每这么多字节 sync 一次，限制写盘脏页占用的内存
const SYNC_EVERY: usize = 32 * 1024 * 1024;
/// 标准错误只保留最后这么多字节
const MAX_STDERR: usize = 64 * 1024;

impl Docker {
    pub fn new(socket: &Path) -> anyhow::Result<Self> {
        // 不设总超时：拉取镜像、等待容器停止都可能要几分钟
        let http = reqwest::Client::builder().unix_socket(socket).build()?;
        Ok(Self { http })
    }

    fn url(path: &str) -> String {
        format!("http://docker{path}")
    }

    async fn check(resp: reqwest::Response, what: &str) -> anyhow::Result<reqwest::Response> {
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        bail!(
            "{what}失败（{status}）：{}",
            body["message"].as_str().unwrap_or("没有说明")
        )
    }

    async fn get(&self, path: &str, what: &str) -> anyhow::Result<Value> {
        let resp = self
            .http
            .get(Self::url(path))
            .send()
            .await
            .with_context(|| format!("{what}：连不上 Docker"))?;
        Ok(Self::check(resp, what).await?.json().await?)
    }

    async fn post(
        &self,
        path: &str,
        body: Option<Value>,
        what: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let mut req = self.http.post(Self::url(path));
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("{what}：连不上 Docker"))?;
        Self::check(resp, what).await
    }

    pub async fn inspect_container(&self, id: &str) -> anyhow::Result<Value> {
        self.get(&format!("/containers/{id}/json"), "查看容器")
            .await
    }

    pub async fn inspect_image(&self, reference: &str) -> anyhow::Result<Value> {
        self.get(&format!("/images/{reference}/json"), "查看镜像")
            .await
    }

    pub async fn image_exists(&self, reference: &str) -> bool {
        self.inspect_image(reference).await.is_ok()
    }

    /// 某个 Compose 项目中某个服务的全部容器（含已停止的）
    pub async fn service_containers(
        &self,
        project: &str,
        service: &str,
    ) -> anyhow::Result<Vec<Value>> {
        let filters = json!({ "label": [format!("com.docker.compose.project={project}"), format!("com.docker.compose.service={service}")] });
        let resp = self
            .http
            .get(Self::url("/containers/json"))
            .query(&[("all", "true"), ("filters", &filters.to_string())])
            .send()
            .await
            .context("列出容器：连不上 Docker")?;
        Ok(Self::check(resp, "列出容器").await?.json().await?)
    }

    /// 拉取镜像。进度是一行一个 JSON，出错时其中带 error。
    /// 边下载边按各层的已下载和总字节数调用 on_progress(已下载, 总数)
    pub async fn pull(
        &self,
        repo: &str,
        tag: &str,
        log: &mut String,
        on_progress: &mut dyn FnMut(u64, u64),
    ) -> anyhow::Result<()> {
        let resp = self
            .http
            .post(Self::url("/images/create"))
            .query(&[("fromImage", repo), ("tag", tag)])
            .send()
            .await
            .context("拉取镜像：连不上 Docker")?;
        let mut resp = Self::check(resp, "拉取镜像").await?;
        let mut layers = PullProgress::default();
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let v: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
                if let Some(e) = v["error"].as_str() {
                    bail!("拉取镜像失败：{e}");
                }
                if let Some(s) = v["status"]
                    .as_str()
                    .filter(|s| s.starts_with("Digest") || s.starts_with("Status"))
                {
                    log.push_str(s);
                    log.push('\n');
                }
                if layers.update(&v) {
                    let (done, total) = layers.sum();
                    on_progress(done, total);
                }
            }
        }
        Ok(())
    }

    pub async fn stop(&self, id: &str, timeout_secs: u64) -> anyhow::Result<()> {
        let resp = self
            .http
            .post(Self::url(&format!("/containers/{id}/stop")))
            .query(&[("t", timeout_secs.to_string())])
            .send()
            .await?;
        // 304：本来就已停止
        if resp.status().as_u16() == 304 {
            return Ok(());
        }
        Self::check(resp, "停止容器").await.map(|_| ())
    }

    pub async fn start(&self, id: &str) -> anyhow::Result<()> {
        let resp = self
            .http
            .post(Self::url(&format!("/containers/{id}/start")))
            .send()
            .await?;
        if resp.status().as_u16() == 304 {
            return Ok(());
        }
        Self::check(resp, "启动容器").await.map(|_| ())
    }

    pub async fn rename(&self, id: &str, name: &str) -> anyhow::Result<()> {
        let resp = self
            .http
            .post(Self::url(&format!("/containers/{id}/rename")))
            .query(&[("name", name)])
            .send()
            .await?;
        Self::check(resp, "重命名容器").await.map(|_| ())
    }

    pub async fn create(&self, name: &str, body: &Value) -> anyhow::Result<String> {
        let resp = self
            .http
            .post(Self::url("/containers/create"))
            .query(&[("name", name)])
            .json(body)
            .send()
            .await?;
        let v: Value = Self::check(resp, "创建容器").await?.json().await?;
        v["Id"]
            .as_str()
            .map(str::to_string)
            .context("创建容器：响应中没有 Id")
    }

    pub async fn remove(&self, id: &str) -> anyhow::Result<()> {
        let resp = self
            .http
            .delete(Self::url(&format!("/containers/{id}")))
            .query(&[("force", "true")])
            .send()
            .await?;
        Self::check(resp, "删除容器").await.map(|_| ())
    }

    /// 等容器结束，返回退出码
    pub async fn wait(&self, id: &str) -> anyhow::Result<i64> {
        let v: Value = self
            .post(&format!("/containers/{id}/wait"), None, "等待容器结束")
            .await?
            .json()
            .await?;
        Ok(v["StatusCode"].as_i64().unwrap_or(-1))
    }

    /// 容器的标准输出和标准错误
    pub async fn logs(&self, id: &str) -> anyhow::Result<(String, String)> {
        let resp = self
            .http
            .get(Self::url(&format!("/containers/{id}/logs")))
            .query(&[("stdout", "true"), ("stderr", "true")])
            .send()
            .await?;
        let bytes = Self::check(resp, "读取容器日志").await?.bytes().await?;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        for (stream, data) in Frames::default().push(&bytes) {
            if stream == 1 {
                out.extend(data)
            } else {
                err.extend(data)
            }
        }
        Ok((
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        ))
    }

    /// 在容器中执行命令，标准输出写入文件；返回退出码和标准错误。
    /// 每写 [`SYNC_EVERY`] 字节就把数据刷到磁盘：备份文件有几百 MB，写脏页如果一直攒在内存里，
    /// 会撞上容器的内存上限被 OOM 杀掉（2026-10-18 升级时实测，见 docs/operations/runbook.md#resource_limits）。
    pub async fn exec_to_file(
        &self,
        container: &str,
        cmd: &[&str],
        out: &Path,
    ) -> anyhow::Result<(i64, String)> {
        let created: Value = self
            .post(
                &format!("/containers/{container}/exec"),
                Some(json!({ "AttachStdout": true, "AttachStderr": true, "Cmd": cmd })),
                "创建执行",
            )
            .await?
            .json()
            .await?;
        let exec = created["Id"]
            .as_str()
            .context("创建执行：响应中没有 Id")?
            .to_string();
        let mut resp = self
            .post(
                &format!("/exec/{exec}/start"),
                Some(json!({ "Detach": false, "Tty": false })),
                "执行命令",
            )
            .await?;
        use std::io::Write;
        let mut file = std::io::BufWriter::new(std::fs::File::create(out)?);
        let mut frames = Frames::default();
        let mut stderr = Vec::new();
        let mut unsynced = 0usize;
        while let Some(chunk) = resp.chunk().await? {
            for (stream, data) in frames.push(&chunk) {
                if stream == 1 {
                    file.write_all(&data)?;
                    unsynced += data.len();
                    if unsynced >= SYNC_EVERY {
                        file.flush()?;
                        file.get_ref().sync_data()?;
                        unsynced = 0;
                    }
                } else {
                    // 只留最后一段错误输出：正常情况下很小，异常时也不该把内存吃光
                    stderr.extend_from_slice(&data);
                    if stderr.len() > MAX_STDERR {
                        stderr.drain(..stderr.len() - MAX_STDERR);
                    }
                }
            }
        }
        file.flush()?;
        file.get_ref().sync_data()?;
        let info = self
            .get(&format!("/exec/{exec}/json"), "查看执行结果")
            .await?;
        Ok((
            info["ExitCode"].as_i64().unwrap_or(-1),
            String::from_utf8_lossy(&stderr).into_owned(),
        ))
    }
}

/// 下载进度：每一层的已下载和总字节数
#[derive(Default)]
pub struct PullProgress {
    layers: std::collections::BTreeMap<String, (u64, u64)>,
}

impl PullProgress {
    /// 读一行进度，返回进度有没有变化
    pub fn update(&mut self, v: &Value) -> bool {
        let (Some(id), Some(status)) = (v["id"].as_str(), v["status"].as_str()) else {
            return false;
        };
        match status {
            "Downloading" => {
                let (cur, total) = (
                    v["progressDetail"]["current"].as_u64(),
                    v["progressDetail"]["total"].as_u64(),
                );
                if let (Some(c), Some(t)) = (cur, total) {
                    self.layers.insert(id.into(), (c, t));
                    return true;
                }
                false
            }
            // 下载完的层算满；本机已有的层没有大小，不计入
            "Download complete" | "Verifying Checksum" | "Pull complete" => {
                match self.layers.get_mut(id) {
                    Some((c, t)) if c != t => {
                        *c = *t;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    pub fn sum(&self) -> (u64, u64) {
        self.layers
            .values()
            .fold((0, 0), |(a, b), (c, t)| (a + c, b + t))
    }
}

/// 拆分 Docker 的多路输出：每帧 8 字节头（第 1 字节是流编号，后 4 字节是长度，大端）加数据
#[derive(Default)]
pub struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<(u8, Vec<u8>)> {
        self.buf.extend_from_slice(chunk);
        let mut out = vec![];
        while self.buf.len() >= 8 {
            let size =
                u32::from_be_bytes([self.buf[4], self.buf[5], self.buf[6], self.buf[7]]) as usize;
            if self.buf.len() < 8 + size {
                break;
            }
            let data = self.buf[8..8 + size].to_vec();
            out.push((self.buf[0], data));
            self.buf.drain(..8 + size);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_progress_sums_layers() {
        let mut p = PullProgress::default();
        assert!(p.update(&json!({"id": "a", "status": "Downloading", "progressDetail": {"current": 10, "total": 100}})));
        assert!(p.update(&json!({"id": "b", "status": "Downloading", "progressDetail": {"current": 50, "total": 50}})));
        assert!(!p.update(&json!({"id": "c", "status": "Already exists"})));
        assert_eq!(p.sum(), (60, 150));
        assert!(p.update(&json!({"id": "a", "status": "Pull complete"})));
        assert_eq!(p.sum(), (150, 150));
    }

    #[test]
    fn splits_multiplexed_stream_across_chunks() {
        let mut f = Frames::default();
        let mut raw = vec![1, 0, 0, 0, 0, 0, 0, 3];
        raw.extend_from_slice(b"abc");
        raw.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 2]);
        raw.extend_from_slice(b"no");
        let first = f.push(&raw[..5]);
        assert!(first.is_empty());
        let rest = f.push(&raw[5..]);
        assert_eq!(rest, vec![(1, b"abc".to_vec()), (2, b"no".to_vec())]);
    }
}
