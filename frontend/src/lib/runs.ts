import type { RunSummary } from "../api/generated/model";

/** 一次同步：有依赖的数据集分几批执行，每批是一个运行。同步记录和同步详情都用它合计，两边的数字一致 */
export interface SyncChain {
  root: number;
  first: RunSummary;
  /** running / succeeded / partial */
  status: string;
  total: number;
  /** 已结束：成功、没有数据或跳过 */
  done: number;
  /** 不再重试的失败 */
  failed: number;
  waves: number;
  finishedAt: string | null;
}

/** waves 是同一次同步的各批，顺序不限 */
export function summarizeChain(waves: RunSummary[]): SyncChain | null {
  const list = [...waves].sort((a, b) => a.run_id - b.run_id);
  const first = list[0];
  const last = list[list.length - 1];
  if (!first || !last) return null;
  // 最后一批结束、下一批还没规划的那一刻，也算进行中
  const more = ((last.params as { pending_waves?: unknown[] } | null)?.pending_waves ?? []).length > 0;
  const running = list.some((r) => r.status === "running") || more;
  const status = running ? "running" : list.some((r) => r.status === "partial") ? "partial" : last.status;
  return {
    root: first.run_id,
    first,
    status,
    total: list.reduce((n, r) => n + r.tasks_total, 0),
    done: list.reduce((n, r) => n + r.tasks_done, 0),
    failed: list.reduce((n, r) => n + r.tasks_failed, 0),
    waves: list.length,
    finishedAt: running ? null : (last.finished_at ?? null),
  };
}

/** 按第一批的编号把运行分成一次次同步，新的在前 */
export function groupChains(rows: RunSummary[]): SyncChain[] {
  const m = new Map<number, RunSummary[]>();
  for (const r of rows) {
    const root = (r.params as { root_run?: number | null } | null)?.root_run ?? r.run_id;
    m.set(root, [...(m.get(root) ?? []), r]);
  }
  return [...m.values()]
    .map(summarizeChain)
    .filter((c): c is SyncChain => c !== null)
    .sort((a, b) => b.root - a.root);
}
