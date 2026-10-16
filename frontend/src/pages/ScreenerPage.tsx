import { useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { Link } from "react-router";
import { useMarketOverview, useSectors } from "../api/generated/market/market";
import type { Overview, SectorStrength, StrategyItem } from "../api/generated/model";
import { useListStrategies, useRunScreening } from "../api/generated/strategy/strategy";
import {
  Badge,
  buttonClass,
  Card,
  Empty,
  ErrorState,
  errorText,
  PageHeader,
  QueryView,
  Segmented,
  Spinner,
} from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt, fmtPctValue, upDownClass } from "../lib/format";

export function ScreenerPage() {
  const qc = useQueryClient();
  const list = useListStrategies();
  const run = useRunScreening({
    // 巡检结果出现在策略列表、策略详情和个股诊断里，全部刷新
    mutation: { onSuccess: () => qc.invalidateQueries() },
  });

  return (
    <>
      <PageHeader
        title="选股"
        desc="策略是一组用白话就能说清的条件，例如「5 日均线上穿 20 日均线，并且成交额不低于 5000 万」。每天同步完收盘数据后，系统自动用开启巡检的策略把全市场过一遍，把符合条件的股票列出来。"
        extra={
          <div className="flex items-center gap-2">
            <button
              type="button"
              className={buttonClass("ghost")}
              disabled={run.isPending}
              title="现在就用开启巡检的策略在最新交易日选一次股，结果会覆盖当天已有的结果"
              onClick={() => run.mutate({})}
            >
              {run.isPending ? "巡检中…" : "立即巡检"}
            </button>
            <Link to="/screener/new" className={buttonClass()}>
              新建策略
            </Link>
          </div>
        }
      />
      {run.data && (
        <p className="text-sm text-zinc-500">
          {run.data.trade_date
            ? `已在 ${run.data.trade_date} 用 ${run.data.strategies} 个策略检查了 ${fmtInt(run.data.scanned)} 只股票，用时 ${(run.data.ms / 1000).toFixed(1)} 秒。`
            : "没有开启巡检的策略，或本地还没有日线。"}
        </p>
      )}
      {!!run.error && <p className="text-sm text-red-600">{errorText(run.error)}</p>}

      <Market />

      <Card title="我的策略">
        <QueryView query={list}>
          {(items) =>
            items.length === 0 ? (
              <Empty>还没有策略。点「新建策略」，从模板开始改最简单。</Empty>
            ) : (
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {items.map((s) => (
                  <StrategyCard key={s.strategy_id} s={s} />
                ))}
              </div>
            )
          }
        </QueryView>
      </Card>
    </>
  );
}

function StrategyCard({ s }: { s: StrategyItem }) {
  const latest = s.latest;
  return (
    <Link
      to={`/screener/${s.strategy_id}`}
      className="flex flex-col gap-2 rounded-lg border border-zinc-200 p-3 transition hover:border-accent/60 hover:bg-zinc-50 dark:border-zinc-800 dark:hover:bg-zinc-800/50"
    >
      <div className="flex items-start justify-between gap-2">
        <span className="font-medium">{s.name}</span>
        {s.screening ? <Badge tone="info">每日巡检</Badge> : <Badge>已暂停</Badge>}
      </div>
      <p className="line-clamp-2 text-xs text-zinc-500">{s.description || s.explained.all.join("，并且")}</p>
      <div className="mt-auto flex items-baseline gap-2 text-sm">
        {latest ? (
          <>
            <span className="num text-lg font-semibold">{fmtInt(latest.matched)}</span>
            <span className="text-xs text-zinc-500">只符合</span>
            {latest.new_hits > 0 && (
              <span className="text-xs text-up" title="上一次巡检时不在结果中">
                新入选 {latest.new_hits}
              </span>
            )}
            <span className="num ml-auto text-xs text-zinc-400">{latest.trade_date}</span>
          </>
        ) : (
          <span className="text-xs text-zinc-400">还没有巡检过</span>
        )}
      </div>
    </Link>
  );
}

/** 市场概况和板块强弱：先看大环境，再看个股 */
function Market() {
  const overview = useMarketOverview();
  return (
    <div className="grid gap-4 lg:grid-cols-3">
      <Card title="市场概况" className="lg:col-span-1">
        {overview.isPending ? (
          <Spinner />
        ) : overview.error ? (
          <ErrorState error={overview.error} onRetry={() => overview.refetch()} />
        ) : (
          <Breadth o={overview.data} />
        )}
      </Card>
      <Sectors />
    </div>
  );
}

function Breadth({ o }: { o: Overview }) {
  const total = Math.max(1, o.up + o.down + o.flat);
  const pct = (n: number) => `${(n / total) * 100}%`;
  const mood =
    o.up / total >= 0.65 ? "普涨，赚钱的股票明显多" : o.down / total >= 0.65 ? "普跌，亏钱的股票明显多" : "涨跌互现";
  return (
    <div className="flex flex-col gap-3 text-sm">
      <p>
        <span className="num text-zinc-500">{o.trade_date}</span> · {mood}
      </p>
      <div>
        <div className="flex h-2 w-full overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800">
          <div className="bg-up" style={{ width: pct(o.up) }} />
          <div className="bg-zinc-300 dark:bg-zinc-600" style={{ width: pct(o.flat) }} />
          <div className="bg-down" style={{ width: pct(o.down) }} />
        </div>
        <div className="num mt-1 flex justify-between text-xs">
          <span className="text-up">上涨 {fmtInt(o.up)}</span>
          <span className="text-zinc-500">平 {fmtInt(o.flat)}</span>
          <span className="text-down">下跌 {fmtInt(o.down)}</span>
        </div>
      </div>
      <dl className="num grid grid-cols-2 gap-2 text-xs">
        <Item label="涨停" value={<span className="text-up">{fmtInt(o.limit_up)} 只</span>} />
        <Item label="跌停" value={<span className="text-down">{fmtInt(o.limit_down)} 只</span>} />
        <Item
          label="涨跌幅中位数"
          hint="把所有股票按涨跌幅排队，排在正中间那只的涨跌幅，比指数更能代表普通股票"
          value={<span className={upDownClass(o.median_change_pct)}>{fmtPctValue(o.median_change_pct)}</span>}
        />
        <Item
          label="站上 20 日均线"
          hint="收盘价高于 20 日均线的股票占比；越高说明中短期走强的股票越多"
          value={`${(o.above_ma20 * 100).toFixed(0)}%`}
        />
      </dl>
    </div>
  );
}

function Item({ label, value, hint }: { label: string; value: ReactNode; hint?: string }) {
  return (
    <div className="flex flex-col rounded-md bg-zinc-50 px-2 py-1.5 dark:bg-zinc-800/50">
      <dt className="text-zinc-500" title={hint}>
        {label}
        {hint && <span className="ml-1 cursor-help text-zinc-400">ⓘ</span>}
      </dt>
      <dd className="text-sm font-medium">{value}</dd>
    </div>
  );
}

const SECTOR_CATEGORIES = [
  { value: "industry", label: "行业" },
  { value: "cn_concept", label: "概念" },
] as const;

const SECTOR_DAYS = [
  { value: "1", label: "今日" },
  { value: "5", label: "近 5 日" },
  { value: "20", label: "近 20 日" },
] as const;

function Sectors() {
  const [s, set] = useSearchState({ cat: "industry", days: "5" });
  const query = useSectors({ category: s.cat, days: Number(s.days) });
  return (
    <Card
      title="板块强弱"
      className="lg:col-span-2"
      extra={
        <div className="flex flex-wrap gap-2">
          <Segmented value={s.cat} options={SECTOR_CATEGORIES} onChange={(cat) => set({ cat })} />
          <Segmented value={s.days} options={SECTOR_DAYS} onChange={(days) => set({ days })} />
        </div>
      }
    >
      <QueryView query={query}>
        {(rows) =>
          rows.length === 0 ? (
            <Empty>还没有板块指数的日线，先在「数据同步」里同步指数行情</Empty>
          ) : (
            <div className="grid gap-x-6 sm:grid-cols-2">
              <SectorList title="最强" rows={rows.slice(0, 8)} />
              <SectorList title="最弱" rows={rows.slice(-8).reverse()} />
            </div>
          )
        }
      </QueryView>
      <p className="mt-3 text-xs text-zinc-500">
        按板块指数的涨跌幅排名。策略里可以设置「只看强势板块的股票」，资金集中的方向更容易延续。
      </p>
    </Card>
  );
}

function SectorList({ title, rows }: { title: string; rows: SectorStrength[] }) {
  const max = Math.max(...rows.map((r) => Math.abs(r.change_pct)), 0.01);
  return (
    <div>
      <h3 className="mb-1 text-xs font-medium text-zinc-500">{title}</h3>
      <ul className="flex flex-col">
        {rows.map((r) => (
          <li key={r.thscode}>
            <Link
              to={`/indices/${r.thscode}`}
              className="grid grid-cols-[2rem_1fr_5rem] items-center gap-2 rounded px-1 py-1 text-sm hover:bg-zinc-50 dark:hover:bg-zinc-800/50"
            >
              <span className="num text-xs text-zinc-400">{r.rank}</span>
              <span className="flex min-w-0 items-center gap-2">
                <span className="truncate" title={`${r.members} 只成分股`}>
                  {r.name}
                </span>
                <span
                  className={`h-1.5 shrink-0 rounded-full ${r.change_pct >= 0 ? "bg-up/60" : "bg-down/60"}`}
                  style={{ width: `${(Math.abs(r.change_pct) / max) * 4}rem` }}
                />
              </span>
              <span className={`num text-right ${upDownClass(r.change_pct)}`}>{fmtPctValue(r.change_pct)}</span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}
