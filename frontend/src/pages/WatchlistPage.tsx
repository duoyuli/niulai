import { keepPreviousData } from "@tanstack/react-query";
import { Link } from "react-router";
import type { WatchItem } from "../api/generated/model";
import { useListWatchlist } from "../api/generated/watchlist/watchlist";
import { MaControls } from "../components/MaControls";
import { SecuritySearch } from "../components/SecuritySearch";
import { Badge, Card, Empty, PageHeader, QueryView, SignalBadge, Table } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useSearchState } from "../hooks/useSearchState";
import { fmtPercent, fmtPrice, upDownClass } from "../lib/format";
import { BOARDS } from "../lib/labels";

export function WatchlistPage() {
  const [s, set] = useSearchState({ fast: "15", slow: "30" });
  const fast = Number(s.fast) || 15;
  const slow = Number(s.slow) || 30;
  const query = useListWatchlist({ fast, slow }, { query: { placeholderData: keepPreviousData } });

  return (
    <>
      <PageHeader
        title="自选"
        desc="收藏的股票和它们的最新状态，打开就能看出哪些在走强、哪些刚发生交叉。均线按等比前复权价计算。"
      />
      <Card>
        <div className="mb-4 flex flex-wrap items-end gap-4">
          <SecuritySearch label="添加股票" action={(code) => <WatchButton thscode={code} />} />
          <MaControls fast={fast} slow={slow} onChange={(f, sl) => set({ fast: String(f), slow: String(sl) })} />
        </div>
        <QueryView query={query}>
          {(data) =>
            data.items.length === 0 ? (
              <Empty>还没有收藏的股票。在上方搜索添加，或在股票列表、详情页点 ☆ 收藏。</Empty>
            ) : (
              <>
                <Overview items={data.items} />
                <WatchTable items={data.items} fast={data.fast} slow={data.slow} />
              </>
            )
          }
        </QueryView>
      </Card>
    </>
  );
}

function Overview({ items }: { items: WatchItem[] }) {
  const above = items.filter((x) => x.diagnosis?.ma?.trend === "above").length;
  const below = items.filter((x) => x.diagnosis?.ma?.trend === "below").length;
  const fresh = items.filter((x) => x.diagnosis?.ma?.last_cross?.bars_ago === 0).length;
  return (
    <p className="mb-3 text-sm text-zinc-500">
      共 {items.length} 只 · <span className="text-up">快线在上 {above} 只</span> ·{" "}
      <span className="text-down">快线在下 {below} 只</span>
      {fresh > 0 && <> · 最新交易日有 {fresh} 只发生交叉</>}
    </p>
  );
}

function Change({ v }: { v: number | null | undefined }) {
  if (v === null || v === undefined) return <span className="text-zinc-400">—</span>;
  return <span className={upDownClass(v)}>{fmtPercent(v)}</span>;
}

function WatchTable({ items, fast, slow }: { items: WatchItem[]; fast: number; slow: number }) {
  return (
    <Table>
      <thead>
        <tr>
          <th className="w-0" aria-label="收藏" />
          <th>股票</th>
          <th className="text-right!">最新收盘</th>
          <th className="text-right!" title="最近 1 个交易日的涨跌幅，按前复权价计算，除权除息不会造成假涨跌">
            当日
          </th>
          <th className="text-right!" title="最近 20 个交易日（约一个月）的涨跌幅">
            近 20 日
          </th>
          <th title={`MA${fast} 在 MA${slow} 上方，说明短期走势强于长期`}>
            均线（MA{fast} / MA{slow}）
          </th>
          <th title="最近约两年内的最后一次交叉">最近交叉</th>
          <th aria-label="操作" />
        </tr>
      </thead>
      <tbody>
        {items.map((x) => {
          const d = x.diagnosis;
          const ma = d?.ma;
          const cross = ma?.last_cross;
          return (
            <tr key={x.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
              <td className="pr-0!">
                <WatchButton thscode={x.thscode} />
              </td>
              <td>
                <Link to={`/securities/${x.thscode}`} className="flex flex-col hover:underline">
                  <span className="flex items-center gap-1.5">
                    {x.name}
                    {x.name.includes("ST") && <Badge tone="warn">ST</Badge>}
                    {x.delisted_on && <Badge tone="bad">已退市</Badge>}
                  </span>
                  <span className="num font-mono text-xs text-zinc-500">
                    {x.thscode} · {BOARDS[x.board] ?? x.board}
                  </span>
                </Link>
              </td>
              <td className="num text-right">
                {x.close ? (
                  <span className="flex flex-col">
                    <span>{fmtPrice(x.close)}</span>
                    <span className="text-xs text-zinc-500">{x.last_date}</span>
                  </span>
                ) : (
                  <span className="text-xs text-zinc-400">暂无日线</span>
                )}
              </td>
              <td className="num text-right">
                <Change v={d?.change_1d} />
              </td>
              <td className="num text-right">
                <Change v={d?.change_20d} />
              </td>
              <td>
                {ma ? (
                  <span className="flex flex-col">
                    {ma.trend === "above" ? (
                      <span className="text-up">快线在上</span>
                    ) : ma.trend === "below" ? (
                      <span className="text-down">快线在下</span>
                    ) : (
                      <span>两线持平</span>
                    )}
                    <span className="num text-xs text-zinc-500" title="快线相对慢线的偏离">
                      偏离 {fmtPercent(ma.ma_fast / ma.ma_slow - 1)}
                    </span>
                  </span>
                ) : (
                  <span className="text-xs text-zinc-400" title="还没有日线，或日线不够算慢线">
                    {d ? `日线不足 ${slow} 根` : "暂无日线"}
                  </span>
                )}
              </td>
              <td>
                {cross ? (
                  <span className="flex flex-col items-start gap-0.5">
                    <SignalBadge signal={cross.signal} />
                    <span className="num text-xs text-zinc-500">
                      {cross.trade_date} · {cross.bars_ago === 0 ? "最新交易日" : `${cross.bars_ago} 个交易日前`}
                    </span>
                  </span>
                ) : (
                  <span className="text-zinc-400">—</span>
                )}
              </td>
              <td>
                <Link
                  to={`/backtest?code=${x.thscode}&fast=${fast}&slow=${slow}`}
                  className="text-xs text-accent hover:underline"
                  title="用这组均线回测这只股票：金叉买入、死叉卖出，看历史上能赚多少"
                >
                  回测
                </Link>
              </td>
            </tr>
          );
        })}
      </tbody>
    </Table>
  );
}
