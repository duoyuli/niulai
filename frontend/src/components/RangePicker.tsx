import { clsx } from "clsx";
import { useEffect, useState } from "react";
import { CHART_RANGES, rangeStart, todayCn } from "../lib/format";
import { inputClass } from "./ui";

export interface ChartRange {
  /** CHART_RANGES 中的值，或 custom（自己改过起止日期） */
  range: string;
  /** 自定义区间的起止日期（yyyy-MM-dd），只在 range 为 custom 时使用 */
  from: string;
  to: string;
}

/** 区间对应的查询参数，开始和结束日期都包含在内 */
export function rangeQuery(r: ChartRange): { start?: string; end?: string } {
  if (r.range === "custom") return { start: r.from || undefined, end: r.to || undefined };
  return { start: rangeStart(r.range) };
}

/** 当前区间显示在日期框里的起止日期：快捷区间是「开始日期到今天」，「全部」从最早的日线开始 */
function shownDates(r: ChartRange, earliest: string | null | undefined, today: string): { from: string; to: string } {
  if (r.range === "custom") return { from: r.from, to: r.to };
  const start = rangeStart(r.range, today) ?? "";
  return { from: earliest && (!start || start < earliest) ? earliest : start, to: today };
}

/** 时间区间：一个起止日期框（两头都包含），旁边是快捷区间。
 * 点快捷区间会把起止日期填进框里；在框里改日期就是自定义区间，快捷区间不再高亮。
 * earliest 是本地最早的日线日期：起点早于它的快捷区间看到的和「全部」一样，置灰，避免点了没有变化 */
export function RangePicker({
  value,
  earliest,
  loading,
  onChange,
}: {
  value: ChartRange;
  earliest?: string | null;
  loading?: boolean;
  onChange: (patch: Partial<ChartRange>) => void;
}) {
  const today = todayCn();
  const shown = shownDates(value, earliest, today);
  return (
    <div className="flex flex-wrap items-center gap-2">
      <DateBox
        from={shown.from}
        to={shown.to}
        earliest={earliest}
        today={today}
        onChange={(from, to) => onChange({ range: "custom", from, to })}
      />
      <div className="flex flex-wrap gap-0.5">
        {CHART_RANGES.map((o) => {
          const start = rangeStart(o.value, today);
          const disabled = o.value !== "all" && !!earliest && !!start && start <= earliest;
          const active = value.range === o.value;
          return (
            <button
              key={o.value}
              type="button"
              disabled={disabled}
              title={
                disabled
                  ? `数据从 ${earliest} 开始，选「全部」即可`
                  : `${o.label}：开始日期改到 ${start ?? earliest ?? "最早"}，结束日期改到今天`
              }
              onClick={() => onChange({ range: o.value })}
              className={clsx(
                "rounded-md px-2 py-1 text-xs transition",
                active
                  ? "bg-accent text-white"
                  : "text-zinc-500 hover:bg-zinc-100 hover:text-zinc-900 disabled:opacity-40 disabled:hover:bg-transparent dark:hover:bg-zinc-800 dark:hover:text-zinc-100",
              )}
            >
              {o.label}
            </button>
          );
        })}
      </div>
      {loading && (
        <span className="size-3 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" title="加载中" />
      )}
    </div>
  );
}

/** 起止日期：选完一个日期就生效；开始晚于结束时不提交 */
function DateBox({
  from: fromProp,
  to: toProp,
  earliest,
  today,
  onChange,
}: {
  from: string;
  to: string;
  earliest?: string | null;
  today: string;
  onChange: (from: string, to: string) => void;
}) {
  const [from, setFrom] = useState(fromProp);
  const [to, setTo] = useState(toProp);
  useEffect(() => {
    setFrom(fromProp);
    setTo(toProp);
  }, [fromProp, toProp]);
  const commit = (f: string, t: string) => {
    if (f && t && f > t) return;
    onChange(f, t);
  };
  const bad = !!from && !!to && from > to;
  return (
    <span
      className="inline-flex items-center gap-1 rounded-lg border border-zinc-200 px-1.5 py-0.5 text-xs text-zinc-500 dark:border-zinc-700"
      title="开始和结束日期都包含在内"
    >
      <input
        type="date"
        aria-label="开始日期"
        className={`${inputClass} border-0 bg-transparent px-1 py-0.5 text-xs focus:ring-0 dark:bg-transparent`}
        value={from}
        min={earliest ?? undefined}
        max={to || today}
        onChange={(e) => {
          setFrom(e.target.value);
          commit(e.target.value, to);
        }}
      />
      至
      <input
        type="date"
        aria-label="结束日期"
        className={`${inputClass} border-0 bg-transparent px-1 py-0.5 text-xs focus:ring-0 dark:bg-transparent`}
        value={to}
        min={from || earliest || undefined}
        max={today}
        onChange={(e) => {
          setTo(e.target.value);
          commit(from, e.target.value);
        }}
      />
      {bad && <span className="text-red-500">开始日期不能晚于结束日期</span>}
    </span>
  );
}
