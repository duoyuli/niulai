import { describe, expect, it } from "vitest";
import type { IndicatorInfo, SignalInfo } from "../api/generated/model";
import { fillName, groupBy, indicatorName, newSignalCondition, signalName } from "./strategy";

const cross: SignalInfo = {
  id: "ma_cross_up",
  name: "{fast} 日均线上穿 {slow} 日均线",
  description: "",
  usage: "",
  group: "均线",
  kind: "event",
  tone: "bullish",
  params: [
    { key: "fast", label: "快线", default: 5, min: 2, max: 250, integer: true },
    { key: "slow", label: "慢线", default: 20, min: 2, max: 250, integer: true },
  ],
};

describe("strategy labels", () => {
  it("fills placeholders and falls back to defaults", () => {
    expect(fillName("{n} 日均线", { n: 60 })).toBe("60 日均线");
    expect(signalName(cross)).toBe("5 日均线上穿 20 日均线");
    expect(signalName(cross, { fast: 10, slow: 30 })).toBe("10 日均线上穿 30 日均线");
    const ma: IndicatorInfo = {
      id: "ma",
      name: "{n} 日均线",
      description: "",
      usage: "",
      group: "均线",
      unit: "price",
      n_default: 20,
    };
    expect(indicatorName(ma)).toBe("20 日均线");
    expect(indicatorName(ma, 5)).toBe("5 日均线");
  });

  it("creates a signal condition with default params", () => {
    expect(newSignalCondition(cross)).toEqual({
      kind: "signal",
      signal: "ma_cross_up",
      params: { fast: 5, slow: 20 },
      within_days: 1,
    });
  });

  it("groups in catalog order", () => {
    const g = groupBy([
      { group: "a", v: 1 },
      { group: "b", v: 2 },
      { group: "a", v: 3 },
    ]);
    expect(g.map(([k, xs]) => [k, xs.map((x) => x.v)])).toEqual([
      ["a", [1, 3]],
      ["b", [2]],
    ]);
  });
});
