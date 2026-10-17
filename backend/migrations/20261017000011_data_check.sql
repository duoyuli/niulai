-- 数据核检：按证券 × 交易日检查日线有没有缺，见 docs/architecture/sync_pipeline.md#data_check。

-- 发现的缺口。重取后补上了就删掉；重取后接口仍然没有这一天，确认为停牌（供应商停牌日不返回日线），以后不再报
CREATE TABLE ops.bar_gap (
  dataset_id  TEXT NOT NULL,                  -- a_share.bar_1d / index.bar_1d
  thscode     TEXT NOT NULL,
  trade_date  DATE NOT NULL,
  status      TEXT NOT NULL CHECK (status IN ('open', 'suspended')),
  found_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  refetch_run BIGINT,                         -- 为它重取的运行；为空表示还没安排
  PRIMARY KEY (dataset_id, thscode, trade_date)
);
CREATE INDEX bar_gap_open_idx ON ops.bar_gap (dataset_id, thscode) WHERE status = 'open';

-- 每次核检的结果
CREATE TABLE ops.data_check (
  check_id    BIGSERIAL PRIMARY KEY,
  trigger     TEXT NOT NULL,                  -- weekly / followup / manual
  started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at TIMESTAMPTZ,
  end_date    DATE,                           -- 查到哪个交易日为止
  result      JSONB,                          -- 每个数据集：证券数、证券交易日数、新发现、已补回、确认停牌、仍缺、重取的运行
  error       TEXT
);
