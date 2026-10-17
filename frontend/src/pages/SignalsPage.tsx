import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { Link, useNavigate, useParams } from "react-router";
import type { Condition, SignalEntry } from "../api/generated/model";
import { useDeleteSignal, useGetSignal, useListSignals } from "../api/generated/signal/signal";
import { signalTarget } from "../components/overlays";
import { SampleChart } from "../components/SampleChart";
import { SignalStatsCard, SignalTodayCard, useSignalToday } from "../components/SignalStats";
import { StrategyNav } from "../components/StrategyNav";
import { Badge, buttonClass, Card, ErrorState, errorText, PageHeader, QueryView, Spinner } from "../components/ui";
import { useIsAdmin } from "../hooks/useIsAdmin";
import { groupBy } from "../lib/strategy";
import { toneOf } from "../lib/tone";

/** 信号：起了名字的条件。内置的按分组列出，自己定义的放在最前面 */
export function SignalsPage() {
  const list = useListSignals();
  const admin = useIsAdmin();
  return (
    <>
      <StrategyNav />
      <PageHeader
        title="信号"
        desc="信号就是起了名字的条件，例如「5 日均线上穿 20 日均线」叫「均线金叉」。点进去能看到它到底怎么判断、画在 K 线上是什么样、过去出现后涨跌如何、被哪些策略用到。内置信号不满意，管理员可以复制一份改。"
        extra={
          admin && (
            <Link to="/signals/new" className={buttonClass()}>
              新建信号
            </Link>
          )
        }
      />
      <div className="grid gap-3 text-sm sm:grid-cols-2">
        <p className="rounded-lg bg-zinc-100 px-3 py-2 text-zinc-600 dark:bg-zinc-800/60 dark:text-zinc-400">
          <b className="text-zinc-800 dark:text-zinc-200">事件</b>
          ：某一天发生，例如「金叉」只在穿过去的那一天成立，适合当买点、卖点。
        </p>
        <p className="rounded-lg bg-zinc-100 px-3 py-2 text-zinc-600 dark:bg-zinc-800/60 dark:text-zinc-400">
          <b className="text-zinc-800 dark:text-zinc-200">状态</b>
          ：持续成立，例如「多头排列」可能连续几十天都是，适合当过滤条件。
        </p>
      </div>
      <QueryView query={list}>
        {(items) => {
          const mine = items.filter((x) => !x.builtin);
          return (
            <div className="flex flex-col gap-4">
              <Card title="自定义信号">
                {mine.length === 0 ? (
                  <p className="text-sm text-zinc-500">
                    {admin ? (
                      <>
                        还没有自定义信号。打开任意一个内置信号点「复制一份改」，或
                        <Link to="/signals/new" className="mx-1 text-accent hover:underline">
                          从空白新建
                        </Link>
                        。
                      </>
                    ) : (
                      "还没有自定义信号。"
                    )}
                  </p>
                ) : (
                  <SignalGrid items={mine} />
                )}
              </Card>
              {groupBy(items.filter((x) => x.builtin)).map(([g, list]) => (
                <Card key={g} title={g}>
                  <SignalGrid items={list} />
                </Card>
              ))}
            </div>
          );
        }}
      </QueryView>
    </>
  );
}

function SignalGrid({ items }: { items: SignalEntry[] }) {
  return (
    <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
      {items.map((s) => (
        <Link
          key={s.key}
          to={`/signals/${s.key}`}
          className="flex flex-col gap-1 rounded-lg border border-zinc-200 p-3 transition hover:border-accent/60 hover:bg-zinc-50 dark:border-zinc-800 dark:hover:bg-zinc-800/50"
        >
          <span className={clsx("font-medium", toneOf(s.tone).text)}>
            {toneOf(s.tone).arrow} {s.name}
          </span>
          <span className="line-clamp-2 text-xs text-zinc-500">{s.explained.join("，并且") || s.description}</span>
          <span className="mt-auto flex flex-wrap gap-1 pt-1">
            <Badge>{s.kind === "state" ? "状态" : "事件"}</Badge>
            {s.special && <Badge tone="warn">规则写在代码里</Badge>}
            {s.used_by.length > 0 && <Badge tone="info">{s.used_by.length} 个策略在用</Badge>}
          </span>
        </Link>
      ))}
    </div>
  );
}

/** 一个信号的全部信息：怎么判断、怎么看、在 K 线上的样子、过去表现、今天出现在哪、被谁用 */
export function SignalDetailPage() {
  const { key = "" } = useParams();
  const q = useGetSignal(key);
  if (q.isPending) return <Spinner />;
  if (q.error) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return <SignalDetail key={key} s={q.data} />;
}

function SignalDetail({ s }: { s: SignalEntry }) {
  const admin = useIsAdmin();
  const nav = useNavigate();
  const qc = useQueryClient();
  const remove = useDeleteSignal({
    mutation: {
      onSuccess: () => {
        qc.invalidateQueries();
        nav("/signals");
      },
    },
  });
  const t = toneOf(s.tone);
  const target = { condition: signalTarget(s.key) as Condition };
  const today = useSignalToday(target);
  return (
    <>
      <StrategyNav />
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <Link to="/signals" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
            ← 全部信号
          </Link>
          <h1 className={clsx("mt-1 text-xl font-semibold", t.text)}>
            {t.arrow} {s.name}
          </h1>
          <div className="mt-1 flex flex-wrap gap-1">
            <Badge tone={s.tone === "bullish" ? "bad" : s.tone === "bearish" ? "ok" : "warn"}>{t.explain}</Badge>
            <Badge>{s.kind === "state" ? "状态：持续成立" : "事件：某一天发生"}</Badge>
            <Badge>{s.builtin ? `内置 · ${s.group}` : "自定义信号"}</Badge>
          </div>
        </div>
        {admin && (
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <Link to={`/strategies/new?from=blank&signal=${s.key}`} className={buttonClass()}>
              用它新建策略
            </Link>
            {!s.special && (
              <Link
                to={`/signals/new?from=${s.key}`}
                className={buttonClass("ghost")}
                title="复制它的定义，改成自己的信号"
              >
                复制一份改
              </Link>
            )}
            {!s.builtin && (
              <>
                <Link to={`/signals/${s.key}/edit`} className={buttonClass("ghost")}>
                  编辑
                </Link>
                <button
                  type="button"
                  className="text-zinc-500 hover:text-red-600"
                  disabled={remove.isPending}
                  onClick={() => {
                    if (s.signal_id && window.confirm(`删除信号「${s.name}」？`))
                      remove.mutate({ signalId: s.signal_id });
                  }}
                >
                  删除
                </button>
              </>
            )}
          </div>
        )}
      </div>
      {!!remove.error && <p className="text-sm text-red-600">{errorText(remove.error)}</p>}

      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="它是怎么判断的">
          <Definition s={s} />
        </Card>
        <Card title="是什么、怎么看">
          {s.description && <p className="text-sm">{s.description}。</p>}
          {s.usage && <p className="mt-2 text-sm text-zinc-600 dark:text-zinc-400">{s.usage}</p>}
          {!s.description && !s.usage && <p className="text-sm text-zinc-400">没有填写说明</p>}
          {s.based_on && (
            <p className="mt-2 text-xs text-zinc-500">
              复制自内置信号
              <Link to={`/signals/${s.based_on}`} className="ml-1 text-accent hover:underline">
                {s.based_on}
              </Link>
            </p>
          )}
        </Card>
      </div>

      <SampleChart
        title="画在 K 线上"
        overlays={[`sig:${s.key}`, ...indicatorOverlays(s)]}
        suggestions={today.data?.hits}
      />
      <SignalStatsCard target={target} />
      <div className="grid gap-4 lg:grid-cols-2">
        <SignalTodayCard target={target} />
        <Card title="被哪些策略使用">
          {s.used_by.length === 0 ? (
            <p className="text-sm text-zinc-400">
              还没有策略用它。
              {admin && (
                <Link to={`/strategies/new?from=blank&signal=${s.key}`} className="text-accent hover:underline">
                  用它新建一个
                </Link>
              )}
            </p>
          ) : (
            <ul className="flex flex-col gap-1 text-sm">
              {s.used_by.map((u) => (
                <li key={u.strategy_id}>
                  <Link to={`/strategies/${u.strategy_id}`} className="hover:underline">
                    {u.name}
                  </Link>
                </li>
              ))}
            </ul>
          )}
          {!s.builtin && s.used_by.length > 0 && (
            <p className="mt-2 text-xs text-zinc-500">修改这个信号后，这些策略下一次巡检起按新的定义判断。</p>
          )}
        </Card>
      </div>
    </>
  );
}

/** 定义里用到的指标一起画出来，能直接看出为什么这天算出现了信号 */
function indicatorOverlays(s: SignalEntry): string[] {
  const out: string[] = [];
  for (const c of s.definition?.conditions ?? []) {
    if (c.kind !== "compare") continue;
    for (const r of [c.left, ...("id" in c.right ? [c.right] : [])])
      if (!["close", "open", "high", "low"].includes(r.id)) out.push(`ind:${r.id}${r.n ? `:${r.n}` : ""}`);
  }
  return [...new Set(out)];
}

function Definition({ s }: { s: SignalEntry }) {
  if (s.special)
    return (
      <div className="flex flex-col gap-2 text-sm">
        <p>{s.explained[0]}</p>
        <p className="text-xs text-zinc-500">
          这个信号要用到涨跌停价、前一天的最高最低价等，没法用「指标 +
          比较」写出来，所以判断规则写在代码里。可以直接在策略里用，但不能复制修改。
        </p>
      </div>
    );
  const d = s.definition;
  return (
    <div className="flex flex-col gap-3 text-sm">
      <p className="text-zinc-500">
        {d && d.conditions.length > 1
          ? d.match_all === false
            ? "下面任意一条满足，就算出现了这个信号："
            : "下面每一条都满足，才算出现了这个信号："
          : "满足下面这条，就算出现了这个信号："}
      </p>
      <ul className="list-disc space-y-1 pl-5">
        {s.explained.map((x) => (
          <li key={x}>{x}</li>
        ))}
      </ul>
      {s.indicators.length > 0 && (
        <p className="text-xs text-zinc-500">
          用到的指标：
          {s.indicators.map((i, n) => (
            <span key={`${i.id}-${i.label}`}>
              {n > 0 && "、"}
              <Link to={`/indicators/${i.id}`} className="text-accent hover:underline">
                {i.label}
              </Link>
            </span>
          ))}
        </p>
      )}
      {s.params.length > 0 && (
        <p className="text-xs text-zinc-500">
          在策略里用它时可以调：
          {s.params.map((p) => `${p.label}（默认 ${p.default}，可设 ${p.min} 到 ${p.max}）`).join("、")}
          。这页按默认值展示。
        </p>
      )}
    </div>
  );
}
