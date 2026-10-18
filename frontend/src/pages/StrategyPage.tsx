import { keepPreviousData, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { usePortfolioBacktest } from "../api/generated/backtest/backtest";
import type { Explained, ScreenResults, StoredHit, StrategyItem } from "../api/generated/model";
import {
  useDeleteStrategy,
  useGetStrategy,
  useResults,
  useRunScreening,
  useUpdateStrategy,
} from "../api/generated/strategy/strategy";
import {
  COST_DEFAULTS,
  CostFields,
  costParams,
  Loaded,
  NumberField,
  PortfolioView,
  RulesNote,
} from "../components/backtest";
import {
  Badge,
  buttonClass,
  Card,
  Empty,
  ErrorState,
  errorText,
  QueryView,
  Segmented,
  Spinner,
  Table,
} from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useIsAdmin } from "../hooks/useIsAdmin";
import { useSearchState } from "../hooks/useSearchState";
import { fmtBig, fmtDateTime, fmtInt, fmtPctValue, fmtPrice, RANGES, rangeStart, upDownClass } from "../lib/format";
import { holdingsText, MAX_HOLDINGS } from "../lib/strategy";

const TABS = [
  { value: "picks", label: "最新买卖点" },
  { value: "history", label: "过去表现如何", hint: "假设过去每天都按这个策略买入、按卖出规则卖出，看看整体能赚多少" },
] as const;

/** 全市场回测最长 5 年 */
const PORTFOLIO_RANGES = RANGES.filter((r) => r.value === "1" || r.value === "3" || r.value === "5");

export function StrategyPage() {
  const id = Number(useParams().id);
  const nav = useNavigate();
  const qc = useQueryClient();
  const [s, set] = useSearchState({ tab: "picks", date: "" });
  const strategy = useGetStrategy(id);
  const refresh = () => qc.invalidateQueries();
  const update = useUpdateStrategy({ mutation: { onSuccess: refresh } });
  const remove = useDeleteStrategy({ mutation: { onSuccess: () => nav("/strategies") } });
  const admin = useIsAdmin();

  if (strategy.isPending) return <Spinner />;
  if (strategy.error) return <ErrorState error={strategy.error} onRetry={() => strategy.refetch()} />;
  const st = strategy.data;
  // 修改接口要提交完整的策略，这里只改其中一个开关
  const save = (patch: { screening?: boolean; show_on_today?: boolean }) =>
    update.mutate({
      strategyId: id,
      data: {
        name: st.name,
        description: st.description,
        definition: st.definition,
        template: st.template,
        screening: st.screening,
        show_on_today: st.show_on_today,
        ...patch,
      },
    });

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <Link to="/strategies" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
            ← 策略
          </Link>
          <h1 className="mt-1 flex flex-wrap items-center gap-2 text-xl font-semibold">
            {st.name}
            {st.screening ? <Badge tone="info">每日巡检</Badge> : <Badge>已暂停巡检</Badge>}
            {st.builtin && <Badge tone="ok">内置</Badge>}
            {!st.show_on_today && <Badge>不在首页展示</Badge>}
          </h1>
          <Conditions e={st.explained} />
        </div>
        {admin && st.builtin && (
          <div className="flex flex-wrap items-center gap-3 text-sm">
            <span className="text-xs text-zinc-500">内置策略由模板生成，不能修改或删除</span>
            <Link to={`/strategies/new?from=${st.template ?? ""}`} className={buttonClass("ghost")}>
              复制一份改
            </Link>
            <button
              type="button"
              className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
              disabled={update.isPending}
              onClick={() => save({ show_on_today: !st.show_on_today })}
            >
              {st.show_on_today ? "不在首页展示" : "在首页展示"}
            </button>
          </div>
        )}
        {admin && !st.builtin && (
          <div className="flex flex-wrap items-center gap-3 text-sm">
            <Link to={`/strategies/${id}/edit`} className={buttonClass("ghost")}>
              修改条件
            </Link>
            <button
              type="button"
              className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
              disabled={update.isPending}
              onClick={() => save({ screening: !st.screening })}
            >
              {st.screening ? "暂停巡检" : "开启巡检"}
            </button>
            <button
              type="button"
              className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
              disabled={update.isPending}
              title="只是试试的策略可以不在首页出现，照常巡检"
              onClick={() => save({ show_on_today: !st.show_on_today })}
            >
              {st.show_on_today ? "不在首页展示" : "在首页展示"}
            </button>
            <button
              type="button"
              className="text-zinc-500 hover:text-red-600"
              disabled={remove.isPending}
              onClick={() => {
                if (window.confirm(`删除策略「${st.name}」？它的巡检记录也会一起删除。`))
                  remove.mutate({ strategyId: id });
              }}
            >
              删除
            </button>
          </div>
        )}
      </div>
      {!!(update.error || remove.error) && (
        <p className="text-sm text-red-600">{errorText(update.error ?? remove.error)}</p>
      )}

      <div>
        <Segmented value={s.tab as (typeof TABS)[number]["value"]} options={TABS} onChange={(tab) => set({ tab })} />
      </div>
      {s.tab === "history" ? (
        <History id={id} />
      ) : (
        <Picks id={id} st={st} date={s.date} onDate={(date) => set({ date })} />
      )}
    </>
  );
}

/** 条件写成一段话：股票池一行，买入条件一行，卖出规则折叠 */
function Conditions({ e }: { e: Explained }) {
  const buy = [e.all.join("，并且"), e.any.length ? `至少满足一个：${e.any.join("；")}` : ""]
    .filter(Boolean)
    .join("；");
  return (
    <div className="mt-2 max-w-3xl space-y-1 text-sm">
      <p>
        <span className="text-zinc-500">买：</span>
        {buy || "—"}
      </p>
      {e.exit.length > 0 && (
        <p>
          <span className="text-zinc-500">卖：</span>
          {e.exit.join("，或")}
        </p>
      )}
      <p className="text-xs text-zinc-500">范围：{e.universe.length ? e.universe.join("，") : "全部 A 股"}</p>
    </div>
  );
}

function Picks({ id, st, date, onDate }: { id: number; st: StrategyItem; date: string; onDate: (d: string) => void }) {
  const qc = useQueryClient();
  const results = useResults(id, date ? { date } : undefined, { query: { placeholderData: keepPreviousData } });
  const run = useRunScreening({ mutation: { onSuccess: () => qc.invalidateQueries() } });
  const admin = useIsAdmin();
  return (
    <Card
      extra={
        <div className="flex flex-wrap items-center gap-2">
          {results.data && results.data.dates.length > 1 && (
            <select
              className="rounded-lg border border-zinc-200 bg-white px-2 py-1 text-sm dark:border-zinc-700 dark:bg-zinc-900"
              value={results.data.trade_date ?? ""}
              onChange={(e) => onDate(e.target.value === results.data?.dates[0] ? "" : e.target.value)}
              aria-label="交易日"
            >
              {results.data.dates.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
          )}
          {admin && (
            <button
              type="button"
              className="text-sm text-zinc-500 hover:text-zinc-900 disabled:opacity-50 dark:hover:text-zinc-100"
              disabled={run.isPending}
              title="现在用这个策略在最新交易日选一次"
              onClick={() => run.mutate({ params: { strategy_id: id } })}
            >
              {run.isPending ? "选股中…" : "重新选一次"}
            </button>
          )}
        </div>
      }
      title={results.data?.trade_date ? `${results.data.trade_date} 收盘后` : "买卖点"}
    >
      {!!run.error && <p className="mb-2 text-sm text-red-600">{errorText(run.error)}</p>}
      <QueryView query={results}>{(r) => <Hits r={r} st={st} />}</QueryView>
    </Card>
  );
}

function History({ id }: { id: number }) {
  const [s, set] = useSearchState({ range: "3", positions: "5", ...COST_DEFAULTS, log: "" });
  const [go, setGo] = useState(false);
  const query = usePortfolioBacktest(
    { strategy_id: id, start: rangeStart(s.range), max_positions: Number(s.positions) || 5, ...costParams(s) },
    { query: { enabled: go, staleTime: 5 * 60_000, placeholderData: keepPreviousData, retry: false } },
  );
  return (
    <>
      <Card>
        <div className="flex flex-wrap items-end gap-4">
          <div className="flex flex-col gap-1 text-xs text-zinc-500">
            <span>看过去多久</span>
            <Segmented value={s.range} options={PORTFOLIO_RANGES} onChange={(range) => set({ range })} />
          </div>
          <NumberField
            label="最多同时拿几只"
            hint="1 到 20。每次买入时，用现金除以空余的仓位数，平均分给新买的股票"
            value={s.positions}
            onCommit={(positions) => set({ positions })}
            width="w-16"
          />
          <CostFields s={s} set={set} />
        </div>
        {!go && (
          <div className="mt-4 flex flex-wrap items-center gap-3">
            <button type="button" className={buttonClass()} onClick={() => setGo(true)}>
              开始回测
            </button>
            <span className="text-xs text-zinc-500">要把全市场几年的日线算一遍，通常需要几秒到十几秒</span>
          </div>
        )}
      </Card>
      {go && (
        <Loaded query={query} pending="正在逐只计算全市场的日线">
          {(d) => <PortfolioView data={d} log={s.log === "1"} onLog={(v) => set({ log: v ? "1" : "" })} />}
        </Loaded>
      )}
      <RulesNote
        items={[
          "每个交易日收盘后用策略选股，下一个交易日开盘买入；同一天候选太多时按策略的排序先买靠前的。",
          "最多同时持有设定的只数，已经持有的不重复买；仓位已满时放弃新的信号。卖出规则满足任意一条后，下一个交易日开盘卖出。",
          "为了让全市场几千只股票能在几秒内算完，收益按等比前复权价计算（相当于分红再投资），允许买零股；ST 按现在的简称判断；「只看强势板块」在回测中不生效。",
          "开盘一字涨停买不进时放弃这次买入；一字跌停卖不出时顺延。手续费按交易成本扣除。",
          "对比基准是同期的沪深 300（没有它的日线时用上证指数），不计分红；本地没有区间开头的指数日线时不显示。",
        ]}
      />
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
      {block("卖出规则", e.exit, "买入后满足任意一条就是卖点，下一个交易日开盘卖出")}
    </div>
  );
}

function Hits({ r, st }: { r: ScreenResults; st: StrategyItem }) {
  const [allHits, setAllHits] = useState(false);
  if (!r.trade_date) return <Empty>还没有选过。点右上角「重新选一次」，或等今天收盘数据同步完成后自动选股。</Empty>;
  const fresh = r.hits.filter((h) => h.is_new).length;
  // 「全部满足」的条件每只股票都一样，只有「至少满足一个」时才值得逐只列出命中了哪条
  const fixed = st.explained.all.length;
  const showReasons = st.explained.any.length > 0;
  const hits = allHits ? r.hits : r.hits.slice(0, 20);
  return (
    <div className="flex flex-col gap-6">
      <section>
        <h3 className="flex flex-wrap items-baseline gap-2 font-medium">
          <span className="text-up">最新买点</span>
          <span className="text-sm font-normal text-zinc-500">
            全市场 {fmtInt(r.scanned)} 只中有 <b className="text-zinc-800 dark:text-zinc-200">{fmtInt(r.matched)}</b>{" "}
            只满足买入条件
            {(r.matched ?? 0) > r.hits.length && `，按排序列出前 ${r.hits.length} 只`}
            {fresh > 0 && <span className="text-up">，{fresh} 只是新出现的</span>}
          </span>
        </h3>
        <p className="mb-2 text-xs text-zinc-500">今天收盘满足买入条件、之前没有持有，按规则下一个交易日开盘买入。</p>
        {r.hits.length === 0 ? (
          <p className="py-4 text-sm text-zinc-400">这一天没有新的买点</p>
        ) : (
          <Table>
            <thead>
              <tr>
                <th className="w-0" aria-label="收藏" />
                <th>股票</th>
                <th className="text-right!">收盘</th>
                <th className="text-right!">涨跌幅</th>
                <th className="text-right!">成交额</th>
                {showReasons && <th>满足的条件</th>}
              </tr>
            </thead>
            <tbody>
              {hits.map((h) => (
                <tr key={h.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
                  <td className="pr-0!">
                    <WatchButton thscode={h.thscode} />
                  </td>
                  <td>
                    <StockLink h={h} strategyId={st.strategy_id}>
                      {h.is_new && (
                        <span className="rounded bg-up/10 px-1 text-xs text-up" title="上一次选股时不在结果中">
                          新
                        </span>
                      )}
                    </StockLink>
                  </td>
                  <td className="num text-right">{fmtPrice(h.close)}</td>
                  <td className={`num text-right ${upDownClass(h.change_pct)}`}>{fmtPctValue(h.change_pct)}</td>
                  <td className="num text-right">{fmtBig(h.amount)}</td>
                  {showReasons && (
                    <td className="max-w-md text-xs whitespace-normal text-zinc-600 dark:text-zinc-400">
                      {h.reasons.slice(fixed).join("；")}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </Table>
        )}
        {r.hits.length > hits.length && (
          <button type="button" className="mt-2 text-sm text-accent hover:underline" onClick={() => setAllHits(true)}>
            显示全部 {r.hits.length} 只
          </button>
        )}
      </section>
      <PositionTable
        title="最新卖点"
        tone="text-down"
        desc="按最近一年的买卖点推算，之前买入、今天触发了卖出规则的股票，按规则下一个交易日开盘卖出。"
        empty="今天没有要卖出的"
        rows={r.sells}
        strategyId={st.strategy_id}
        reason
      />
      <PositionTable
        title="持有中"
        tone="text-zinc-800 dark:text-zinc-200"
        desc="按最近一年的买卖点推算，买入后还没有触发卖出规则的股票。这是按规则推算的，不是你的实际持仓。"
        empty="按规则推算，现在没有持有中的股票"
        rows={r.holdings}
        strategyId={st.strategy_id}
      />
      <p className="text-xs text-zinc-400">
        选股于 {fmtDateTime(r.ran_at)}。点股票打开 K 线，会标出这个策略在它身上的历次买卖点。
      </p>
    </div>
  );
}

/** 股票名：点进去时带上策略，个股页的 K 线上直接画出这个策略的买卖点 */
function StockLink({ h, strategyId, children }: { h: StoredHit; strategyId: number; children?: ReactNode }) {
  return (
    <Link
      to={`/securities/${h.thscode}?overlay=strategy:${strategyId}`}
      className="flex items-baseline gap-1.5 hover:underline"
    >
      {h.name}
      <span className="num font-mono text-xs text-zinc-500">{h.thscode}</span>
      {children}
    </Link>
  );
}

function PositionTable({
  title,
  tone,
  desc,
  empty,
  rows,
  strategyId,
  reason = false,
}: {
  title: string;
  tone: string;
  desc: string;
  empty: string;
  rows: StoredHit[];
  strategyId: number;
  reason?: boolean;
}) {
  const [all, setAll] = useState(false);
  const shown = all ? rows : rows.slice(0, 20);
  return (
    <section>
      <h3 className="flex items-baseline gap-2 font-medium">
        <span className={tone}>{title}</span>
        <span className="text-sm font-normal text-zinc-500">
          {holdingsText(rows.length)} 只
          {rows.length >= MAX_HOLDINGS && `（只保存买入以来涨得最多的 ${MAX_HOLDINGS} 只）`}
        </span>
      </h3>
      <p className="mb-2 text-xs text-zinc-500">{desc}</p>
      {rows.length === 0 ? (
        <p className="py-4 text-sm text-zinc-400">{empty}</p>
      ) : (
        <Table>
          <thead>
            <tr>
              <th className="w-0" aria-label="收藏" />
              <th>股票</th>
              <th>买入日</th>
              <th className="text-right!">买入价</th>
              <th className="text-right!">收盘</th>
              <th className="text-right!" title="按前复权价计算，含分红送转，不扣费用">
                买入以来
              </th>
              <th className="text-right!">已持有</th>
              {reason && <th>卖出原因</th>}
            </tr>
          </thead>
          <tbody>
            {shown.map((h) => (
              <tr key={h.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
                <td className="pr-0!">
                  <WatchButton thscode={h.thscode} />
                </td>
                <td>
                  <StockLink h={h} strategyId={strategyId} />
                </td>
                <td className="num">{h.entry_date ?? "—"}</td>
                <td className="num text-right">{fmtPrice(h.entry_price)}</td>
                <td className="num text-right">{fmtPrice(h.close)}</td>
                <td className={`num text-right ${upDownClass(h.return_pct)}`}>{fmtPctValue(h.return_pct)}</td>
                <td className="num text-right">
                  {h.bars_held === null || h.bars_held === undefined ? "—" : `${h.bars_held} 天`}
                </td>
                {reason && (
                  <td className="text-xs whitespace-normal text-zinc-600 dark:text-zinc-400">{h.reasons.join("；")}</td>
                )}
              </tr>
            ))}
          </tbody>
        </Table>
      )}
      {rows.length > shown.length && (
        <button type="button" className="mt-2 text-sm text-accent hover:underline" onClick={() => setAll(true)}>
          显示全部 {rows.length} 只
        </button>
      )}
    </section>
  );
}
