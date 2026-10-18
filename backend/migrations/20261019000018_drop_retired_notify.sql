-- 通知推送功能已移除，清理 2026-10-18-00009 引入但不再使用的字段。
ALTER TABLE app.user DROP COLUMN IF EXISTS notify_webhook;
ALTER TABLE app.user DROP COLUMN IF EXISTS notify_error;
ALTER TABLE app.advice DROP COLUMN IF EXISTS notified_at;
