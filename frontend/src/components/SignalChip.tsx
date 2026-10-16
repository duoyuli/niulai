import { clsx } from "clsx";
import { Link } from "react-router";
import type { SignalEvent } from "../api/generated/model";

/** 一个信号：看多用红色、看空用绿色，与 A 股红涨绿跌一致。withDate 时在后面显示日期（月-日）。
 * 默认点击进入「指标与信号」里这个信号的说明；放在本身可点击的行里时传 link={false} */
export function SignalChip({
  event,
  withDate = false,
  link = true,
}: {
  event: SignalEvent;
  withDate?: boolean;
  link?: boolean;
}) {
  const bullish = event.tone === "bullish";
  const className = clsx(
    "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs font-medium whitespace-nowrap",
    bullish ? "bg-up/10 text-up" : "bg-down/10 text-down",
    link && "hover:ring-1 hover:ring-current",
  );
  const title = `${event.trade_date} ${event.name}（${bullish ? "偏多" : "偏空"}）${link ? "，点击看这个信号是什么意思" : ""}`;
  const body = (
    <>
      {bullish ? "▲" : "▼"} {event.name}
      {withDate && <span className="num font-normal opacity-70">{event.trade_date.slice(5)}</span>}
    </>
  );
  return link ? (
    <Link to={`/guide#sig-${event.signal}`} title={title} className={className}>
      {body}
    </Link>
  ) : (
    <span title={title} className={className}>
      {body}
    </span>
  );
}
