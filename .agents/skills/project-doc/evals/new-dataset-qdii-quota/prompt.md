---
tags: ["dataset"]
max_turns: 80
timeout_seconds: 1500
budget_usd: 6
---

同花顺有个基金接口 /api/fund/quota/list，返回 QDII 基金的申购额度（记录在 data 字段里，按 tab 分类，已知 tab 有 remen、nazhi100）。把它作为新数据集 fund.qdii_quota 接进来：P3、默认关闭、每周同步，只存原始记录就行。不要提交，中途不要停下来问我。
