import { describe, expect, it } from "vitest";
import { formatFactors, parseFactors, planResearchLink, toggleFactor, verdictTone } from "./factors";

describe("factor selection", () => {
  it("parses ids with optional weights", () => {
    expect(parseFactors("reversal, idio_vol:0.5,,reversal,roe:x")).toEqual([
      { id: "reversal", weight: 1 },
      { id: "idio_vol", weight: 0.5 },
      { id: "roe", weight: 1 },
    ]);
  });

  it("round-trips and omits unit weights", () => {
    const picks = parseFactors("reversal,idio_vol:-1");
    expect(formatFactors(picks)).toBe("reversal,idio_vol:-1");
    expect(formatFactors(picks, false)).toBe("reversal,idio_vol");
  });

  it("toggles keeping order", () => {
    const picks = parseFactors("a,b:2");
    expect(formatFactors(toggleFactor(picks, "c"))).toBe("a,b:2,c");
    expect(formatFactors(toggleFactor(picks, "a"))).toBe("b:2");
  });
});

describe("verdictTone", () => {
  it("colors by significance and direction", () => {
    expect(verdictTone({ significant: true, consistent: true, rank_ic_t: 4 })).toBe("ok");
    expect(verdictTone({ significant: false, consistent: true, rank_ic_t: 2.5 })).toBe("warn");
    expect(verdictTone({ significant: false, consistent: false, rank_ic_t: -3.5 })).toBe("bad");
    expect(verdictTone({ significant: false, consistent: true, rank_ic_t: 0.4 })).toBe("muted");
  });
});

describe("planResearchLink", () => {
  it("把账户的因子计划带到组合回测", () => {
    const link = planResearchLink({
      factors: [
        { factor: "volatility", weight: 1 },
        { factor: "accruals", weight: 0.5 },
      ],
      top_n: 30,
      buffer: 1.5,
    });
    const q = new URLSearchParams(link.split("?")[1]);
    expect(link.startsWith("/research?")).toBe(true);
    expect(q.get("tab")).toBe("portfolio");
    expect(q.get("f")).toBe("volatility,accruals:0.5");
    expect(q.get("top")).toBe("30");
    expect(q.get("buffer")).toBe("1.5");
  });
});
