-- 量化研究可追溯性、历史证券状态和影子执行记录。

CREATE TABLE ref.instrument_history (
  thscode       TEXT NOT NULL REFERENCES ref.instrument (thscode) ON DELETE CASCADE,
  valid_from    DATE NOT NULL,
  valid_to      DATE,
  name          TEXT NOT NULL,
  asset_type    TEXT NOT NULL CHECK (asset_type IN ('stock', 'etf')),
  exchange      TEXT NOT NULL,
  board         TEXT NOT NULL,
  list_date     DATE,
  delisted_on   DATE,
  recorded_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (thscode, valid_from),
  CHECK (valid_to IS NULL OR valid_to > valid_from)
);
CREATE INDEX instrument_history_asof_idx ON ref.instrument_history (thscode, valid_from, valid_to);

INSERT INTO ref.instrument_history (thscode, valid_from, name, asset_type, exchange, board, list_date, delisted_on)
SELECT thscode, COALESCE(first_seen, CURRENT_DATE), name, asset_type, exchange, board, list_date, delisted_on
FROM ref.instrument
ON CONFLICT (thscode, valid_from) DO NOTHING;

CREATE OR REPLACE FUNCTION ref.record_instrument_history()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'UPDATE' AND ROW(OLD.name, OLD.asset_type, OLD.exchange, OLD.board, OLD.list_date, OLD.delisted_on)
      IS NOT DISTINCT FROM ROW(NEW.name, NEW.asset_type, NEW.exchange, NEW.board, NEW.list_date, NEW.delisted_on) THEN
    RETURN NEW;
  END IF;
  UPDATE ref.instrument_history
     SET valid_to = CURRENT_DATE
   WHERE thscode = NEW.thscode AND valid_to IS NULL AND valid_from < CURRENT_DATE;
  INSERT INTO ref.instrument_history (thscode, valid_from, name, asset_type, exchange, board, list_date, delisted_on)
  VALUES (NEW.thscode, CURRENT_DATE, NEW.name, NEW.asset_type, NEW.exchange, NEW.board, NEW.list_date, NEW.delisted_on)
  ON CONFLICT (thscode, valid_from) DO UPDATE SET
    name = EXCLUDED.name, asset_type = EXCLUDED.asset_type, exchange = EXCLUDED.exchange,
    board = EXCLUDED.board, list_date = EXCLUDED.list_date, delisted_on = EXCLUDED.delisted_on;
  RETURN NEW;
END;
$$;
CREATE TRIGGER instrument_history_record
AFTER INSERT OR UPDATE ON ref.instrument
FOR EACH ROW EXECUTE FUNCTION ref.record_instrument_history();

CREATE TABLE ops.backtest_run (
  run_id          BIGSERIAL PRIMARY KEY,
  strategy_id     BIGINT,
  account_id      BIGINT,
  mode            TEXT NOT NULL CHECK (mode IN ('single', 'portfolio', 'account')),
  start_date      DATE NOT NULL,
  end_date        DATE NOT NULL,
  definition_hash TEXT NOT NULL,
  manifest        JSONB NOT NULL,
  summary         JSONB NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX backtest_run_lookup_idx ON ops.backtest_run (strategy_id, created_at DESC);

CREATE TABLE app.shadow_order (
  shadow_order_id BIGSERIAL PRIMARY KEY,
  account_id      BIGINT NOT NULL REFERENCES app.account (account_id) ON DELETE CASCADE,
  advice_id       BIGINT REFERENCES app.advice (advice_id) ON DELETE SET NULL,
  item_key        TEXT NOT NULL,
  trade_date      DATE NOT NULL,
  thscode         TEXT NOT NULL REFERENCES ref.instrument (thscode),
  side            TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
  planned_shares  BIGINT NOT NULL CHECK (planned_shares > 0),
  reference_price NUMERIC(18,6) NOT NULL CHECK (reference_price > 0),
  expected_price  NUMERIC(18,6),
  actual_shares   BIGINT,
  actual_price    NUMERIC(18,6),
  expected_slippage_bps NUMERIC(12,4),
  actual_slippage_bps   NUMERIC(12,4),
  status          TEXT NOT NULL DEFAULT 'planned' CHECK (status IN ('planned', 'filled', 'partial', 'cancelled', 'unknown')),
  reason          TEXT NOT NULL DEFAULT '',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (advice_id, item_key)
);
CREATE INDEX shadow_order_account_date_idx ON app.shadow_order (account_id, trade_date DESC);
