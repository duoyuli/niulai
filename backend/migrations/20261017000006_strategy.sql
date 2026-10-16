-- 选股策略与每日巡检结果，见 docs/domains/strategies.md。
-- 策略是一段 JSON 定义（股票池、条件、卖出规则、排序），不是代码；内置模板在程序中，首次启动时复制为普通策略。
CREATE TABLE app.strategy (
  strategy_id BIGSERIAL PRIMARY KEY,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  definition  JSONB NOT NULL,
  screening   BOOLEAN NOT NULL DEFAULT true,     -- 是否参加每日巡检
  template    TEXT,                              -- 由哪个内置模板创建（只作记录）
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 一个策略在一个交易日的巡检。重新巡检同一天时覆盖
CREATE TABLE app.screen_run (
  run_id      BIGSERIAL PRIMARY KEY,
  strategy_id BIGINT NOT NULL REFERENCES app.strategy (strategy_id) ON DELETE CASCADE,
  trade_date  DATE NOT NULL,
  scanned     INTEGER NOT NULL,                  -- 当天有日线、进入股票池的股票数
  matched     INTEGER NOT NULL,                  -- 满足条件的股票数（排序截断前）
  ran_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (strategy_id, trade_date)
);

-- 入选的股票。reasons 是命中条件的白话说明，metrics 是当天的关键数值
CREATE TABLE app.screen_hit (
  run_id   BIGINT NOT NULL REFERENCES app.screen_run (run_id) ON DELETE CASCADE,
  thscode  TEXT NOT NULL,
  rank     INTEGER NOT NULL,
  is_new   BOOLEAN NOT NULL,                     -- 上一次巡检时不在结果中
  reasons  JSONB NOT NULL,
  metrics  JSONB NOT NULL,
  PRIMARY KEY (run_id, thscode)
);
