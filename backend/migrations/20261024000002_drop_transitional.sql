-- 删掉只为旧数据、旧写入路径保留的过渡逻辑：同步层对股票和 ETF 都显式写 ref.instrument，不再需要触发器；
-- 研究运行都带完整配置；同步完成事件都带阶段。

-- ref.security → ref.instrument 的同步触发器：正常同步本来就显式写 ref.instrument
DROP TRIGGER security_instrument_sync ON ref.security;
DROP FUNCTION ref.sync_stock_instrument();

-- 没有保存完整配置的研究运行（2026-10-23 之前）：配置哈希的口径也和现在不同，留着只会多算试验次数
DELETE FROM ops.backtest_run WHERE config IS NULL;
ALTER TABLE ops.backtest_run ALTER COLUMN config SET NOT NULL;

-- 没有阶段的同步完成事件标记（2026-10-23 之前写入）：直接清掉，下一次同步或维护会照常刷新建议
UPDATE ops.run SET params = params - 'sync_event_pending'
WHERE params ? 'sync_event_pending' AND NOT (params->'sync_event_pending' ? 'stage');
