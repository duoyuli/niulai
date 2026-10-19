-- ETF 与股票使用独立的事实数据表；ref.instrument 只负责跨资产路由和用户数据外键。
CREATE TABLE ref.instrument (
  thscode     TEXT PRIMARY KEY,
  ticker      TEXT NOT NULL,
  name        TEXT NOT NULL,
  asset_type  TEXT NOT NULL CHECK (asset_type IN ('stock', 'etf')),
  exchange    TEXT NOT NULL CHECK (exchange IN ('SH', 'SZ', 'BJ')),
  board       TEXT NOT NULL DEFAULT '',
  list_date   DATE,
  first_seen  DATE NOT NULL,
  last_seen   DATE NOT NULL,
  delisted_on DATE,
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO ref.instrument (thscode, ticker, name, asset_type, exchange, board, list_date, first_seen, last_seen, delisted_on)
SELECT thscode, ticker, name, 'stock', exchange, board, list_date, first_seen, last_seen, delisted_on
FROM ref.security
ON CONFLICT (thscode) DO NOTHING;

CREATE TABLE ref.etf_security (
  thscode     TEXT PRIMARY KEY REFERENCES ref.instrument (thscode) ON DELETE CASCADE,
  ticker      TEXT NOT NULL,
  name        TEXT NOT NULL,
  exchange    TEXT NOT NULL CHECK (exchange IN ('SH', 'SZ')),
  list_date   DATE,
  first_seen  DATE NOT NULL,
  last_seen   DATE NOT NULL,
  delisted_on DATE,
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE mkt.etf_bar_1d (
  thscode     TEXT NOT NULL REFERENCES ref.etf_security (thscode) ON DELETE CASCADE,
  trade_date  DATE NOT NULL,
  open        NUMERIC(18,6) NOT NULL,
  high        NUMERIC(18,6) NOT NULL,
  low         NUMERIC(18,6) NOT NULL,
  close       NUMERIC(18,6) NOT NULL,
  volume      BIGINT NOT NULL,
  amount      NUMERIC(24,2) NOT NULL,
  pre_close   NUMERIC(18,6),
  source      TEXT NOT NULL CHECK (source IN ('hist', 'snapshot')),
  price_basis TEXT NOT NULL CHECK (price_basis IN ('vendor_forward_adjusted', 'market_snapshot')),
  run_id      BIGINT NOT NULL,
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (thscode, trade_date)
);
CREATE INDEX etf_bar_1d_date_idx ON mkt.etf_bar_1d (trade_date);

ALTER TABLE app.watchlist DROP CONSTRAINT IF EXISTS watchlist_thscode_fkey;
ALTER TABLE app.watchlist ADD CONSTRAINT watchlist_thscode_fkey FOREIGN KEY (thscode) REFERENCES ref.instrument (thscode) ON DELETE CASCADE;
ALTER TABLE app.position DROP CONSTRAINT IF EXISTS position_thscode_fkey;
ALTER TABLE app.position ADD CONSTRAINT position_thscode_fkey FOREIGN KEY (thscode) REFERENCES ref.instrument (thscode);
