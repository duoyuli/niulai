import { keepPreviousData } from "@tanstack/react-query";
import { Link } from "react-router";
import { useGetSecurity, useSecurityBars } from "../api/generated/market/market";
import { useListSignals } from "../api/generated/signal/signal";
import { useCatalog, useListStrategies } from "../api/generated/strategy/strategy";
import { useSearchState } from "../hooks/useSearchState";
import { CHART_RANGES, rangeStart } from "../lib/format";
import { type Overlay, OverlayPicker, useOverlays } from "./overlays";
import { PriceChart } from "./PriceChart";
import { SecuritySearch } from "./SecuritySearch";
import { Card, Empty, ErrorState, Segmented, Spinner } from "./ui";

const RANGES = CHART_RANGES.filter((r) => ["6m", "1", "3", "5"].includes(r.value));
const DEFAULT_CODE = "600519.SH";

/** 在一只股票的 K 线上看信号或指标：可以换股票、换区间，也可以再叠加别的 */
export function SampleChart({
  title,
  overlays,
  suggestions = [],
  suggestLabel = "今天出现的：",
}: {
  title: string;
  /** 固定叠加的项（这一页讲的那个信号或指标） */
  overlays: Overlay[];
  /** 推荐看的股票，例如最近出现过这个信号的 */
  suggestions?: { thscode: string; name: string }[];
  suggestLabel?: string;
}) {
  const [s, set] = useSearchState({ code: "", range: "1", more: "" });
  const code = s.code || suggestions[0]?.thscode || DEFAULT_CODE;
  const start = rangeStart(s.range);
  const sec = useGetSecurity(code);
  const bars = useSecurityBars(
    code,
    { adjust: "qfq", start, limit: 10000 },
    { query: { placeholderData: keepPreviousData } },
  );
  const catalog = useCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const signals = useListSignals({ query: { staleTime: 60_000 } });
  const strategies = useListStrategies();
  const more = s.more ? s.more.split(",") : [];
  const all = [...overlays, ...more.filter((m) => !overlays.includes(m))];
  const ov = useOverlays({
    thscode: code,
    overlays: all,
    start,
    lastDate: bars.data?.bars.at(-1)?.trade_date,
    catalog: catalog.data,
    signals: signals.data,
    strategies: strategies.data,
  });
  return (
    <Card title={title} extra={<Segmented value={s.range} options={RANGES} onChange={(range) => set({ range })} />}>
      <div className="mb-3 flex flex-wrap items-center gap-2 text-sm">
        <span className="text-zinc-500">股票</span>
        <Link to={`/securities/${code}`} className="font-medium hover:underline">
          {sec.data?.name ?? code}
        </Link>
        <SecuritySearch onPick={(c) => set({ code: c })} width="w-40" />
        {suggestions.length > 0 && <span className="text-xs text-zinc-400">{suggestLabel}</span>}
        {suggestions
          .filter((x) => x.thscode !== code)
          .slice(0, 6)
          .map((x) => (
            <button
              key={x.thscode}
              type="button"
              className="rounded-md border border-zinc-200 px-1.5 py-0.5 text-xs hover:border-accent/60 dark:border-zinc-700"
              onClick={() => set({ code: x.thscode })}
            >
              {x.name}
            </button>
          ))}
      </div>
      <div className="mb-3">
        <OverlayPicker
          overlays={all}
          onChange={(o) => set({ more: o.filter((x) => !overlays.includes(x)).join(",") })}
          infos={ov.infos}
          catalog={catalog.data}
          signals={signals.data}
          strategies={strategies.data}
        />
      </div>
      {bars.error ? (
        <ErrorState error={bars.error} onRetry={() => bars.refetch()} />
      ) : !bars.data ? (
        <Spinner />
      ) : bars.data.bars.length === 0 ? (
        <Empty>本地还没有这只股票的日线</Empty>
      ) : (
        <PriceChart bars={bars.data.bars} lines={ov.lines} markers={ov.markers} height={380} />
      )}
      <p className="mt-2 text-xs text-zinc-400">K 线是等比前复权价，指标和信号都按它计算。</p>
    </Card>
  );
}
