import { useQueries } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router";
import type { Catalog, Condition, SignalEntry, StrategyItem } from "../api/generated/model";
import {
  getIndicatorSeriesQueryOptions,
  getStrategyPointsQueryOptions,
  signalDays,
} from "../api/generated/signal/signal";
import { useSearchState } from "../hooks/useSearchState";
import { fmtPctValue } from "../lib/format";
import { findIndicator, groupBy, indicatorName } from "../lib/strategy";
import { toneOf } from "../lib/tone";
import type { ChartLine, ChartMarker } from "./PriceChart";
import { inputClass, Segmented } from "./ui";

/** K 线上叠加的东西，写在地址的 overlay 参数里，逗号分隔：
 * ind:<指标>:<周期> 画指标线，sig:<信号 key> 标出信号出现的日子，strategy:<编号> 标出策略的买卖点 */
export type Overlay = string;

const UP = "#d93a3a";
const DOWN = "#16a05d";
const PALETTE = ["#0ea5e9", "#a855f7", "#f97316", "#14b8a6", "#e11d48", "#84cc16"];
const DEFAULT_INDICATOR_COLORS: Record<string, string> = {
  "ind:ma:15": "#0284c7",
  "ind:ma:30": "#ea580c",
};

export function parseOverlays(v: string): Overlay[] {
  return v ? [...new Set(v.split(",").filter(Boolean))] : [];
}

/** 个股 K 线默认叠加的指标：15 日和 30 日均线，和其他叠加项一样可以去掉 */
export const DEFAULT_OVERLAYS = "ind:ma:15,ind:ma:30";

/** 个股页地址里的 overlay 参数：K 线卡片和诊断卡片共用（诊断里点信号或策略，就画到 K 线上）。
 * 全部去掉时记为 -，否则会回到默认值 */
export function useOverlayParam(): [Overlay[], (o: Overlay[]) => void, (o: Overlay) => void] {
  const [s, set] = useSearchState({ overlay: DEFAULT_OVERLAYS });
  const overlays = parseOverlays(s.overlay === "-" ? "" : s.overlay);
  const setAll = (o: Overlay[]) => set({ overlay: o.join(",") || "-" });
  const toggle = (o: Overlay) => setAll(overlays.includes(o) ? overlays.filter((x) => x !== o) : [...overlays, o]);
  return [overlays, setAll, toggle];
}

export function signalTarget(key: string): Condition {
  return key.startsWith("my-")
    ? { kind: "my_signal", signal_id: Number(key.slice(3)) }
    : ({ kind: "signal", signal: key } as Condition);
}

export interface OverlayInfo {
  overlay: Overlay;
  label: string;
  color: string;
  /** 图上的东西怎么看 */
  note: string;
  /** 详细说明页 */
  href?: string;
  loading: boolean;
  error?: unknown;
}

/** 把选中的叠加项变成 K 线上的线和标记，以及每一项的图例说明 */
export function useOverlays({
  thscode,
  overlays,
  start,
  end,
  lastDate,
  catalog,
  signals,
  strategies,
}: {
  thscode: string;
  overlays: Overlay[];
  start?: string;
  end?: string;
  /** 图上最后一根 K 线的日期：今天是买点或卖点时标在这里 */
  lastDate?: string;
  catalog?: Catalog;
  signals?: SignalEntry[];
  strategies?: StrategyItem[];
}) {
  const results = useQueries({
    queries: overlays.map((o) => {
      const [kind, a = "", b] = o.split(":");
      if (kind === "ind")
        return getIndicatorSeriesQueryOptions(
          thscode,
          { id: a as never, n: b ? Number(b) : undefined, start, end },
          { query: { staleTime: 5 * 60_000 } },
        );
      if (kind === "sig")
        return {
          queryKey: ["signal-days", thscode, a, start, end],
          queryFn: () => signalDays(thscode, { target: { condition: signalTarget(a) }, start, end }),
          staleTime: 5 * 60_000,
        };
      return getStrategyPointsQueryOptions(
        thscode,
        { strategy_id: Number(a), start: start ?? "1990-01-01" },
        { query: { staleTime: 5 * 60_000 } },
      );
    }),
  });

  // 只在数据真的变了时重新生成，否则 K 线图每次渲染都会重建、丢掉放大的范围
  const version = results.map((r) => `${r.dataUpdatedAt}:${r.status}`).join(",");
  // biome-ignore lint/correctness/useExhaustiveDependencies: results 每次渲染都是新数组，用 version 代表它的内容
  return useMemo(() => build(), [version, overlays.join(","), lastDate, catalog, signals, strategies]);

  function build() {
    const lines: ChartLine[] = [];
    const markers: ChartMarker[] = [];
    const infos: OverlayInfo[] = [];
    overlays.forEach((o, i) => {
      const [kind, a = "", b] = o.split(":");
      const r = results[i];
      const color = DEFAULT_INDICATOR_COLORS[o] ?? PALETTE[i % PALETTE.length] ?? "#0ea5e9";
      const base = { overlay: o, color, loading: !!r?.isPending, error: r?.error ?? undefined };
      if (kind === "ind") {
        const info = catalog ? findIndicator(catalog, a) : undefined;
        const label = info ? indicatorName(info, b ? Number(b) : null) : a;
        const data = r?.data as { points: { date: string; value: number }[] } | undefined;
        const onPrice = info?.unit === "price";
        if (data)
          lines.push({
            key: o,
            label,
            color,
            pane: onPrice ? "price" : (info?.group ?? a),
            points: data.points,
            format: (v) => (info?.unit === "pct" ? `${v.toFixed(2)}%` : v.toFixed(2)),
          });
        infos.push({
          ...base,
          label,
          href: `/indicators/${a}`,
          note: onPrice ? "画在 K 线上，和价格同一个刻度" : "画在 K 线下方的副图里",
        });
        return;
      }
      if (kind === "sig") {
        const entry = signals?.find((s) => s.key === a);
        const t = toneOf(entry?.tone);
        const dates = (r?.data as { dates: string[] } | undefined)?.dates ?? [];
        const label = entry?.name ?? a;
        for (const d of dates)
          markers.push({ date: d, position: t.marker.position, shape: t.marker.shape, color: t.color });
        infos.push({
          ...base,
          color: t.color,
          label: `${t.arrow} ${label}`,
          href: `/signals/${a}`,
          note: r?.data
            ? `${t.marker.word}是出现的日子，图上区间内 ${dates.length} 次（连续几天都满足只标第一天）`
            : "",
        });
        return;
      }
      const st = strategies?.find((s) => s.strategy_id === Number(a));
      const data = r?.data as
        | {
            trades: {
              entry_date: string;
              exit_date?: string | null;
              exit_reason?: string | null;
              exit_text?: string | null;
              return_pct: number;
            }[];
            today: string;
            today_reason?: string | null;
          }
        | undefined;
      const trades = data?.trades ?? [];
      for (const t of trades) {
        markers.push({ date: t.entry_date, position: "below", shape: "arrowUp", color: UP, text: "买" });
        if (t.exit_date)
          markers.push({
            date: t.exit_date,
            position: "above",
            shape: "arrowDown",
            color: DOWN,
            text: `卖 ${fmtPctValue(t.return_pct, 1)}`,
          });
      }
      if (lastDate && data?.today === "buy")
        markers.push({ date: lastDate, position: "below", shape: "circle", color: UP, text: "最新买点" });
      if (lastDate && data?.today === "sell")
        markers.push({ date: lastDate, position: "above", shape: "circle", color: DOWN, text: "最新卖点" });
      const closed = trades.filter((t) => t.exit_date);
      const wins = closed.filter((t) => t.return_pct > 0).length;
      const now =
        data?.today === "buy"
          ? "今天是买点，按规则明天开盘买入"
          : data?.today === "sell"
            ? `今天是卖点（${data.today_reason ?? ""}），按规则明天开盘卖出`
            : data?.today === "holding"
              ? "现在按规则持有中"
              : "现在按规则空仓";
      infos.push({
        ...base,
        color: UP,
        label: `策略「${st?.name ?? a}」的买卖点`,
        href: `/strategies/${a}`,
        note: data
          ? `▲ 买：买入的那天（前一天收盘满足买入条件，当天开盘买入）；▼ 卖：卖出的那天和这一笔的涨跌。图上区间内完成 ${closed.length} 笔，赚钱 ${wins} 笔。${now}。`
          : "",
      });
    });
    return { lines, markers, infos };
  }
}

/** 叠加到 K 线上的东西：上面一行是已经叠加的（点 × 去掉），最后是「＋ 叠加到图上」按钮，点开后按策略、信号、指标分组挑选，可以搜索。
 * 每一项在图上怎么看，收在「图例说明」里，需要时展开 */
export function OverlayPicker({
  overlays,
  onChange,
  infos,
  catalog,
  signals,
  strategies,
}: {
  overlays: Overlay[];
  onChange: (o: Overlay[]) => void;
  infos: OverlayInfo[];
  catalog?: Catalog;
  signals?: SignalEntry[];
  strategies?: StrategyItem[];
}) {
  const [open, setOpen] = useState(false);
  const [notes, setNotes] = useState(false);
  const toggle = (o: Overlay) => onChange(overlays.includes(o) ? overlays.filter((x) => x !== o) : [...overlays, o]);
  return (
    <div className="flex flex-col gap-2 text-xs">
      <div className="relative flex flex-wrap items-center gap-1.5">
        {infos.map((x) => (
          <span
            key={x.overlay}
            className="inline-flex items-center gap-1 rounded-full border border-zinc-200 py-0.5 pr-1 pl-2 dark:border-zinc-700"
            title={x.loading ? "加载中…" : x.error ? "加载失败" : x.note}
          >
            <span className="size-2 rounded-full" style={{ background: x.color }} />
            <span style={{ color: x.color }}>{x.label}</span>
            {x.loading && <span className="text-zinc-400">…</span>}
            <button
              type="button"
              className="rounded-full px-1 text-zinc-400 hover:text-red-600"
              aria-label={`去掉 ${x.label}`}
              onClick={() => onChange(overlays.filter((o) => o !== x.overlay))}
            >
              ×
            </button>
          </span>
        ))}
        <button
          type="button"
          className="rounded-full border border-dashed border-zinc-300 px-2.5 py-0.5 text-zinc-600 hover:border-accent hover:text-accent dark:border-zinc-600 dark:text-zinc-300"
          aria-expanded={open}
          onClick={() => setOpen((v) => !v)}
        >
          ＋ 叠加到图上
        </button>
        {infos.length > 0 && (
          <>
            <button
              type="button"
              className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
              onClick={() => setNotes((v) => !v)}
            >
              {notes ? "收起说明" : "图例说明"}
            </button>
            <button
              type="button"
              className="text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100"
              onClick={() => onChange([])}
            >
              全部去掉
            </button>
          </>
        )}
        {open && (
          <OverlayMenu
            overlays={overlays}
            onToggle={toggle}
            onClose={() => setOpen(false)}
            catalog={catalog}
            signals={signals}
            strategies={strategies}
          />
        )}
      </div>
      {notes && infos.length > 0 && (
        <ul className="flex flex-col gap-1 rounded-lg bg-zinc-50 p-2 dark:bg-zinc-800/50">
          {infos.map((x) => (
            <li key={x.overlay} className="flex flex-wrap items-baseline gap-x-2">
              <span className="font-medium" style={{ color: x.color }}>
                {x.label}
              </span>
              <span className="text-zinc-500">{x.loading ? "加载中…" : x.error ? "加载失败" : x.note}</span>
              {x.href && (
                <Link to={x.href} className="text-accent hover:underline">
                  说明
                </Link>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** 挑选叠加项的面板：策略的买卖点、信号、指标三栏，按分组列出，可以搜索；点一下加上，再点一下去掉 */
function OverlayMenu({
  overlays,
  onToggle,
  onClose,
  catalog,
  signals,
  strategies,
}: {
  overlays: Overlay[];
  onToggle: (o: Overlay) => void;
  onClose: () => void;
  catalog?: Catalog;
  signals?: SignalEntry[];
  strategies?: StrategyItem[];
}) {
  const [tab, setTab] = useState<"strategy" | "signal" | "indicator">(strategies?.length ? "strategy" : "signal");
  const [q, setQ] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // 打开面板的那次点击不算点在外面
    const t = setTimeout(() => document.addEventListener("mousedown", down));
    document.addEventListener("keydown", key);
    return () => {
      clearTimeout(t);
      document.removeEventListener("mousedown", down);
      document.removeEventListener("keydown", key);
    };
  }, [onClose]);
  const match = (name: string) => !q || name.toLowerCase().includes(q.trim().toLowerCase());
  const groups: [string, { value: Overlay; label: string; color?: string }[]][] =
    tab === "strategy"
      ? [["策略", (strategies ?? []).map((s) => ({ value: `strategy:${s.strategy_id}`, label: s.name }))]]
      : tab === "signal"
        ? groupBy(signals ?? []).map(([g, list]) => [
            g,
            list.map((s) => ({
              value: `sig:${s.key}`,
              label: `${toneOf(s.tone).arrow} ${s.name}`,
              color: toneOf(s.tone).color,
            })),
          ])
        : groupBy((catalog?.indicators ?? []).filter((x) => !["close", "open", "high", "low"].includes(x.id))).map(
            ([g, list]) => [
              g,
              list.flatMap((x) => {
                const periods = x.id === "ma" ? [15, 20, 30, 60] : x.n_default ? [x.n_default] : [null];
                return periods.map((n) => ({
                  value: `ind:${x.id}${n ? `:${n}` : ""}`,
                  label: indicatorName(x, n),
                }));
              }),
            ],
          );
  const shown = groups
    .map(([g, list]) => [g, list.filter((x) => match(x.label))] as const)
    .filter(([, l]) => l.length > 0);
  const TABS = [
    { value: "strategy", label: "策略买卖点", hint: "标出策略在这只股票上的每一次买入和卖出" },
    { value: "signal", label: "信号", hint: "标出信号出现的日子" },
    { value: "indicator", label: "指标", hint: "画出指标的线：价格类画在 K 线上，其他画在下方副图" },
  ] as const;
  return (
    <div
      ref={ref}
      className="absolute top-full left-0 z-20 mt-1 flex max-h-[28rem] w-[min(40rem,calc(100vw-2rem))] flex-col gap-2 rounded-xl border border-zinc-200 bg-white p-3 shadow-xl dark:border-zinc-700 dark:bg-zinc-900"
    >
      <div className="flex flex-wrap items-center gap-2">
        <Segmented value={tab} options={TABS} onChange={setTab} />
        <input
          className={`${inputClass} min-w-0 flex-1 py-1 text-xs`}
          placeholder="搜索，例如 MACD、均线、背离"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <button type="button" className="text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100" onClick={onClose}>
          关闭
        </button>
      </div>
      <div className="flex flex-col gap-3 overflow-y-auto pr-1">
        {shown.length === 0 && (
          <p className="py-4 text-center text-zinc-400">
            {tab === "strategy" && !strategies?.length ? "还没有策略" : "没有找到"}
          </p>
        )}
        {shown.map(([g, list]) => (
          <section key={g}>
            <h4 className="mb-1 text-zinc-400">{g}</h4>
            <div className="flex flex-wrap gap-1">
              {list.map((x) => {
                const on = overlays.includes(x.value);
                return (
                  <button
                    key={x.value}
                    type="button"
                    onClick={() => onToggle(x.value)}
                    className={
                      on
                        ? "rounded-md bg-accent px-2 py-1 text-white"
                        : "rounded-md border border-zinc-200 px-2 py-1 hover:border-accent/60 dark:border-zinc-700"
                    }
                    style={!on && x.color ? { color: x.color } : undefined}
                    title={on ? "已在图上，点一下去掉" : "点一下画到图上"}
                  >
                    {x.label}
                  </button>
                );
              })}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
