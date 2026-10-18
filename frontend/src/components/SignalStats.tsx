import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import type { HorizonStat, SignalStats, SignalTarget } from "../api/generated/model";
import { signalStats, signalToday } from "../api/generated/signal/signal";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt, fmtPctValue, upDownClass } from "../lib/format";
import { Card, Empty, ErrorState, errorText, Segmented, Spinner, Table } from "./ui";

const YEARS = [
  { value: "1", label: "近 1 年" },
  { value: "3", label: "近 3 年" },
  { value: "5", label: "近 5 年" },
] as const;

const ratio = (v: number | null | undefined) => (v === null || v === undefined ? "—" : `${(v * 100).toFixed(0)}%`);

/** 过去表现：这个信号出现之后几天的涨跌，和同期所有股票对比。全市场统计第一次要十几秒，之后有缓存 */
export function SignalStatsCard({ target, enabled = true }: { target: SignalTarget; enabled?: boolean }) {
  const [s, set] = useSearchState({ years: "3" });
  const q = useQuery({
    queryKey: ["signal-stats", JSON.stringify(target), s.years],
    queryFn: () => signalStats({ target, years: Number(s.years) }),
    enabled,
    staleTime: 10 * 60_000,
    retry: false,
  });
  return (
    <Card
      title="过去表现：出现之后涨了还是跌了"
      extra={<Segmented value={s.years} options={YEARS} onChange={(years) => set({ years })} />}
    >
      {q.isPending ? (
        <Spinner label="正在用全市场的日线逐只统计，第一次要十几秒" />
      ) : q.error ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <StatsView d={q.data} />
      )}
    </Card>
  );
}

function StatsView({ d }: { d: SignalStats }) {
  if (d.occurrences === 0) return <Empty>{`${d.start} 至 ${d.end} 没有出现过`}</Empty>;
  const h20 = d.horizons.find((h) => h.days === 20);
  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm">
        {d.start} 至 {d.end}，在 {fmtInt(d.stocks)} 只股票上一共出现 <b>{fmtInt(d.occurrences)}</b> 次。
        {h20 && h20.avg_pct !== null && h20.avg_pct !== undefined && <Summary h={h20} />}
      </p>
      <Table>
        <thead>
          <tr>
            <th>出现之后</th>
            <th className="text-right!" title="有这么多天后续数据的次数">
              次数
            </th>
            <th className="text-right!">平均涨跌</th>
            <th className="text-right!" title="把每次的涨跌从小到大排，排在正中间的那个；不受个别暴涨暴跌影响">
              中间值
            </th>
            <th className="text-right!">上涨的比例</th>
            <th className="text-right!" title="同一段时间里，所有股票任意一天之后同样天数的表现">
              同期所有股票平均
            </th>
            <th className="text-right!">同期上涨比例</th>
          </tr>
        </thead>
        <tbody>
          {d.horizons.map((h) => (
            <tr key={h.days}>
              <td>{h.days} 个交易日</td>
              <td className="num text-right">{fmtInt(h.samples)}</td>
              <td className={`num text-right ${upDownClass(h.avg_pct)}`}>{fmtPctValue(h.avg_pct)}</td>
              <td className={`num text-right ${upDownClass(h.median_pct)}`}>{fmtPctValue(h.median_pct)}</td>
              <td className="num text-right">{ratio(h.up_ratio)}</td>
              <td className={`num text-right text-zinc-500 ${upDownClass(h.market_avg_pct)}`}>
                {fmtPctValue(h.market_avg_pct)}
              </td>
              <td className="num text-right text-zinc-500">{ratio(h.market_up_ratio)}</td>
            </tr>
          ))}
        </tbody>
      </Table>
      <details className="text-sm">
        <summary className="cursor-pointer text-zinc-500 select-none">最近的 {d.recent.length} 次</summary>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {d.recent.map((o) => (
            <Link
              key={`${o.trade_date}-${o.thscode}`}
              to={`/securities/${o.thscode}`}
              className="inline-flex items-baseline gap-1 rounded-md border border-zinc-200 px-1.5 py-0.5 text-xs hover:border-accent/60 dark:border-zinc-700"
              title="出现后 20 个交易日的涨跌；还不满 20 天时为空"
            >
              <span className="num text-zinc-400">{o.trade_date.slice(2)}</span>
              {o.name}
              <span className={`num ${upDownClass(o.after_20d_pct)}`}>{fmtPctValue(o.after_20d_pct, 1)}</span>
            </Link>
          ))}
        </div>
      </details>
      <ul className="list-disc space-y-0.5 pl-5 text-xs text-zinc-500">
        <li>涨跌按收盘价算：出现那天收盘到之后第 5、10、20 个交易日收盘，前复权，不扣手续费。</li>
        <li>连续几天都满足只算第一天。股票池：{d.universe.join("，")}。</li>
        <li>和「同期所有股票」比，才知道它是不是真的比随便买好；只是过去的统计，不代表以后。</li>
      </ul>
    </div>
  );
}

/** 用 20 个交易日的结果写一句话结论 */
function Summary({ h }: { h: HorizonStat }) {
  const avg = h.avg_pct ?? 0;
  const base = h.market_avg_pct ?? 0;
  const diff = avg - base;
  const verdict =
    Math.abs(diff) < 0.3
      ? "和随便买差不多"
      : diff > 0
        ? `比同期所有股票平均多涨 ${diff.toFixed(2)}%`
        : `比同期所有股票平均少涨 ${(-diff).toFixed(2)}%`;
  return (
    <>
      {" "}
      出现后 20 个交易日平均 <b className={upDownClass(avg)}>{fmtPctValue(avg)}</b>，上涨的占 {ratio(h.up_ratio)}，
      {verdict}。
    </>
  );
}

export function useSignalToday(target: SignalTarget, enabled = true) {
  return useQuery({
    queryKey: ["signal-today", JSON.stringify(target)],
    queryFn: () => signalToday({ target }),
    enabled,
    staleTime: 60_000,
    retry: false,
  });
}

/** 今天哪些股票出现了这个信号 */
export function SignalTodayCard({ target, enabled = true }: { target: SignalTarget; enabled?: boolean }) {
  const q = useSignalToday(target, enabled);
  return (
    <Card title={q.data ? `${q.data.trade_date} 出现在哪些股票上` : "今天出现在哪些股票上"}>
      {q.isPending ? (
        <Spinner />
      ) : q.error ? (
        <p className="text-sm text-red-600">{errorText(q.error)}</p>
      ) : q.data.matched === 0 ? (
        <p className="text-sm text-zinc-400">今天没有股票出现这个信号</p>
      ) : (
        <>
          <p className="mb-2 text-sm text-zinc-500">
            共 {fmtInt(q.data.matched)} 只（{q.data.universe.join("，")}）
            {q.data.matched > q.data.hits.length && `，成交额最大的 ${q.data.hits.length} 只：`}
          </p>
          <div className="flex flex-wrap gap-1.5">
            {q.data.hits.map((h) => (
              <Link
                key={h.thscode}
                to={`/securities/${h.thscode}`}
                className="inline-flex items-center gap-1 rounded-md border border-zinc-200 px-2 py-0.5 text-sm hover:border-accent/60 dark:border-zinc-700"
              >
                {h.name}
                <span className={`num text-xs ${upDownClass(h.change_pct)}`}>{fmtPctValue(h.change_pct, 1)}</span>
              </Link>
            ))}
          </div>
        </>
      )}
    </Card>
  );
}
