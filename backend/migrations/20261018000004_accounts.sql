-- 账户、持仓、流水与每日调仓建议，见 docs/domains/accounts.md。账户只属于一个用户，别人（包括管理员）看不到。

-- 内置策略：每个模板一行，启动时按模板内容同步，只读、不能删，总是参加巡检；账户的风格引用它们
ALTER TABLE app.strategy ADD COLUMN builtin_key TEXT UNIQUE;
-- 原来「第一次启动时把模板复制成普通策略」的标记：内置策略取代了这种复制
DELETE FROM app.setting WHERE name = 'strategies_seeded';

CREATE TABLE app.account (
  account_id      BIGSERIAL PRIMARY KEY,
  user_id         BIGINT NOT NULL REFERENCES app.user (user_id) ON DELETE CASCADE,
  name            TEXT NOT NULL,
  style           TEXT NOT NULL CHECK (style IN ('conservative', 'balanced', 'aggressive')),
  overrides       JSONB NOT NULL DEFAULT '{}',          -- 高级设置中改过的风格参数
  cash            NUMERIC(18,2) NOT NULL CHECK (cash >= 0),
  commission_rate NUMERIC(8,6) NOT NULL DEFAULT 0.00025,
  min_commission  NUMERIC(8,2) NOT NULL DEFAULT 5,
  note            TEXT NOT NULL DEFAULT '',
  checked_at      TIMESTAMPTZ,                          -- 最近一次确认「和券商一致」
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX account_name_uniq ON app.account (user_id, lower(name));

CREATE TABLE app.position (
  account_id  BIGINT NOT NULL REFERENCES app.account (account_id) ON DELETE CASCADE,
  thscode     TEXT NOT NULL REFERENCES ref.security (thscode),
  shares      BIGINT NOT NULL CHECK (shares > 0),       -- 送转后可能有零股，不要求整手
  cost_price  NUMERIC(18,4) CHECK (cost_price > 0),     -- 每股成本（未复权，分红送转后摊薄），可空
  opened_on   DATE,                                     -- 买入日，可空
  strategy_id BIGINT REFERENCES app.strategy (strategy_id) ON DELETE SET NULL,  -- 按哪个策略的买入建议买的
  since       DATE NOT NULL,                            -- 股数最近一次变动的日期：之后的除权除息才自动记账
  PRIMARY KEY (account_id, thscode)
);

-- 资金和持仓的每一次变动。持仓表和现金是它的结果，两者在同一个事务里更新
CREATE TABLE app.account_trade (
  trade_id    BIGSERIAL PRIMARY KEY,
  account_id  BIGINT NOT NULL REFERENCES app.account (account_id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('buy', 'sell', 'dividend', 'bonus', 'cash_in', 'cash_out', 'adjust')),
  trade_date  DATE NOT NULL,
  thscode     TEXT,
  shares      BIGINT,                                   -- 持仓股数的变化：买入、送转为正，卖出为负
  price       NUMERIC(18,4),
  amount      NUMERIC(18,2) NOT NULL,                   -- 现金的变化（含费用）：买入为负，卖出、分红为正
  fee         NUMERIC(18,2) NOT NULL DEFAULT 0,
  source      TEXT NOT NULL CHECK (source IN ('user', 'advice', 'corp_action')),
  note        TEXT NOT NULL DEFAULT '',
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX account_trade_idx ON app.account_trade (account_id, trade_id DESC);

-- 每个账户每个交易日一份建议；持仓、现金或设置改了就重新生成，旧的一份标记被取代，不删除
CREATE TABLE app.advice (
  advice_id     BIGSERIAL PRIMARY KEY,
  account_id    BIGINT NOT NULL REFERENCES app.account (account_id) ON DELETE CASCADE,
  trade_date    DATE NOT NULL,                          -- 依据的收盘日
  superseded_by BIGINT REFERENCES app.advice (advice_id) ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED,  -- 先标记旧的、再插入新的，提交时才检查
  rules_version INTEGER NOT NULL,                       -- 生成规则的版本：规则改了不改写历史
  input         JSONB NOT NULL,                         -- 生成时的全部输入：现金、持仓、风格参数、市场、策略表现
  result        JSONB NOT NULL,                         -- 结论、各条建议、执行后预计、没选上的候选
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX advice_current_idx ON app.advice (account_id, trade_date) WHERE superseded_by IS NULL;

-- 用户对每条建议的处理：已执行（对应一笔流水）或不做
CREATE TABLE app.advice_item_state (
  advice_id  BIGINT NOT NULL REFERENCES app.advice (advice_id) ON DELETE CASCADE,
  item_key   TEXT NOT NULL,                             -- 例：buy-601012.SH
  state      TEXT NOT NULL CHECK (state IN ('executed', 'skipped')),
  trade_id   BIGINT REFERENCES app.account_trade (trade_id) ON DELETE SET NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (advice_id, item_key)
);

-- 内置策略过去 3 年在全市场的表现，建议的理由引用它；每周或策略定义变化后重算
CREATE TABLE app.strategy_stat (
  strategy_id     BIGINT PRIMARY KEY REFERENCES app.strategy (strategy_id) ON DELETE CASCADE,
  definition_hash TEXT NOT NULL,
  end_date        DATE NOT NULL,
  stats           JSONB NOT NULL,
  computed_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 三种风格过去 3 年的组合回测，风格卡片上展示；和策略表现一起每周重算
CREATE TABLE app.style_stat (
  style       TEXT PRIMARY KEY,
  params_hash TEXT NOT NULL,
  end_date    DATE NOT NULL,
  result      JSONB NOT NULL,
  computed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
