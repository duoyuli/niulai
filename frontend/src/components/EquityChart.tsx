import { ColorType, createChart, LineSeries, PriceScaleMode, type Time } from "lightweight-charts";
import { useEffect, useRef, useState } from "react";
import { useDarkMode } from "../hooks/useDarkMode";
import { fmtBig, fmtPercent } from "../lib/format";

const PORTFOLIO = "#3b6ef5";
const BENCHMARK = "#a1a1aa";

/** 曲线上的一个点：组合总值、基准（缺数据时为空）和持有只数 */
export interface ChartPoint {
  date: string;
  equity: number;
  benchmark?: number | null;
  positions?: number;
}

/** 组合回测的总值曲线：组合与基准对比。log 为真时用对数坐标，长周期里早期的涨跌也看得清 */
export function EquityChart({
  points,
  capital,
  log,
  benchmarkLabel,
  height = 360,
}: {
  points: ChartPoint[];
  capital: number;
  log: boolean;
  benchmarkLabel: string;
  height?: number;
}) {
  const el = useRef<HTMLDivElement>(null);
  const dark = useDarkMode();
  const [hover, setHover] = useState<ChartPoint | null>(null);

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
    const line = (color: string, width: 1 | 2, pick: (p: ChartPoint) => number | null | undefined) =>
      chart
        .addSeries(LineSeries, { color, lineWidth: width, priceLineVisible: false, crosshairMarkerVisible: true })
        .setData(
          points.flatMap((p) => {
            const value = pick(p);
            return value === null || value === undefined ? [] : [{ time: p.date as Time, value }];
          }),
        );
    line(BENCHMARK, 1, (p) => p.benchmark);
    line(PORTFOLIO, 2, (p) => p.equity);
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
          <span style={{ color: PORTFOLIO }}>
            组合 {fmtBig(shown.equity)}（{fmtPercent(shown.equity / capital - 1)}）
          </span>
          {shown.benchmark !== null && shown.benchmark !== undefined && (
            <span>
              {benchmarkLabel} {fmtBig(shown.benchmark)}（{fmtPercent(shown.benchmark / capital - 1)}）
            </span>
          )}
          {shown.positions !== undefined && <span>{shown.positions > 0 ? `持有 ${shown.positions} 只` : "空仓"}</span>}
        </div>
      )}
      <div ref={el} style={{ height }} className="w-full" />
    </div>
  );
}
