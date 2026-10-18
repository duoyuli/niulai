import { clsx } from "clsx";
import { useRulesGuide } from "../api/generated/accounts/accounts";
import type { Rules, RulesBacktest, StrategyQuality, WeakMarket } from "../api/generated/model";
import { WEAK_MARKET } from "../lib/accounts";
import { fmtPctValue, fmtPercent, upDownClass } from "../lib/format";
import { Field, inputClass, QueryView } from "./ui";

/** 后端总是返回完整的规则（缺的字段按稳健型补齐） */
export type FullRules = Required<Rules>;

/** 规则写成几句白话 */
export function rulesText(p: FullRules): string[] {
  return [
    `参考${p.strategies ? `勾选的 ${p.strategies.length} 个策略` : "全部策略"}：过去 3 年比随便买多涨 ${p.min_edge_pct}% 以上才算有效，至少 ${p.min_picks} 个有效策略同时看好才买`,
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
      {s.win_rate !== null && s.win_rate !== undefined && <>，{(s.win_rate * 100).toFixed(0)}% 赚钱</>}；平均{" "}
      {s.avg_effective.toFixed(1)} 个策略算作有效（每年只用之前 3 年的数据判断）。
    </p>
  );
}

/** 过去 3 年买点少于这个次数的策略在观察期，不算有效（和后端 MIN_SIGNALS 一致） */
const MIN_SIGNALS = 30;

const excessOf = (s: StrategyQuality) =>
  s.stat?.after20_avg_pct != null && s.stat.base20_avg_pct != null
    ? s.stat.after20_avg_pct - s.stat.base20_avg_pct
    : null;

type NumKey =
  | "max_positions"
  | "max_weight_pct"
  | "min_cash_pct"
  | "stop_loss_pct"
  | "trend_ma"
  | "weak_threshold_pct"
  | "chase_limit_pct";
const RISK_FIELDS: { key: NumKey; label: string }[] = [
  { key: "max_positions", label: "最多持有几只" },
  { key: "max_weight_pct", label: "单只最多占总资产（%）" },
  { key: "min_cash_pct", label: "至少留现金（%）" },
  { key: "stop_loss_pct", label: "止损线：比成本亏（%）" },
  { key: "trend_ma", label: "收盘跌破几日均线就卖" },
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
        // 够门槛的策略列在上面；不够门槛的、还在观察期的（没统计出来或买点太少）在分界线下面，灰显
        const qualifies = (s: StrategyQuality) => {
          const e = excessOf(s);
          const se = s.stat?.excess_se_pct;
          return (
            e !== null && se != null && (s.stat?.signals ?? 0) >= MIN_SIGNALS && e > value.min_edge_pct && e > 2 * se
          );
        };
        const above = g.strategies.filter(qualifies);
        const below = g.strategies.filter((s) => !qualifies(s));
        // 勾选只表示参考它；够不够门槛单独显示。null 表示全部（以后新增的也自动参考）
        const picked = value.strategies;
        const checked = (s: StrategyQuality) => picked === null || picked.includes(s.strategy_id);
        const toggle = (s: StrategyQuality) => {
          const now = picked ?? g.strategies.map((x) => x.strategy_id);
          set({
            strategies: now.includes(s.strategy_id) ? now.filter((x) => x !== s.strategy_id) : [...now, s.strategy_id],
          });
        };
        const row = (s: StrategyQuality) => {
          const e = excessOf(s);
          const ok = qualifies(s);
          const se = s.stat?.excess_se_pct;
          return (
            <li key={s.strategy_id}>
              <label className="flex cursor-pointer items-center gap-2 py-0.5">
                <input type="checkbox" checked={checked(s)} onChange={() => toggle(s)} />
                <span className={clsx("min-w-0 flex-1 truncate", (!checked(s) || !ok) && "text-zinc-400")}>
                  {s.name}
                  {!s.builtin && <span className="ml-1 text-xs text-zinc-400">自建</span>}
                </span>
                <span
                  className={clsx("num w-24 text-right text-xs", upDownClass(e))}
                  title={se != null && e !== null && e <= 2 * se ? "多涨的不到误差，说不清是不是运气" : undefined}
                >
                  {e === null || se == null ? "观察期" : `${fmtPctValue(e, 1)} ±${(2 * se).toFixed(1)}`}
                </span>
                <span className="num hidden w-20 text-right text-xs text-zinc-400 sm:inline">
                  {s.stat ? `${s.stat.signals} 次${s.stat.signals < MIN_SIGNALS ? "，太少" : ""}` : "统计中"}
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
                  <Field
                    label="有效门槛：过去 3 年比随便买至少多涨（个百分点）"
                    help={
                      <>
                        <span className="block font-medium">「随便买」是一个对照组</span>
                        <span className="mt-1 block">
                          策略每出现一个买点，就看同一天所有正常交易的股票平均涨多少，相当于那天闭着眼睛随便买一只。和同一天比，大盘整体涨跌就抵消了。
                        </span>
                        <span className="mt-1 block">
                          策略的数字 = 它的买点之后的平均涨跌 − 同一天随便买的平均涨跌。比如买点之后平均涨
                          3%、同一天随便买平均涨 1%，就是多涨 2 个百分点；负数说明照着它挑还不如闭眼买。
                        </span>
                        <span className="mt-1 block">
                          数字后面的 ± 是误差：多涨的不到误差这么多，就说不清是策略好还是运气好，也不算有效。
                        </span>
                        <span className="mt-1 block text-zinc-500">
                          都按能做到的算：买点第二天开盘买入（一字涨停买不进的不算），拿 20 个交易日。正常交易指排除
                          ST、股价低于 2 元、成交额不到 5000 万、上市不满 60
                          个交易日的。不扣费用，是平均数，不代表以后；每周用最新数据重算。
                        </span>
                      </>
                    }
                  >
                    <input
                      className={inputClass}
                      inputMode="decimal"
                      value={value.min_edge_pct}
                      onChange={(e) => set({ min_edge_pct: num(e.target.value) })}
                    />
                  </Field>
                  <Field label="至少几个有效策略同时看好才买">
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
                    参考的策略（右边是过去 3 年比随便买多涨多少）：
                    {picked === null ? "现在是全部参考，以后新增的策略也自动参考。" : `现在勾了 ${picked.length} 个。`}
                  </span>
                  <span className="flex shrink-0 gap-3">
                    <button
                      type="button"
                      className="text-accent hover:underline"
                      title="全部参考，以后新增的策略也自动参考"
                      onClick={() => set({ strategies: null })}
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
                <ul className="mt-1 divide-y divide-zinc-100 text-sm dark:divide-zinc-800">{above.map(row)}</ul>
                {above.length === 0 && (
                  <p className="mt-1 text-sm text-zinc-500">没有策略够这个门槛，可以把门槛调低一些</p>
                )}
                {below.length > 0 && (
                  <details className="mt-2 border-t border-dashed border-zinc-300 pt-1 text-sm dark:border-zinc-700">
                    <summary className="cursor-pointer text-xs text-zinc-500">
                      以下 {below.length} 个比随便买多涨不到 {value.min_edge_pct}%，多涨的不到误差（±
                      后面的数），或还在观察期（没统计出来、3 年里买点不到 {MIN_SIGNALS} 次），暂时不用来选股
                    </summary>
                    <ul className="mt-1 divide-y divide-zinc-100 dark:divide-zinc-800">{below.map(row)}</ul>
                  </details>
                )}
              </section>

              <section>
                <h3 className="text-sm font-medium">风控</h3>
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
                  {p.backtest && <BacktestLine b={p.backtest} />}
                  <button
                    type="button"
                    className="self-start text-xs text-zinc-500 underline decoration-dotted hover:text-zinc-900 dark:hover:text-zinc-100"
                    onClick={() => onChange({ ...(p.rules as FullRules), strategies: null })}
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
