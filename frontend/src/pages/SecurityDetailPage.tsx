import { keepPreviousData } from "@tanstack/react-query";
import { useState } from "react";
import { useParams } from "react-router";
import { useGetSecurity, useSecurityBars, useSecurityFundamentals } from "../api/generated/market/market";
import { PriceChart } from "../components/PriceChart";
import { RangePicker, rangeQuery } from "../components/RangePicker";
import { Badge, Card, Empty, ErrorState, Segmented, Spinner, Table } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useSearchState } from "../hooks/useSearchState";
import { fmtBig, fmtInt, fmtPercent, fmtPrice, toNumber, upDownClass } from "../lib/format";
import { ADJUSTS, type Adjust, ASSET_TYPES, BOARDS, financialIndicatorName } from "../lib/labels";
import { DEFAULT_MA, fetchStart, maPeriod } from "../lib/movingAverage";

export function SecurityDetailPage() {
  const { thscode = "" } = useParams();
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
      <ChartCard thscode={thscode} assetType={sec?.asset_type} earliest={sec?.first_bar} />
      <FundamentalsCard thscode={thscode} assetType={sec?.asset_type} />
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
    ...DEFAULT_MA,
  });
  const ma = [maPeriod(s.ma1, DEFAULT_MA.ma1), maPeriod(s.ma2, DEFAULT_MA.ma2)];
  const [maSlot, setMaSlot] = useState<HTMLDivElement | null>(null);
  const adjustOptions = assetType === "etf" ? ADJUSTS.filter((x) => ["none", "qfq", "hfq"].includes(x.value)) : ADJUSTS;
  const adjust = (assetType === "etf" && !["none", "qfq", "hfq"].includes(s.adjust) ? "qfq" : s.adjust) as Adjust;
  const { start, end } = rangeQuery(s);
  const bars = useSecurityBars(
    thscode,
    { adjust, start: fetchStart(start, ma), end, limit: 10000 },
    { query: { placeholderData: keepPreviousData } },
  );
  return (
    <Card title="K 线">
      <div className="mb-3 flex flex-wrap items-start justify-between gap-2">
        <RangePicker value={s} earliest={earliest} loading={bars.isPlaceholderData} onChange={set} />
        {/* 均线控件由 K 线图渲染到这里 */}
        <div ref={setMaSlot} className="ml-auto" />
      </div>
      {bars.error ? (
        <ErrorState error={bars.error} onRetry={() => bars.refetch()} />
      ) : !bars.data ? (
        <Spinner />
      ) : bars.data.bars.length === 0 ? (
        <Empty>本地还没有这只股票的日线，可能仍在回补中</Empty>
      ) : (
        <PriceChart
          bars={bars.data.bars}
          ma={ma}
          onMa={(i, n) => set(i === 0 ? { ma1: String(n) } : { ma2: String(n) })}
          showFrom={start}
          maSlot={maSlot}
        />
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
