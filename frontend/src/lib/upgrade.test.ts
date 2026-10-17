import { describe, expect, it } from "vitest";
import { progressOf } from "../components/VersionBadge";

const step = (state: string, detail = "") => ({ key: state, label: state, state, detail });

describe("progressOf", () => {
  it("做完和跳过的算满，进行中的按百分比算一部分", () => {
    expect(progressOf([])).toBe(0);
    expect(
      progressOf([step("done"), step("skipped"), step("running", "50%（10 / 20 MB）"), step("pending")]),
    ).toBeCloseTo(2.5 / 4);
    // 没有百分比的进行中步骤算 30%
    expect(progressOf([step("done"), step("running", "等新版本启动，已等 35 秒")])).toBeCloseTo(1.3 / 2);
  });
});
