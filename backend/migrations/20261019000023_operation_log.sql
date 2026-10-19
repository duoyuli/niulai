-- 操作日志目前只记录成功登录，保留登录用户、来源地址和设备（User-Agent）。
CREATE TABLE app.operation_log (
  log_id      BIGSERIAL PRIMARY KEY,
  user_id     BIGINT REFERENCES app.user (user_id) ON DELETE SET NULL,
  username    TEXT NOT NULL,
  action      TEXT NOT NULL CHECK (action = 'login'),
  ip_address  TEXT NOT NULL,
  device      TEXT NOT NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX operation_log_created_idx ON app.operation_log (created_at DESC, log_id DESC);
