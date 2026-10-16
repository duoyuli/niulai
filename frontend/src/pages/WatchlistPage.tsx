import { Link } from "react-router";
import type { WatchItem } from "../api/generated/model";
import { useListWatchlist } from "../api/generated/watchlist/watchlist";
import { SecuritySearch } from "../components/SecuritySearch";
import { SignalChip } from "../components/SignalChip";
import { Badge, Card, Empty, PageHeader, QueryView, Table } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { fmtPercent, fmtPrice, upDownClass } from "../lib/format";

/** 自选：收藏的股票一览，加减收藏。每只股票的详细诊断在个股页 */
export function WatchlistPage() {
  const query = useListWatchlist();
  return (
    <>
      <PageHeader
        title="自选"
        desc="收藏的股票。红色信号偏多、绿色偏空；点股票看完整诊断。"
        extra={<SecuritySearch action={(code) => <WatchButton thscode={code} />} width="w-64" />}
      />
      <Card>
        <QueryView query={query}>
          {(data) =>
            data.items.length === 0 ? (
              <Empty>还没有收藏的股票。在右上角搜索，点 ☆ 收藏。</Empty>
            ) : (
              <WatchTable items={data.items} />
            )
          }
        </QueryView>
      </Card>
    </>
  );
}

function Change({ v }: { v: number | null | undefined }) {
  if (v === null || v === undefined) return <span className="text-zinc-400">—</span>;
  return <span className={upDownClass(v)}>{fmtPercent(v)}</span>;
}

function WatchTable({ items }: { items: WatchItem[] }) {
  return (
    <Table>
      <thead>
        <tr>
          <th className="w-0" aria-label="收藏" />
          <th>股票</th>
          <th className="text-right!">收盘</th>
          <th className="text-right!">当日</th>
          <th className="text-right!" title="最近 20 个交易日（约一个月）的涨跌幅">
            近 20 日
          </th>
          <th title="15 日均线在 30 日均线上方，说明短期走势强于长期">趋势</th>
          <th title="最近 5 个交易日出现过的信号">最近信号</th>
        </tr>
      </thead>
      <tbody>
        {items.map((x) => {
          const d = x.diagnosis;
          const trend = d?.ma?.trend;
          return (
            <tr key={x.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
              <td className="pr-0!">
                <WatchButton thscode={x.thscode} />
              </td>
              <td>
                <Link to={`/securities/${x.thscode}`} className="flex items-baseline gap-1.5 hover:underline">
                  {x.name}
                  <span className="num font-mono text-xs text-zinc-500">{x.thscode}</span>
                  {x.delisted_on && <Badge tone="bad">已退市</Badge>}
                </Link>
              </td>
              <td className="num text-right">
                {x.close ? fmtPrice(x.close) : <span className="text-xs text-zinc-400">暂无日线</span>}
              </td>
              <td className="num text-right">
                <Change v={d?.change_1d} />
              </td>
              <td className="num text-right">
                <Change v={d?.change_20d} />
              </td>
              <td>
                {trend === "above" ? (
                  <span className="text-up">走强</span>
                ) : trend === "below" ? (
                  <span className="text-down">走弱</span>
                ) : (
                  <span className="text-zinc-400">—</span>
                )}
              </td>
              <td className="whitespace-normal">
                {x.recent_signals.length > 0 ? (
                  <span className="flex flex-wrap gap-1">
                    {x.recent_signals.slice(0, 4).map((e) => (
                      <SignalChip key={`${e.trade_date}-${e.signal}`} event={e} withDate />
                    ))}
                  </span>
                ) : (
                  <span className="text-zinc-400">—</span>
                )}
              </td>
            </tr>
          );
        })}
      </tbody>
    </Table>
  );
}
