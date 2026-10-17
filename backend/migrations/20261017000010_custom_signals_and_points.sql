-- 自定义信号：起了名字的条件，见 docs/domains/strategies.md#signals。
-- definition 是 {"match_all": true, "conditions": [...]}，条件格式与策略相同，只能用指标和内置信号
CREATE TABLE app.signal (
  signal_id   BIGSERIAL PRIMARY KEY,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  bullish     BOOLEAN NOT NULL DEFAULT true,     -- 偏多（true）或偏空
  definition  JSONB NOT NULL,
  based_on    TEXT,                              -- 从哪个内置信号复制来（只作记录）
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 巡检结果分三类：今日买点、今日卖点、持有中，见 docs/domains/strategies.md#points。
-- 原来的结果都是「今天满足买入条件」，归为 buy
ALTER TABLE app.screen_hit ADD COLUMN kind TEXT NOT NULL DEFAULT 'buy' CHECK (kind IN ('buy', 'sell', 'holding'));
