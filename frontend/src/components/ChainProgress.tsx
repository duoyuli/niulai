import { fmtInt } from "../lib/format";
import type { SyncChain } from "../lib/runs";
import { ProgressBar } from "./ui";

/** 进度条和「已结束 / 总数（失败）」，同步记录和同步详情共用 */
export function ChainProgress({ c }: { c: SyncChain }) {
  return (
    <div className="flex flex-col gap-1">
      <ProgressBar done={c.done} failed={c.failed} total={c.total} />
      <span className="num text-xs text-zinc-500">
        {fmtInt(c.done + c.failed)} / {fmtInt(c.total)}
        {c.failed > 0 && <span className="text-red-500">（失败 {fmtInt(c.failed)}）</span>}
      </span>
    </div>
  );
}
