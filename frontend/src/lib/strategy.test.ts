import { describe, expect, it } from "vitest";
import type { Catalog, IndicatorInfo, SignalInfo } from "../api/generated/model";
import {
  describeCondition,
  fillName,
  formatValue,
  groupBy,
  indicatorName,
  newSignalCondition,
  rangeOf,
  signalName,
  withRange,
} from "./strategy";

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
      formula: "",
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

const catalog = {
  indicators: [
    { id: "close", name: "收盘价", description: "", usage: "", group: "价格", unit: "price", formula: "" },
    {
      id: "ma",
      name: "{n} 日均线",
      description: "",
      usage: "",
      group: "均线",
      unit: "price",
      n_default: 20,
      formula: "",
    },
    { id: "amount", name: "成交额", description: "", usage: "", group: "量能", unit: "yuan", formula: "" },
  ],
  signals: [cross],
  templates: [],
  default_universe: {},
} as unknown as Catalog;

describe("describeCondition", () => {
  const mine = new Map([[7, { name: "放量站上", state: false }]]);
  it("matches the backend wording", () => {
    expect(describeCondition(catalog, newSignalCondition(cross), mine)).toBe("当天出现「5 日均线上穿 20 日均线」");
    expect(describeCondition(catalog, { kind: "my_signal", signal_id: 7, within_days: 5 }, mine)).toBe(
      "最近 5 个交易日内出现过「放量站上」",
    );
    expect(
      describeCondition(
        catalog,
        { kind: "compare", left: { id: "close" }, op: "crosses_above", right: { id: "ma", n: 60 } },
        mine,
      ),
    ).toBe("收盘价上穿 60 日均线");
    expect(
      describeCondition(
        catalog,
        { kind: "compare", left: { id: "close" }, op: ">", right: { id: "ma" }, streak_days: 3 },
        mine,
      ),
    ).toBe("连续 3 个交易日收盘价高于 20 日均线");
    expect(formatValue(5e7, "yuan")).toBe("5000 万元");
  });

  it("switches between time ranges", () => {
    const c = newSignalCondition(cross);
    expect(rangeOf(withRange(c, "streak", 3))).toEqual({ mode: "streak", n: 3 });
    expect(rangeOf(withRange(c, "within", 5))).toEqual({ mode: "within", n: 5 });
    expect(rangeOf(withRange(c, "today", 5))).toEqual({ mode: "today", n: 1 });
  });
});
