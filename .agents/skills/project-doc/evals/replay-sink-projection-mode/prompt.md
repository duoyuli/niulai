---
tags: ["replay", "dataset"]
max_turns: 80
timeout_seconds: 1500
budget_usd: 6
---

我把类型化表的写入投影从 sinks.rs 里的硬编码分支改成了在 datasets.yaml 里用新字段 sink_mode 声明（工作区未提交）。代码我已经确认过，不需要再审查。直接收尾并提交，不用跑测试，不要 push，中途不要停下来问我。
