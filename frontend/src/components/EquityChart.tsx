import { ColorType, createChart, LineSeries, PriceScaleMode, type Time } from "lightweight-charts";
import { useEffect, useRef, useState } from "react";
import type { EquityPoint } from "../api/generated/model";
import { useDarkMode } from "../hooks/useDarkMode";
import { fmtBig, fmtPercent } from "../lib/format";

const STRATEGY = "#3b6ef5";
const BENCHMARK = "#a1a1aa";

/** 回测的账户总值曲线：策略与买入持有对比。log 为真时用对数坐标，长周期里早期的涨跌也看得清 */
export function EquityChart({
  points,
  capital,
  log,
  height = 360,
}: {
  points: EquityPoint[];
  capital: number;
  log: boolean;
  height?: number;
}) {
  const el = useRef<HTMLDivElement>(null);
  const dark = useDarkMode();
  const [hover, setHover] = useState<EquityPoint | null>(null);

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
      rightPriceScale: { borderVisible: false, mode: log ? PriceScaleMode.Logarithmic : PriceScaleMode.Normal },
      timeScale: { borderVisible: false, minBarSpacing: 0.05 },
      localization: { locale: "zh-CN", dateFormat: "yyyy-MM-dd", priceFormatter: (v: number) => fmtBig(v) },
    });
    const line = (color: string, width: 1 | 2, pick: (p: EquityPoint) => number) =>
      chart
        .addSeries(LineSeries, { color, lineWidth: width, priceLineVisible: false, crosshairMarkerVisible: true })
        .setData(points.map((p) => ({ time: p.date as Time, value: pick(p) })));
    line(BENCHMARK, 1, (p) => p.benchmark);
    line(STRATEGY, 2, (p) => p.equity);
    const byDate = new Map(points.map((p) => [p.date, p]));
    chart.subscribeCrosshairMove((p) => setHover(p.time ? (byDate.get(String(p.time)) ?? null) : null));
    chart.timeScale().fitContent();
    return () => chart.remove();
  }, [points, dark, log]);

  const shown = hover ?? points.at(-1);
  return (
    <div className="flex flex-col gap-2">
      {shown && (
        <div className="num flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-zinc-500">
          <span className="font-medium text-zinc-800 dark:text-zinc-200">{shown.date}</span>
          <span style={{ color: STRATEGY }}>
            策略 {fmtBig(shown.equity)}（{fmtPercent(shown.equity / capital - 1)}）
          </span>
          <span>
            买入持有 {fmtBig(shown.benchmark)}（{fmtPercent(shown.benchmark / capital - 1)}）
          </span>
          <span>{shown.holding ? "持仓中" : "空仓"}</span>
        </div>
      )}
      <div ref={el} style={{ height }} className="w-full" />
    </div>
  );
}
