import { useQueries } from "@tanstack/react-query";
import { clsx } from "clsx";
import { Link } from "react-router";
import { useMarketHot, useMarketOverview, useSectors } from "../api/generated/market/market";
import type { HotMarket, Overview, ScreenResults, StrategyItem, WatchItem } from "../api/generated/model";
import { getResultsQueryOptions, useListStrategies } from "../api/generated/strategy/strategy";
import { useDataStatus } from "../api/generated/system/system";
import { useListWatchlist } from "../api/generated/watchlist/watchlist";
import { SignalChip } from "../components/SignalChip";
import { Card, Empty, ErrorState, HelpTip, QueryView, Spinner } from "../components/ui";
import { useIsAdmin } from "../hooks/useIsAdmin";
import { dayLabel, fmtInt, fmtPctValue, fmtPercent, upDownClass } from "../lib/format";
import { TodayAccounts } from "./AccountsPage";

const HOME_WATCHLIST_LIMIT = 6;

/** 每天收盘后打开看的一页：先看账户和自选，再看最近一天收盘大盘、策略和热点。 */
export function TodayPage() {
  const overview = useMarketOverview();
  return (
    <>
      <DataPreparing />
      <TodayAccounts />
      <Watch />
      <div>
        <h1 className="text-xl font-semibold">
          {overview.data ? `${dayLabel(overview.data.trade_date)}收盘` : "最近收盘"}
        </h1>
        <p className="mt-0.5 text-xs text-zinc-500">
          数据都截至这天收盘；下面的策略买点按规则在下一个交易日开盘执行，账户的具体操作建议以「我的账户」为准。
        </p>
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
      <Hotspots />
      <Picks />
    </>
  );
}

/** 核心数据（全市场历史日线）还在首次下载时提醒：这时的大盘统计不全，运营建议也先不生成 */
function DataPreparing() {
  const q = useDataStatus({ query: { refetchInterval: (x) => (x.state.data?.core_ready === false ? 60_000 : false) } });
  if (!q.data || q.data.core_ready) return null;
  return (
    <p className="rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
      {q.data.message}
    </p>
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

/** 今日热点：权威涨停、跌停、炸板池。这些数据每周同步，最近几天可能还没更新，没更新时显示白话空态 */
function Hotspots() {
  const q = useMarketHot();
  return (
    <Card
      title={
        <>
          今日热点
          <HelpTip>
            涨停是当天涨到允许的上限；炸板是盘中涨停过、收盘又打开；跌停是跌到当天的下限。数据由同花顺整理，每周同步一次，最近几天可能还没更新。
          </HelpTip>
        </>
      }
      extra={
        <Link to="/hot" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
          全部热点 →
        </Link>
      }
    >
      <QueryView query={q}>{(d) => <HotBrief d={d} />}</QueryView>
    </Card>
  );
}

function HotBrief({ d }: { d: HotMarket }) {
  if (!d.available) {
    return <p className="py-6 text-center text-sm text-zinc-500">{d.note ?? "今天的涨停数据还没同步，稍后再看"}</p>;
  }
  const chip =
    "inline-flex items-center gap-1 rounded-md border border-zinc-200 px-2 py-0.5 text-sm hover:border-accent/60 dark:border-zinc-700";
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-baseline gap-x-6 gap-y-1 text-sm">
        <span>
          涨停 <b className="num text-up">{fmtInt(d.limit_up_total)}</b> 家
        </span>
        <span>
          跌停 <b className="num text-down">{fmtInt(d.limit_down_total)}</b> 家
        </span>
        <span>
          炸板 <b className="num">{fmtInt(d.broken_total)}</b> 家
        </span>
        <span title="连板高度指连续涨停的天数，1 天就是首板">
          {d.max_board >= 2 ? (
            <>
              最高 <b className="num text-up">{d.max_board} 板</b>
            </>
          ) : d.limit_up_total > 0 ? (
            "最高首板"
          ) : (
            "没有涨停"
          )}
        </span>
        {d.stats?.seal_rate != null && (
          <span title="收盘还封在涨停的，占盘中触及过涨停的比例">
            封板率 <b className="num">{(d.stats.seal_rate * 100).toFixed(0)}%</b>
          </span>
        )}
      </div>
      {d.limit_up_groups.length === 0 ? (
        <p className="text-sm text-zinc-500">这一天没有涨停的股票。</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {d.limit_up_groups.slice(0, 4).map((g) => (
            <li key={g.board_num} className="flex min-w-0 flex-wrap items-center gap-1.5">
              <span className="w-16 shrink-0 text-xs font-medium text-zinc-500">
                {g.label}
                <span className="num ml-1 text-zinc-400">{fmtInt(g.count)} 家</span>
              </span>
              {g.stocks.slice(0, 6).map((x) => (
                <Link
                  key={x.thscode}
                  to={`/securities/${x.thscode}`}
                  title={[x.continue_day_text, x.limit_up_reason].filter(Boolean).join("；")}
                  className={chip}
                >
                  {x.name}
                  {x.is_st && <span className="text-xs text-zinc-400">ST</span>}
                </Link>
              ))}
              {g.count > 6 && (
                <Link
                  to={`/hot${d.trade_date ? `?date=${d.trade_date}` : ""}`}
                  className="text-xs text-zinc-500 hover:underline"
                >
                  还有 {fmtInt(g.count - 6)} 只 →
                </Link>
              )}
            </li>
          ))}
        </ul>
      )}
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

/** 每个开启巡检的策略最近一次巡检的买点，新出现的买点排在前面 */
function Picks() {
  const admin = useIsAdmin();
  const list = useListStrategies();
  // 只为测试建的策略可以关掉「在首页展示」
  const active = (list.data ?? []).filter((s) => s.screening && s.show_on_today && s.latest);
  const results = useQueries({ queries: active.map((s) => getResultsQueryOptions(s.strategy_id)) });
  return (
    <Card
      title="策略筛选出的买点"
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
          没有要在这里展示的策略（需要开启每日巡检和「在首页展示」）。
          {admin && (
            <Link to="/strategies/new" className="text-accent hover:underline">
              新建一个
            </Link>
          )}
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
  const chip =
    "inline-flex items-center gap-1 rounded-md border border-zinc-200 px-2 py-0.5 text-sm hover:border-accent/60 dark:border-zinc-700";
  return (
    <li className="flex flex-col gap-1.5 py-3 sm:flex-row sm:items-start sm:gap-4">
      <Link
        to={`/strategies/${s.strategy_id}`}
        className="flex shrink-0 flex-col hover:underline sm:w-48"
        title={s.description}
      >
        <span className="font-medium">{s.name}</span>
        <span className="text-xs text-zinc-500">{latest ? <>买点 {fmtInt(latest.matched)}</> : "还没有巡检"}</span>
      </Link>
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          <span className="text-xs text-up">买</span>
          {hits.length === 0 && latest && <span className="text-sm text-zinc-400">没有新的买点</span>}
          {hits.map((h) => (
            <Link
              key={h.thscode}
              to={`/securities/${h.thscode}?overlay=strategy:${s.strategy_id}`}
              title={h.reasons.join("；")}
              className={chip}
            >
              {h.name}
              <span className={`num text-xs ${upDownClass(h.change_pct)}`}>{fmtPctValue(h.change_pct, 1)}</span>
              {h.is_new && <span className="text-xs text-up">新</span>}
            </Link>
          ))}
          {latest && latest.matched > hits.length && hits.length > 0 && (
            <Link to={`/strategies/${s.strategy_id}`} className="text-xs text-zinc-500 hover:underline">
              全部 {fmtInt(latest.matched)} 只 →
            </Link>
          )}
        </div>
      </div>
    </li>
  );
}

/** 自选股里值得看一眼的：有新信号的排在前面 */
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
            <>
              <WatchRows items={d.items} />
              {d.items.length > HOME_WATCHLIST_LIMIT && (
                <div className="mt-2 border-t border-zinc-100 pt-2 text-center dark:border-zinc-800">
                  <Link to="/watchlist" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
                    查看更多自选（还有 {d.items.length - HOME_WATCHLIST_LIMIT} 只）→
                  </Link>
                </div>
              )}
            </>
          )
        }
      </QueryView>
    </Card>
  );
}

function WatchRows({ items }: { items: WatchItem[] }) {
  const sorted = [...items]
    .sort((a, b) => b.recent_signals.length - a.recent_signals.length)
    .slice(0, HOME_WATCHLIST_LIMIT);
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
