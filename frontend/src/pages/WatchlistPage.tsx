import { Link } from "react-router";
import type { WatchItem } from "../api/generated/model";
import { useListWatchlist } from "../api/generated/watchlist/watchlist";
import { SecuritySearch } from "../components/SecuritySearch";
import { Badge, Card, Empty, PageHeader, QueryView, Table } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { fmtPercent, fmtPrice, upDownClass } from "../lib/format";

/** 自选：收藏的股票一览，加减收藏。点股票看 K 线和财务 */
export function WatchlistPage() {
  const query = useListWatchlist();
  return (
    <>
      <PageHeader
        title="自选"
        desc="收藏的股票。点股票看 K 线和财务。"
        extra={<SecuritySearch action={(code) => <WatchButton thscode={code} />} width="w-64" />}
      />
      <Card>
        <QueryView query={query}>
          {(data) =>
            data.length === 0 ? <Empty>还没有收藏的股票。在右上角搜索，点 ☆ 收藏。</Empty> : <WatchTable items={data} />
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
        </tr>
      </thead>
      <tbody>
        {items.map((x) => (
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
              <Change v={x.change_1d} />
            </td>
            <td className="num text-right">
              <Change v={x.change_20d} />
            </td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
