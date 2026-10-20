# ADR：网页登录（部分被取代）

- 状态：部分被取代（superseded），2026-10-16。
- 取代关系：用户、密码和角色由 [ADR：多用户登录与角色](multi_user_login.md) 定义；本 ADR 只保留会话 Cookie 和统一拦截的决定。
- 当前实现：[登录与权限](../interfaces/rest_api.md#auth)、[配置](../interfaces/configuration.md#settings)。

## 保留的决策

- 后端中间件统一拦截需要登录的 `/api/v1/*` 接口；健康检查、登录、退出、OpenAPI 和监控指标按接口约定放行。
- 登录成功后使用签名 Cookie，不在服务端保存会话列表。Cookie 使用 `HttpOnly`、`SameSite=Lax`，有效期为 30 天；签名密钥保存在 `app.setting`。
- 项目不负责 HTTPS 和公网入口。生产部署应在项目外部限制网络暴露，并按部署环境配置安全传输。

## 已取代的决定

早期方案只有一个由环境变量提供的管理员账户，并把默认密码、修改密码和密码存储都绑定在该方案上。现在由数据库用户、管理员与普通用户角色、密码哈希和用户数据隔离取代；详情以 [多用户登录与角色](multi_user_login.md) 和 [REST API](../interfaces/rest_api.md#auth) 为准。
