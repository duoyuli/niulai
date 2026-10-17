-- 网页用户：不开放注册，由管理员添加并设置密码。规则见 docs/interfaces/rest_api.md#auth。
-- 第一个管理员由本迁移创建、还没有密码；程序启动时用 NIULAI_ADMIN_USER / NIULAI_ADMIN_PASSWORD 设置它的账号和密码（只在它还没有密码时）。
CREATE TABLE app.user (
  user_id       BIGSERIAL PRIMARY KEY,
  username      TEXT NOT NULL,
  password_hash TEXT NOT NULL,                      -- Argon2id 的 PHC 字符串；空字符串表示还没有设置密码，不能登录
  role          TEXT NOT NULL CHECK (role IN ('admin', 'member')),
  disabled      BOOLEAN NOT NULL DEFAULT false,     -- 停用后不能登录，数据保留
  must_change_password BOOLEAN NOT NULL DEFAULT false, -- 密码是管理员设置的或默认的：页面提醒本人修改
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- 用户名不区分大小写
CREATE UNIQUE INDEX user_name_uniq ON app.user (lower(username));

INSERT INTO app.user (username, password_hash, role) VALUES ('niulai', '', 'admin');

-- 自选股按用户保存，已有的归第一个管理员
ALTER TABLE app.watchlist ADD COLUMN user_id BIGINT REFERENCES app.user (user_id) ON DELETE CASCADE;
UPDATE app.watchlist SET user_id = (SELECT min(user_id) FROM app.user);
ALTER TABLE app.watchlist ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE app.watchlist DROP CONSTRAINT watchlist_pkey;
ALTER TABLE app.watchlist ADD PRIMARY KEY (user_id, thscode);
