-- raw.upsert_records 改成一条语句：原来先写临时表 _in 再关联 raw.record，临时表没有统计信息，
-- 规划器对几百万行的 raw.record 选了很差的连接方式，回补时每批 2~3 秒；改成 CTE 后按主键查找，每批约 1 毫秒。
-- 语义不变：同一批里重复的 rkey 只保留最后一条；值变化时旧值先进 raw.record_version；返回新增数和变化数。
CREATE OR REPLACE FUNCTION raw.upsert_records(p_dataset TEXT, p_task BIGINT, p_records JSONB,
                                              OUT inserted INTEGER, OUT changed INTEGER)
LANGUAGE sql AS $$
  WITH incoming AS (
    SELECT DISTINCT ON (r->>'rkey') r->>'rkey' AS rkey, (r->>'obs_date')::date AS obs_date, r->>'thscode' AS thscode,
           r->'payload' AS payload, md5((r->'payload')::text) AS h
    FROM jsonb_array_elements(p_records) WITH ORDINALITY AS t(r, ord)
    ORDER BY r->>'rkey', ord DESC
  ),
  -- 同一条语句里的各个 CTE 看到的都是语句开始前的数据，所以这里读到的是写入前的旧值
  existing AS (
    SELECT c.dataset_id, c.rkey, c.changed_at, c.payload, c.payload_hash, c.task_id, c.payload_hash <> i.h AS is_changed
    FROM incoming i JOIN raw.record c ON c.dataset_id = p_dataset AND c.rkey = i.rkey
  ),
  versioned AS (
    INSERT INTO raw.record_version (dataset_id, rkey, valid_from, valid_to, payload, payload_hash, task_id)
    SELECT dataset_id, rkey, changed_at, now(), payload, payload_hash, task_id FROM existing WHERE is_changed
    RETURNING 1
  ),
  written AS (
    INSERT INTO raw.record AS c (dataset_id, rkey, obs_date, thscode, payload, payload_hash, task_id)
    SELECT p_dataset, rkey, obs_date, thscode, payload, h, p_task FROM incoming
    ON CONFLICT (dataset_id, rkey) DO UPDATE
      SET last_seen_at = now(),
          payload      = CASE WHEN c.payload_hash <> EXCLUDED.payload_hash THEN EXCLUDED.payload ELSE c.payload END,
          changed_at   = CASE WHEN c.payload_hash <> EXCLUDED.payload_hash THEN now() ELSE c.changed_at END,
          task_id      = CASE WHEN c.payload_hash <> EXCLUDED.payload_hash THEN EXCLUDED.task_id ELSE c.task_id END,
          obs_date     = EXCLUDED.obs_date,
          thscode      = EXCLUDED.thscode,
          payload_hash = EXCLUDED.payload_hash
    RETURNING 1
  )
  SELECT ((SELECT count(*) FROM written) - (SELECT count(*) FROM existing))::int, (SELECT count(*) FROM versioned)::int
$$;
