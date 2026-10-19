-- 账户保护线已移除；历史建议保留原始结果，新账户规则不再携带旧字段。
UPDATE app.account
SET rules = rules - 'protection_stop_loss_pct' - 'stop_loss_pct' - 'trend_ma'
WHERE rules ? 'protection_stop_loss_pct'
   OR rules ? 'stop_loss_pct'
   OR rules ? 'trend_ma';
