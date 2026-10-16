-- 去掉原始响应存档：没有代码读取它、也没有自动清理，内容与 raw.record（记录原文）和 ops.api_call（请求元数据）重复。
-- 见 docs/architecture/data_model.md#generic_records。
DROP TABLE raw.api_page;
