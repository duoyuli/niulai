import { Link, useParams, useSearchParams } from "react-router";
import type { IndicatorEntry } from "../api/generated/model";
import { useListIndicators } from "../api/generated/signal/signal";
import { SampleChart } from "../components/SampleChart";
import { StrategyNav } from "../components/StrategyNav";
import { Badge, Card, Empty, PageHeader, QueryView } from "../components/ui";
import { useIsAdmin } from "../hooks/useIsAdmin";
import { groupBy, indicatorName } from "../lib/strategy";

const MA_PERIODS = [15, 20, 30, 60] as const;

const UNIT_TEXT: Record<string, string> = {
  price: "单位是元（等比前复权价）",
  pct: "单位是 %，5 表示 5%",
  ratio: "单位是倍",
  yuan: "单位是元",
  shares: "单位是股",
  index: "没有单位，是一个指数值",
};

/** 指标：每天算出的一个数。信号和策略都由指标比较组成 */
export function IndicatorsPage() {
  const list = useListIndicators({ query: { staleTime: 60_000 } });
  return (
    <>
      <StrategyNav />
      <PageHeader
        title="指标"
        desc="指标是每个交易日算出的一个数，描述股票当天的某个方面，例如 20 日均线、RSI、成交额。拿指标和一个数或另一个指标比较，就是一条条件；条件起了名字，就是信号。点进去看它怎么算、怎么看、画在 K 线上是什么样。"
      />
      <QueryView query={list}>
        {(items) => (
          <div className="flex flex-col gap-4">
            {groupBy(items).map(([g, xs]) => (
              <Card key={g} title={g}>
                <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                  {xs
                    .flatMap((i) => (i.id === "ma" ? MA_PERIODS.map((n) => ({ i, n })) : [{ i, n: i.n_default }]))
                    .map(({ i, n }) => (
                      <Link
                        key={`${i.id}:${n ?? ""}`}
                        to={`/indicators/${i.id}${n ? `?n=${n}` : ""}`}
                        className="flex flex-col gap-1 rounded-lg border border-zinc-200 p-3 transition hover:border-accent/60 hover:bg-zinc-50 dark:border-zinc-800 dark:hover:bg-zinc-800/50"
                      >
                        <span className="font-medium">{indicatorName(i, n)}</span>
                        <span className="line-clamp-2 text-xs text-zinc-500">
                          {i.description.replaceAll("{n}", String(n ?? ""))}
                        </span>
                        <span className="mt-auto flex flex-wrap gap-1 pt-1">
                          {i.signals.length > 0 && <Badge>{i.signals.length} 个信号用到</Badge>}
                          {i.strategies.length > 0 && <Badge tone="info">{i.strategies.length} 个策略用到</Badge>}
                        </span>
                      </Link>
                    ))}
                </div>
              </Card>
            ))}
          </div>
        )}
      </QueryView>
    </>
  );
}

export function IndicatorDetailPage() {
  const { id = "" } = useParams();
  const [query] = useSearchParams();
  const list = useListIndicators({ query: { staleTime: 60_000 } });
  return (
    <QueryView query={list}>
      {(items) => {
        const i = items.find((x) => x.id === id);
        const n = i?.n_default !== null && i?.n_default !== undefined ? Number(query.get("n")) || i.n_default : null;
        return i ? <IndicatorDetail i={i} n={n} /> : <Empty>没有这个指标</Empty>;
      }}
    </QueryView>
  );
}

function IndicatorDetail({ i, n }: { i: IndicatorEntry; n: number | null }) {
  const admin = useIsAdmin();
  const hasN = n !== null;
  const nText = String(n ?? "");
  // 开高低收本身就是 K 线，不用再画一条
  const drawable = !["open", "high", "low", "close"].includes(i.id);
  return (
    <>
      <StrategyNav />
      <div>
        <Link to="/indicators" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
          ← 全部指标
        </Link>
        <h1 className="mt-1 text-xl font-semibold">{indicatorName(i, n)}</h1>
        <p className="mt-1 text-xs text-zinc-500">
          {i.group} · {UNIT_TEXT[i.unit] ?? i.unit}
          {hasN && ` · 当前周期 ${n}，目录默认 ${i.n_default}，可设 ${i.n_min} 到 ${i.n_max}`}
        </p>
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="是什么、怎么看">
          <p className="text-sm">{i.description.replaceAll("{n}", nText)}。</p>
          <p className="mt-2 text-sm text-zinc-600 dark:text-zinc-400">{i.usage}</p>
        </Card>
        <Card title="怎么算">
          <p className="text-sm whitespace-pre-line">{i.formula.replaceAll("{n}", nText)}</p>
          <p className="mt-2 text-xs text-zinc-500">
            价格都用等比前复权价，分红送转不会让指标突然跳一下。数据不够算的头几天没有值，条件判断为「不满足」。
          </p>
        </Card>
      </div>
      {drawable && <SampleChart title="画在 K 线上" overlays={[`ind:${i.id}${hasN ? `:${n}` : ""}`]} />}
      <Card title={admin ? "常用写法：点一下拿去新建信号" : "常用写法"}>
        {i.examples.length === 0 ? (
          <p className="text-sm text-zinc-400">这个指标一般不单独用，常和别的指标比较，或作为排序依据。</p>
        ) : (
          <ul className="flex flex-col gap-2 text-sm">
            {i.examples.map((x) => (
              <li key={x.text} className="flex flex-wrap items-baseline gap-2">
                <span>{x.text}</span>
                {admin && (
                  <Link
                    to={`/signals/new?cond=${encodeURIComponent(JSON.stringify(x.condition))}`}
                    className="text-xs text-accent hover:underline"
                  >
                    用它新建信号 →
                  </Link>
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>
      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="哪些信号用到它">
          {i.signals.length === 0 ? (
            <p className="text-sm text-zinc-400">还没有信号用到它</p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {i.signals.map((s) => (
                <Link
                  key={s.key}
                  to={`/signals/${s.key}`}
                  className="rounded-md border border-zinc-200 px-2 py-0.5 text-sm hover:border-accent/60 dark:border-zinc-700"
                >
                  {s.name}
                </Link>
              ))}
            </div>
          )}
        </Card>
        <Card title="哪些策略用到它">
          {i.strategies.length === 0 ? (
            <p className="text-sm text-zinc-400">还没有策略直接用到它（通过信号间接用到的不算）</p>
          ) : (
            <ul className="flex flex-col gap-1 text-sm">
              {i.strategies.map((s) => (
                <li key={s.strategy_id}>
                  <Link to={`/strategies/${s.strategy_id}`} className="hover:underline">
                    {s.name}
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </>
  );
}
