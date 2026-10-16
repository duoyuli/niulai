import { clsx } from "clsx";
import { Link } from "react-router";
import { useSecurityDiagnosis } from "../api/generated/market/market";
import type { Diagnosis, Level, Verdict } from "../api/generated/model";
import { fmtPctValue, fmtPrice } from "../lib/format";
import { SignalChip } from "./SignalChip";
import { Card, ErrorState, Spinner } from "./ui";

const VERDICT: Record<Verdict, { label: string; className: string }> = {
  strong: { label: "偏强", className: "bg-up/10 text-up" },
  neutral: { label: "中性", className: "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300" },
  weak: { label: "偏弱", className: "bg-down/10 text-down" },
};

/** 个股诊断：四个维度各一句结论，下面是最近的信号、离当前价最近的支撑和压力，以及今天符合的策略（有才显示） */
export function DiagnosisCard({ thscode }: { thscode: string }) {
  const q = useSecurityDiagnosis(thscode, { query: { retry: false } });
  return (
    <Card title="诊断">
      {q.isPending ? (
        <Spinner />
      ) : q.error ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <Body d={q.data} thscode={thscode} />
      )}
    </Card>
  );
}

function Body({ d, thscode }: { d: Diagnosis; thscode: string }) {
  // 价位按价格从高到低排列：上方压力里最后一个离当前价最近，下方支撑里第一个最近
  const resistance = d.levels.filter((l) => l.kind === "resistance").at(-1);
  const support = d.levels.find((l) => l.kind === "support");
  return (
    <div className="flex flex-col gap-4">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {d.dimensions.map((x) => (
          <div key={x.key} className="flex flex-col gap-1">
            <span className="flex items-center gap-2 text-xs text-zinc-500">
              {x.title}
              <span className={clsx("rounded px-1.5 py-0.5 font-medium", VERDICT[x.verdict].className)}>
                {VERDICT[x.verdict].label}
              </span>
            </span>
            <span className="text-sm font-medium">{x.headline}</span>
            {x.detail && <span className="text-xs text-zinc-500">{x.detail}</span>}
          </div>
        ))}
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-100 pt-4 text-sm lg:flex-row lg:gap-8 dark:border-zinc-800">
        <div className="min-w-0 flex-1">
          <span className="mr-2 text-xs text-zinc-500">最近 20 个交易日</span>
          {d.recent_signals.length === 0 ? (
            <span className="text-zinc-400">没有出现信号</span>
          ) : (
            <span className="inline-flex flex-wrap gap-1.5 align-middle">
              {d.recent_signals.slice(0, 6).map((e) => (
                <SignalChip key={`${e.trade_date}-${e.signal}`} event={e} withDate />
              ))}
              {d.recent_signals.length > 6 && (
                <span
                  className="text-xs text-zinc-400"
                  title={d.recent_signals
                    .slice(6)
                    .map((e) => e.name)
                    .join("、")}
                >
                  等 {d.recent_signals.length} 个
                </span>
              )}
            </span>
          )}
        </div>
        <div
          className="num flex shrink-0 gap-6"
          title="上方最近的压力：涨到那里容易遇到卖盘；下方最近的支撑：跌到那里容易有人接。按前复权价计算"
        >
          <LevelText l={resistance} label="上方压力" className="text-up" />
          <LevelText l={support} label="下方支撑" className="text-down" />
        </div>
      </div>

      {d.strategies.length > 0 && (
        <p className="text-sm">
          <span className="mr-2 text-xs text-zinc-500">今天符合</span>
          {d.strategies.map((s, i) => (
            <span key={s.strategy_id}>
              {i > 0 && "、"}
              <Link
                to={`/strategies/${s.strategy_id}`}
                className="font-medium hover:underline"
                title={s.reasons.join("；")}
              >
                {s.name}
              </Link>
              <Link
                to={`/securities/${thscode}?tab=backtest&method=${s.strategy_id}`}
                className="ml-1 text-xs text-accent hover:underline"
                title="在这只股票上回测这个策略"
              >
                回测
              </Link>
            </span>
          ))}
        </p>
      )}
    </div>
  );
}

function LevelText({ l, label, className }: { l?: Level; label: string; className: string }) {
  if (!l) return null;
  return (
    <span className="flex flex-col">
      <span className="text-xs text-zinc-500">
        {label} · {l.name}
      </span>
      <span className={className}>
        {fmtPrice(l.price)} <span className="text-xs">{fmtPctValue(l.distance_pct, 1)}</span>
      </span>
    </span>
  );
}
