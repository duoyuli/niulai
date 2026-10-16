-- 用户在网页上维护的数据，不由同步写入。目前只有收藏的股票（自选股）。
CREATE SCHEMA IF NOT EXISTS app;

CREATE TABLE app.watchlist (
  thscode  TEXT PRIMARY KEY REFERENCES ref.security (thscode) ON DELETE CASCADE,
  added_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
