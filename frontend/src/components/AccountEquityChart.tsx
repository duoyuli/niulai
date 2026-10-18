import { ColorType, createChart, LineSeries, type Time } from "lightweight-charts";
import { useEffect, useRef, useState } from "react";
import type { AccountEquityPoint } from "../api/generated/model";
import { useDarkMode } from "../hooks/useDarkMode";
import { yuan } from "../lib/accounts";
import { fmtBig, upDownClass } from "../lib/format";

const LINE = "#3b6ef5";

/** 账户净值曲线：每个交易日收盘后的总资产。鼠标移上去显示那一天的现金、持仓市值和当天盈亏 */
export function AccountEquityChart({ points, height = 260 }: { points: AccountEquityPoint[]; height?: number }) {
  const el = useRef<HTMLDivElement>(null);
  const dark = useDarkMode();
  const [hover, setHover] = useState<AccountEquityPoint | null>(null);

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
      localization: { locale: "zh-CN", dateFormat: "yyyy-MM-dd", priceFormatter: (v: number) => fmtBig(v) },
    });
    chart
      .addSeries(LineSeries, { color: LINE, lineWidth: 2, priceLineVisible: false, crosshairMarkerVisible: true })
      .setData(points.map((p) => ({ time: p.trade_date as Time, value: p.total_assets })));
    const byDate = new Map(points.map((p) => [p.trade_date, p]));
    chart.subscribeCrosshairMove((p) => setHover(p.time ? (byDate.get(String(p.time)) ?? null) : null));
    chart.timeScale().fitContent();
    return () => chart.remove();
  }, [points, dark]);

  const shown = hover ?? points.at(-1);
  return (
    <div className="flex flex-col gap-2">
      {shown && (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-zinc-500">
          <span className="num font-medium text-zinc-800 dark:text-zinc-200">{shown.trade_date}</span>
          <span className="num" style={{ color: LINE }}>
            总资产 {yuan(shown.total_assets)}
          </span>
          <span className={upDownClass(shown.day_pnl)}>当天 {yuan(shown.day_pnl)}</span>
          <span className="num">现金 {yuan(shown.cash)}</span>
          <span className="num">持仓市值 {yuan(shown.market_value)}</span>
        </div>
      )}
      <div ref={el} style={{ height }} className="w-full" />
    </div>
  );
}
