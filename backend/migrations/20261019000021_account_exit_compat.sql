-- 账户退出规则改为由买入策略负责，账户只保留可选保护线。
-- 旧账户数据继续保留：把旧的止损线迁移为保护线，避免升级时丢失持仓、流水和建议。
UPDATE app.account
SET rules = (rules - 'stop_loss_pct' - 'trend_ma')
    || CASE
         WHEN jsonb_typeof(rules->'stop_loss_pct') = 'number'
           THEN jsonb_build_object('protection_stop_loss_pct', rules->'stop_loss_pct')
         ELSE '{}'::jsonb
       END
WHERE rules ? 'stop_loss_pct' OR rules ? 'trend_ma';

-- 参考值回测是旧退出口径的派生结果，清掉后由用户主动研究时重新生成。
DELETE FROM app.preset_stat;
