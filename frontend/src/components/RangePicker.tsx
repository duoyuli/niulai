import { useEffect, useState } from "react";
import { CHART_RANGES, rangeStart, todayCn } from "../lib/format";
import { inputClass, Segmented } from "./ui";

export interface ChartRange {
  /** CHART_RANGES 中的值，或 custom */
  range: string;
  /** 自定义区间的起止日期（yyyy-MM-dd），只在 range 为 custom 时使用 */
  from: string;
  to: string;
}

/** 区间对应的查询参数 */
export function rangeQuery(r: ChartRange): { start?: string; end?: string } {
  if (r.range === "custom") return { start: r.from || undefined, end: r.to || undefined };
  return { start: rangeStart(r.range) };
}

/** K 线的时间区间：1 月到全部，或自定义起止日期。
 * earliest 是本地最早的日线日期：比它更长的区间和「全部」看到的一样，置灰，避免点了没有变化 */
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
  // 起点已经早于最早日线的区间，看到的和「全部」一样
  const options = [
    ...CHART_RANGES.map((o) => {
      const start = rangeStart(o.value, today);
      const disabled = o.value !== "all" && !!earliest && !!start && start <= earliest;
      return { ...o, disabled, hint: disabled ? `数据从 ${earliest} 开始，选「全部」即可` : undefined };
    }),
    { value: "custom", label: "自定义", hint: "自己选开始和结束日期" },
  ];
  const pick = (range: string) =>
    // 第一次切到自定义时先填上最近一年，图上不会空着
    range === "custom"
      ? onChange({ range, from: value.from || rangeStart("1", today), to: value.to || today })
      : onChange({ range });
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Segmented value={value.range} options={options} onChange={pick} />
      {value.range === "custom" && <CustomDates value={value} earliest={earliest} today={today} onChange={onChange} />}
      {loading && (
        <span className="size-3 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" title="加载中" />
      )}
    </div>
  );
}

/** 起止日期：选完一个日期就生效；开始晚于结束时不提交 */
function CustomDates({
  value,
  earliest,
  today,
  onChange,
}: {
  value: ChartRange;
  earliest?: string | null;
  today: string;
  onChange: (patch: Partial<ChartRange>) => void;
}) {
  const [from, setFrom] = useState(value.from);
  const [to, setTo] = useState(value.to);
  useEffect(() => {
    setFrom(value.from);
    setTo(value.to);
  }, [value.from, value.to]);
  const commit = (f: string, t: string) => {
    if (f && t && f > t) return;
    onChange({ from: f, to: t });
  };
  const bad = !!from && !!to && from > to;
  return (
    <span className="flex items-center gap-1 text-xs text-zinc-500">
      <input
        type="date"
        aria-label="开始日期"
        className={`${inputClass} py-1 text-xs`}
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
        className={`${inputClass} py-1 text-xs`}
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
