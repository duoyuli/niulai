import { describe, expect, it } from "vitest";
import type { RunSummary } from "../api/generated/model";
import { groupChains, summarizeChain } from "./runs";

const run = (run_id: number, over: Partial<RunSummary> = {}): RunSummary => ({
  run_id,
  kind: "incremental",
  trade_date: "2026-10-16",
  status: "succeeded",
  tasks_total: 3,
  tasks_done: 3,
  tasks_failed: 0,
  started_at: "2026-10-17T06:44:33Z",
  finished_at: "2026-10-17T06:44:40Z",
  params: {},
  ...over,
});

describe("summarizeChain", () => {
  it("第一批结束、后面还有批次时仍是进行中，数字是各批合计", () => {
    const first = run(21, { params: { pending_waves: [["a"], ["b"]] } });
    expect(summarizeChain([first])?.status).toBe("running");
    const second = run(22, {
      status: "running",
      tasks_total: 100,
      tasks_done: 10,
      tasks_failed: 2,
      finished_at: null,
      params: { root_run: 21, pending_waves: [["b"]] },
    });
    const c = summarizeChain([second, first]);
    expect(c).toMatchObject({
      root: 21,
      status: "running",
      total: 103,
      done: 13,
      failed: 2,
      waves: 2,
      finishedAt: null,
    });
  });

  it("全部结束：有失败的批次就是部分失败", () => {
    const c = summarizeChain([run(21), run(22, { status: "partial", params: { root_run: 21 } })]);
    expect(c?.status).toBe("partial");
    expect(c?.finishedAt).toBe("2026-10-17T06:44:40Z");
  });

  it("按第一批分组，新的在前", () => {
    const chains = groupChains([
      run(24, { params: { root_run: 23 } }),
      run(23),
      run(22, { params: { root_run: 21 } }),
      run(21),
    ]);
    expect(chains.map((c) => [c.root, c.waves])).toEqual([
      [23, 2],
      [21, 2],
    ]);
  });
});
