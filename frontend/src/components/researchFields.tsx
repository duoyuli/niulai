/** 因子研究页的表单零件：交易成本、数字输入、规则说明和坐标切换。规则见 docs/domains/factor_research.md */
import { useEffect, useState } from "react";
import { Field, inputClass, Segmented } from "./ui";

/** 页面上的成本输入：初始资金（元）、佣金（万分之）、最低佣金（元）、印花税（千分之）、滑点（基点） */
export interface CostState {
  capital: string;
  commission: string;
  minFee: string;
  stamp: string;
  slippage: string;
}

/** 页面上的成本输入换成接口参数 */
export function costParams(s: CostState) {
  return {
    capital: Number(s.capital) || undefined,
    commission_rate: Number(s.commission) / 10000,
    min_commission: Number(s.minFee),
    stamp_tax_rate: Number(s.stamp) / 1000,
    slippage_bps: Number(s.slippage) || 0,
  };
}

/** 初始资金和折叠起来的交易成本 */
export function CostFields({ s, set }: { s: CostState; set: (patch: Partial<CostState>) => void }) {
  return (
    <>
      <NumberField label="初始资金（元）" value={s.capital} onCommit={(capital) => set({ capital })} width="w-28" />
      <details className="w-full text-sm">
        <summary className="cursor-pointer text-zinc-500 select-none">交易成本</summary>
        <div className="mt-3 flex flex-wrap items-end gap-4">
          <NumberField
            label="佣金（万分之）"
            hint="券商佣金，买卖都收。万 2.5 表示每成交 1 万元收 2.5 元"
            value={s.commission}
            onCommit={(commission) => set({ commission })}
          />
          <NumberField
            label="最低佣金（元）"
            hint="单笔佣金不足这个数时按这个数收"
            value={s.minFee}
            onCommit={(minFee) => set({ minFee })}
          />
          <NumberField
            label="印花税（千分之）"
            hint="国家收的税，只在卖出时收；2023 年 8 月 28 日起为千分之 0.5"
            value={s.stamp}
            onCommit={(stamp) => set({ stamp })}
          />
          <NumberField
            label="滑点（基点）"
            hint="每次成交在开盘价上让出的价差，10 基点 = 0.1%，买卖两边都收。建议 10～20"
            value={s.slippage}
            onCommit={(slippage) => set({ slippage })}
          />
        </div>
      </details>
    </>
  );
}

/** 回测规则说明，折叠起来 */
export function RulesNote({ items }: { items: string[] }) {
  return (
    <details className="text-sm text-zinc-500">
      <summary className="cursor-pointer select-none">回测怎么算的</summary>
      <ul className="mt-2 list-disc space-y-1 pl-5 text-zinc-600 dark:text-zinc-400">
        {items.map((x) => (
          <li key={x}>{x}</li>
        ))}
        <li>历史表现不代表未来，回测结果仅供研究。</li>
      </ul>
    </details>
  );
}

/** 数字输入：失焦或回车时提交 */
export function NumberField({
  label,
  hint,
  value,
  onCommit,
  width = "w-20",
}: {
  label: string;
  hint?: string;
  value: string;
  onCommit: (v: string) => void;
  width?: string;
}) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  const commit = () => {
    if (v !== value && Number.isFinite(Number(v)) && Number(v) >= 0) onCommit(v);
    else setV(value);
  };
  return (
    <Field label={label} hint={hint}>
      <input
        className={`${inputClass} num ${width}`}
        inputMode="decimal"
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
    </Field>
  );
}

/** 曲线下方：坐标切换和说明 */
export function LogToggle({ log, onLog, note }: { log: boolean; onLog: (v: boolean) => void; note: string }) {
  return (
    <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-zinc-500">
      <span>{note}</span>
      <Segmented
        value={log ? "log" : "normal"}
        options={[
          { value: "normal", label: "普通坐标" },
          { value: "log", label: "对数坐标", hint: "同样的涨跌幅在图上高度相同，适合看很长的区间" },
        ]}
        onChange={(v) => onLog(v === "log")}
      />
    </div>
  );
}
