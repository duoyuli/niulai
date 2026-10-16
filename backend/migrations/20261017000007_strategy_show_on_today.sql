-- 策略是否在「今日」页展示：只为测试建的策略可以照常巡检，但不出现在首页。见 docs/domains/strategies.md#screening
ALTER TABLE app.strategy ADD COLUMN show_on_today BOOLEAN NOT NULL DEFAULT true;
