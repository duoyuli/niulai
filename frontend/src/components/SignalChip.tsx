import { clsx } from "clsx";
import type { SignalEvent } from "../api/generated/model";

/** 一个信号：看多用红色、看空用绿色，与 A 股红涨绿跌一致。withDate 时在后面显示日期（月-日） */
export function SignalChip({ event, withDate = false }: { event: SignalEvent; withDate?: boolean }) {
  const bullish = event.tone === "bullish";
  return (
    <span
      title={`${event.trade_date} ${event.name}（${bullish ? "偏多" : "偏空"}）`}
      className={clsx(
        "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs font-medium whitespace-nowrap",
        bullish ? "bg-up/10 text-up" : "bg-down/10 text-down",
      )}
    >
      {bullish ? "▲" : "▼"} {event.name}
      {withDate && <span className="num font-normal opacity-70">{event.trade_date.slice(5)}</span>}
    </span>
  );
}
