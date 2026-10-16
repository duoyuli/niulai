import {
  CandlestickSeries,
  ColorType,
  CrosshairMode,
  createChart,
  createSeriesMarkers,
  HistogramSeries,
  type IChartApi,
  LineSeries,
  type SeriesMarker,
  type Time,
} from "lightweight-charts";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Bar, MaPoint } from "../api/generated/model";
import { useDarkMode } from "../hooks/useDarkMode";
import { fmtBig, fmtPercent, fmtPrice, toNumber, upDownClass } from "../lib/format";

const UP = "#d93a3a";
const DOWN = "#16a05d";
const MA_FAST = "#f59e0b";
const MA_SLOW = "#6366f1";

interface Props {
  bars: Bar[];
  /** 均线与交叉点；只在等比前复权口径下传入 */
  ma?: { points: MaPoint[]; fast: number; slow: number };
  height?: number;
  /** 价格小数位：个股 2 位，指数 2 位 */
  digits?: number;
}

/** 悬停时展示的一根 K 线 */
interface Hover {
  date: string;
  open: number;
  high: number;
  low: number;
  close: number;
  change: number | null;
  volume: number;
  amount: string;
  maFast?: number;
  maSlow?: number;
}

// @project-doc docs/architecture/frontend.md#conventions
/** K 线 + 成交量 + 均线。A 股配色：红涨绿跌 */
export function PriceChart({ bars, ma, height = 460, digits = 2 }: Props) {
  const el = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const dark = useDarkMode();
  const [hover, setHover] = useState<Hover | null>(null);

  const rows = useMemo(() => {
    const out: Hover[] = [];
    const maByDate = new Map(ma?.points.map((p) => [p.trade_date, p]));
    let prevClose: number | null = null;
    for (const b of bars) {
      const close = toNumber(b.close) ?? 0;
      const m = maByDate.get(b.trade_date);
      out.push({
        date: b.trade_date,
        open: toNumber(b.open) ?? 0,
        high: toNumber(b.high) ?? 0,
        low: toNumber(b.low) ?? 0,
        close,
        change: prevClose ? close / prevClose - 1 : null,
        volume: b.volume,
        amount: b.amount,
        maFast: m?.ma_fast,
        maSlow: m?.ma_slow,
      });
      prevClose = close;
    }
    return out;
  }, [bars, ma]);

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
        panes: { separatorColor: grid },
      },
      grid: { vertLines: { color: grid }, horzLines: { color: grid } },
      crosshair: { mode: CrosshairMode.Normal },
      rightPriceScale: { borderVisible: false },
      // 默认每根 K 线至少 0.5 像素，「全部」区间的二十年日线放不下；调小后 fitContent 能放下全部
      timeScale: { borderVisible: false, rightOffset: 4, minBarSpacing: 0.05 },
      localization: { locale: "zh-CN", dateFormat: "yyyy-MM-dd" },
    });
    chartRef.current = chart;

    const candles = chart.addSeries(CandlestickSeries, {
      upColor: UP,
      downColor: DOWN,
      borderUpColor: UP,
      borderDownColor: DOWN,
      wickUpColor: UP,
      wickDownColor: DOWN,
      priceFormat: { type: "price", precision: digits, minMove: 1 / 10 ** digits },
    });
    candles.setData(
      rows.map((r) => ({ time: r.date as Time, open: r.open, high: r.high, low: r.low, close: r.close })),
    );

    const volume = chart.addSeries(
      HistogramSeries,
      { priceFormat: { type: "volume" }, priceLineVisible: false, lastValueVisible: false },
      1,
    );
    volume.setData(
      rows.map((r) => ({ time: r.date as Time, value: r.volume, color: r.close >= r.open ? `${UP}99` : `${DOWN}99` })),
    );
    chart.panes()[1]?.setStretchFactor(0.25);

    if (ma) {
      const line = (color: string, pick: (p: MaPoint) => number) => {
        const s = chart.addSeries(LineSeries, {
          color,
          lineWidth: 2,
          priceLineVisible: false,
          lastValueVisible: false,
          crosshairMarkerVisible: false,
        });
        s.setData(ma.points.map((p) => ({ time: p.trade_date as Time, value: pick(p) })));
      };
      line(MA_FAST, (p) => p.ma_fast);
      line(MA_SLOW, (p) => p.ma_slow);
      const crosses = ma.points.filter((p) => p.signal);
      // 交叉太多时文字会挤在一起，只画箭头
      const label = crosses.length <= 30;
      const markers: SeriesMarker<Time>[] = crosses.map((p) =>
        p.signal === "golden"
          ? { time: p.trade_date as Time, position: "belowBar", shape: "arrowUp", color: UP, text: label ? "金叉" : "" }
          : {
              time: p.trade_date as Time,
              position: "aboveBar",
              shape: "arrowDown",
              color: DOWN,
              text: label ? "死叉" : "",
            },
      );
      createSeriesMarkers(candles, markers);
    }

    const byDate = new Map(rows.map((r) => [r.date, r]));
    chart.subscribeCrosshairMove((p) => {
      setHover(p.time ? (byDate.get(String(p.time)) ?? null) : null);
    });
    // 展示传入的全部 K 线，页面上选的区间（1 年、3 年……）就是图上看到的范围；可以拖动和缩放
    chart.timeScale().fitContent();

    return () => {
      chart.remove();
      chartRef.current = null;
    };
  }, [rows, ma, dark, digits]);

  const shown = hover ?? rows.at(-1) ?? null;

  return (
    <div className="flex flex-col gap-2">
      <Legend row={shown} digits={digits} ma={ma} />
      <div ref={el} style={{ height }} className="w-full" />
    </div>
  );
}

function Legend({ row, digits, ma }: { row: Hover | null; digits: number; ma?: Props["ma"] }) {
  if (!row) return null;
  const tone = upDownClass(row.change);
  return (
    <div className="num flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-zinc-500">
      <span className="font-medium text-zinc-800 dark:text-zinc-200">{row.date}</span>
      <span>
        开 <b className={tone}>{fmtPrice(row.open, digits)}</b>
      </span>
      <span>
        高 <b className={tone}>{fmtPrice(row.high, digits)}</b>
      </span>
      <span>
        低 <b className={tone}>{fmtPrice(row.low, digits)}</b>
      </span>
      <span>
        收 <b className={tone}>{fmtPrice(row.close, digits)}</b>
      </span>
      <span>
        涨跌 <b className={tone}>{fmtPercent(row.change)}</b>
      </span>
      <span>量 {fmtBig(row.volume)}</span>
      <span>额 {fmtBig(row.amount)}</span>
      {ma && (
        <>
          <span style={{ color: MA_FAST }}>
            MA{ma.fast} {fmtPrice(row.maFast, digits)}
          </span>
          <span style={{ color: MA_SLOW }}>
            MA{ma.slow} {fmtPrice(row.maSlow, digits)}
          </span>
        </>
      )}
    </div>
  );
}
