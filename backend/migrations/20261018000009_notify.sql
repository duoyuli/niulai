-- 运营建议推送：每个用户一个群机器人地址（企业微信、飞书、钉钉），最近一次推送失败的原因
ALTER TABLE app.user ADD COLUMN notify_webhook TEXT;
ALTER TABLE app.user ADD COLUMN notify_error TEXT;
-- 这份建议推送过的时间：同一个账户同一个交易日只推一次
ALTER TABLE app.advice ADD COLUMN notified_at TIMESTAMPTZ;
