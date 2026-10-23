import { Link } from "react-router";
import type { Rules } from "../api/generated/model";
import { useFactorCatalog } from "../api/generated/research/research";
import { planResearchLink } from "../lib/factors";
import { Field, inputClass } from "./ui";

/** 已持有的股票排名在这个名次以内时继续拿 */
export function keepLimit(r: Pick<Rules, "top_n" | "buffer">): number {
  return Math.ceil(r.top_n * (1 + r.buffer));
}

/** 规则写成几句白话 */
export function rulesText(r: Rules): string[] {
  return [
    `每月最后一个交易日收盘，按 ${r.factors.length} 个因子的合成得分给全市场排名，持有前 ${r.top_n} 只`,
    `每只约占总资产的 1/${r.top_n}${r.min_cash_pct > 0 ? `，至少留 ${r.min_cash_pct}% 现金` : ""}；已持有的跌出前 ${keepLimit(r)} 名才卖`,
    "没有持满时（例如刚建账户）每天按排名补仓",
    `${r.sell_st ? "持有的股票被 ST 时卖出；" : ""}第二天开盘比参考价高 ${r.chase_limit_pct}% 以上就不追`,
  ];
}

type NumKey = "top_n" | "buffer" | "min_cash_pct" | "chase_limit_pct";
const FIELDS: { key: NumKey; label: string }[] = [
  { key: "top_n", label: "持有几只（5 到 50）" },
  { key: "buffer", label: "缓冲区（0 到 2）" },
  { key: "min_cash_pct", label: "至少留现金（%）" },
  { key: "chase_limit_pct", label: "追高上限：开盘比参考价高（%）" },
];

/** 账户的调仓规则：参与排名的因子（带方向）、持有只数、缓冲区、留现金、追高上限和 ST。规则见 docs/domains/accounts.md#rules */
export function RulesForm({ value, onChange }: { value: Rules; onChange: (r: Rules) => void }) {
  const catalog = useFactorCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const label = (id: string, weight: number) => {
    const f = catalog.data?.find((x) => x.id === id);
    if (!f) return id;
    // 权重为负时反过来用
    return `${f.name}${f.higher_is_better === weight > 0 ? "高" : "低"}`;
  };
  const set = (patch: Partial<Rules>) => onChange({ ...value, ...patch });
  return (
    <div className="flex flex-col gap-4">
      <section>
        <h3 className="text-sm font-medium">参与排名的因子</h3>
        <ul className="mt-2 flex flex-wrap gap-2 text-sm">
          {value.factors.map((f) => (
            <li key={f.factor} className="rounded bg-zinc-100 px-2 py-0.5 dark:bg-zinc-800">
              {label(f.factor, f.weight)}
              {f.weight !== 1 && <span className="ml-1 text-xs text-zinc-500">× {f.weight}</span>}
            </li>
          ))}
        </ul>
        <p className="mt-2 text-xs text-zinc-500">
          股票池、去极值和行业、规模中性化与因子研究的默认值相同。
          <Link className="ml-1 text-accent hover:underline" to={planResearchLink(value)}>
            在因子研究里回测这组规则
          </Link>
        </p>
      </section>
      <section>
        <h3 className="text-sm font-medium">持仓和保护</h3>
        <div className="mt-2 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {FIELDS.map((f) => (
            <Field key={f.key} label={f.label}>
              <input
                className={inputClass}
                inputMode="decimal"
                value={value[f.key]}
                onChange={(e) => set({ [f.key]: Number(e.target.value) || 0 })}
              />
            </Field>
          ))}
        </div>
        <label className="mt-3 flex items-center gap-2 text-sm">
          <input type="checkbox" checked={value.sell_st} onChange={(e) => set({ sell_st: e.target.checked })} />
          持有的股票被 ST 时卖出
        </label>
        <ul className="mt-3 list-disc pl-4 text-xs text-zinc-500">
          {rulesText(value).map((r) => (
            <li key={r}>{r}</li>
          ))}
        </ul>
      </section>
    </div>
  );
}
