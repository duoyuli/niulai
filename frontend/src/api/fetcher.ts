/** 后端错误响应体，见 docs/interfaces/rest_api.md#errors */
export interface ApiErrorBody {
  code: string;
  message: string;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

// @project-doc docs/architecture/frontend.md#api_client
/** orval 生成的请求函数都经过这里：非 2xx 时抛出 ApiError，2xx 时返回解析后的 JSON */
export async function apiFetch<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  const text = await res.text();
  if (!res.ok) {
    let body: Partial<ApiErrorBody> = {};
    try {
      body = JSON.parse(text) as ApiErrorBody;
    } catch {
      // 框架直接返回的 400（例如日期格式不对）是纯文本
    }
    throw new ApiError(res.status, body.code ?? "http_error", body.message ?? (text || res.statusText));
  }
  return (text ? JSON.parse(text) : undefined) as T;
}
