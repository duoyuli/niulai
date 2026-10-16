# niulai 前端

React 单页应用，页面和接口的对应关系见 [前端](../docs/architecture/frontend.md)，选型见 [ADR：前端技术选型](../docs/decisions/frontend_stack.md)。

需要 Node.js 24 和 pnpm 12。

```bash
pnpm install
pnpm dev          # http://localhost:5173/，/api 转发到 http://127.0.0.1:8080
pnpm typecheck && pnpm lint && pnpm test
pnpm build        # 产物在 dist/，由 niulai serve 通过 NIULAI_WEB_DIR 提供
```

后端接口变更后重新生成客户端：

```bash
pnpm api:spec     # 运行 niulai openapi，更新 openapi.json
pnpm api:gen      # 生成 src/api/generated/
pnpm typecheck
```
