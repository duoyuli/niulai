//! 记录：从响应中取出记录，按主键模板生成 rkey / thscode / obs_date，写入 raw.record。

use std::collections::HashMap;

use anyhow::Context;
use serde_json::{Map, Value, json};
use sqlx::PgPool;

use super::spec::DatasetSpec;
use super::template::{self, Scope};

/// raw.upsert_records 每批的记录数
const UPSERT_CHUNK: usize = 2000;

/// 一条待写入的记录
#[derive(Debug, Clone)]
pub struct RecordRow {
    pub rkey: String,
    pub obs_date: Option<String>,
    pub thscode: Option<String>,
    pub seq: i32,
    pub payload: Value,
}

/// 按路径取记录：数组展开为多条；对象作为一条；null 或缺失时没有记录
pub fn extract(body: &Value, path: &str) -> Vec<Value> {
    match template::path_get(body, path) {
        Some(Value::Array(a)) => a.clone(),
        Some(v @ Value::Object(_)) => vec![v.clone()],
        _ => vec![],
    }
}

// @project-doc docs/interfaces/dataset_registry.md#keys_and_templates
/// 计算主键、证券、业务日期和同主键内的序号 _seq
pub fn build_rows(spec: &DatasetSpec, vars: &Map<String, Value>, records: &[(String, Value)], trade_date: Option<&str>) -> anyhow::Result<Vec<RecordRow>> {
    let mut seq_counter: HashMap<String, i32> = HashMap::new();
    let mut rows = Vec::with_capacity(records.len());
    for (array, rec) in records {
        // 先用占位符算出不含 _seq 的主键，再按源顺序编号
        let placeholder = [("_seq", "#".to_string()), ("_array", array.clone())];
        let scope = Scope { part: vars, record: Some(rec), run_trade_date: trade_date, extra: &placeholder };
        let base = template::render(&spec.key, &scope)?;
        let seq = {
            let c = seq_counter.entry(base.clone()).or_insert(0);
            *c += 1;
            *c
        };
        let extra = [("_seq", seq.to_string()), ("_array", array.clone())];
        let scope = Scope { part: vars, record: Some(rec), run_trade_date: trade_date, extra: &extra };
        let rkey = if spec.key.contains("{_seq}") { template::render(&spec.key, &scope)? } else { base };
        let render_opt = |t: &Option<String>| -> anyhow::Result<Option<String>> {
            match t {
                Some(t) => Ok(Some(template::render(t, &scope)?).filter(|s| !s.is_empty())),
                None => Ok(None),
            }
        };
        rows.push(RecordRow { rkey, obs_date: render_opt(&spec.obs_date)?, thscode: render_opt(&spec.thscode)?, seq, payload: rec.clone() });
    }
    Ok(rows)
}

/// 写入 raw.record（值变化时旧值进入 raw.record_version），返回新增或变化的记录数
pub async fn store(pool: &PgPool, dataset_id: &str, task_id: i64, rows: &[RecordRow]) -> anyhow::Result<usize> {
    let mut changed = 0usize;
    for chunk in rows.chunks(UPSERT_CHUNK) {
        let arr: Vec<Value> = chunk.iter().map(|r| json!({"rkey": r.rkey, "obs_date": r.obs_date, "thscode": r.thscode, "payload": r.payload})).collect();
        let (_inserted, ch): (i32, i32) = sqlx::query_as("SELECT inserted, changed FROM raw.upsert_records($1, $2, $3)")
            .bind(dataset_id)
            .bind(task_id)
            .bind(Value::Array(arr))
            .fetch_one(pool)
            .await
            .context("写入 raw.record")?;
        changed += ch as usize;
    }
    Ok(changed)
}
