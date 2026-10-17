import { clsx } from "clsx";
import { type ReactNode, useEffect, useState } from "react";
import { Link } from "react-router";
import type { Catalog, Condition, IndicatorRef, Op, Operand, SignalEntry } from "../api/generated/model";
import {
  defaults,
  describeCondition,
  fillName,
  findIndicator,
  findSignal,
  groupBy,
  indicatorName,
  isIndicatorOperand,
  type MySignals,
  mySignalsOf,
  newSignalCondition,
  OPS,
  type RangeMode,
  rangeOf,
  signalHref,
  signalName,
  UNIT_SUFFIX,
  withRange,
} from "../lib/strategy";
import { buttonClass, inputClass } from "./ui";

/** 条件编辑器：策略的买入、卖出和自定义信号都用它。每条条件下面实时显示白话，信号和指标都能点进去看是什么 */

export function Labeled({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    // biome-ignore lint/a11y/noLabelWithoutControl: 控件通过 children 传入
    <label className={clsx("flex flex-col gap-1 text-xs text-zinc-500", className)}>
      <span>{label}</span>
      {children}
    </label>
  );
}

/** 数字输入：输入合法数字时立即生效；allowEmpty 时清空表示不设置 */
export function NumInput({
  value,
  onChange,
  allowEmpty = false,
  integer = false,
  width = "w-24",
}: {
  value: number | null | undefined;
  onChange: (v: number | null) => void;
  allowEmpty?: boolean;
  integer?: boolean;
  width?: string;
}) {
  const [text, setText] = useState(value === null || value === undefined ? "" : String(value));
  // 外部改了值（例如换了指标、参数被重置）时同步显示
  useEffect(() => {
    setText((t) => (t !== "" && Number(t) === value ? t : value === null || value === undefined ? "" : String(value)));
  }, [value]);
  const bad =
    text === "" ? !allowEmpty : !Number.isFinite(Number(text)) || (integer && !Number.isInteger(Number(text)));
  return (
    <input
      className={clsx(inputClass, "num", width, bad && "border-red-400!")}
      inputMode="decimal"
      value={text}
      placeholder={allowEmpty ? "不设置" : undefined}
      onChange={(e) => {
        const t = e.target.value.trim();
        setText(t);
        if (t === "") {
          if (allowEmpty) onChange(null);
        } else if (Number.isFinite(Number(t)) && (!integer || Number.isInteger(Number(t)))) onChange(Number(t));
      }}
    />
  );
}

/** 条件列表。mine 给出时，信号下拉框里可以选自定义信号；自定义信号的定义里不给，避免信号套信号 */
export function ConditionList({
  title,
  hint,
  items,
  onChange,
  catalog,
  mine,
  empty,
}: {
  title?: string;
  hint?: string;
  items: Condition[];
  onChange: (items: Condition[]) => void;
  catalog: Catalog;
  mine?: SignalEntry[];
  empty?: string;
}) {
  const first = catalog.signals[0];
  const names = mySignalsOf(mine);
  const add = (c: Condition) => onChange([...items, c]);
  return (
    <div>
      {title && <h3 className="text-sm font-medium">{title}</h3>}
      {hint && <p className="mb-2 text-xs text-zinc-500">{hint}</p>}
      <div className="flex flex-col gap-2">
        {items.length === 0 && empty && <p className="text-xs text-zinc-400">{empty}</p>}
        {items.map((c, i) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: 条件没有自己的 id，顺序就是身份
            key={i}
            className="flex flex-col gap-2 rounded-lg border border-zinc-200 bg-zinc-50/50 p-2 text-sm dark:border-zinc-800 dark:bg-zinc-800/30"
          >
            <div className="flex flex-wrap items-center gap-2">
              <ConditionRow
                c={c}
                catalog={catalog}
                mine={mine}
                onChange={(nc) => onChange(items.map((x, j) => (j === i ? nc : x)))}
              />
              <button
                type="button"
                className="ml-auto rounded px-2 py-1 text-zinc-400 hover:bg-zinc-100 hover:text-red-600 dark:hover:bg-zinc-800"
                title="删除这条"
                aria-label="删除这条"
                onClick={() => onChange(items.filter((_, j) => j !== i))}
              >
                ×
              </button>
            </div>
            <ConditionHelp c={c} catalog={catalog} names={names} />
          </div>
        ))}
        <div className="flex flex-wrap gap-2">
          {first && (
            <button
              type="button"
              className={buttonClass("ghost")}
              title="用一个已有的信号，例如「均线金叉」「放量」"
              onClick={() => add(newSignalCondition(first))}
            >
              + 信号
            </button>
          )}
          <button
            type="button"
            className={buttonClass("ghost")}
            title="比较一个指标和一个数值或另一个指标，例如「RSI（14）低于 30」「收盘价上穿 60 日均线」"
            onClick={() => add({ kind: "compare", left: { id: "rsi", n: 14 }, op: "<", right: { value: 30 } })}
          >
            + 指标比较
          </button>
        </div>
      </div>
    </div>
  );
}

/** 条件下面的一行：白话，以及它用到的信号、指标的说明链接 */
function ConditionHelp({ c, catalog, names }: { c: Condition; catalog: Catalog; names: MySignals }) {
  const text = describeCondition(catalog, c, names);
  const href = signalHref(c);
  const sig = c.kind === "signal" ? findSignal(catalog, c.signal) : undefined;
  const inds = c.kind === "compare" ? [c.left, ...(isIndicatorOperand(c.right) ? [c.right] : [])].map((r) => r.id) : [];
  const cross = c.kind === "compare" && (c.op === "crosses_above" || c.op === "crosses_below");
  return (
    <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-xs">
      <span className="text-zinc-700 dark:text-zinc-300">
        <span className="text-zinc-400">意思是：</span>
        {text}
      </span>
      {sig && c.kind === "signal" && (
        <span className="text-zinc-400">{fillName(sig.description, { ...defaults(sig), ...c.params })}</span>
      )}
      {cross && <span className="text-zinc-400">上穿、下穿只在穿过去的那一天成立，适合当买卖点</span>}
      {href && (
        <Link to={href} target="_blank" className="text-accent hover:underline">
          这个信号是什么？
        </Link>
      )}
      {[...new Set(inds)].map((id) => {
        const info = findIndicator(catalog, id);
        return (
          <Link
            key={id}
            to={`/indicators/${id}`}
            target="_blank"
            className="text-accent hover:underline"
            title={info?.description}
          >
            {info ? info.name.replace(/\{n\} ?日?/, "").replace("（）", "") : id} 是什么？
          </Link>
        );
      })}
    </div>
  );
}

const MAX_DAYS = 20;

function ConditionRow({
  c,
  catalog,
  mine,
  onChange,
}: {
  c: Condition;
  catalog: Catalog;
  mine?: SignalEntry[];
  onChange: (c: Condition) => void;
}) {
  if (c.kind === "signal" || c.kind === "my_signal") {
    const info = c.kind === "signal" ? findSignal(catalog, c.signal) : undefined;
    const value = c.kind === "signal" ? c.signal : `my-${c.signal_id}`;
    const keep = { within_days: c.within_days, streak_days: c.streak_days };
    const custom = (mine ?? []).filter((m) => !m.builtin);
    return (
      <>
        <select
          className={clsx(inputClass, "max-w-full")}
          value={value}
          onChange={(e) => {
            const v = e.target.value;
            if (v.startsWith("my-")) onChange({ kind: "my_signal", signal_id: Number(v.slice(3)), ...keep });
            else {
              const next = findSignal(catalog, v);
              if (next) onChange({ ...newSignalCondition(next), ...keep });
            }
          }}
        >
          {custom.length > 0 && (
            <optgroup label="自定义信号">
              {custom.map((m) => (
                <option key={m.key} value={m.key}>
                  {m.name}
                </option>
              ))}
            </optgroup>
          )}
          {groupBy(catalog.signals).map(([g, list]) => (
            <optgroup key={g} label={g}>
              {list.map((s) => (
                <option key={s.id} value={s.id}>
                  {signalName(s)}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
        {c.kind === "signal" &&
          info?.params.map((p) => (
            <span key={p.key} className="flex items-center gap-1 text-xs text-zinc-500">
              {p.label}
              <NumInput
                value={c.params?.[p.key] ?? p.default}
                integer={p.integer}
                width="w-16"
                onChange={(v) => v !== null && onChange({ ...c, params: { ...c.params, [p.key]: v } })}
              />
            </span>
          ))}
        <TimeRange c={c} onChange={onChange} />
      </>
    );
  }
  const right = c.right;
  const unit = findIndicator(catalog, c.left.id)?.unit ?? "";
  return (
    <>
      <IndicatorPicker catalog={catalog} value={c.left} onChange={(left) => onChange({ ...c, left })} />
      <select
        className={inputClass}
        value={c.op}
        title={OPS.find((o) => o.value === c.op)?.hint}
        onChange={(e) => onChange({ ...c, op: e.target.value as Op })}
      >
        {OPS.map((o) => (
          <option key={o.value} value={o.value} title={o.hint}>
            {o.label}
          </option>
        ))}
      </select>
      <select
        className={inputClass}
        value={isIndicatorOperand(right) ? "indicator" : "value"}
        onChange={(e) =>
          onChange({ ...c, right: (e.target.value === "value" ? { value: 0 } : { id: "ma", n: 20 }) as Operand })
        }
      >
        <option value="value">一个数</option>
        <option value="indicator">另一个指标</option>
      </select>
      {isIndicatorOperand(right) ? (
        <IndicatorPicker catalog={catalog} value={right} onChange={(r) => onChange({ ...c, right: r })} />
      ) : (
        <span className="flex items-center gap-1">
          <NumInput value={right.value} onChange={(v) => v !== null && onChange({ ...c, right: { value: v } })} />
          <span className="text-xs text-zinc-500">{UNIT_SUFFIX[unit]}</span>
        </span>
      )}
      <TimeRange c={c} onChange={onChange} />
    </>
  );
}

const RANGE_LABEL: Record<RangeMode, string> = {
  today: "当天",
  within: "最近几天内出现过",
  streak: "连续几天",
};

/** 时间范围：当天、最近 N 个交易日内出现过、连续 N 个交易日 */
function TimeRange({ c, onChange }: { c: Condition; onChange: (c: Condition) => void }) {
  const { mode, n } = rangeOf(c);
  return (
    <span className="flex items-center gap-1 text-xs text-zinc-500">
      <select
        className={inputClass}
        value={mode}
        title="当天：只看这一天；最近几天内出现过：这几天里有一天满足就行；连续几天：这几天每天都满足"
        onChange={(e) => {
          const m = e.target.value as RangeMode;
          onChange(withRange(c, m, m === "today" ? 1 : Math.max(n, m === "within" ? 5 : 3)));
        }}
      >
        {(Object.keys(RANGE_LABEL) as RangeMode[]).map((m) => (
          <option key={m} value={m}>
            {RANGE_LABEL[m]}
          </option>
        ))}
      </select>
      {mode !== "today" && (
        <>
          <NumInput
            value={n}
            integer
            width="w-14"
            onChange={(v) => v !== null && v >= 2 && v <= MAX_DAYS && onChange(withRange(c, mode, v))}
          />
          个交易日
        </>
      )}
    </span>
  );
}

export function IndicatorPicker({
  catalog,
  value,
  onChange,
}: {
  catalog: Catalog;
  value: IndicatorRef;
  onChange: (v: IndicatorRef) => void;
}) {
  const info = findIndicator(catalog, value.id);
  return (
    <span className="flex items-center gap-1" title={info?.description}>
      <select
        className={inputClass}
        value={value.id}
        onChange={(e) => {
          const next = findIndicator(catalog, e.target.value);
          if (next) onChange({ id: next.id, n: next.n_default ?? null });
        }}
      >
        {groupBy(catalog.indicators).map(([g, list]) => (
          <optgroup key={g} label={g}>
            {list.map((x) => (
              <option key={x.id} value={x.id}>
                {indicatorName(x, x.id === value.id ? value.n : undefined)}
              </option>
            ))}
          </optgroup>
        ))}
      </select>
      {info && info.n_default !== null && info.n_default !== undefined && (
        <span className="flex items-center gap-1 text-xs text-zinc-500">
          周期
          <NumInput
            value={value.n ?? info.n_default}
            integer
            width="w-16"
            onChange={(n) => n !== null && onChange({ ...value, n })}
          />
        </span>
      )}
    </span>
  );
}
