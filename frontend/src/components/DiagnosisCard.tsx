import { clsx } from "clsx";
import { Link } from "react-router";
import { useSecurityDiagnosis } from "../api/generated/market/market";
import type { Diagnosis, Verdict } from "../api/generated/model";
import { fmtPctValue, fmtPrice } from "../lib/format";
import { SignalChip } from "./SignalChip";
import { Card, ErrorState, Spinner } from "./ui";

const VERDICT: Record<Verdict, { label: string; className: string }> = {
  strong: { label: "偏强", className: "bg-up/10 text-up" },
  neutral: { label: "中性", className: "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300" },
  weak: { label: "偏弱", className: "bg-down/10 text-down" },
};

/** 个股诊断：趋势、动量、量能、位置各一句结论，加上最近的信号、关键价位和符合的策略 */
export function DiagnosisCard({ thscode }: { thscode: string }) {
  const q = useSecurityDiagnosis(thscode, { query: { retry: false } });
  return (
    <Card
      title={
        <span>
          诊断
          {q.data && <span className="num ml-2 text-xs font-normal text-zinc-500">{q.data.trade_date} 收盘后</span>}
        </span>
      }
    >
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
  return (
    <div className="flex flex-col gap-4">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {d.dimensions.map((x) => (
          <div key={x.key} className="flex flex-col gap-1 rounded-lg border border-zinc-100 p-3 dark:border-zinc-800">
            <div className="flex items-center justify-between">
              <span className="text-xs text-zinc-500">{x.title}</span>
              <span className={clsx("rounded px-1.5 py-0.5 text-xs font-medium", VERDICT[x.verdict].className)}>
                {VERDICT[x.verdict].label}
              </span>
            </div>
            <span className="text-sm font-medium">{x.headline}</span>
            {x.detail && <span className="text-xs text-zinc-500">{x.detail}</span>}
          </div>
        ))}
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        <section>
          <h3 className="mb-2 text-xs font-medium text-zinc-500">最近 20 个交易日的信号</h3>
          {d.recent_signals.length === 0 ? (
            <p className="text-sm text-zinc-400">没有出现过</p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {d.recent_signals.map((e) => (
                <SignalChip key={`${e.trade_date}-${e.signal}`} event={e} withDate />
              ))}
            </div>
          )}
        </section>

        <section>
          <h3
            className="mb-2 text-xs font-medium text-zinc-500"
            title="上方的价位是压力：涨到那里容易遇到卖盘；下方的是支撑：跌到那里容易有人接"
          >
            关键价位 <span className="cursor-help text-zinc-400">ⓘ</span>
          </h3>
          <ul className="num flex flex-col gap-0.5 text-sm">
            {d.levels.map((l) => (
              <li key={l.name} className="flex items-center justify-between gap-2">
                <span className={l.kind === "resistance" ? "text-up" : "text-down"}>
                  {l.kind === "resistance" ? "压力" : "支撑"} · {l.name}
                </span>
                <span>
                  {fmtPrice(l.price)} <span className="text-xs text-zinc-500">{fmtPctValue(l.distance_pct, 1)}</span>
                </span>
              </li>
            ))}
          </ul>
          <p className="mt-1 text-xs text-zinc-400">按前复权价计算，百分比是距当前收盘价的距离。</p>
        </section>

        <section>
          <h3 className="mb-2 text-xs font-medium text-zinc-500">今天符合的策略</h3>
          {d.strategies.length === 0 ? (
            <p className="text-sm text-zinc-400">
              不符合任何已保存的策略。
              <Link to="/screener" className="text-accent hover:underline">
                去选股
              </Link>
            </p>
          ) : (
            <ul className="flex flex-col gap-2 text-sm">
              {d.strategies.map((s) => (
                <li key={s.strategy_id} className="flex flex-col">
                  <span className="flex items-center gap-2">
                    <Link to={`/screener/${s.strategy_id}`} className="font-medium hover:underline">
                      {s.name}
                    </Link>
                    <Link
                      to={`/backtest?mode=strategy&strategy=${s.strategy_id}&code=${thscode}`}
                      className="text-xs text-accent hover:underline"
                      title="在这只股票上回测这个策略"
                    >
                      回测
                    </Link>
                  </span>
                  <span className="text-xs text-zinc-500">{s.reasons.join("；")}</span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
      <p className="text-xs text-zinc-400">诊断只描述已经发生的走势，不预测涨跌。</p>
    </div>
  );
}
