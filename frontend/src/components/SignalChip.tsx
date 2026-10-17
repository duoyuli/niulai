import { clsx } from "clsx";
import { Link } from "react-router";
import type { SignalEvent } from "../api/generated/model";
import { toneOf } from "../lib/tone";

/** 一个信号：看多用红色、看空用绿色，与 A 股红涨绿跌一致；中性用橙色。withDate 时在后面显示日期（月-日）。
 * 默认点击进入这个信号的详情页；放在本身可点击的行里时传 link={false} */
export function SignalChip({
  event,
  withDate = false,
  link = true,
}: {
  event: SignalEvent;
  withDate?: boolean;
  link?: boolean;
}) {
  const t = toneOf(event.tone);
  const className = clsx(
    "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs font-medium whitespace-nowrap",
    t.chip,
    link && "hover:ring-1 hover:ring-current",
  );
  const title = `${event.trade_date} ${event.name}（${t.word}）${link ? "，点击看这个信号是什么意思" : ""}`;
  const body = (
    <>
      {t.arrow} {event.name}
      {withDate && <span className="num font-normal opacity-70">{event.trade_date.slice(5)}</span>}
    </>
  );
  return link ? (
    <Link to={`/signals/${event.signal}`} title={title} className={className}>
      {body}
    </Link>
  ) : (
    <span title={title} className={className}>
      {body}
    </span>
  );
}
