import { describe, expect, it } from "vitest";
import { crosses, daysBefore, fetchStart, maPeriod, movingAverage } from "./movingAverage";

describe("movingAverage", () => {
  it("不满 n 天为空，之后是最近 n 天的平均", () => {
    expect(movingAverage([1, 2, 3, 4, 5], 3)).toEqual([null, null, 2, 3, 4]);
    expect(movingAverage([], 3)).toEqual([]);
  });
});

describe("maPeriod", () => {
  it("范围外或不是整数时用默认值", () => {
    expect(maPeriod("20", "15")).toBe(20);
    expect(maPeriod("1", "15")).toBe(15);
    expect(maPeriod("2.5", "15")).toBe(15);
    expect(maPeriod("abc", "30")).toBe(30);
  });
});

describe("fetchStart", () => {
  it("按最长的均线往前多取，跨月跨年", () => {
    expect(daysBefore("2026-03-01", 1)).toBe("2026-02-28");
    expect(fetchStart("2026-01-10", [15, 30])).toBe("2025-11-08");
    expect(fetchStart(undefined, [15, 30])).toBeUndefined();
  });
});

describe("crosses", () => {
  it("上穿是买点、下穿是卖点，贴着不算，缺值不算", () => {
    const fast = [null, 1, 2, 3, 3, 1];
    const slow = [null, 2, 2, 2, 3, 2];
    expect(crosses(fast, slow)).toEqual([null, null, null, "buy", null, "sell"]);
  });
});
