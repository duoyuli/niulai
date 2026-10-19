-- 账户卖出规则改为由策略负责，旧账户数据按新项目约定直接清理。
-- 账户、持仓、流水、建议及其明细均由外键级联删除。
DELETE FROM app.account;
-- 组合回测也使用旧的账户卖出规则，不能继续展示旧结果。
DELETE FROM app.preset_stat;
