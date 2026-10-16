import { describe, expect, it } from "vitest";
import {
  fmtAgo,
  fmtBig,
  fmtDuration,
  fmtPercent,
  fmtPrice,
  rangeStart,
  todayCn,
  upDownClass,
  yearsBefore,
} from "./format";

describe("format", () => {
  it("formats decimal strings from the API", () => {
    expect(fmtPrice("1312.9700")).toBe("1312.97");
    expect(fmtPrice(null)).toBe("—");
    expect(fmtPrice("abc")).toBe("—");
  });

  it("uses 万 / 亿 for large amounts", () => {
    expect(fmtBig("7537706223.71")).toBe("75.38 亿");
    expect(fmtBig(5767722)).toBe("576.77 万");
    expect(fmtBig(950)).toBe("950");
  });

  it("signs percentages", () => {
    expect(fmtPercent(0.0213)).toBe("+2.13%");
    expect(fmtPercent(-0.01)).toBe("-1.00%");
  });

  it("computes Shanghai dates and ranges", () => {
    expect(todayCn(new Date("2026-10-15T17:00:00Z"))).toBe("2026-10-16");
    expect(yearsBefore("2026-10-15", 3)).toBe("2023-10-15");
  });

  it("describes elapsed time", () => {
    const now = Date.parse("2026-10-15T12:00:00Z");
    expect(fmtAgo("2026-10-15T11:55:00Z", now)).toBe("5 分钟前");
    expect(fmtDuration("2026-10-15T10:00:00Z", "2026-10-15T11:05:00Z")).toBe("1 小时 5 分");
  });
});

it("upDownClass 红涨绿跌", () => {
  expect(upDownClass(0.01)).toBe("text-up");
  expect(upDownClass(0)).toBe("text-up");
  expect(upDownClass(-0.01)).toBe("text-down");
  expect(upDownClass(null)).toBe("");
});

it("rangeStart 全部不限制", () => {
  expect(rangeStart("all")).toBeUndefined();
  expect(rangeStart("1")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
});
