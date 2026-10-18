//! 模板：`{name}` 或 `{name:date}`。
//!
//! 变量查找顺序：
//! - `_part.x`：任务的分区变量（例如 thscode、date、start_ms）
//! - `_run.trade_date`：运行对应的交易日
//! - `_seq`、`_array`：由记录提取阶段补充
//! - 其余名称：先查分区变量，再查记录字段（支持 a.b 形式的嵌套路径）
//!
//! `:date` 把毫秒时间戳（或 yyyyMMdd 字符串）转换为 yyyy-MM-dd 日期。
//! 变量不存在时报错，避免生成错误的主键或请求。

use anyhow::{Context, bail};
use serde_json::{Map, Value};

use crate::time::ms_to_cn_date;

pub struct Scope<'a> {
    pub part: &'a Map<String, Value>,
    pub record: Option<&'a Value>,
    pub run_trade_date: Option<&'a str>,
    pub extra: &'a [(&'a str, String)],
}

impl Scope<'_> {
    fn lookup(&self, name: &str) -> Option<Value> {
        if let Some(k) = name.strip_prefix("_part.") {
            return self.part.get(k).cloned();
        }
        if name == "_run.trade_date" {
            return self.run_trade_date.map(|d| Value::String(d.to_string()));
        }
        if let Some((_, v)) = self.extra.iter().find(|(k, _)| *k == name) {
            return Some(Value::String(v.clone()));
        }
        if let Some(rec) = self.record
            && let Some(v) = path_get(rec, name)
        {
            return Some(v.clone());
        }
        self.part.get(name).cloned()
    }
}

pub fn path_get<'v>(v: &'v Value, path: &str) -> Option<&'v Value> {
    path.split('.').try_fold(v, |cur, seg| cur.get(seg))
}

pub fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.to_string(),
            (None, Some(f)) if f.fract() == 0.0 && f.abs() < 9.0e15 => (f as i64).to_string(),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

fn to_date(v: &Value) -> anyhow::Result<String> {
    let ms = match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) if s.len() == 8 && s.chars().all(|c| c.is_ascii_digit()) => {
            return Ok(format!("{}-{}-{}", &s[0..4], &s[4..6], &s[6..8]));
        }
        Value::String(s) if s.len() == 10 && s.as_bytes()[4] == b'-' => return Ok(s.clone()),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
    .with_context(|| format!("无法把 {v} 转换为日期"))?;
    Ok(ms_to_cn_date(ms)
        .context("时间戳超出范围")?
        .format("%Y-%m-%d")
        .to_string())
}

// @project-doc docs/interfaces/dataset_registry.md#keys_and_templates
pub fn render(template: &str, scope: &Scope) -> anyhow::Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let end = rest[start..]
            .find('}')
            .map(|e| start + e)
            .with_context(|| format!("模板缺少 }}：{template}"))?;
        let token = &rest[start + 1..end];
        let (name, fmt) = token
            .split_once(':')
            .map_or((token, None), |(n, f)| (n, Some(f)));
        let value = scope
            .lookup(name)
            .with_context(|| format!("模板变量不存在：{{{token}}}（模板 {template}）"))?;
        match fmt {
            None => out.push_str(&value_to_string(&value)),
            Some("date") => out.push_str(&to_date(&value)?),
            Some(f) => bail!("未知的模板格式 :{f}"),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// 模板里引用了哪些变量（不含格式后缀）
pub fn variables(template: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let token = &rest[start + 1..start + end];
        out.push(token.split(':').next().unwrap_or(token));
        rest = &rest[start + end + 1..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_keys_from_part_record_and_run() {
        let part = json!({"thscode": "600519.SH", "date": "2026-10-14"})
            .as_object()
            .unwrap()
            .clone();
        let rec =
            json!({"date_ms": 1_791_993_600_000_i64, "rank": 3, "x": {"y": "z"}, "pct": 9.99});
        let extra = [("_seq", "1".to_string())];
        let scope = Scope {
            part: &part,
            record: Some(&rec),
            run_trade_date: Some("2026-10-15"),
            extra: &extra,
        };
        assert_eq!(
            render("{_part.thscode}|{date_ms:date}", &scope).unwrap(),
            "600519.SH|2026-10-15"
        );
        assert_eq!(
            render("{_run.trade_date}|{rank}|{x.y}|{_seq}", &scope).unwrap(),
            "2026-10-15|3|z|1"
        );
        assert_eq!(render("{pct}", &scope).unwrap(), "9.99");
        assert!(render("{missing}", &scope).is_err());
    }

    #[test]
    fn date_format_accepts_compact_and_iso_strings() {
        let part = Map::new();
        let rec = json!({"a": "20261014", "b": "2026-10-14"});
        let scope = Scope {
            part: &part,
            record: Some(&rec),
            run_trade_date: None,
            extra: &[],
        };
        assert_eq!(
            render("{a:date}/{b:date}", &scope).unwrap(),
            "2026-10-14/2026-10-14"
        );
    }

    #[test]
    fn lists_variables() {
        assert_eq!(
            variables("{thscode}|{start_ms}-{end_ms:date}"),
            vec!["thscode", "start_ms", "end_ms"]
        );
    }
}
