import { keepPreviousData, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "react-router";
import type { Explained, ScreenResults, StrategyItem } from "../api/generated/model";
import {
  useDeleteStrategy,
  useGetStrategy,
  useResults,
  useRunScreening,
  useUpdateStrategy,
} from "../api/generated/strategy/strategy";
import {
  Badge,
  buttonClass,
  Card,
  Empty,
  ErrorState,
  errorText,
  PageHeader,
  QueryView,
  Spinner,
  Table,
} from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useSearchState } from "../hooks/useSearchState";
import { fmtBig, fmtDateTime, fmtInt, fmtPctValue, fmtPrice, upDownClass } from "../lib/format";

export function StrategyPage() {
  const id = Number(useParams().id);
  const nav = useNavigate();
  const qc = useQueryClient();
  const [s, set] = useSearchState({ date: "" });
  const strategy = useGetStrategy(id);
  const results = useResults(id, s.date ? { date: s.date } : undefined, {
    query: { placeholderData: keepPreviousData },
  });
  const refresh = () => qc.invalidateQueries();
  const update = useUpdateStrategy({ mutation: { onSuccess: refresh } });
  const remove = useDeleteStrategy({ mutation: { onSuccess: () => nav("/screener") } });
  const run = useRunScreening({ mutation: { onSuccess: refresh } });

  if (strategy.isPending) return <Spinner />;
  if (strategy.error) return <ErrorState error={strategy.error} onRetry={() => strategy.refetch()} />;
  const st = strategy.data;
  const toggle = () =>
    update.mutate({
      strategyId: id,
      data: {
        name: st.name,
        description: st.description,
        definition: st.definition,
        template: st.template,
        screening: !st.screening,
      },
    });

  return (
    <>
      <PageHeader
        title={
          <span className="flex flex-wrap items-center gap-2">
            {st.name}
            {st.screening ? <Badge tone="info">每日巡检</Badge> : <Badge>已暂停巡检</Badge>}
          </span>
        }
        desc={st.description}
        extra={
          <div className="flex flex-wrap items-center gap-2">
            <Link to={`/screener/${id}/edit`} className={buttonClass("ghost")}>
              编辑
            </Link>
            <Link
              to={`/backtest?mode=portfolio&strategy=${id}`}
              className={buttonClass("ghost")}
              title="假设过去几年每天都按这个策略买入、按卖出规则卖出，看看整体能赚多少"
            >
              回测
            </Link>
            <button type="button" className={buttonClass("ghost")} disabled={update.isPending} onClick={toggle}>
              {st.screening ? "暂停巡检" : "开启巡检"}
            </button>
            <button
              type="button"
              className={`${buttonClass("ghost")} text-red-600`}
              disabled={remove.isPending}
              onClick={() => {
                if (window.confirm(`删除策略「${st.name}」？它的巡检记录也会一起删除。`))
                  remove.mutate({ strategyId: id });
              }}
            >
              删除
            </button>
            <Link to="/screener" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
              ← 返回
            </Link>
          </div>
        }
      />
      {!!(update.error || remove.error) && (
        <p className="text-sm text-red-600">{errorText(update.error ?? remove.error)}</p>
      )}

      <Card title="选股条件">
        <ExplainedView e={st.explained} />
      </Card>

      <Card
        title="巡检结果"
        extra={
          <div className="flex flex-wrap items-center gap-2">
            {results.data && results.data.dates.length > 0 && (
              <select
                className="rounded-lg border border-zinc-200 bg-white px-2 py-1 text-sm dark:border-zinc-700 dark:bg-zinc-900"
                value={results.data.trade_date ?? ""}
                onChange={(e) => set({ date: e.target.value === results.data?.dates[0] ? "" : e.target.value })}
                aria-label="交易日"
              >
                {results.data.dates.map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
              </select>
            )}
            <button
              type="button"
              className={buttonClass("ghost")}
              disabled={run.isPending}
              title="现在用这个策略在最新交易日选一次股"
              onClick={() => run.mutate({ params: { strategy_id: id } })}
            >
              {run.isPending ? "巡检中…" : "立即巡检"}
            </button>
          </div>
        }
      >
        {!!run.error && <p className="mb-2 text-sm text-red-600">{errorText(run.error)}</p>}
        <QueryView query={results}>{(r) => <Hits r={r} st={st} />}</QueryView>
      </Card>
    </>
  );
}

/** 策略条件的白话说明 */
export function ExplainedView({ e, compact = false }: { e: Explained; compact?: boolean }) {
  const block = (title: string, items: string[], hint?: string) =>
    items.length > 0 && (
      <div>
        <h3 className="mb-1 text-xs font-medium text-zinc-500" title={hint}>
          {title}
        </h3>
        <ul className="list-disc space-y-0.5 pl-5 text-sm">
          {items.map((x) => (
            <li key={x}>{x}</li>
          ))}
        </ul>
      </div>
    );
  return (
    <div className={compact ? "flex flex-col gap-3" : "grid gap-4 sm:grid-cols-2 lg:grid-cols-4"}>
      {block("股票池", e.universe.length ? e.universe : ["全部 A 股"], "先排除不想看的股票，再判断条件")}
      {block("全部满足", e.all)}
      {block("至少满足一个", e.any)}
      {block("卖出规则（回测用）", e.exit, "选股时不用；回测时按这些规则卖出，满足任意一条就卖")}
    </div>
  );
}

function Hits({ r, st }: { r: ScreenResults; st: StrategyItem }) {
  if (!r.trade_date) return <Empty>还没有巡检过。点右上角「立即巡检」，或等今天收盘数据同步完成后自动巡检。</Empty>;
  const fresh = r.hits.filter((h) => h.is_new).length;
  return (
    <>
      <p className="mb-3 text-sm text-zinc-500">
        {r.trade_date} 检查了 {fmtInt(r.scanned)} 只股票，
        <b className="text-zinc-800 dark:text-zinc-200">{fmtInt(r.matched)}</b> 只符合条件
        {(r.matched ?? 0) > r.hits.length && `，按排序保留前 ${r.hits.length} 只`}
        {fresh > 0 && (
          <>
            ，其中 <span className="text-up">{fresh} 只是新入选</span>
          </>
        )}
        。巡检于 {fmtDateTime(r.ran_at)}。
      </p>
      {r.hits.length === 0 ? (
        <Empty>这一天没有符合条件的股票</Empty>
      ) : (
        <Table>
          <thead>
            <tr>
              <th className="w-0" aria-label="收藏" />
              <th className="w-0">#</th>
              <th>股票</th>
              <th className="text-right!">收盘</th>
              <th className="text-right!">涨跌幅</th>
              <th className="text-right!">成交额</th>
              <th>符合的条件</th>
              <th aria-label="操作" />
            </tr>
          </thead>
          <tbody>
            {r.hits.map((h) => (
              <tr key={h.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
                <td className="pr-0!">
                  <WatchButton thscode={h.thscode} />
                </td>
                <td className="num text-xs text-zinc-400">{h.rank}</td>
                <td>
                  <Link to={`/securities/${h.thscode}`} className="flex flex-col hover:underline">
                    <span className="flex items-center gap-1.5">
                      {h.name}
                      {h.is_new && (
                        <span className="rounded bg-up/10 px-1 text-xs text-up" title="上一次巡检时不在结果中">
                          新
                        </span>
                      )}
                    </span>
                    <span className="num font-mono text-xs text-zinc-500">{h.thscode}</span>
                  </Link>
                </td>
                <td className="num text-right">{fmtPrice(h.close)}</td>
                <td className={`num text-right ${upDownClass(h.change_pct)}`}>{fmtPctValue(h.change_pct)}</td>
                <td className="num text-right">{fmtBig(h.amount)}</td>
                <td className="max-w-md text-xs whitespace-normal text-zinc-600 dark:text-zinc-400">
                  {h.reasons.join("；")}
                </td>
                <td>
                  <Link
                    to={`/backtest?mode=strategy&strategy=${st.strategy_id}&code=${h.thscode}`}
                    className="text-xs text-accent hover:underline"
                    title="只在这只股票上回测这个策略"
                  >
                    回测
                  </Link>
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
      {r.removed.length > 0 && (
        <p className="mt-4 text-xs text-zinc-500">
          上一次入选、这次不再符合：
          {r.removed.map((x, i) => (
            <span key={x.thscode}>
              {i > 0 && "、"}
              <Link to={`/securities/${x.thscode}`} className="hover:underline">
                {x.name}
              </Link>
            </span>
          ))}
        </p>
      )}
    </>
  );
}
