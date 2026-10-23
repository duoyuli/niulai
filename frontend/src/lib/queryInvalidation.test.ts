import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { affectedQueries, invalidateAccountQueries } from "./queryInvalidation";

describe("账户修改后的缓存刷新", () => {
  it("改了账本或执行了建议，流水、净值、回看和影子订单都要刷新", () => {
    for (const change of ["ledger", "advice"] as const) {
      for (const q of ["trades", "equity", "review", "shadowOrders", "account", "accounts", "advice"] as const) {
        expect(affectedQueries(change)).toContain(q);
      }
    }
  });

  it("只改设置时不刷新流水和净值", () => {
    expect(affectedQueries("settings")).not.toContain("trades");
    expect(affectedQueries("settings")).not.toContain("equity");
    expect(affectedQueries("settings")).toContain("advice");
  });

  it("接口已经返回的新结果直接放进缓存，其余的判脏", async () => {
    const qc = new QueryClient();
    const tradesKey = ["/api/v1/accounts/7/trades"];
    qc.setQueryData(tradesKey, []);
    const account = { summary: {}, positions: [] } as never;
    invalidateAccountQueries(qc, 7, "ledger", { account });
    expect(qc.getQueryData(["/api/v1/accounts/7"])).toBe(account);
    expect(qc.getQueryState(["/api/v1/accounts/7"])?.isInvalidated).toBe(false);
    expect(qc.getQueryState(tradesKey)?.isInvalidated).toBe(true);
  });
});
