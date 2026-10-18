//! 自定义信号（app.signal）的读取：策略和信号判断时需要知道每个自定义信号的定义。
//! 自定义信号很少（几十个以内），每次用到时整表读出即可。

use std::collections::BTreeMap;
use std::sync::Arc;

use sqlx::PgPool;

use super::strategy::{CustomSignal, SignalDef, SignalLib};

/// 全部自定义信号；定义无法解析的跳过并记日志
pub async fn load(pool: &PgPool) -> sqlx::Result<SignalLib> {
    let rows: Vec<(i64, String, bool, serde_json::Value)> = sqlx::query_as(
        "SELECT signal_id, name, bullish, definition FROM app.signal ORDER BY signal_id",
    )
    .fetch_all(pool)
    .await?;
    let mut map = BTreeMap::new();
    for (id, name, bullish, v) in rows {
        match serde_json::from_value::<SignalDef>(v) {
            Ok(def) => {
                map.insert(id, CustomSignal { name, bullish, def });
            }
            Err(e) => tracing::warn!(signal_id = id, error = %e, "自定义信号的定义无法解析，跳过"),
        }
    }
    Ok(Arc::new(map))
}
