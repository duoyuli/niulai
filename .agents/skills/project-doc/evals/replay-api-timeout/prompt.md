---
tags: ["replay", "implement"]
max_turns: 80
timeout_seconds: 1500
budget_usd: 6
---

API 请求现在写死 120 秒超时，组合回测这类长任务会被掐断。改成可配置：环境变量 NIULAI_API_TIMEOUT_S，默认 300 秒。改完提交，不要 push，中途不要停下来问我。
