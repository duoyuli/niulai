//! 把 datasets.yaml 同步到 ops.dataset，供 API 查询、供任务外键引用。

use sqlx::PgPool;

use super::spec::Registry;

pub async fn sync_to_db(pool: &PgPool, reg: &Registry) -> anyhow::Result<usize> {
    let mut tx = pool.begin().await?;
    for d in &reg.datasets {
        sqlx::query(
            "INSERT INTO ops.dataset (dataset_id, domain, endpoint, priority, enabled, spec, spec_hash)
             VALUES ($1, $2, $3, $4, $5, $6, md5($6::text))
             ON CONFLICT (dataset_id) DO UPDATE
               SET domain = EXCLUDED.domain, endpoint = EXCLUDED.endpoint, priority = EXCLUDED.priority,
                   enabled = EXCLUDED.enabled, spec = EXCLUDED.spec, spec_hash = EXCLUDED.spec_hash, updated_at = now()
             WHERE ops.dataset.spec_hash IS DISTINCT FROM EXCLUDED.spec_hash OR ops.dataset.enabled <> EXCLUDED.enabled",
        )
        .bind(&d.id)
        .bind(&d.domain)
        .bind(&d.endpoint)
        .bind(&d.priority)
        .bind(d.enabled)
        .bind(&d.raw)
        .execute(&mut *tx)
        .await?;
    }
    // 配置里删掉的数据集：保留历史记录，但标记为关闭
    let ids: Vec<&str> = reg.datasets.iter().map(|d| d.id.as_str()).collect();
    sqlx::query("UPDATE ops.dataset SET enabled = false, updated_at = now() WHERE NOT (dataset_id = ANY($1)) AND enabled")
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(reg.datasets.len())
}
