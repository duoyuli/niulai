//! 取数：按数据集的分页方式逐页请求，或下载全市场导出文件；每次 HTTP 尝试写一行 ops.api_call。

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use serde_json::{Map, Value};
use sqlx::PgPool;

use super::records::extract;
use super::spec::{DatasetSpec, Paging, Records};
use crate::vendor::{CallError, CallResult, HithinkClient};

/// 翻页上限，防止分页信息异常时无限翻页
const MAX_PAGES: i32 = 10_000;
/// 导出文件的预签名链接约 5 分钟有效：下载失败时重新签名的次数
const DUMP_ATTEMPTS: usize = 3;

/// 一个任务取到的原始记录
pub struct Fetched {
    /// (记录所在数组的名字, 记录)
    pub records: Vec<(String, Value)>,
    pub pages: i32,
    /// 按 empty_codes 判为无数据时的说明
    pub empty_note: Option<String>,
}

/// 一次取数请求的身份，用于写调用日志
pub struct Target<'a> {
    pub task_id: i64,
    pub part_key: &'a str,
    pub spec: &'a DatasetSpec,
}

pub struct Fetcher {
    pool: PgPool,
    client: Arc<HithinkClient>,
    /// 单次请求内可重试错误的退避秒数
    backoff_s: Vec<u64>,
}

impl Fetcher {
    pub fn new(pool: PgPool, client: Arc<HithinkClient>, backoff_s: Vec<u64>) -> Self {
        Fetcher { pool, client, backoff_s }
    }

    // @project-doc docs/architecture/sync_pipeline.md#record_writes
    pub async fn fetch(&self, t: &Target<'_>, query: &[(String, String)]) -> anyhow::Result<Fetched> {
        match &t.spec.records {
            Records::Parquet => Ok(Fetched { records: self.fetch_dump(t, query).await?, pages: 1, empty_note: None }),
            Records::Paths(paths) => self.fetch_pages(t, paths, query).await,
        }
    }

    async fn fetch_pages(&self, t: &Target<'_>, paths: &[String], base: &[(String, String)]) -> anyhow::Result<Fetched> {
        let spec = t.spec;
        let mut out = vec![];
        let mut empty_note = None;
        let mut page: i32 = 0;
        loop {
            page += 1;
            let mut query = base.to_vec();
            let size = match &spec.paging {
                Paging::None => None,
                Paging::Offset { limit_param, offset_param, size } => {
                    query.push((limit_param.clone(), size.to_string()));
                    query.push((offset_param.clone(), ((page as usize - 1) * size).to_string()));
                    Some(*size)
                }
                Paging::Page { page_param, size_param, size } => {
                    query.push((page_param.clone(), page.to_string()));
                    if let Some(sp) = size_param {
                        query.push((sp.clone(), size.to_string()));
                    }
                    Some(*size)
                }
            };
            let resp = match self.call(t, &query, page).await {
                Ok(r) => r,
                // 配置为「无数据」的错误码：结束翻页，按空结果处理
                Err(e) if is_empty_code(spec, &e) => {
                    empty_note = Some(format!("接口返回 {}，按无数据处理", e.code().unwrap_or_default()));
                    break;
                }
                Err(e) => return Err(e.into()),
            };
            let mut n_page = 0usize;
            for path in paths {
                let tag = path.rsplit('.').next().unwrap_or(path).to_string();
                for rec in extract(&resp.body, path) {
                    out.push((tag.clone(), rec));
                    n_page += 1;
                }
            }
            let has_next = resp.body.pointer("/data/pagination/has_next").and_then(Value::as_bool);
            let pages_total = resp.body.pointer("/data/pagination/pages").and_then(Value::as_i64);
            let more = match size {
                None => false,
                Some(sz) => match (has_next, pages_total) {
                    (Some(h), _) => h,
                    (None, Some(p)) => (page as i64) < p,
                    _ => n_page >= sz,
                },
            };
            if !more || page >= MAX_PAGES {
                break;
            }
        }
        Ok(Fetched { records: out, pages: page, empty_note })
    }

    async fn fetch_dump(&self, t: &Target<'_>, base: &[(String, String)]) -> anyhow::Result<Vec<(String, Value)>> {
        // 签名和下载在同一次尝试里完成，下载失败就重新签名
        let mut last_err = None;
        for attempt in 0..DUMP_ATTEMPTS {
            let signed = self.call(t, base, 1).await?;
            let url = signed.body.pointer("/data/presigned_url").and_then(Value::as_str).context("响应中没有 data.presigned_url")?;
            match self.client.download(url).await {
                Ok(bytes) => {
                    let rows = tokio::task::spawn_blocking(move || parquet_rows(bytes)).await??;
                    return Ok(rows.into_iter().map(|r| ("parquet".to_string(), r)).collect());
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "下载导出文件失败，重新签名");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.map(anyhow::Error::from).unwrap_or_else(|| anyhow!("下载失败")))
    }

    /// 带退避重试的单次调用。每次尝试写一行调用日志。
    async fn call(&self, t: &Target<'_>, query: &[(String, String)], page: i32) -> Result<CallResult, CallError> {
        let mut attempt = 0usize;
        loop {
            let res = self.client.get(&t.spec.endpoint, query).await;
            self.log_call(t, query, page, &res).await;
            match res {
                Ok(r) => return Ok(r),
                Err(e) if is_empty_code(t.spec, &e) => return Err(e),
                Err(e) if e.is_retryable() && attempt < self.backoff_s.len() => {
                    tracing::debug!(task_id = t.task_id, attempt, error = %e, "请求失败，退避后重试");
                    tokio::time::sleep(Duration::from_secs(self.backoff_s[attempt])).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn log_call(&self, t: &Target<'_>, query: &[(String, String)], page: i32, res: &Result<CallResult, CallError>) {
        let params: Map<String, Value> = query.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        let (http, code, msg, rid, bytes, latency) = match res {
            Ok(r) => (Some(r.http_status as i16), r.code(), None, r.request_id().map(str::to_string), Some(r.bytes as i32), r.latency_ms),
            Err(e) => (e.http_status().map(|s| s as i16), e.code(), Some(e.to_string()), None, None, e.latency_ms()),
        };
        let r = sqlx::query(
            "INSERT INTO ops.api_call (task_id, dataset_id, endpoint, params, page, http_status, api_code, message, request_id, bytes, latency_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(t.task_id)
        .bind(&t.spec.id)
        .bind(&t.spec.endpoint)
        .bind(Value::Object(params))
        .bind(page)
        .bind(http)
        .bind(code.map(|c| c as i32))
        .bind(msg)
        .bind(rid)
        .bind(bytes)
        .bind(latency as i32)
        .execute(&self.pool)
        .await;
        if let Err(e) = r {
            tracing::warn!(error = %e, "写调用日志失败");
        }
    }
}

fn is_empty_code(spec: &DatasetSpec, e: &CallError) -> bool {
    e.code().is_some_and(|c| spec.empty_codes.contains(&c))
}

fn parquet_rows(bytes: bytes::Bytes) -> anyhow::Result<Vec<Value>> {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    let reader = SerializedFileReader::new(bytes).context("解析 Parquet 失败")?;
    let mut out = Vec::with_capacity(reader.metadata().file_metadata().num_rows() as usize);
    for row in reader.get_row_iter(None)? {
        out.push(row?.to_json_value());
    }
    Ok(out)
}
