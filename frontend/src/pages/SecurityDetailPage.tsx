import { keepPreviousData } from "@tanstack/react-query";
import { useMemo } from "react";
import { useParams } from "react-router";
import { useMaCrossBacktest, useStrategyBacktest } from "../api/generated/backtest/backtest";
import { useGetSecurity, useSecurityBars } from "../api/generated/market/market";
import { useListSignals } from "../api/generated/signal/signal";
import { useCatalog, useListStrategies } from "../api/generated/strategy/strategy";
import { COST_DEFAULTS, CostFields, costParams, Loaded, Result, RulesNote } from "../components/backtest";
import { DiagnosisCard } from "../components/DiagnosisCard";
import { MaControls } from "../components/MaControls";
import { OverlayPicker, useOverlayParam, useOverlays } from "../components/overlays";
import { PriceChart } from "../components/PriceChart";
import { RangePicker, rangeQuery } from "../components/RangePicker";
import { Badge, Card, Empty, ErrorState, Field, inputClass, Segmented, Spinner } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt, fmtPercent, fmtPrice, RANGES, rangeStart, toNumber, upDownClass } from "../lib/format";
import { ADJUSTS, type Adjust, BOARDS } from "../lib/labels";

const TABS = [
  { value: "chart", label: "走势与诊断" },
  { value: "backtest", label: "回测", hint: "用历史数据检验：按某个方法买卖这只股票，过去能赚多少" },
] as const;

export function SecurityDetailPage() {
  const { thscode = "" } = useParams();
  const [s, set] = useSearchState({ tab: "chart" });
  const info = useGetSecurity(thscode);
  // 页头的最新价：最近两根日线就够
  const last = useSecurityBars(thscode, { adjust: "qfq", limit: 2 });

  if (info.error) return <ErrorState error={info.error} onRetry={() => info.refetch()} />;
  const sec = info.data;
  const bars = last.data?.bars ?? [];
  const close = toNumber(bars.at(-1)?.close);
  const prev = toNumber(bars.at(-2)?.close);
  const change = close !== null && prev ? close / prev - 1 : null;

  return (
    <>
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h1 className="flex flex-wrap items-baseline gap-2 text-xl font-semibold">
            {sec?.name ?? thscode}
            <span className="num font-mono text-sm font-normal text-zinc-500">{thscode}</span>
            {sec && <Badge>{BOARDS[sec.board] ?? sec.board}</Badge>}
            {sec?.delisted_on && <Badge tone="bad">已退市 {sec.delisted_on}</Badge>}
          </h1>
          {close !== null && (
            <p className="num mt-1 flex items-baseline gap-3">
              <span className={`text-2xl font-semibold ${upDownClass(change)}`}>{fmtPrice(close)}</span>
              <span className={upDownClass(change)}>{fmtPercent(change)}</span>
              <span className="text-xs text-zinc-500">{bars.at(-1)?.trade_date} 收盘</span>
            </p>
          )}
          {sec && (
            <p className="mt-1 text-xs text-zinc-400">
              上市 {sec.list_date ?? "—"} · 本地日线 {sec.first_bar ?? "—"} 至 {sec.last_bar ?? "—"}，共{" "}
              {fmtInt(sec.bars)} 根
              {sec.name_history.length > 1 && (
                <>
                  {" "}
                  · 曾用名{" "}
                  {sec.name_history
                    .slice(1)
                    .map((n) => `${n.name}（至 ${n.valid_to ?? "今"}）`)
                    .join("、")}
                </>
              )}
            </p>
          )}
        </div>
        <WatchButton thscode={thscode} withLabel />
      </div>
      <div>
        <Segmented value={s.tab as (typeof TABS)[number]["value"]} options={TABS} onChange={(tab) => set({ tab })} />
      </div>
      {s.tab === "backtest" ? (
        <StockBacktest thscode={thscode} />
      ) : (
        <>
          <DiagnosisCard thscode={thscode} />
          <ChartCard thscode={thscode} earliest={sec?.first_bar} />
        </>
      )}
    </>
  );
}

function ChartCard({ thscode, earliest }: { thscode: string; earliest?: string | null }) {
  const [s, set] = useSearchState({
    adjust: "qfq",
    range: "3",
    from: "",
    to: "",
  });
  const catalog = useCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const signals = useListSignals({ query: { staleTime: 60_000 } });
  const strategies = useListStrategies();
  const adjust = s.adjust as Adjust;
  const { start, end } = rangeQuery(s);
  const bars = useSecurityBars(
    thscode,
    { adjust, start, end, limit: 10000 },
    { query: { placeholderData: keepPreviousData } },
  );
  const [overlays, setOverlays] = useOverlayParam();
  const ov = useOverlays({
    thscode,
    overlays,
    start,
    end,
    lastDate: bars.data?.bars.at(-1)?.trade_date,
    catalog: catalog.data,
    signals: signals.data,
    strategies: strategies.data,
  });
  // 指标按等比前复权价计算，画在其他口径的 K 线上会对不上
  const lines = useMemo(
    () => (adjust === "qfq" ? ov.lines : ov.lines.filter((l) => l.pane !== "price")),
    [adjust, ov.lines],
  );
  return (
    <Card title="K 线">
      {/* 操作都放在图的上方：时间区间、均线周期、叠加的指标信号和策略买卖点，改完马上能在图上看到 */}
      <div className="mb-3 flex flex-col gap-3">
        <RangePicker value={s} earliest={earliest} loading={bars.isPlaceholderData} onChange={set} />
        <OverlayPicker
          overlays={overlays}
          onChange={setOverlays}
          infos={ov.infos}
          catalog={catalog.data}
          signals={signals.data}
          strategies={strategies.data}
        />
        {adjust !== "qfq" && (
          <p className="text-xs text-zinc-500">指标和信号按等比前复权价计算，切换回「等比前复权」后画在 K 线上。</p>
        )}
      </div>
      {bars.error ? (
        <ErrorState error={bars.error} onRetry={() => bars.refetch()} />
      ) : !bars.data ? (
        <Spinner />
      ) : bars.data.bars.length === 0 ? (
        <Empty>本地还没有这只股票的日线，可能仍在回补中</Empty>
      ) : (
        <PriceChart bars={bars.data.bars} lines={lines} markers={ov.markers} />
      )}
      <details className="mt-3 text-sm">
        <summary className="cursor-pointer text-zinc-500 select-none">价格口径</summary>
        <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-zinc-500">
          <Segmented value={adjust} options={ADJUSTS} onChange={(adjust) => set({ adjust })} />
          <span>{ADJUSTS.find((a) => a.value === adjust)?.hint}</span>
        </div>
      </details>
    </Card>
  );
}

/** 在这只股票上回测：均线交叉，或任意一个已保存的策略 */
function StockBacktest({ thscode }: { thscode: string }) {
  const [s, set] = useSearchState({ method: "ma", fast: "15", slow: "30", years: "10", ...COST_DEFAULTS, log: "" });
  const strategies = useListStrategies();
  const fast = Number(s.fast) || 15;
  const slow = Number(s.slow) || 30;
  const strategyId = s.method === "ma" ? 0 : Number(s.method) || 0;
  const common = { thscode, start: rangeStart(s.years), ...costParams(s) };
  const opts = { placeholderData: keepPreviousData, retry: false } as const;
  const ma = useMaCrossBacktest({ ...common, fast, slow }, { query: { enabled: strategyId === 0, ...opts } });
  const st = useStrategyBacktest(
    { ...common, strategy_id: strategyId },
    { query: { enabled: strategyId > 0, ...opts } },
  );
  const log = s.log === "1";
  const onLog = (v: boolean) => set({ log: v ? "1" : "" });
  return (
    <>
      <Card>
        <div className="flex flex-wrap items-end gap-4">
          <Field label="买卖方法">
            <select className={`${inputClass} w-56`} value={s.method} onChange={(e) => set({ method: e.target.value })}>
              <option value="ma">均线交叉：金叉买、死叉卖</option>
              {(strategies.data ?? []).length > 0 && (
                <optgroup label="按策略">
                  {strategies.data?.map((x) => (
                    <option key={x.strategy_id} value={x.strategy_id}>
                      {x.name}
                    </option>
                  ))}
                </optgroup>
              )}
            </select>
          </Field>
          {strategyId === 0 && (
            <MaControls fast={fast} slow={slow} onChange={(f, sl) => set({ fast: String(f), slow: String(sl) })} />
          )}
          <div className="flex flex-col gap-1 text-xs text-zinc-500">
            <span>看过去多久</span>
            <Segmented value={s.years} options={RANGES} onChange={(years) => set({ years })} />
          </div>
          <CostFields s={s} set={set} />
        </div>
      </Card>
      {strategyId === 0 ? (
        <Loaded query={ma}>
          {(d) => (
            <Result
              label={`${d.fast} 日 / ${d.slow} 日均线交叉`}
              summary={d.summary}
              equity={d.equity}
              trades={d.trades}
              log={log}
              onLog={onLog}
            />
          )}
        </Loaded>
      ) : (
        <Loaded query={st}>
          {(d) => (
            <Result
              label={`策略「${d.strategy_name}」`}
              summary={d.summary}
              equity={d.equity}
              trades={d.trades}
              log={log}
              onLog={onLog}
            />
          )}
        </Loaded>
      )}
      <RulesNote
        items={[
          strategyId === 0
            ? "收盘后才知道当天是否金叉或死叉，所以在下一个交易日开盘时按开盘价买入或卖出。"
            : "收盘后才知道当天是否符合策略，所以在下一个交易日开盘时买入；卖出规则（卖出信号、止损、止盈、持有到期）满足任意一条后，下一个交易日开盘卖出。",
          "只做多，买入时用全部可用资金、按每手 100 股买；卖出时全部卖出。剩余不够一手的钱留作现金。",
          "开盘就一字涨停买不进、一字跌停卖不出时，顺延到下一个交易日。",
          "信号按等比前复权价判断；买卖和市值按交易所实际价格计算。现金分红（税前）计入现金、不再买入，送转股按比例增加股数，不参与配股。",
          "对比「一直拿着不动」：同样的钱在区间第一天开盘全部买入，一直拿到最后，不计手续费。",
        ]}
      />
    </>
  );
}
