import { defineConfig } from "orval";

// @project-doc docs/architecture/frontend.md#api_client
// 从后端导出的 OpenAPI（pnpm api:spec）生成类型和 TanStack Query hooks（pnpm api:gen）
export default defineConfig({
  niulai: {
    input: { target: "./openapi.json" },
    output: {
      mode: "tags-split",
      target: "./src/api/generated",
      schemas: "./src/api/generated/model",
      client: "react-query",
      httpClient: "fetch",
      baseUrl: "/api/v1",
      clean: true,
      override: {
        mutator: { path: "./src/api/fetcher.ts", name: "apiFetch" },
        fetch: { includeHttpResponseReturnType: false },
        query: { signal: true },
      },
    },
  },
});
