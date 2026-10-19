import { clsx } from "clsx";
import { useRulesGuide } from "../api/generated/accounts/accounts";
import type { HistoryWindow, Rules, RulesBacktest, StrategyQuality, WeakMarket } from "../api/generated/model";
import { WEAK_MARKET } from "../lib/accounts";
import { fmtPercent, upDownClass } from "../lib/format";
import { Field, inputClass, QueryView } from "./ui";

/** 后端总是返回完整的规则（缺的字段按稳健型补齐） */
export type FullRules = Required<Rules>;

const HISTORY_WINDOWS: { value: HistoryWindow; label: string; months: number }[] = [
  { value: "three_months", label: "3 个月", months: 3 },
  { value: "six_months", label: "6 个月", months: 6 },
  { value: "one_year", label: "1 年", months: 12 },
  { value: "three_years", label: "3 年", months: 36 },
  { value: "five_years", label: "5 年", months: 60 },
];

/** 规则写成几句白话 */
export function rulesText(p: FullRules): string[] {
  return [
    `参考${p.strategies ? `勾选的 ${p.strategies.length} 个策略` : "全部策略"}：至少 ${p.min_picks} 个勾选的策略同时看好才买`,
    ...(p.history_check
      ? [
          `买入前回看：每个策略在这只股票上过去 ${HISTORY_WINDOWS.filter((w) =>
            p.history_check?.windows.includes(w.value),
          )
            .map((w) => w.label)
            .join("、")}的总收益都大于 ${p.history_check.min_return_pct ?? 0}%，才计入同时看好的策略数`,
        ]
      : []),
    `最多同时持有 ${p.max_positions} 只，单只最多占总资产的 ${p.max_weight_pct}%${p.min_cash_pct > 0 ? `，至少留 ${p.min_cash_pct}% 现金` : ""}`,
    "持仓卖出由买入依据的策略卖出规则决定",
    `全市场站上 20 日均线的股票不到 ${p.weak_threshold_pct}% 时算弱市：${WEAK_MARKET[p.weak_market]}`,
    `第二天开盘比参考价高 ${p.chase_limit_pct}% 以上就不追`,
  ];
}

/** 规则回测的一句话 */
export function BacktestLine({ b }: { b: RulesBacktest }) {
  const s = b.summary;
  return (
    <p className="text-xs text-zinc-500">
      {s.start} 至 {s.end} 回测：
      <span className={clsx("num font-medium", upDownClass(s.total_return))}>{fmtPercent(s.total_return, 1)}</span>
      {s.annual_return !== null && s.annual_return !== undefined && <>（年化 {fmtPercent(s.annual_return, 1)}）</>}
      ，最大回撤 <span className="num">{fmtPercent(-s.max_drawdown, 1)}</span>
      {s.benchmark_return !== null && s.benchmark_return !== undefined && (
        <>
          ；同期{s.benchmark_name} <span className="num">{fmtPercent(s.benchmark_return, 1)}</span>
        </>
      )}
      。平均持有 {s.avg_positions.toFixed(1)} 只，{s.trades} 笔交易
      {s.win_rate !== null && s.win_rate !== undefined && <>，{(s.win_rate * 100).toFixed(0)}% 赚钱</>}。
    </p>
  );
}

type NumKey = "max_positions" | "max_weight_pct" | "min_cash_pct" | "weak_threshold_pct" | "chase_limit_pct";
const RISK_FIELDS: { key: NumKey; label: string }[] = [
  { key: "max_positions", label: "最多持有几只" },
  { key: "max_weight_pct", label: "单只最多占总资产（%）" },
  { key: "min_cash_pct", label: "至少留现金（%）" },
  { key: "weak_threshold_pct", label: "弱市：站上 20 日均线的股票低于（%）" },
  { key: "chase_limit_pct", label: "追高上限：开盘比参考价高（%）" },
];

/** 选股和风控规则：左边是门槛、参考的策略和风控参数；右边是三类账户的建议，只是参考，可以照着填 */
export function RulesEditor({ value, onChange }: { value: FullRules; onChange: (r: FullRules) => void }) {
  const guide = useRulesGuide();
  const set = (patch: Partial<FullRules>) => onChange({ ...value, ...patch });
  const num = (v: string) => Number(v) || 0;
  return (
    <QueryView query={guide}>
      {(g) => {
        // 空值表示全部当前策略；保存勾选时写入当前策略快照，后续新增策略不会自动加入
        const picked = value.strategies;
        const checked = (s: StrategyQuality) => picked === null || picked.includes(s.strategy_id);
        const toggle = (s: StrategyQuality) => {
          const now = picked ?? g.strategies.map((x) => x.strategy_id);
          set({
            strategies: now.includes(s.strategy_id) ? now.filter((x) => x !== s.strategy_id) : [...now, s.strategy_id],
          });
        };
        const row = (s: StrategyQuality) => {
          return (
            <li key={s.strategy_id}>
              <label className="flex cursor-pointer items-center gap-2 py-0.5">
                <input type="checkbox" checked={checked(s)} onChange={() => toggle(s)} />
                <span className={clsx("min-w-0 flex-1 truncate", !checked(s) && "text-zinc-400")}>
                  {s.name}
                  {!s.builtin && <span className="ml-1 text-xs text-zinc-400">自建</span>}
                </span>
              </label>
            </li>
          );
        };
        return (
          <div className="grid gap-6 lg:grid-cols-3">
            <div className="flex flex-col gap-5 lg:col-span-2">
              <section>
                <h3 className="text-sm font-medium">选股</h3>
                <div className="mt-2 grid gap-3 sm:grid-cols-2">
                  <Field label="至少几个勾选的策略同时看好才买">
                    <input
                      className={inputClass}
                      inputMode="numeric"
                      value={value.min_picks}
                      onChange={(e) => set({ min_picks: num(e.target.value) })}
                    />
                  </Field>
                </div>
                <div className="mt-3 flex items-baseline justify-between gap-2 text-xs text-zinc-500">
                  <span>
                    参考的策略：
                    {picked === null
                      ? `当前全部 ${g.strategies.length} 个（保存后固定）`
                      : `现在勾了 ${picked.length} 个。`}
                  </span>
                  <span className="flex shrink-0 gap-3">
                    <button
                      type="button"
                      className="text-accent hover:underline"
                      title="勾选当前全部策略；以后新增的策略不会自动加入"
                      onClick={() => set({ strategies: g.strategies.map((x) => x.strategy_id) })}
                    >
                      全选
                    </button>
                    <button
                      type="button"
                      className="text-accent hover:underline"
                      onClick={() =>
                        set({
                          strategies: g.strategies
                            .map((x) => x.strategy_id)
                            .filter((id) => picked !== null && !picked.includes(id)),
                        })
                      }
                    >
                      反选
                    </button>
                  </span>
                </div>
                {picked !== null && picked.length === 0 && (
                  <p className="mt-1 text-sm text-red-600">至少勾选一个策略，否则没有候选股</p>
                )}
                <ul className="mt-1 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">{g.strategies.map(row)}</ul>
              </section>

              <section>
                <label className="flex items-center gap-2 text-sm font-medium">
                  <input
                    type="checkbox"
                    checked={!!value.history_check}
                    onChange={(e) =>
                      set({
                        history_check: e.target.checked
                          ? { windows: ["one_year", "three_years"], min_return_pct: 0 }
                          : null,
                      })
                    }
                  />
                  买入前回看
                </label>
                <p className="mt-1 text-xs text-zinc-500">
                  只买勾选的策略在这只股票上，过去所选周期按它的买卖点操作都赚钱的股票。通过回看的策略才计入「同时看好」，不影响持仓的卖出规则。
                </p>
                {value.history_check && (
                  <div className="mt-3 space-y-3">
                    <div className="flex flex-wrap gap-4 text-sm">
                      {HISTORY_WINDOWS.map((w) => (
                        <label key={w.value} className="flex items-center gap-1.5">
                          <input
                            type="checkbox"
                            checked={value.history_check?.windows.includes(w.value) ?? false}
                            onChange={() => {
                              const check = value.history_check;
                              if (!check) return;
                              set({
                                history_check: {
                                  ...check,
                                  windows: check.windows.includes(w.value)
                                    ? check.windows.filter((x) => x !== w.value)
                                    : [...check.windows, w.value],
                                },
                              });
                            }}
                          />
                          {w.label}
                        </label>
                      ))}
                    </div>
                    {value.history_check.windows.length === 0 && (
                      <p className="text-sm text-red-600">至少选择一个回看周期。</p>
                    )}
                    <Field label="各周期总收益必须大于（%）">
                      <input
                        className={inputClass}
                        type="number"
                        min="0"
                        step="any"
                        value={value.history_check.min_return_pct ?? 0}
                        onChange={(e) => {
                          if (value.history_check)
                            set({ history_check: { ...value.history_check, min_return_pct: num(e.target.value) } });
                        }}
                      />
                    </Field>
                    <p className="text-xs text-zinc-500">
                      上市不满{" "}
                      {HISTORY_WINDOWS.filter((w) => value.history_check?.windows.includes(w.value)).at(-1)?.label ??
                        "所选最长周期"}
                      的股票不会买入；日线或指标预热数据不足也不会买入。回看统一按本金 10
                      万元、默认费率计算，含期末持仓市值，历史收益不代表未来收益。
                    </p>
                  </div>
                )}
              </section>

              <section>
                <h3 className="text-sm font-medium">组合约束与保护</h3>
                <div className="mt-2 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                  {RISK_FIELDS.map((f) => (
                    <Field key={f.key} label={f.label}>
                      <input
                        className={inputClass}
                        inputMode="decimal"
                        value={value[f.key]}
                        onChange={(e) => set({ [f.key]: num(e.target.value) })}
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
                  <label className="flex items-center gap-2 self-end pb-2 text-sm">
                    <input
                      type="checkbox"
                      checked={value.sell_st}
                      onChange={(e) => set({ sell_st: e.target.checked })}
                    />
                    持有的股票被 ST 时卖出
                  </label>
                </div>
                <ul className="mt-3 list-disc pl-4 text-xs text-zinc-500">
                  {rulesText(value).map((r) => (
                    <li key={r}>{r}</li>
                  ))}
                </ul>
              </section>
            </div>

            <aside className="flex flex-col gap-3 rounded-lg bg-zinc-50 p-4 text-sm dark:bg-zinc-800/40">
              <div>
                <h3 className="font-medium">建议参考</h3>
                <p className="mt-1 text-xs text-zinc-500">
                  不同类型的账户一般这样设计，只是参考，不用必须选。觉得合适可以照着填，再按自己的情况改。
                </p>
              </div>
              {g.presets.map((p) => (
                <div key={p.preset} className="flex flex-col gap-1 border-t border-zinc-200 pt-3 dark:border-zinc-700">
                  <p>
                    <span className="font-medium">{p.name}</span>
                    <span className="text-zinc-500">：{p.tagline}</span>
                  </p>
                  <ul className="list-disc space-y-0.5 pl-4 text-xs text-zinc-600 dark:text-zinc-400">
                    {p.design.map((d) => (
                      <li key={d}>{d}</li>
                    ))}
                  </ul>
                  <button
                    type="button"
                    className="self-start text-xs text-zinc-500 underline decoration-dotted hover:text-zinc-900 dark:hover:text-zinc-100"
                    onClick={() =>
                      onChange({
                        ...(p.rules as FullRules),
                        strategies: value.strategies ?? g.strategies.map((x) => x.strategy_id),
                      })
                    }
                  >
                    照这个填
                  </button>
                </div>
              ))}
            </aside>
          </div>
        );
      }}
    </QueryView>
  );
}
