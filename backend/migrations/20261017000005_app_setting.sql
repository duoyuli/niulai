-- 在网页上配置的设置。目前只有同花顺金融数据服务的 API Key（name = 'hithink_api_key'），
-- 在首次登录引导中填写；环境变量 HITHINK_FINANCE_API_KEY 设置时优先使用环境变量。
-- 值以明文保存：能读这张表的人也能读全部数据，数据库的访问控制就是它的保护边界。
CREATE TABLE app.setting (
  name       TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
