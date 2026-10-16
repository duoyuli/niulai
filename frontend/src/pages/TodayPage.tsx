import { useQueries } from "@tanstack/react-query";
import { clsx } from "clsx";
import { Link } from "react-router";
import { useMarketOverview, useSectors } from "../api/generated/market/market";
import type { Overview, ScreenResults, StrategyItem, WatchItem } from "../api/generated/model";
import { getResultsQueryOptions, useListStrategies } from "../api/generated/strategy/strategy";
import { useListWatchlist } from "../api/generated/watchlist/watchlist";
import { SignalChip } from "../components/SignalChip";
import { Card, Empty, ErrorState, QueryView, Spinner } from "../components/ui";
import { fmtInt, fmtPctValue, fmtPercent, upDownClass } from "../lib/format";

/** 每天打开看的一页：大盘怎么样 → 我的策略今天选出了什么 → 我关注的股票有什么变化 */
export function TodayPage() {
  const overview = useMarketOverview();
  return (
    <>
      <div>
        <h1 className="text-xl font-semibold">
          今日
          {overview.data && (
            <span className="num ml-2 text-sm font-normal text-zinc-500">{overview.data.trade_date} 收盘</span>
          )}
        </h1>
      </div>
      <div className="grid gap-4 lg:grid-cols-5">
        <Card className="lg:col-span-2">
          {overview.isPending ? (
            <Spinner />
          ) : overview.error ? (
            <ErrorState error={overview.error} onRetry={() => overview.refetch()} />
          ) : (
            <Market o={overview.data} />
          )}
        </Card>
        <Sectors />
      </div>
      <Picks />
      <Watch />
    </>
  );
}

function Market({ o }: { o: Overview }) {
  const total = Math.max(1, o.up + o.down + o.flat);
  const pct = (n: number) => `${(n / total) * 100}%`;
  const upShare = o.up / total;
  const mood = upShare >= 0.65 ? "普涨" : upShare <= 0.35 ? "普跌" : "涨跌互现";
  return (
    <div className="flex h-full flex-col justify-between gap-3">
      <div className="flex items-baseline justify-between">
        <h2 className="text-sm font-semibold">大盘</h2>
        <span className={clsx("text-sm font-medium", upShare >= 0.5 ? "text-up" : "text-down")}>{mood}</span>
      </div>
      <div>
        <div className="flex h-2 w-full overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800">
          <div className="bg-up" style={{ width: pct(o.up) }} />
          <div className="bg-zinc-300 dark:bg-zinc-600" style={{ width: pct(o.flat) }} />
          <div className="bg-down" style={{ width: pct(o.down) }} />
        </div>
        <div className="num mt-1 flex justify-between text-xs">
          <span className="text-up">涨 {fmtInt(o.up)}</span>
          <span className="text-down">跌 {fmtInt(o.down)}</span>
        </div>
      </div>
      <p className="num text-xs text-zinc-500">
        涨停 <span className="text-up">{o.limit_up}</span> · 跌停 <span className="text-down">{o.limit_down}</span> ·
        中间那只股票 <span className={upDownClass(o.median_change_pct)}>{fmtPctValue(o.median_change_pct)}</span> ·{" "}
        <span title="收盘价高于 20 日均线的股票占比，越高说明中短期走强的股票越多">
          {(o.above_ma20 * 100).toFixed(0)}% 站上 20 日均线
        </span>
      </p>
    </div>
  );
}

function Sectors() {
  const q = useSectors({ category: "industry", days: 5 });
  return (
    <Card className="lg:col-span-3">
      <div className="mb-2 flex items-baseline justify-between">
        <h2 className="text-sm font-semibold">近 5 日最强的行业</h2>
        <Link
          to="/indices?category=industry"
          className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
        >
          全部板块 →
        </Link>
      </div>
      <QueryView query={q}>
        {(rows) =>
          rows.length === 0 ? (
            <p className="text-sm text-zinc-400">还没有板块指数的日线</p>
          ) : (
            <ul className="grid grid-cols-2 gap-x-6 gap-y-1 text-sm sm:grid-cols-3">
              {rows.slice(0, 9).map((r) => (
                <li key={r.thscode}>
                  <Link
                    to={`/indices/${r.thscode}`}
                    className="flex justify-between gap-2 rounded px-1 py-0.5 hover:bg-zinc-50 dark:hover:bg-zinc-800/50"
                  >
                    <span className="truncate">{r.name}</span>
                    <span className={`num ${upDownClass(r.change_pct)}`}>{fmtPctValue(r.change_pct, 1)}</span>
                  </Link>
                </li>
              ))}
            </ul>
          )
        }
      </QueryView>
    </Card>
  );
}

/** 每个开启巡检的策略今天选出的股票，新入选的排在前面 */
function Picks() {
  const list = useListStrategies();
  const active = (list.data ?? []).filter((s) => s.screening && s.latest);
  const results = useQueries({ queries: active.map((s) => getResultsQueryOptions(s.strategy_id)) });
  return (
    <Card
      title="策略今天选出的股票"
      extra={
        <Link to="/strategies" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
          管理策略 →
        </Link>
      }
    >
      {list.isPending ? (
        <Spinner />
      ) : active.length === 0 ? (
        <Empty>
          还没有开启每日巡检的策略。
          <Link to="/strategies/new" className="text-accent hover:underline">
            新建一个
          </Link>
        </Empty>
      ) : (
        <ul className="flex flex-col divide-y divide-zinc-100 dark:divide-zinc-800">
          {active.map((s, i) => (
            <PickRow key={s.strategy_id} s={s} r={results[i]?.data} />
          ))}
        </ul>
      )}
    </Card>
  );
}

function PickRow({ s, r }: { s: StrategyItem; r?: ScreenResults }) {
  const latest = s.latest;
  const hits = r ? [...r.hits].sort((a, b) => Number(b.is_new) - Number(a.is_new) || a.rank - b.rank).slice(0, 6) : [];
  return (
    <li className="flex flex-col gap-1.5 py-3 sm:flex-row sm:items-center sm:gap-4">
      <Link
        to={`/strategies/${s.strategy_id}`}
        className="flex shrink-0 flex-col hover:underline sm:w-48"
        title={s.description}
      >
        <span className="font-medium">{s.name}</span>
        <span className="text-xs text-zinc-500">
          {latest ? (
            <>
              {fmtInt(latest.matched)} 只符合
              {latest.new_hits > 0 && <span className="text-up"> · 新 {latest.new_hits}</span>}
            </>
          ) : (
            "还没有巡检"
          )}
        </span>
      </Link>
      <div className="flex min-w-0 flex-wrap gap-1.5">
        {hits.length === 0 && latest && latest.matched === 0 && (
          <span className="text-sm text-zinc-400">今天没有符合的股票</span>
        )}
        {hits.map((h) => (
          <Link
            key={h.thscode}
            to={`/securities/${h.thscode}`}
            title={h.reasons.join("；")}
            className="inline-flex items-center gap-1 rounded-md border border-zinc-200 px-2 py-0.5 text-sm hover:border-accent/60 dark:border-zinc-700"
          >
            {h.name}
            <span className={`num text-xs ${upDownClass(h.change_pct)}`}>{fmtPctValue(h.change_pct, 1)}</span>
            {h.is_new && <span className="text-xs text-up">新</span>}
          </Link>
        ))}
        {latest && latest.matched > hits.length && hits.length > 0 && (
          <Link to={`/strategies/${s.strategy_id}`} className="self-center text-xs text-zinc-500 hover:underline">
            全部 {fmtInt(latest.matched)} 只 →
          </Link>
        )}
      </div>
    </li>
  );
}

/** 自选股里今天值得看一眼的：有新信号的排在前面 */
function Watch() {
  const q = useListWatchlist();
  return (
    <Card
      title="我的自选"
      extra={
        <Link to="/watchlist" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
          管理自选 →
        </Link>
      }
    >
      <QueryView query={q}>
        {(d) =>
          d.items.length === 0 ? (
            <Empty>还没有收藏的股票。在页头搜索股票，打开后点 ☆ 收藏。</Empty>
          ) : (
            <WatchRows items={d.items} />
          )
        }
      </QueryView>
    </Card>
  );
}

function WatchRows({ items }: { items: WatchItem[] }) {
  const sorted = [...items].sort((a, b) => b.recent_signals.length - a.recent_signals.length);
  return (
    <ul className="grid gap-x-6 sm:grid-cols-2">
      {sorted.map((x) => (
        <li
          key={x.thscode}
          className="border-t border-zinc-100 first:border-t-0 sm:[&:nth-child(2)]:border-t-0 dark:border-zinc-800"
        >
          <Link
            to={`/securities/${x.thscode}`}
            className="flex items-center gap-3 rounded px-1 py-2 hover:bg-zinc-50 dark:hover:bg-zinc-800/50"
          >
            <span className="w-24 shrink-0 truncate font-medium">{x.name}</span>
            <span className={`num w-16 shrink-0 text-right text-sm ${upDownClass(x.diagnosis?.change_1d)}`}>
              {fmtPercent(x.diagnosis?.change_1d)}
            </span>
            <span className="flex min-w-0 flex-wrap gap-1">
              {x.recent_signals.length === 0 ? (
                <span className="text-xs text-zinc-400">最近 5 天没有新信号</span>
              ) : (
                x.recent_signals
                  .slice(0, 3)
                  .map((e) => <SignalChip key={`${e.trade_date}-${e.signal}`} event={e} withDate link={false} />)
              )}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
