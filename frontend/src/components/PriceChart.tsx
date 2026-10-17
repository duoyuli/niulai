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
import { type PointerEvent, useEffect, useMemo, useRef, useState } from "react";
import type { Bar, MaPoint } from "../api/generated/model";
import { useDarkMode } from "../hooks/useDarkMode";
import { fmtBig, fmtPercent, fmtPrice, toNumber, upDownClass } from "../lib/format";

const UP = "#d93a3a";
const DOWN = "#16a05d";
const MA_FAST = "#f59e0b";
const MA_SLOW = "#6366f1";

/** 叠加的一条指标线。pane 为 price 时画在 K 线上（价格类指标），否则同名的画在 K 线下方同一个副图里 */
export interface ChartLine {
  key: string;
  label: string;
  color: string;
  pane: "price" | string;
  points: { date: string; value: number }[];
  /** 悬停时数值的格式 */
  format?: (v: number) => string;
}

/** K 线上的一个标记：信号出现、买点、卖点 */
export interface ChartMarker {
  date: string;
  position: "above" | "below";
  color: string;
  shape: "arrowUp" | "arrowDown" | "circle";
  text?: string;
}

interface Props {
  bars: Bar[];
  /** 均线与交叉点；只在等比前复权口径下传入 */
  ma?: { points: MaPoint[]; fast: number; slow: number };
  lines?: ChartLine[];
  markers?: ChartMarker[];
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

/** 框选中的区间：起点和当前位置的横坐标（像素）和对应的 K 线下标 */
interface Brush {
  x0: number;
  x1: number;
  i0: number;
  i1: number;
}

// @project-doc docs/architecture/frontend.md#conventions
/** K 线 + 成交量 + 均线。A 股配色：红涨绿跌。
 * 在图上按住鼠标左右拖动框选一段时间，松开后放大到这段；滚轮缩放，拖动底部时间轴平移，「还原」回到全部 */
export function PriceChart({ bars, ma, lines, markers, height = 460, digits = 2 }: Props) {
  const el = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const dark = useDarkMode();
  const [hover, setHover] = useState<Hover | null>(null);
  const [brush, setBrush] = useState<Brush | null>(null);
  const [zoom, setZoom] = useState<{ from: string; to: string } | null>(null);

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
      // 在图上按住拖动用来框选，不再平移；平移用滚轮缩放后拖动底部时间轴，触屏仍可以拖动平移
      handleScroll: { mouseWheel: true, pressedMouseMove: false, horzTouchDrag: true, vertTouchDrag: false },
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

    const maMarkers: SeriesMarker<Time>[] = [];
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
      const crossMarkers: SeriesMarker<Time>[] = crosses.map((p) =>
        p.signal === "golden"
          ? { time: p.trade_date as Time, position: "belowBar", shape: "arrowUp", color: UP, text: "金叉" }
          : {
              time: p.trade_date as Time,
              position: "aboveBar",
              shape: "arrowDown",
              color: DOWN,
              text: "死叉",
            },
      );
      maMarkers.push(...crossMarkers);
    }

    // 副图：每组一个，放在成交量下面
    const panes = [...new Set((lines ?? []).filter((l) => l.pane !== "price").map((l) => l.pane))];
    for (const l of lines ?? []) {
      const pane = l.pane === "price" ? 0 : 2 + panes.indexOf(l.pane);
      const series = chart.addSeries(
        LineSeries,
        {
          color: l.color,
          lineWidth: 2,
          priceLineVisible: false,
          lastValueVisible: false,
          crosshairMarkerVisible: false,
        },
        pane,
      );
      series.setData(l.points.map((p) => ({ time: p.date as Time, value: p.value })));
    }
    for (let i = 0; i < panes.length; i++) chart.panes()[2 + i]?.setStretchFactor(0.4);

    const extra: SeriesMarker<Time>[] = (markers ?? []).map((m) => ({
      time: m.date as Time,
      position: m.position === "above" ? "aboveBar" : "belowBar",
      shape: m.shape,
      color: m.color,
      text: m.text ?? "",
    }));
    // 标记太多时文字会叠在一起，只画箭头，含义看图下方的说明
    const merged = [...maMarkers, ...extra];
    const all = (merged.length > 40 ? merged.map((m) => ({ ...m, text: "" })) : merged).sort((a, b) =>
      String(a.time).localeCompare(String(b.time)),
    );
    if (all.length > 0) createSeriesMarkers(candles, all);

    const byDate = new Map(rows.map((r) => [r.date, r]));
    chart.subscribeCrosshairMove((p) => {
      setHover(p.time ? (byDate.get(String(p.time)) ?? null) : null);
    });
    // 展示传入的全部 K 线，页面上选的区间（1 年、3 年……）就是图上看到的范围；可以拖动和缩放
    chart.timeScale().fitContent();
    setZoom(null);

    return () => {
      chart.remove();
      chartRef.current = null;
    };
  }, [rows, ma, lines, markers, dark, digits]);

  const shown = hover ?? rows.at(-1) ?? null;
  // 每个副图加高一些，K 线不会被压扁
  const subPanes = new Set((lines ?? []).filter((l) => l.pane !== "price").map((l) => l.pane)).size;

  /** 鼠标位置对应的 K 线下标；在价格轴上或图外时为空 */
  const indexAt = (e: PointerEvent<HTMLDivElement>): { x: number; i: number } | null => {
    const chart = chartRef.current;
    const box = el.current?.getBoundingClientRect();
    if (!chart || !box || rows.length === 0) return null;
    const x = e.clientX - box.left;
    if (x < 0 || x > chart.timeScale().width()) return null;
    const logical = chart.timeScale().coordinateToLogical(x);
    if (logical === null) return null;
    return { x, i: Math.min(rows.length - 1, Math.max(0, Math.round(logical))) };
  };
  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.pointerType === "touch" || e.button !== 0) return;
    // 底部时间轴保留自带的拖动缩放
    const box = el.current?.getBoundingClientRect();
    const axis = chartRef.current?.timeScale().height() ?? 0;
    if (box && e.clientY - box.top > box.height - axis) return;
    const p = indexAt(e);
    if (!p) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setBrush({ x0: p.x, x1: p.x, i0: p.i, i1: p.i });
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!brush) return;
    const p = indexAt(e);
    if (p) setBrush({ ...brush, x1: p.x, i1: p.i });
  };
  const onUp = () => {
    const b = brush;
    setBrush(null);
    const chart = chartRef.current;
    if (!b || !chart) return;
    const [a, z] = b.i0 <= b.i1 ? [b.i0, b.i1] : [b.i1, b.i0];
    // 拖动太短当作点击；至少放大到 5 根 K 线
    if (Math.abs(b.x1 - b.x0) < 8 || z - a < 4) return;
    chart.timeScale().setVisibleLogicalRange({ from: a - 0.5, to: z + 0.5 });
    setZoom({ from: rows[a]?.date ?? "", to: rows[z]?.date ?? "" });
  };
  const reset = () => {
    chartRef.current?.timeScale().fitContent();
    setZoom(null);
  };
  const sel = brush && Math.abs(brush.x1 - brush.x0) >= 8 ? brush : null;
  const selFrom = sel ? rows[Math.min(sel.i0, sel.i1)]?.date : undefined;
  const selTo = sel ? rows[Math.max(sel.i0, sel.i1)]?.date : undefined;

  return (
    <div className="flex flex-col gap-2">
      <Legend row={shown} digits={digits} ma={ma} lines={lines} />
      <div
        className="relative w-full cursor-crosshair select-none"
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={() => setBrush(null)}
      >
        <div ref={el} style={{ height: height + 140 * subPanes }} className="w-full" />
        {sel && (
          <div
            className="pointer-events-none absolute top-0 bottom-0 border-x border-accent/60 bg-accent/10"
            style={{ left: Math.min(sel.x0, sel.x1), width: Math.abs(sel.x1 - sel.x0) }}
          >
            <span className="num absolute top-1 left-1 rounded bg-white/90 px-1 text-xs whitespace-nowrap text-zinc-700 dark:bg-zinc-900/90 dark:text-zinc-200">
              {selFrom} 至 {selTo}
            </span>
          </div>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-3 text-xs text-zinc-500">
        {zoom ? (
          <>
            <span className="num">
              已放大到 {zoom.from} 至 {zoom.to}
            </span>
            <button type="button" className="text-accent hover:underline" onClick={reset}>
              还原
            </button>
          </>
        ) : (
          <span>在图上按住鼠标左右拖动，框选一段时间放大；滚轮缩放。</span>
        )}
      </div>
    </div>
  );
}

function Legend({
  row,
  digits,
  ma,
  lines,
}: {
  row: Hover | null;
  digits: number;
  ma?: Props["ma"];
  lines?: ChartLine[];
}) {
  const byLine = useMemo(() => (lines ?? []).map((l) => new Map(l.points.map((p) => [p.date, p.value]))), [lines]);
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
      {lines?.map((l, i) => {
        const v = byLine[i]?.get(row.date);
        return (
          <span key={l.key} style={{ color: l.color }}>
            {l.label} {v === undefined ? "—" : l.format ? l.format(v) : v.toFixed(2)}
          </span>
        );
      })}
    </div>
  );
}
