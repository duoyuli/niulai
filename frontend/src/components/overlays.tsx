import { useQueries } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link } from "react-router";
import type { Catalog, Condition, SignalEntry, StrategyItem } from "../api/generated/model";
import {
  getIndicatorSeriesQueryOptions,
  getStrategyPointsQueryOptions,
  signalDays,
} from "../api/generated/signal/signal";
import { fmtPctValue } from "../lib/format";
import { findIndicator, groupBy, indicatorName } from "../lib/strategy";
import { toneOf } from "../lib/tone";
import type { ChartLine, ChartMarker } from "./PriceChart";
import { inputClass } from "./ui";

/** K 线上叠加的东西，写在地址的 overlay 参数里，逗号分隔：
 * ind:<指标>:<周期> 画指标线，sig:<信号 key> 标出信号出现的日子，strategy:<编号> 标出策略的买卖点 */
export type Overlay = string;

const UP = "#d93a3a";
const DOWN = "#16a05d";
const PALETTE = ["#0ea5e9", "#a855f7", "#f97316", "#14b8a6", "#e11d48", "#84cc16"];

export function parseOverlays(v: string): Overlay[] {
  return v ? [...new Set(v.split(",").filter(Boolean))] : [];
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
      const color = PALETTE[i % PALETTE.length] ?? "#0ea5e9";
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
        markers.push({ date: lastDate, position: "below", shape: "circle", color: UP, text: "今日买点" });
      if (lastDate && data?.today === "sell")
        markers.push({ date: lastDate, position: "above", shape: "circle", color: DOWN, text: "今日卖点" });
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

/** 选择要叠加的指标、信号、策略，并逐项说明图上画的是什么 */
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
  const [pick, setPick] = useState("");
  const add = (o: string) => {
    if (o && !overlays.includes(o)) onChange([...overlays, o]);
    setPick("");
  };
  return (
    <div className="flex flex-col gap-2 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <select
          className={`${inputClass} text-xs`}
          value={pick}
          aria-label="在 K 线上叠加"
          onChange={(e) => add(e.target.value)}
        >
          <option value="">＋ 在 K 线上叠加指标、信号或策略买卖点…</option>
          {(strategies ?? []).length > 0 && (
            <optgroup label="策略的买卖点">
              {strategies?.map((s) => (
                <option key={s.strategy_id} value={`strategy:${s.strategy_id}`}>
                  {s.name}
                </option>
              ))}
            </optgroup>
          )}
          {groupBy(signals ?? []).map(([g, list]) => (
            <optgroup key={g} label={`信号 · ${g}`}>
              {list.map((s) => (
                <option key={s.key} value={`sig:${s.key}`}>
                  {s.name}
                </option>
              ))}
            </optgroup>
          ))}
          {groupBy(catalog?.indicators ?? []).map(([g, list]) => (
            <optgroup key={g} label={`指标 · ${g}`}>
              {list
                .filter((x) => x.id !== "close" && x.id !== "open" && x.id !== "high" && x.id !== "low")
                .map((x) => (
                  <option key={x.id} value={`ind:${x.id}${x.n_default ? `:${x.n_default}` : ""}`}>
                    {indicatorName(x)}
                  </option>
                ))}
            </optgroup>
          ))}
        </select>
        {overlays.length > 0 && (
          <button
            type="button"
            className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
            onClick={() => onChange([])}
          >
            全部清除
          </button>
        )}
      </div>
      {infos.length > 0 && (
        <ul className="flex flex-col gap-1">
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
              <button
                type="button"
                className="text-zinc-400 hover:text-red-600"
                aria-label={`去掉 ${x.label}`}
                onClick={() => onChange(overlays.filter((o) => o !== x.overlay))}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
