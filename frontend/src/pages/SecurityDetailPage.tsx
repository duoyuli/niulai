import { keepPreviousData } from "@tanstack/react-query";
import { useMemo } from "react";
import { useParams } from "react-router";
import { useStrategyBacktest } from "../api/generated/backtest/backtest";
import { useGetSecurity, useSecurityBars, useSecurityFundamentals } from "../api/generated/market/market";
import { useListSignals } from "../api/generated/signal/signal";
import { useCatalog, useListStrategies } from "../api/generated/strategy/strategy";
import { COST_DEFAULTS, CostFields, costParams, Loaded, Result, RulesNote } from "../components/backtest";
import { DiagnosisCard } from "../components/DiagnosisCard";
import { SINGLE_STOCK_ESTIMATE, SINGLE_STOCK_ESTIMATE_BASIS } from "../components/longTask";
import { OverlayPicker, useOverlayParam, useOverlays } from "../components/overlays";
import { PriceChart } from "../components/PriceChart";
import { RangePicker, rangeQuery } from "../components/RangePicker";
import { Badge, Card, Empty, ErrorState, Field, inputClass, Segmented, Spinner, Table } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useSearchState } from "../hooks/useSearchState";
import { fmtBig, fmtInt, fmtPercent, fmtPrice, RANGES, rangeStart, toNumber, upDownClass } from "../lib/format";
import { ADJUSTS, type Adjust, ASSET_TYPES, BOARDS, financialIndicatorName } from "../lib/labels";

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
            {sec && <Badge>{ASSET_TYPES[sec.asset_type] ?? sec.asset_type}</Badge>}
            {sec && sec.asset_type === "stock" && <Badge>{BOARDS[sec.board] ?? sec.board}</Badge>}
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
          <FundamentalsCard thscode={thscode} assetType={sec?.asset_type} />
          <ChartCard thscode={thscode} assetType={sec?.asset_type} earliest={sec?.first_bar} />
        </>
      )}
    </>
  );
}

function FundamentalsCard({ thscode, assetType }: { thscode: string; assetType?: string }) {
  const q = useSecurityFundamentals(thscode, { limit: 90 }, { query: { staleTime: 300_000 } });
  if (q.isPending)
    return (
      <Card title="基本面">
        <Spinner />
      </Card>
    );
  if (q.error)
    return (
      <Card title="基本面">
        <p className="text-sm text-zinc-500">基本面暂时无法加载。</p>
      </Card>
    );
  const d = q.data;
  if (assetType === "etf") {
    const e = d?.etf;
    return (
      <Card title="ETF 基本资料" extra={<span className="text-xs text-zinc-500">按 ETF 数据口径展示</span>}>
        {!e ? (
          <Empty>本地还没有 ETF 资料或日线，可能仍在同步中</Empty>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <FundamentalStat label="最新收盘" value={e.latest_close} />
            <FundamentalStat label="单位净值" value={e.unit_nav} />
            <FundamentalStat label="基金规模" value={e.fund_scale} big />
            <FundamentalStat label="管理费" value={e.management_fee} />
            <FundamentalText label="管理人" value={e.management_company} />
            <FundamentalText label="历史日线" value={e.bars ? `${e.bars} 根` : null} />
            <FundamentalText label="最新日期" value={e.latest_date} />
            <FundamentalText label="价格口径" value={priceBasisLabel(e.price_basis)} />
          </div>
        )}
      </Card>
    );
  }
  if (!d || (d.valuation.length === 0 && !d.income && !d.balance && !d.cashflow && d.indicators.length === 0)) {
    return (
      <Card title="基本面">
        <Empty>本地还没有估值或财务数据，可能仍在回补中</Empty>
      </Card>
    );
  }
  const latest = d.valuation[0];
  return (
    <Card title="基本面" extra={<span className="text-xs text-zinc-500">数据来自本地已同步内容</span>}>
      {latest && (
        <>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
            <FundamentalStat label="市盈率 TTM" value={latest.pe_ttm} />
            <FundamentalStat label="市盈率 MRQ" value={latest.pe_mrq} />
            <FundamentalStat label="市净率" value={latest.pb_mrq} />
            <FundamentalStat label="市销率 TTM" value={latest.ps_ttm} />
            <FundamentalStat label="市现率 TTM" value={latest.pcf_ttm} />
            <FundamentalStat label="流通市值" value={latest.float_market_cap} big />
          </div>
          <p className="mt-2 text-xs text-zinc-500">估值截至 {latest.trade_date}；估值没有数据时显示「—」。</p>
        </>
      )}
      {d.valuation.length > 1 && (
        <details className="mt-4 text-sm">
          <summary className="cursor-pointer text-zinc-500 select-none">
            查看估值历史（最近 {d.valuation.length} 个交易日）
          </summary>
          <div className="mt-2 overflow-x-auto">
            <Table>
              <thead>
                <tr>
                  <th>日期</th>
                  <th>PE TTM</th>
                  <th>PB</th>
                  <th>PS TTM</th>
                  <th>流通市值</th>
                </tr>
              </thead>
              <tbody>
                {d.valuation.map((x) => (
                  <tr key={x.trade_date}>
                    <td className="num">{x.trade_date}</td>
                    <td className="num">{x.pe_ttm ?? "—"}</td>
                    <td className="num">{x.pb_mrq ?? "—"}</td>
                    <td className="num">{x.ps_ttm ?? "—"}</td>
                    <td className="num">{x.float_market_cap ? fmtBig(x.float_market_cap) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </Table>
          </div>
        </details>
      )}
      <div className="mt-4 grid gap-3 lg:grid-cols-3">
        <StatementBlock title="利润表" data={d.income} />
        <StatementBlock title="资产负债表" data={d.balance} />
        <StatementBlock title="现金流量表" data={d.cashflow} />
      </div>
      {d.indicators.length > 0 && (
        <div className="mt-4">
          <h3 className="text-sm font-medium">财务指标</h3>
          <div className="mt-2 grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
            {d.indicators.map((group) => (
              <div key={group.name} className="rounded-lg bg-zinc-50 p-3 dark:bg-zinc-800/60">
                <p className="text-xs font-medium text-zinc-500">{abilityName(group.name)}</p>
                <div className="mt-2 flex flex-col gap-1">
                  {group.metrics.map((m) => (
                    <div key={m.name} className="flex justify-between gap-2 text-xs">
                      <span className="truncate" title={financialIndicatorName(m.name)}>
                        {financialIndicatorName(m.name)}
                      </span>
                      <span className="num">{m.value.toFixed(2)}</span>
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      <p className="mt-3 text-xs text-zinc-500">
        报表按报告期末统计，同时显示披露日期；披露日期晚于报告期时，历史回测不能把它提前使用。
      </p>
    </Card>
  );
}

function FundamentalStat({ label, value, big = false }: { label: string; value?: string | null; big?: boolean }) {
  const n = value === null || value === undefined ? null : toNumber(value);
  return (
    <div>
      <p className="text-xs text-zinc-500">{label}</p>
      <p className="num mt-0.5 font-medium">{n === null ? "—" : big ? fmtBig(n) : n.toFixed(2)}</p>
    </div>
  );
}

function FundamentalText({ label, value }: { label: string; value?: string | null }) {
  return (
    <div>
      <p className="text-xs text-zinc-500">{label}</p>
      <p className="mt-0.5 truncate text-sm font-medium" title={value ?? undefined}>
        {value ?? "—"}
      </p>
    </div>
  );
}

function priceBasisLabel(value?: string | null) {
  switch (value) {
    case "vendor_forward_adjusted":
      return "供应商前复权（同花顺）";
    case "market_snapshot":
      return "收盘行情快照";
    default:
      return value;
  }
}

function StatementBlock({
  title,
  data,
}: {
  title: string;
  data?: { period_end?: string | null; report_date?: string | null; metrics: { name: string; value: number }[] } | null;
}) {
  if (!data)
    return (
      <div className="rounded-lg border border-dashed border-zinc-200 p-3 text-xs text-zinc-400 dark:border-zinc-700">
        {title}：暂无
      </div>
    );
  return (
    <div className="rounded-lg border border-zinc-200 p-3 dark:border-zinc-700">
      <p className="text-sm font-medium">{title}</p>
      <p className="mt-1 text-xs text-zinc-500">
        报告期 {data.period_end ?? "—"} · 披露 {data.report_date ?? "—"}
      </p>
      <div className="mt-2 flex flex-col gap-1">
        {data.metrics.map((m) => (
          <div key={m.name} className="flex justify-between gap-2 text-xs">
            <span>{m.name}</span>
            <span className="num">{fmtBig(m.value)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function abilityName(name: string) {
  return (
    (
      { growth: "成长", profitability: "盈利", solvency: "偿债", operation: "营运", "cash-flow": "现金流" } as Record<
        string,
        string
      >
    )[name] ?? "其他能力"
  );
}

function ChartCard({
  thscode,
  assetType,
  earliest,
}: {
  thscode: string;
  assetType?: string;
  earliest?: string | null;
}) {
  const [s, set] = useSearchState({
    adjust: "qfq",
    range: "3",
    from: "",
    to: "",
  });
  const catalog = useCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const signals = useListSignals({ query: { staleTime: 60_000 } });
  const strategies = useListStrategies();
  const adjustOptions = assetType === "etf" ? ADJUSTS.filter((x) => ["none", "qfq", "hfq"].includes(x.value)) : ADJUSTS;
  const adjust = (assetType === "etf" && !["none", "qfq", "hfq"].includes(s.adjust) ? "qfq" : s.adjust) as Adjust;
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
          <p className="text-xs text-zinc-500">
            {assetType === "etf"
              ? "ETF 历史价格按供应商前复权口径保存，指标和信号使用这份价格计算。"
              : "指标和信号按等比前复权价计算，切换回「等比前复权」后画在 K 线上。"}
          </p>
        )}
        {overlays.some((a) => a.startsWith("strategy:")) && (
          <p className="text-xs text-zinc-500">
            策略买卖点从所选区间起点空仓算起，不扣费用、不考虑资金；「只看强势板块」不生效（它只在每日巡检和预览里生效）。
          </p>
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
          <Segmented value={adjust} options={adjustOptions} onChange={(adjust) => set({ adjust })} />
          <span>{adjustOptions.find((a) => a.value === adjust)?.hint}</span>
        </div>
      </details>
    </Card>
  );
}

/** 在这只股票上回测：默认使用均线交叉策略，也可以选择任意一个已保存的策略 */
function StockBacktest({ thscode }: { thscode: string }) {
  const [s, set] = useSearchState({ method: "", years: "10", ...COST_DEFAULTS, log: "" });
  const strategies = useListStrategies();
  const defaultStrategy = strategies.data?.find((x) => x.template === "ma_cross_15_30");
  const strategyId = Number(s.method) || defaultStrategy?.strategy_id || 0;
  const selectedMethod = Number(s.method) > 0 ? s.method : defaultStrategy ? String(defaultStrategy.strategy_id) : "";
  const common = { thscode, start: rangeStart(s.years), ...costParams(s) };
  const opts = { placeholderData: keepPreviousData, retry: false } as const;
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
          <Field label="策略">
            <select
              className={`${inputClass} w-56`}
              value={selectedMethod}
              onChange={(e) => set({ method: e.target.value })}
              disabled={!strategies.data}
            >
              {!strategies.data && <option value="">加载中…</option>}
              {(strategies.data ?? []).length > 0 &&
                strategies.data?.map((x) => (
                  <option key={x.strategy_id} value={x.strategy_id}>
                    {x.name}
                  </option>
                ))}
            </select>
            <p className="mt-1 max-w-sm text-xs text-zinc-500">
              默认使用「均线交叉」（15 日 / 30 日）；它现在和其他策略一样，可用于个股回测和策略页面。
            </p>
          </Field>
          <div className="flex flex-col gap-1 text-xs text-zinc-500">
            <span>看过去多久</span>
            <Segmented value={s.years} options={RANGES} onChange={(years) => set({ years })} />
          </div>
          <CostFields s={s} set={set} />
          <span className="pb-2 text-xs text-zinc-500" title={SINGLE_STOCK_ESTIMATE_BASIS}>
            {SINGLE_STOCK_ESTIMATE}
          </span>
        </div>
      </Card>
      {strategyId > 0 ? (
        <Loaded query={st} pending="正在计算（这只股票通常几十毫秒）">
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
      ) : (
        <Card>
          <Spinner label="正在加载策略…" />
        </Card>
      )}
      <RulesNote
        items={[
          "收盘后才知道当天是否符合策略，所以在下一个交易日开盘时买入；卖出规则（卖出信号、止损、止盈、持有到期）满足任意一条后，下一个交易日开盘卖出。",
          "只做多，买入时用全部可用资金、按每手 100 股买；卖出时全部卖出。剩余不够一手的钱留作现金。",
          "开盘就一字涨停买不进、一字跌停卖不出时，顺延到下一个交易日。",
          "信号按等比前复权价判断；买卖和市值按交易所实际价格计算。现金分红（税前）计入现金、不再买入，送转股按比例增加股数，不参与配股。",
          "从回测开始日空仓起步。策略的买卖点和每日巡检、K 线买卖点用同一套规则；「只看强势板块」只在每日巡检和预览里生效，这里不生效。",
          "对比「一直拿着不动」：同样的钱在区间第一天开盘全部买入，一直拿到最后，不计手续费。",
        ]}
      />
    </>
  );
}
