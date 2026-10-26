-- 用户保存的多因子预设：在因子研究的「多因子组合」里把因子、权重、持有只数和缓冲区存成一个名字，
-- 账户设置和因子研究页都可以一键套用。每个用户各自一份，同名覆盖。
CREATE TABLE app.factor_preset (
  preset_id  BIGSERIAL PRIMARY KEY,
  user_id    BIGINT NOT NULL REFERENCES app.user (user_id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  factors    JSONB NOT NULL,            -- [{factor, weight}]，与账户规则的 factors 同格式
  top_n      INT NOT NULL,
  buffer     DOUBLE PRECISION NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX factor_preset_name_uniq ON app.factor_preset (user_id, lower(name));
