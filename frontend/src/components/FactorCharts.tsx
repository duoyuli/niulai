import { ColorType, createChart, LineSeries, type Time } from "lightweight-charts";
import { useEffect, useRef } from "react";
import { useDarkMode } from "../hooks/useDarkMode";

export interface Line {
  label: string;
  color: string;
  width?: 1 | 2;
  points: { date: string; value: number }[];
}

/** 从红（最差一组）到绿（最好一组）的分组颜色；中间组偏灰 */
export function quantileColors(n: number): string[] {
  const ends = ["#dc2626", "#f97316", "#a1a1aa", "#22c55e", "#15803d"];
  if (n === 5) return ends;
  return Array.from({ length: n }, (_, i) => {
    const t = n === 1 ? 1 : i / (n - 1);
    const r = Math.round(220 - t * 199);
    const g = Math.round(38 + t * 90);
    const b = Math.round(38 + t * 23);
    return `rgb(${r}, ${g}, ${b})`;
  });
}

/** 几条净值曲线叠在一张图上，例如因子的各组净值 */
export function MultiLineChart({ lines, height = 300 }: { lines: Line[]; height?: number }) {
  const el = useRef<HTMLDivElement>(null);
  const dark = useDarkMode();
  useEffect(() => {
    if (!el.current) return;
    const text = dark ? "#a1a1aa" : "#52525b";
    const grid = dark ? "#27272a" : "#f4f4f5";
    const chart = createChart(el.current, {
      autoSize: true,
      layout: {
        background: { type: ColorType.Solid, color: "transparent" },
        textColor: text,
        fontFamily: "Inter, PingFang SC, system-ui, sans-serif",
      },
      grid: { vertLines: { color: grid }, horzLines: { color: grid } },
      rightPriceScale: { borderVisible: false },
      timeScale: { borderVisible: false, minBarSpacing: 0.05 },
      localization: { locale: "zh-CN", dateFormat: "yyyy-MM-dd", priceFormatter: (v: number) => v.toFixed(2) },
    });
    for (const l of lines) {
      chart
        .addSeries(LineSeries, {
          color: l.color,
          lineWidth: l.width ?? 1,
          priceLineVisible: false,
          lastValueVisible: false,
        })
        .setData(l.points.map((p) => ({ time: p.date as Time, value: p.value })));
    }
    chart.timeScale().fitContent();
    return () => chart.remove();
  }, [lines, dark]);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-zinc-500">
        {lines.map((l) => (
          <span key={l.label} className="inline-flex items-center gap-1">
            <span className="inline-block h-0.5 w-4" style={{ background: l.color }} />
            {l.label}
          </span>
        ))}
      </div>
      <div ref={el} style={{ height }} className="w-full" />
    </div>
  );
}
