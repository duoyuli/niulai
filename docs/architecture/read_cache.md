# 共享读取缓存与预热

本文说明页面读取缓存和首页结果预热的边界。数据同步、数据库表和指标计算的规则分别见 [同步流水线](sync_pipeline.md)、[数据库分层](data_model.md) 和 [领域文档](../domains/index.md)。

## 组成

`REDIS_URL` 配置后，`niulai serve` 使用 Redis 保存可重建的 JSON 页面结果；当前接入首页直接依赖的市场概况、今日热点和板块强弱。Redis 不保存账户、登录状态、API Key 或同步任务状态，PostgreSQL 仍是唯一事实来源。

未配置 Redis 时，服务继续使用已有的进程内缓存和 PostgreSQL。Redis 连接或读写失败也只记录调试或告警日志，并回退到数据库计算，不能让页面请求因为缓存故障失败。

## 键与失效

缓存键使用 `niulai:v1:` 前缀，并包含 `ops.derived_version` 中 `adj_factor`、`limit_pool` 的时间戳（`analysis::versions::CacheVersions::market_token`）、交易日和接口参数，由 `cache::derived_key` 统一生成。接口和预热都通过 `Cache::cached_json` / `warm_json` 读写：先读缓存，没有再计算并回填，Redis 故障只记调试日志，键格式、TTL 和降级只在 `backend/src/cache.rs` 一处。行情或权威涨跌停池发生变化时，新的版本自然生成新键；旧键由 Redis TTL 和 `allkeys-lru` 回收，不执行影响业务数据的删除操作。默认 TTL 为 15 分钟。

## 页面预热

启用 Redis 时，服务启动一个后台预热组件（`backend/src/precompute.rs`，由 `server` 监督，计算在 `analysis`，占后台的全市场计算槽位，不和页面请求抢前台槽位）。它启动后立即计算最新交易日的市场概况、行业和概念板块近 5 日排名、今日热点，之后每 5 分钟检查一次。预热只写入缺失键；没有行情时不做计算。预热失败会在下一轮重试，API 请求仍可按需计算并回填缓存。

## 部署

Docker Compose 默认启动一个无持久化的 Redis 容器，使用 `REDIS_URL=redis://redis:6379`。Redis 是加速层，删除其容器或清空数据不会影响 PostgreSQL；服务器资源不足时可以去掉 `REDIS_URL`，服务仍然可用但首次读取会重新计算。
