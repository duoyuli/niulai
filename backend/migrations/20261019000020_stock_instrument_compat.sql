-- 兼容仍直接写入 ref.security 的旧运维和测试路径；正常同步同时显式写 ref.instrument。
CREATE OR REPLACE FUNCTION ref.sync_stock_instrument()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO ref.instrument (thscode, ticker, name, asset_type, exchange, board, list_date, first_seen, last_seen, delisted_on)
  VALUES (NEW.thscode, NEW.ticker, NEW.name, 'stock', NEW.exchange, NEW.board, NEW.list_date, NEW.first_seen, NEW.last_seen, NEW.delisted_on)
  ON CONFLICT (thscode) DO UPDATE SET
    ticker = EXCLUDED.ticker, name = EXCLUDED.name, asset_type = 'stock', exchange = EXCLUDED.exchange,
    board = EXCLUDED.board, list_date = EXCLUDED.list_date, first_seen = EXCLUDED.first_seen,
    last_seen = EXCLUDED.last_seen, delisted_on = EXCLUDED.delisted_on, updated_at = now();
  RETURN NEW;
END;
$$;

CREATE TRIGGER security_instrument_sync
AFTER INSERT OR UPDATE ON ref.security
FOR EACH ROW EXECUTE FUNCTION ref.sync_stock_instrument();
