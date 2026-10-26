/** 按类别选因子的标签组，因子研究页和账户的调仓规则共用。规则见 docs/domains/factor_research.md */
import { useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";
import type { FactorInfo } from "../api/generated/model";
import {
  getListPresetsQueryKey,
  useDeletePreset,
  useFactorCatalog,
  useListPresets,
} from "../api/generated/research/research";
import { FACTOR_PRESETS, type FactorWeight, matchesPreset, sameFactors, toggleFactor } from "../lib/factors";
import { HelpTip, inputClass, QueryView } from "./ui";

/** 套用预设时交给调用方的内容：用户保存的预设还带持有只数和缓冲区 */
export interface PresetChoice {
  factors: FactorWeight[];
  top_n?: number;
  buffer?: number;
}

/** 分两步：上面是一键套用的预设（内置的和自己保存的），下面逐个选因子。
 * 点预设替换整个选择；onPreset 不传时只替换因子。组合模式下每个选中的因子可以填权重 */
export function FactorPicker({
  picks,
  onChange,
  onPreset,
  presets = true,
  weights,
}: {
  picks: FactorWeight[];
  onChange: (p: FactorWeight[]) => void;
  onPreset?: (p: PresetChoice) => void;
  /** 显示预设组合；因子检验是逐个体检因子，不需要 */
  presets?: boolean;
  weights: boolean;
}) {
  const catalog = useFactorCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const apply = (p: PresetChoice) => (onPreset ? onPreset(p) : onChange(p.factors));
  return (
    <QueryView query={catalog}>
      {(items) => {
        const names = (ids: readonly string[]) =>
          ids.map((id) => items.find((f) => f.id === id)?.name ?? id).join("、");
        return (
          <div className="flex flex-col gap-4">
            {presets && (
              <section className="rounded-xl border border-zinc-200 bg-zinc-50 p-3 dark:border-zinc-700 dark:bg-zinc-800/40">
                <StepTitle n={1} title="一键套用预设" note="点一下会替换下面的全部选择" />
                <div className="mt-2 grid gap-2 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
                  {FACTOR_PRESETS.map((p) => (
                    <PresetCard
                      key={p.id}
                      name={p.name}
                      sub={`${p.factors.length} 个因子，等权`}
                      active={matchesPreset(picks, p.factors)}
                      onApply={() => apply({ factors: p.factors.map((id) => ({ id, weight: 1 })) })}
                      help={
                        <>
                          {p.desc}。
                          <br />
                          因子：{names(p.factors)}
                        </>
                      }
                    />
                  ))}
                </div>
                <MyPresets picks={picks} names={names} onApply={apply} />
              </section>
            )}
            <section className="flex flex-col gap-3 px-1">
              {presets && <StepTitle n={2} title="或者逐个选因子" note="点因子选中或取消" />}
              {(
                [
                  ["price_volume", "价量因子（只用日线）"],
                  ["fundamental", "财务因子（按法定披露截止日才使用，避免偷看未来）"],
                ] as const
              ).map(([cat, title]) => (
                <div key={cat} className="flex flex-col gap-1.5">
                  <span className="text-xs text-zinc-500">{title}</span>
                  <div className="flex flex-wrap gap-2">
                    {items
                      .filter((f) => f.category === cat)
                      .map((f) => (
                        <FactorChip
                          key={f.id}
                          f={f}
                          pick={picks.find((p) => p.id === f.id)}
                          weights={weights}
                          // 至少保留一个因子：最后一个不能取消
                          locked={picks.length === 1 && picks[0]?.id === f.id}
                          onToggle={() => onChange(toggleFactor(picks, f.id))}
                          onWeight={(w) => onChange(picks.map((p) => (p.id === f.id ? { ...p, weight: w } : p)))}
                        />
                      ))}
                  </div>
                </div>
              ))}
            </section>
          </div>
        );
      }}
    </QueryView>
  );
}

function StepTitle({ n, title, note }: { n: number; title: string; note: string }) {
  return (
    <div className="flex items-center gap-2">
      <span className="flex size-5 items-center justify-center rounded-full bg-zinc-800 text-xs font-semibold text-white dark:bg-zinc-200 dark:text-zinc-900">
        {n}
      </span>
      <span className="text-sm font-medium">{title}</span>
      <span className="text-xs text-zinc-500">{note}</span>
    </div>
  );
}

/** 预设卡片：和下面的因子标签形状不同（白底卡片、两行字），选中时整块描边并标「当前」 */
function PresetCard({
  name,
  sub,
  active,
  onApply,
  help,
  onDelete,
}: {
  name: string;
  sub: string;
  active: boolean;
  onApply: () => void;
  help?: ReactNode;
  onDelete?: () => void;
}) {
  return (
    <div
      className={`flex items-start gap-1 rounded-lg bg-white px-3 py-2 shadow-sm dark:bg-zinc-900 ${
        active ? "ring-2 ring-accent" : "ring-1 ring-zinc-200 hover:ring-zinc-400 dark:ring-zinc-700"
      }`}
    >
      <button type="button" onClick={onApply} aria-pressed={active} className="min-w-0 flex-1 text-left">
        <span className="flex items-center gap-1.5 text-sm font-medium">
          <span className="truncate">{name}</span>
          {active && (
            <span className="shrink-0 rounded bg-accent px-1 text-[10px] leading-4 font-normal text-white">当前</span>
          )}
        </span>
        <span className="block truncate text-xs text-zinc-500">{sub}</span>
      </button>
      {help && <HelpTip label={`${name}的说明`}>{help}</HelpTip>}
      {onDelete && (
        <button
          type="button"
          aria-label={`删除预设${name}`}
          title="删除这个预设"
          onClick={onDelete}
          className="ml-1 text-xs leading-4 text-zinc-400 hover:text-red-500"
        >
          ✕
        </button>
      )}
    </div>
  );
}

/** 自己在「多因子组合」里保存的预设；没有时只显示一句提示 */
function MyPresets({
  picks,
  names,
  onApply,
}: {
  picks: FactorWeight[];
  names: (ids: readonly string[]) => string;
  onApply: (p: PresetChoice) => void;
}) {
  const qc = useQueryClient();
  const list = useListPresets({ query: { staleTime: 60_000 } });
  const remove = useDeletePreset({
    mutation: { onSuccess: () => qc.invalidateQueries({ queryKey: getListPresetsQueryKey() }) },
  });
  const mine = list.data ?? [];
  return (
    <div className="mt-3">
      <span className="text-xs text-zinc-500">我的预设</span>
      {mine.length === 0 ? (
        <p className="mt-1 text-xs text-zinc-400">还没有。在因子研究的「多因子组合」里调好后点「保存为预设」。</p>
      ) : (
        <div className="mt-1 grid gap-2 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
          {mine.map((p) => {
            const factors = p.factors.map((f) => ({ id: f.factor, weight: f.weight }));
            return (
              <PresetCard
                key={p.preset_id}
                name={p.name}
                sub={`${factors.length} 个因子 · 持有 ${p.top_n} 只 · 缓冲 ${p.buffer}`}
                active={sameFactors(picks, factors)}
                onApply={() => onApply({ factors, top_n: p.top_n, buffer: p.buffer })}
                help={
                  <>
                    因子：
                    {p.factors.map((f) => `${names([f.factor])}${f.weight === 1 ? "" : ` × ${f.weight}`}`).join("、")}
                  </>
                }
                onDelete={() => {
                  if (window.confirm(`删除预设「${p.name}」？已经用它设置过的账户不受影响。`))
                    remove.mutate({ presetId: p.preset_id });
                }}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

function FactorChip({
  f,
  pick,
  weights,
  locked,
  onToggle,
  onWeight,
}: {
  locked: boolean;
  f: FactorInfo;
  pick?: FactorWeight;
  weights: boolean;
  onToggle: () => void;
  onWeight: (w: number) => void;
}) {
  const on = pick !== undefined;
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-lg border px-2 py-1 text-sm ${
        on ? "border-accent bg-accent/5" : "border-zinc-200 dark:border-zinc-700"
      }`}
    >
      <button
        type="button"
        onClick={onToggle}
        disabled={locked}
        className="inline-flex items-center gap-1 disabled:cursor-not-allowed"
        title={locked ? "至少保留一个因子：先选上别的因子，再取消这个" : undefined}
      >
        <span className={on ? "text-accent" : "text-zinc-400"}>{on ? "✓" : "+"}</span>
        {f.name}
        <span className="text-xs text-zinc-400">{f.higher_is_better ? "越大越好" : "越小越好"}</span>
      </button>
      <HelpTip label={`${f.name}的说明`}>
        <b>怎么算：</b>
        {f.description}。
        <br />
        <b>{f.higher_is_better ? "越大越好" : "越小越好"}：</b>
        {f.rationale}。
      </HelpTip>
      {on && weights && (
        <input
          // 套用预设会把权重改回 1，换 key 让输入框跟着刷新
          key={pick.weight}
          aria-label={`${f.name}的权重`}
          className={`${inputClass} num ml-1 w-14 px-1.5 py-0.5 text-xs`}
          defaultValue={pick.weight}
          title="权重：作用在按预期方向统一后的得分上，负数表示反过来用"
          onBlur={(e) => {
            const w = Number(e.target.value);
            if (Number.isFinite(w) && w !== pick.weight) onWeight(w);
          }}
        />
      )}
    </span>
  );
}
