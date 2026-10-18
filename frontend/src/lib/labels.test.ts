import { describe, expect, it } from "vitest";
import { financialIndicatorName } from "./labels";

describe("financial indicator labels", () => {
  it("translates the supplier IDs used by the fundamentals page", () => {
    expect(financialIndicatorName("calculate_operating_income_yoy_growth_ratio")).toBe("营业收入同比增长率");
    expect(financialIndicatorName("inventory_turnover_ratio")).toBe("存货周转率");
    expect(financialIndicatorName("cash_meet_invest_ratio")).toBe("现金满足投资比率");
  });

  it("does not expose an unknown English field ID", () => {
    expect(financialIndicatorName("future_provider_metric")).toBe("其他财务指标");
    expect(financialIndicatorName("供应商自定义指标")).toBe("供应商自定义指标");
  });
});
