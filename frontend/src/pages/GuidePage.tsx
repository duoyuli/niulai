import { clsx } from "clsx";
import { type ReactNode, useEffect } from "react";
import { Link, useLocation } from "react-router";
import type { Catalog, IndicatorInfo, SignalInfo } from "../api/generated/model";
import { useCatalog, usePreview } from "../api/generated/strategy/strategy";
import { Card, ErrorState, errorText, PageHeader, Segmented, Spinner } from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt, fmtPctValue, upDownClass } from "../lib/format";
import {
  defaults,
  fillName,
  groupBy,
  indicatorName,
  newSignalCondition,
  signalName,
  UNIT_SUFFIX,
} from "../lib/strategy";

const TABS = [
  { value: "signals", label: "信号" },
  { value: "indicators", label: "指标" },
] as const;

const UNIT_TEXT: Record<string, string> = {
  price: "单位是元（前复权价）",
  pct: "单位是 %，5 表示 5%",
  ratio: "单位是倍",
  yuan: "单位是元",
  shares: "单位是股",
  index: "没有单位，是一个指数值",
};

/** 指标与信号说明：每个信号、指标是什么、怎么看、要注意什么。各页面的信号标签点进来，定位到对应条目 */
export function GuidePage() {
  const catalog = useCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const { hash } = useLocation();
  const [s, set] = useSearchState({ tab: hash.startsWith("#ind-") ? "indicators" : "signals" });
  // 从信号标签进来时滚动到对应条目
  useEffect(() => {
    if (catalog.data && hash) document.getElementById(hash.slice(1))?.scrollIntoView({ block: "start" });
  }, [catalog.data, hash]);

  return (
    <>
      <PageHeader title="指标与信号" desc="页面上出现的每个信号、策略里能用的每个指标，在这里都有一段白话说明。" />
      <Concepts />
      {catalog.isPending ? (
        <Spinner />
      ) : !catalog.data ? (
        <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />
      ) : (
        <>
          <div className="sticky top-12 z-[5] -mx-4 bg-zinc-50/90 px-4 py-2 backdrop-blur dark:bg-zinc-950/90">
            <Segmented
              value={s.tab as (typeof TABS)[number]["value"]}
              options={TABS}
              onChange={(tab) => set({ tab })}
            />
          </div>
          {s.tab === "indicators" ? <Indicators catalog={catalog.data} /> : <Signals catalog={catalog.data} />}
        </>
      )}
    </>
  );
}

function Concepts() {
  return (
    <div className="grid gap-3 sm:grid-cols-3">
      <Concept title="指标" example="例：20 日均线、RSI、成交额">
        每个交易日算出的一个数，描述股票当天的某个方面。策略里可以拿指标和一个数值比较，例如「RSI 小于
        30」，或和另一个指标比较，例如「收盘价高于 60 日均线」。
      </Concept>
      <Concept title="信号" example="例：金叉、放量、创新高">
        某天发生的一件事（事件），或某天所处的状态。<span className="text-up">红色偏多</span>，常被看作走强；
        <span className="text-down">绿色偏空</span>，常被看作走弱。单个信号只是线索，不是买卖指令。
      </Concept>
      <Concept title="策略" example="例：放量并且站上 20 日均线">
        一组用信号和指标写成的条件。每天收盘后系统用它在全市场选股，也可以回测它过去的表现。
        <Link to="/strategies" className="ml-1 text-accent hover:underline">
          看我的策略
        </Link>
      </Concept>
    </div>
  );
}

function Concept({ title, example, children }: { title: string; example: string; children: ReactNode }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <h2 className="font-semibold">{title}</h2>
      <p className="mt-1 text-sm text-zinc-600 dark:text-zinc-400">{children}</p>
      <p className="mt-2 text-xs text-zinc-400">{example}</p>
    </div>
  );
}

function Signals({ catalog }: { catalog: Catalog }) {
  return (
    <div className="flex flex-col gap-4">
      {groupBy(catalog.signals).map(([group, list]) => (
        <Card key={group} title={group}>
          <div className="flex flex-col divide-y divide-zinc-100 dark:divide-zinc-800">
            {list.map((x) => (
              <SignalEntry key={x.id} s={x} catalog={catalog} />
            ))}
          </div>
        </Card>
      ))}
    </div>
  );
}

function SignalEntry({ s, catalog }: { s: SignalInfo; catalog: Catalog }) {
  const { hash } = useLocation();
  const preview = usePreview();
  const bullish = s.tone === "bullish";
  const params = s.params.map((p) => `${p.label}（默认 ${p.default}，可设 ${p.min} 到 ${p.max}）`).join("、");
  return (
    <section
      id={`sig-${s.id}`}
      className={clsx(
        "scroll-mt-28 py-4 first:pt-0 last:pb-0",
        hash === `#sig-${s.id}` && "rounded-lg bg-accent/5 px-3",
      )}
    >
      <h3 className="flex flex-wrap items-center gap-2 font-medium">
        <span className={bullish ? "text-up" : "text-down"}>
          {bullish ? "▲" : "▼"} {signalName(s)}
        </span>
        <span className="text-xs font-normal text-zinc-500">
          {bullish ? "偏多" : "偏空"} · {s.kind === "state" ? "状态：按当天是否处于这个状态判断" : "事件：某一天发生"}
        </span>
      </h3>
      <p className="mt-1 text-sm">{fillName(s.description, defaults(s))}。</p>
      <p className="mt-1 text-sm text-zinc-600 dark:text-zinc-400">{s.usage}</p>
      {params && <p className="mt-1 text-xs text-zinc-500">可以调的参数：{params}。</p>}
      <div className="mt-2 flex flex-wrap items-center gap-3 text-sm">
        <button
          type="button"
          className="text-accent hover:underline disabled:opacity-50"
          disabled={preview.isPending}
          onClick={() =>
            preview.mutate({
              data: { definition: { universe: catalog.default_universe, all: [newSignalCondition(s)], limit: 8 } },
            })
          }
        >
          {preview.isPending ? "查找中…" : "今天哪些股票出现了"}
        </button>
        <Link
          to={`/strategies/new?from=blank&signal=${s.id}`}
          className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
        >
          用它新建策略
        </Link>
      </div>
      {!!preview.error && <p className="mt-1 text-sm text-red-600">{errorText(preview.error)}</p>}
      {preview.data && (
        <p className="mt-2 text-sm">
          <span className="text-zinc-500">
            {preview.data.trade_date} 共 {fmtInt(preview.data.matched)} 只（已排除 ST、低价和成交太少的股票）
            {preview.data.hits.length > 0 && "，成交额最大的："}
          </span>
          {preview.data.hits.map((h) => (
            <Link key={h.thscode} to={`/securities/${h.thscode}`} className="mr-2 inline-flex gap-1 hover:underline">
              {h.name}
              <span className={`num text-xs ${upDownClass(h.change_pct)}`}>{fmtPctValue(h.change_pct, 1)}</span>
            </Link>
          ))}
        </p>
      )}
    </section>
  );
}

function Indicators({ catalog }: { catalog: Catalog }) {
  return (
    <div className="flex flex-col gap-4">
      {groupBy(catalog.indicators).map(([group, list]) => (
        <Card key={group} title={group}>
          <div className="flex flex-col divide-y divide-zinc-100 dark:divide-zinc-800">
            {list.map((x) => (
              <IndicatorEntry key={x.id} i={x} />
            ))}
          </div>
        </Card>
      ))}
    </div>
  );
}

function IndicatorEntry({ i }: { i: IndicatorInfo }) {
  const { hash } = useLocation();
  const hasN = i.n_default !== null && i.n_default !== undefined;
  return (
    <section
      id={`ind-${i.id}`}
      className={clsx(
        "scroll-mt-28 py-4 first:pt-0 last:pb-0",
        hash === `#ind-${i.id}` && "rounded-lg bg-accent/5 px-3",
      )}
    >
      <h3 className="flex flex-wrap items-baseline gap-2 font-medium">
        {indicatorName(i)}
        <span className="text-xs font-normal text-zinc-500">
          {UNIT_TEXT[i.unit] ?? UNIT_SUFFIX[i.unit]}
          {hasN && `；周期默认 ${i.n_default}，可设 ${i.n_min} 到 ${i.n_max}`}
        </span>
      </h3>
      <p className="mt-1 text-sm">{i.description.replaceAll("{n}", String(i.n_default ?? ""))}。</p>
      <p className="mt-1 text-sm text-zinc-600 dark:text-zinc-400">{i.usage}</p>
    </section>
  );
}
