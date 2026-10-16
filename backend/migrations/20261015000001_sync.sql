-- niulai 同步框架。PostgreSQL 16。与采集程序的实现语言无关，程序只需按这里的约定读写。
--
--   config/datasets.yaml ──加载──► ops.dataset          每个数据集一份声明式配置
--                                     │ 规划器（planner）
--                                     ▼
--                                ops.task              待执行的请求单元（某只股票、某个交易日、某个报告期……）
--                                     │ 执行器（worker）用 FOR UPDATE SKIP LOCKED 认领
--                                     ▼
--        HTTP 调用 ──► ops.api_call（调用日志） + raw.api_page（可选，保留原始响应）
--                                     │ 按配置拆出记录、算主键
--                                     ▼
--                     raw.record（每个数据集每个主键一行，最新值）
--                     raw.record_version（值变化时追加，保留修订历史）
--                                     │ 投影（直接写入，或从 raw.record 物化）
--                                     ▼
--                     ref.* / mkt.* / fin.* 类型化表（见 002_market.sql）
--
-- 设计目标：所有可用的数据集从第一天起都能同步进 raw.record。
-- 暂时不用的数据不需要建专表，以后需要时直接从 raw.record 解析，不必重新拉取。

CREATE SCHEMA IF NOT EXISTS ops;
CREATE SCHEMA IF NOT EXISTS raw;

-- 供应商时间戳是 Asia/Shanghai 当地零点的毫秒数（含 1986–1991 年的夏令时）。
-- 必须按 Asia/Shanghai 时区换算，不能用固定 +8 小时，否则夏令时期间的日期会早一天。
-- 这里多加 1 小时作为保险，结果仍落在同一天。
CREATE OR REPLACE FUNCTION ops.ms_to_cn_date(ms BIGINT) RETURNS DATE
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
  SELECT (to_timestamp((ms + 3600000) / 1000.0) AT TIME ZONE 'Asia/Shanghai')::date
$$;

-- ───────────────────────── 配置 ─────────────────────────

-- 数据集注册表。程序启动时把 config/datasets.yaml 同步到这里，spec 保存完整配置。
CREATE TABLE ops.dataset (
  dataset_id   TEXT PRIMARY KEY,                -- 例：a_share.bar_1d
  domain       TEXT NOT NULL,                   -- meta / a_share / index / special / fund / futures / options / hk_ipo
  endpoint     TEXT NOT NULL,                   -- /api/a-share/prices/historical
  priority     TEXT NOT NULL CHECK (priority IN ('P0', 'P1', 'P2', 'P3')),
  enabled      BOOLEAN NOT NULL DEFAULT true,
  spec         JSONB NOT NULL,                  -- 分区方式、参数模板、分页、主键、回补、调度、写入目标
  spec_hash    TEXT NOT NULL,                   -- 配置变化时据此决定是否重新规划
  updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ───────────────────────── 运行与任务 ─────────────────────────

-- 一次运行 = 一次规划的批次（例：2026-10-15 的 T+1 增量、某数据集的首次回补）
CREATE TABLE ops.run (
  run_id       BIGSERIAL PRIMARY KEY,
  kind         TEXT NOT NULL CHECK (kind IN ('backfill', 'incremental', 'repair', 'calc')),
  dataset_id   TEXT REFERENCES ops.dataset,     -- NULL = 跨数据集（例：每日调度）
  trade_date   DATE,                            -- 增量运行对应的交易日
  params       JSONB NOT NULL DEFAULT '{}',
  status       TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'succeeded', 'failed', 'partial', 'cancelled')),
  tasks_total  INTEGER NOT NULL DEFAULT 0,
  tasks_done   INTEGER NOT NULL DEFAULT 0,
  tasks_failed INTEGER NOT NULL DEFAULT 0,
  started_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at  TIMESTAMPTZ,
  error        TEXT
);

-- 任务：一个请求单元，内部可以翻多页。part_key 唯一标识分区，例如
--   'thscode=600519.SH|window=2005-01-01..2014-12-31'、'date=2026-10-14'、'asset_type=futures'
CREATE TABLE ops.task (
  task_id      BIGSERIAL PRIMARY KEY,
  run_id       BIGINT NOT NULL REFERENCES ops.run,
  dataset_id   TEXT NOT NULL REFERENCES ops.dataset,
  part_key     TEXT NOT NULL,
  params       JSONB NOT NULL,                  -- 已展开的请求参数（不含 API Key）
  priority     SMALLINT NOT NULL DEFAULT 100,   -- 越小越先执行
  status       TEXT NOT NULL DEFAULT 'pending'
               CHECK (status IN ('pending', 'running', 'done', 'empty', 'failed', 'dead', 'skipped')),
  attempts     SMALLINT NOT NULL DEFAULT 0,
  max_attempts SMALLINT NOT NULL DEFAULT 4,
  not_before   TIMESTAMPTZ NOT NULL DEFAULT now(),   -- 退避重试时推迟
  locked_by    TEXT,
  locked_at    TIMESTAMPTZ,
  pages        INTEGER NOT NULL DEFAULT 0,
  records      INTEGER NOT NULL DEFAULT 0,
  changed      INTEGER NOT NULL DEFAULT 0,           -- 新增或值发生变化的记录数
  last_api_code INTEGER,
  last_error   TEXT,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at  TIMESTAMPTZ
);
-- 同一分区同时只能有一个未完成任务，规划器重复规划时自然去重
CREATE UNIQUE INDEX task_open_uniq ON ops.task (dataset_id, part_key) WHERE status IN ('pending', 'running', 'failed');
CREATE INDEX task_claim_idx ON ops.task (priority, not_before) WHERE status IN ('pending', 'failed');
CREATE INDEX task_run_idx ON ops.task (run_id);
CREATE INDEX task_dataset_idx ON ops.task (dataset_id, status);

-- 执行器认领任务的标准写法（任何语言都可以照抄）：
--   UPDATE ops.task SET status = 'running', locked_by = $worker, locked_at = now(), attempts = attempts + 1
--   WHERE task_id = (SELECT task_id FROM ops.task
--                    WHERE status IN ('pending', 'failed') AND not_before <= now()
--                    ORDER BY priority, task_id FOR UPDATE SKIP LOCKED LIMIT 1)
--   RETURNING *;
-- 执行器崩溃导致 running 超时的任务，用 ops.requeue_stale() 放回队列。
CREATE OR REPLACE FUNCTION ops.requeue_stale(p_timeout INTERVAL DEFAULT '10 minutes') RETURNS INTEGER
LANGUAGE sql AS $$
  WITH r AS (
    UPDATE ops.task SET status = CASE WHEN attempts >= max_attempts THEN 'dead' ELSE 'failed' END,
                        locked_by = NULL, last_error = coalesce(last_error, '') || ' [stale lock]'
    WHERE status = 'running' AND locked_at < now() - p_timeout RETURNING 1)
  SELECT count(*)::int FROM r
$$;

-- 每个数据集、每个分区的进度游标。规划增量时从这里接着往后算。
--   例：{"synced_to": "2026-10-15"}、{"last_report": "2026-2"}、{"pages_seen": 37}
CREATE TABLE ops.cursor (
  dataset_id TEXT NOT NULL REFERENCES ops.dataset,
  part_key   TEXT NOT NULL,                   -- 不分区的数据集用 '*'
  state      JSONB NOT NULL,
  task_id    BIGINT,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (dataset_id, part_key)
);

-- HTTP 调用日志：每次请求一行，用于排查、限流分析和统计供应商稳定性。按月分区，保留 90 天。
CREATE TABLE ops.api_call (
  call_id     BIGSERIAL,
  task_id     BIGINT,
  dataset_id  TEXT NOT NULL,
  endpoint    TEXT NOT NULL,
  params      JSONB NOT NULL,
  page        INTEGER,
  http_status SMALLINT,
  api_code    INTEGER,                         -- 响应信封里的 code；非 JSON 响应为 NULL
  message     TEXT,
  request_id  TEXT,                            -- 供应商追踪号，报障时需要
  records     INTEGER,
  bytes       INTEGER,
  latency_ms  INTEGER,
  called_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (call_id, called_at)
) PARTITION BY RANGE (called_at);
CREATE TABLE ops.api_call_default PARTITION OF ops.api_call DEFAULT;

-- ───────────────────────── 原始数据 ─────────────────────────

-- 原始响应存档（可选，配置项 keep_raw: true 时写入）。用于审计，以及解析规则改变后重放。
-- 日线这类大数据集不存（全量约 1GB JSON），小数据集默认都存。
CREATE TABLE raw.api_page (
  call_id    BIGINT NOT NULL,
  dataset_id TEXT NOT NULL,
  part_key   TEXT NOT NULL,
  page       INTEGER NOT NULL DEFAULT 1,
  body       JSONB NOT NULL,                   -- 完整响应信封
  fetched_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (dataset_id, part_key, page, fetched_at)
);

-- 通用记录库：每个数据集、每个主键一行，保存最新值。
-- rkey 由配置里的 key 模板生成。按交易日观察的快照类数据集，日期是主键的一部分，
-- 例：'2026-10-14|600519.SH'；参考类数据集只用自然键，例：'600519.SH'。
-- 按 dataset_id 分区，大数据集可以单独分区、单独维护。
CREATE TABLE raw.record (
  dataset_id    TEXT NOT NULL,
  rkey          TEXT NOT NULL,
  obs_date      DATE,                           -- 记录对应的业务日期（交易日、报告期、除权日……），便于按日期查询
  thscode       TEXT,                           -- 记录关联的证券（若有），便于按证券查询
  payload       JSONB NOT NULL,                 -- 单条记录，字段保持供应商原样
  payload_hash  TEXT NOT NULL,                  -- md5(payload::text)，用于判断记录是否变化
  first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),   -- 最近一次在响应中出现，用于判断是否已下架
  changed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  task_id       BIGINT,
  PRIMARY KEY (dataset_id, rkey)
) PARTITION BY LIST (dataset_id);
CREATE TABLE raw.record_default PARTITION OF raw.record DEFAULT;
CREATE INDEX record_thscode_idx ON raw.record (dataset_id, thscode, obs_date);
CREATE INDEX record_obs_date_idx ON raw.record (dataset_id, obs_date);

-- 修订历史：记录的值变化时，先把旧值存到这里，再覆盖 raw.record。
-- 用来发现供应商事后修改的数据，例如财报修订、行情更正、代码表改名。
CREATE TABLE raw.record_version (
  dataset_id   TEXT NOT NULL,
  rkey         TEXT NOT NULL,
  valid_from   TIMESTAMPTZ NOT NULL,          -- 旧值的 changed_at
  valid_to     TIMESTAMPTZ NOT NULL,          -- 被替换的时刻
  payload      JSONB NOT NULL,
  payload_hash TEXT NOT NULL,
  task_id      BIGINT,
  PRIMARY KEY (dataset_id, rkey, valid_from)
);

-- 写入一批记录的标准入口（任何语言都可以调用）：
--   SELECT * FROM raw.upsert_records($dataset_id, $task_id, $records::jsonb)
-- records 是 [{rkey, obs_date, thscode, payload}, ...] 形式的 JSON 数组。
-- 返回新增条数和变化条数；值没变的记录只刷新 last_seen_at。
CREATE OR REPLACE FUNCTION raw.upsert_records(p_dataset TEXT, p_task BIGINT, p_records JSONB,
                                              OUT inserted INTEGER, OUT changed INTEGER)
LANGUAGE plpgsql AS $$
BEGIN
  CREATE TEMP TABLE IF NOT EXISTS _in (rkey TEXT PRIMARY KEY, obs_date DATE, thscode TEXT, payload JSONB, h TEXT) ON COMMIT DROP;
  TRUNCATE _in;
  -- 同一批里重复的 rkey 只保留最后一条，与供应商口径一致
  INSERT INTO _in
  SELECT DISTINCT ON (r->>'rkey') r->>'rkey', (r->>'obs_date')::date, r->>'thscode', r->'payload', md5((r->'payload')::text)
  FROM jsonb_array_elements(p_records) WITH ORDINALITY AS t(r, ord)
  ORDER BY r->>'rkey', ord DESC;

  SELECT count(*) INTO inserted FROM _in i
  WHERE NOT EXISTS (SELECT 1 FROM raw.record c WHERE c.dataset_id = p_dataset AND c.rkey = i.rkey);

  INSERT INTO raw.record_version (dataset_id, rkey, valid_from, valid_to, payload, payload_hash, task_id)
  SELECT c.dataset_id, c.rkey, c.changed_at, now(), c.payload, c.payload_hash, c.task_id
  FROM raw.record c JOIN _in i ON c.dataset_id = p_dataset AND c.rkey = i.rkey
  WHERE c.payload_hash <> i.h;
  GET DIAGNOSTICS changed = ROW_COUNT;

  INSERT INTO raw.record AS c (dataset_id, rkey, obs_date, thscode, payload, payload_hash, task_id)
  SELECT p_dataset, rkey, obs_date, thscode, payload, h, p_task FROM _in
  ON CONFLICT (dataset_id, rkey) DO UPDATE
    SET last_seen_at = now(),
        payload      = CASE WHEN c.payload_hash <> EXCLUDED.payload_hash THEN EXCLUDED.payload ELSE c.payload END,
        changed_at   = CASE WHEN c.payload_hash <> EXCLUDED.payload_hash THEN now() ELSE c.changed_at END,
        task_id      = CASE WHEN c.payload_hash <> EXCLUDED.payload_hash THEN EXCLUDED.task_id ELSE c.task_id END,
        obs_date     = EXCLUDED.obs_date,
        thscode      = EXCLUDED.thscode,
        payload_hash = EXCLUDED.payload_hash;
END
$$;

-- 运行进度概览
CREATE VIEW ops.v_progress AS
SELECT t.run_id, t.dataset_id,
       count(*)                                         AS tasks,
       count(*) FILTER (WHERE status IN ('done', 'empty')) AS done,
       count(*) FILTER (WHERE status = 'pending')        AS pending,
       count(*) FILTER (WHERE status = 'running')        AS running,
       count(*) FILTER (WHERE status IN ('failed', 'dead')) AS failed,
       sum(records)                                     AS records,
       sum(changed)                                     AS changed
FROM ops.task t GROUP BY 1, 2;

-- ───────────────────────── 规划器辅助 ─────────────────────────

-- 证券范围（universe）：datasets.yaml 中 partition.*.universe 是作用在这个视图上的 WHERE 条件，
-- 例：dataset_id = 'meta.tickers' AND asset_type = 'a-share'
-- active：最近一次同步时仍在列表中（last_seen_at 与该数据集最新的 last_seen_at 相差不超过 1 天）
CREATE VIEW ops.v_universe AS
SELECT r.dataset_id, r.rkey,
       coalesce(r.thscode, r.payload->>'thscode') AS thscode,
       r.payload->>'asset_type'                   AS asset_type,
       CASE WHEN r.payload->>'list_date' ~ '^\d{4}-\d{2}-\d{2}$' THEN (r.payload->>'list_date')::date END AS list_date,
       r.obs_date, r.payload, r.last_seen_at,
       r.last_seen_at >= m.max_seen - interval '1 day' AS active
FROM raw.record r
JOIN (SELECT dataset_id, max(last_seen_at) AS max_seen FROM raw.record GROUP BY dataset_id) m USING (dataset_id);

-- 调度器状态：每个定时作业最近一次触发的日期，保证一天只触发一次（多个 worker 并存时也一样）
CREATE TABLE ops.scheduler (
  job        TEXT PRIMARY KEY,        -- 例：after_close
  last_fired DATE NOT NULL,
  run_id     BIGINT,
  fired_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
