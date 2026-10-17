import { clsx } from "clsx";
import { useRulesGuide } from "../api/generated/accounts/accounts";
import type { Rules, RulesBacktest, WeakMarket } from "../api/generated/model";
import { WEAK_MARKET } from "../lib/accounts";
import { fmtPctValue, fmtPercent, upDownClass } from "../lib/format";
import { Badge, buttonClass, Field, inputClass, QueryView } from "./ui";

/** 后端总是返回完整的规则（缺的字段按稳健型补齐） */
export type FullRules = Required<Rules>;

/** 规则写成几句白话 */
export function rulesText(p: FullRules): string[] {
  return [
    `参考${p.strategies.length ? `勾选的 ${p.strategies.length} 个策略` : "全部参加巡检的策略"}，至少 ${p.min_picks} 个有效策略同时看好才买`,
    `最多同时持有 ${p.max_positions} 只，单只最多占总资产的 ${p.max_weight_pct}%${p.min_cash_pct > 0 ? `，至少留 ${p.min_cash_pct}% 现金` : ""}`,
    `比成本价亏 ${p.stop_loss_pct}% 就卖（止损），收盘跌破 ${p.trend_ma} 日均线也卖`,
    `全市场站上 20 日均线的股票不到 ${p.weak_threshold_pct}% 时算弱市：${WEAK_MARKET[p.weak_market]}`,
    `第二天开盘比参考价高 ${p.chase_limit_pct}% 以上就不追`,
  ];
}

/** 规则回测的一句话 */
export function BacktestLine({ b }: { b: RulesBacktest }) {
  const s = b.summary;
  return (
    <p className="text-xs text-zinc-500">
      过去 3 年回测：
      <span className={clsx("num font-medium", upDownClass(s.total_return))}>{fmtPercent(s.total_return, 1)}</span>
      {s.annual_return !== null && s.annual_return !== undefined && <>（年化 {fmtPercent(s.annual_return, 1)}）</>}
      ，最大回撤 <span className="num">{fmtPercent(-s.max_drawdown, 1)}</span>
      {s.benchmark_return !== null && s.benchmark_return !== undefined && (
        <>
          ；同期{s.benchmark_name} <span className="num">{fmtPercent(s.benchmark_return, 1)}</span>
        </>
      )}
      。平均持有 {s.avg_positions.toFixed(1)} 只，{s.trades} 笔交易
      {s.win_rate !== null && s.win_rate !== undefined && <>，{(s.win_rate * 100).toFixed(0)}% 赚钱</>}；参考{" "}
      {s.strategies} 个策略，平均 {s.avg_effective.toFixed(1)} 个算作有效（每年只用之前 3 年的数据判断）。
    </p>
  );
}

type NumKey =
  | "min_picks"
  | "max_positions"
  | "max_weight_pct"
  | "min_cash_pct"
  | "stop_loss_pct"
  | "trend_ma"
  | "weak_threshold_pct"
  | "chase_limit_pct";
const NUM_FIELDS: { key: NumKey; label: string }[] = [
  { key: "min_picks", label: "至少几个有效策略同时看好" },
  { key: "max_positions", label: "最多持有几只" },
  { key: "max_weight_pct", label: "单只最多占总资产（%）" },
  { key: "min_cash_pct", label: "至少留现金（%）" },
  { key: "stop_loss_pct", label: "止损线：比成本亏（%）" },
  { key: "trend_ma", label: "收盘跌破几日均线就卖" },
  { key: "weak_threshold_pct", label: "弱市：站上 20 日均线的股票低于（%）" },
  { key: "chase_limit_pct", label: "追高上限：开盘比参考价高（%）" },
];

/** 选股和风控规则：参考哪些策略、各项参数，以及三组参考值（辅助，可以一键套用） */
export function RulesEditor({ value, onChange }: { value: FullRules; onChange: (r: FullRules) => void }) {
  const guide = useRulesGuide();
  const set = (patch: Partial<FullRules>) => onChange({ ...value, ...patch });
  const custom = value.strategies.length > 0;
  return (
    <QueryView query={guide}>
      {(g) => (
        <div className="flex flex-col gap-5">
          <section>
            <h3 className="text-sm font-medium">参考哪些策略</h3>
            <p className="mt-1 text-xs text-zinc-500">
              候选股来自参考的策略当天的买点。只有过去 3
              年买点之后的表现好于随便买的「有效策略」选中的股票才考虑买入，按「同时选中的有效策略数 → 它们的超额之和 →
              成交额」排序。
            </p>
            <div className="mt-2 flex flex-wrap gap-4 text-sm">
              <label className="flex items-center gap-1">
                <input type="radio" checked={!custom} onChange={() => set({ strategies: [] })} />
                全部参加巡检的策略（推荐，以后新增的策略自动参考）
              </label>
              <label className="flex items-center gap-1">
                <input
                  type="radio"
                  checked={custom}
                  onChange={() =>
                    set({ strategies: g.strategies.filter((s) => s.effective !== false).map((s) => s.strategy_id) })
                  }
                />
                只参考勾选的
              </label>
            </div>
            <ul className="mt-2 grid gap-1 text-sm md:grid-cols-2">
              {g.strategies.map((s) => {
                const st = s.stat;
                const excess =
                  st?.after20_avg_pct != null && st.base20_avg_pct != null
                    ? st.after20_avg_pct - st.base20_avg_pct
                    : null;
                const on = !custom || value.strategies.includes(s.strategy_id);
                return (
                  <li key={s.strategy_id} className={clsx("flex items-start gap-2", !on && "opacity-50")}>
                    <input
                      type="checkbox"
                      className="mt-1"
                      disabled={!custom}
                      checked={on}
                      onChange={() =>
                        set({
                          strategies: value.strategies.includes(s.strategy_id)
                            ? value.strategies.filter((x) => x !== s.strategy_id)
                            : [...value.strategies, s.strategy_id],
                        })
                      }
                    />
                    <span className="min-w-0">
                      {s.name}{" "}
                      {s.effective === true ? (
                        <Badge tone="ok">有效</Badge>
                      ) : s.effective === false ? (
                        <Badge>不如随便买</Badge>
                      ) : (
                        <Badge tone="warn">统计中</Badge>
                      )}
                      {st && excess !== null && (
                        <span className="block text-xs text-zinc-500">
                          过去 3 年 {st.signals} 次买点，20 个交易日后平均 {fmtPctValue(st.after20_avg_pct, 1)}
                          ，比随便买 <span className={upDownClass(excess)}>{fmtPctValue(excess, 1)}</span>
                        </span>
                      )}
                    </span>
                  </li>
                );
              })}
            </ul>
          </section>

          <section>
            <h3 className="text-sm font-medium">风控参数</h3>
            <div className="mt-2 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              {NUM_FIELDS.map((f) => (
                <Field key={f.key} label={f.label}>
                  <input
                    className={inputClass}
                    inputMode="decimal"
                    value={value[f.key]}
                    onChange={(e) => set({ [f.key]: Number(e.target.value) || 0 })}
                  />
                </Field>
              ))}
              <Field label="弱市时">
                <select
                  className={inputClass}
                  value={value.weak_market}
                  onChange={(e) => set({ weak_market: e.target.value as WeakMarket })}
                >
                  {(Object.keys(WEAK_MARKET) as WeakMarket[]).map((k) => (
                    <option key={k} value={k}>
                      {WEAK_MARKET[k]}
                    </option>
                  ))}
                </select>
              </Field>
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" checked={value.sell_st} onChange={(e) => set({ sell_st: e.target.checked })} />
                持有的股票被 ST 时卖出
              </label>
            </div>
            <ul className="mt-3 list-disc pl-4 text-xs text-zinc-500">
              {rulesText(value).map((r) => (
                <li key={r}>{r}</li>
              ))}
            </ul>
          </section>

          <details className="rounded-lg border border-dashed border-zinc-300 p-3 dark:border-zinc-700">
            <summary className="cursor-pointer text-sm text-zinc-600 dark:text-zinc-400">
              参考：不同类型的账户一般怎么设计
            </summary>
            <p className="mt-2 text-xs text-zinc-500">
              下面三组只是参考值，点「套用」会把风控参数换成这一组（参考的策略不变），之后还可以再改。
            </p>
            <div className="mt-2 grid gap-3 md:grid-cols-3">
              {g.presets.map((p) => (
                <div
                  key={p.preset}
                  className="flex flex-col gap-2 rounded-lg border border-zinc-200 p-3 text-sm dark:border-zinc-700"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-semibold">{p.name}</span>
                    <button
                      type="button"
                      className={buttonClass("ghost")}
                      onClick={() => onChange({ ...(p.rules as FullRules), strategies: value.strategies })}
                    >
                      套用
                    </button>
                  </div>
                  <p className="text-zinc-600 dark:text-zinc-400">{p.tagline}</p>
                  <ul className="list-disc space-y-0.5 pl-4 text-xs text-zinc-600 dark:text-zinc-400">
                    {p.design.map((d) => (
                      <li key={d}>{d}</li>
                    ))}
                  </ul>
                  {p.backtest ? (
                    <BacktestLine b={p.backtest} />
                  ) : (
                    <p className="text-xs text-zinc-400">过去 3 年的回测还在计算（每周算一次）</p>
                  )}
                </div>
              ))}
            </div>
          </details>
        </div>
      )}
    </QueryView>
  );
}
