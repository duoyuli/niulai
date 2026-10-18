-- 配股：流水新增 rights 类型（用户认购配股），每个账户对每个公司行动记录处理状态。
-- 自动记账只处理现金分红和送转；配股要不要认购由用户决定，所以记成提醒，处理过就不再提醒。
ALTER TABLE app.account_trade DROP CONSTRAINT account_trade_kind_check;
ALTER TABLE app.account_trade ADD CONSTRAINT account_trade_kind_check
  CHECK (kind IN ('buy', 'sell', 'dividend', 'bonus', 'cash_in', 'cash_out', 'adjust', 'rights'));

-- 公司行动的处理状态：目前只有配股。subscribed 表示记了认购流水，ignored 表示用户选了「暂不处理」。
-- 同一只股票同一个除权日只有一条，提醒按它判断是否已经处理过，避免重复提醒、重复记账。
CREATE TABLE app.account_corp_action (
  account_id BIGINT NOT NULL REFERENCES app.account (account_id) ON DELETE CASCADE,
  thscode    TEXT NOT NULL,
  ex_date    DATE NOT NULL,
  kind       TEXT NOT NULL CHECK (kind IN ('rights')),
  state      TEXT NOT NULL CHECK (state IN ('subscribed', 'ignored')),
  shares     BIGINT,                                     -- 认购了多少股（subscribed 时）
  trade_id   BIGINT REFERENCES app.account_trade (trade_id) ON DELETE SET NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (account_id, thscode, ex_date, kind)
);

-- 每个账户每个交易日的净值快照：现金、持仓市值、总资产、当日盈亏。
-- 由净值接口按有效流水和当天收盘价重算并 upsert，补记或撤销流水后历史也会跟着修正。
CREATE TABLE app.account_equity (
  account_id   BIGINT NOT NULL REFERENCES app.account (account_id) ON DELETE CASCADE,
  trade_date   DATE NOT NULL,
  cash         NUMERIC(18,2) NOT NULL,
  market_value NUMERIC(18,2) NOT NULL,
  total_assets NUMERIC(18,2) NOT NULL,
  day_pnl      NUMERIC(18,2) NOT NULL DEFAULT 0,          -- 已经剔除当天的转入转出
  updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (account_id, trade_date)
);
