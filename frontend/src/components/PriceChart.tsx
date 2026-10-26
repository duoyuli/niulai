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
import { createPortal } from "react-dom";
import type { Bar } from "../api/generated/model";
import { useDarkMode } from "../hooks/useDarkMode";
import { fmtBig, fmtPercent, fmtPrice, toNumber, upDownClass } from "../lib/format";
import { crosses, MA_MAX, MA_MIN, movingAverage } from "../lib/movingAverage";
import { inputClass } from "./ui";

const UP = "#d93a3a";
const DOWN = "#16a05d";
/** 快线、慢线的颜色：按行情软件的习惯，较短的均线用黄色、较长的用紫色，和红绿涨跌色区分开。
 * 浅色背景下黄色调深一些，深色背景下两种颜色调亮一些 */
const MA_COLORS = { light: ["#d99a00", "#b5179e"], dark: ["#facc15", "#e879f9"] };

interface Props {
  bars: Bar[];
  height?: number;
  /** 价格小数位：个股 2 位，指数 2 位 */
  digits?: number;
  /** 均线周期（交易日），默认快线、慢线两条；onMa 改第 i 条的周期 */
  ma?: number[];
  onMa?: (i: number, n: number) => void;
  /** 只显示这一天及以后的 K 线；更早的几根只用来算均线，让均线从第一根就有值 */
  showFrom?: string;
  /** 均线控件放到这个元素里（页面上日期区间那一行的右侧）；不传时不显示控件 */
  maSlot?: HTMLElement | null;
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
  /** 各条均线在这一天的值，不够天数时为空 */
  ma: (number | null)[];
  /** 快线这一天上穿（buy）或下穿（sell）慢线 */
  cross: "buy" | "sell" | null;
}

/** 框选中的区间：起点和当前位置的横坐标（像素）和对应的 K 线下标 */
interface Brush {
  x0: number;
  x1: number;
  i0: number;
  i1: number;
}

// @project-doc docs/architecture/frontend.md#conventions
/** K 线 + 均线 + 成交量。A 股配色：红涨绿跌。
 * 在图上按住鼠标左右拖动框选一段时间，松开后放大到这段；滚轮缩放，拖动底部时间轴平移，「还原」回到全部 */
export function PriceChart({ bars, height = 460, digits = 2, ma = [], onMa, showFrom, maSlot }: Props) {
  const el = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const dark = useDarkMode();
  const [hover, setHover] = useState<Hover | null>(null);
  const [brush, setBrush] = useState<Brush | null>(null);
  const [zoom, setZoom] = useState<{ from: string; to: string } | null>(null);
  const [marks, setMarks] = useState(true);

  const maKey = ma.join(",");
  // 颜色跟着快慢走：快线黄色、慢线紫色，不管它是第几条
  const maColors = useMemo(() => {
    const [fast = "", slow = ""] = MA_COLORS[dark ? "dark" : "light"];
    return fastFirst(maKey) ? [fast, slow] : [slow, fast];
  }, [dark, maKey]);
  const rows = useMemo(() => {
    const closes = bars.map((b) => toNumber(b.close) ?? 0);
    const lines = maKey ? maKey.split(",").map((n) => movingAverage(closes, Number(n))) : [];
    // 周期短的是快线：两个周期被改成前长后短时，交叉的方向仍按短线穿过长线来判断
    const [fast, slow] = fastFirst(maKey) ? [lines[0], lines[1]] : [lines[1], lines[0]];
    const cross = fast && slow ? crosses(fast, slow) : [];
    const out: Hover[] = [];
    let prevClose: number | null = null;
    for (const [i, b] of bars.entries()) {
      const close = closes[i] ?? 0;
      if (showFrom && b.trade_date < showFrom) {
        prevClose = close;
        continue;
      }
      out.push({
        date: b.trade_date,
        open: toNumber(b.open) ?? 0,
        high: toNumber(b.high) ?? 0,
        low: toNumber(b.low) ?? 0,
        close,
        change: prevClose ? close / prevClose - 1 : null,
        volume: b.volume,
        amount: b.amount,
        ma: lines.map((l) => l[i] ?? null),
        cross: cross[i] ?? null,
      });
      prevClose = close;
    }
    return out;
  }, [bars, maKey, showFrom]);

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

    for (const [k, color] of maColors.entries()) {
      if (!rows.some((r) => r.ma[k] !== undefined)) continue;
      const line = chart.addSeries(LineSeries, {
        color,
        lineWidth: 2,
        priceLineVisible: false,
        lastValueVisible: false,
        crosshairMarkerVisible: false,
        priceFormat: { type: "price", precision: digits, minMove: 1 / 10 ** digits },
      });
      line.setData(rows.flatMap((r) => (r.ma[k] == null ? [] : [{ time: r.date as Time, value: r.ma[k] }])));
    }

    // 金叉标在 K 线下方（红色「买」），死叉标在上方（绿色「卖」）
    if (marks) {
      createSeriesMarkers(
        candles,
        rows.flatMap((r): SeriesMarker<Time>[] =>
          r.cross === "buy"
            ? [{ time: r.date as Time, position: "belowBar", shape: "arrowUp", color: UP, text: "买" }]
            : r.cross === "sell"
              ? [{ time: r.date as Time, position: "aboveBar", shape: "arrowDown", color: DOWN, text: "卖" }]
              : [],
        ),
      );
    }

    const volume = chart.addSeries(
      HistogramSeries,
      { priceFormat: { type: "volume" }, priceLineVisible: false, lastValueVisible: false },
      1,
    );
    volume.setData(
      rows.map((r) => ({ time: r.date as Time, value: r.volume, color: r.close >= r.open ? `${UP}99` : `${DOWN}99` })),
    );
    chart.panes()[1]?.setStretchFactor(0.25);

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
  }, [rows, dark, digits, marks, maColors]);

  const shown = hover ?? rows.at(-1) ?? null;

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
      {ma.length > 0 &&
        maSlot &&
        createPortal(
          <MaControls
            ma={ma}
            colors={maColors}
            row={shown}
            digits={digits}
            onMa={onMa}
            marks={marks}
            onMarks={setMarks}
          />,
          maSlot,
        )}
      <Legend row={shown} digits={digits} />
      <div
        className="relative w-full cursor-crosshair select-none"
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={() => setBrush(null)}
      >
        <div ref={el} style={{ height }} className="w-full" />
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

function Legend({ row, digits }: { row: Hover | null; digits: number }) {
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
    </div>
  );
}

/** 第一条均线的周期不长于第二条时，第一条是快线 */
function fastFirst(maKey: string): boolean {
  const [a = 0, b = 0] = maKey.split(",").map(Number);
  return a <= b;
}

/** 均线的图例和周期：色块、可改的周期（交易日）、当前悬停这一天的均线值，以及是否标出金叉、死叉 */
function MaControls({
  ma,
  colors,
  row,
  digits,
  onMa,
  marks,
  onMarks,
}: {
  ma: number[];
  colors: string[];
  row: Hover | null;
  digits: number;
  onMa?: (i: number, n: number) => void;
  marks: boolean;
  onMarks: (v: boolean) => void;
}) {
  return (
    <div className="num flex flex-col items-end gap-1 text-xs text-zinc-500">
      <div className="flex flex-wrap items-center justify-end gap-x-3 gap-y-1">
        {ma.map((n, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: 第几条均线就是身份
          <span key={i} className="inline-flex items-center gap-1">
            <span className="inline-block h-[3px] w-4 rounded" style={{ background: colors[i] }} />
            {(i === 0) === fastFirst(ma.join(",")) ? "快线" : "慢线"}
            {onMa ? <PeriodInput value={n} label={`第 ${i + 1} 条均线的周期`} onCommit={(v) => onMa(i, v)} /> : n}日
            <b style={{ color: colors[i] }}>{row?.ma[i] == null ? "—" : fmtPrice(row.ma[i], digits)}</b>
          </span>
        ))}
      </div>
      {ma.length >= 2 && (
        <label className="inline-flex items-center gap-1">
          <input type="checkbox" checked={marks} onChange={(e) => onMarks(e.target.checked)} />
          标出买卖点（快线上穿慢线为买，下穿为卖）
          {row?.cross && (
            <b className={row.cross === "buy" ? "text-up" : "text-down"}>
              {row.cross === "buy" ? "当天金叉" : "当天死叉"}
            </b>
          )}
        </label>
      )}
    </div>
  );
}

/** 均线周期输入：失焦或回车时提交，范围外时恢复原值 */
function PeriodInput({ value, label, onCommit }: { value: number; label: string; onCommit: (n: number) => void }) {
  const [v, setV] = useState(String(value));
  useEffect(() => setV(String(value)), [value]);
  const commit = () => {
    const n = Number(v);
    if (Number.isInteger(n) && n >= MA_MIN && n <= MA_MAX) {
      if (n !== value) onCommit(n);
    } else setV(String(value));
  };
  return (
    <input
      aria-label={label}
      title={`${MA_MIN} 到 ${MA_MAX} 个交易日`}
      inputMode="numeric"
      className={`${inputClass} w-12 px-1 py-0 text-center text-xs`}
      value={v}
      onChange={(e) => setV(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && commit()}
    />
  );
}
