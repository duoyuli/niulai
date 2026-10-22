/** 策略、信号、指标的展示辅助。名称模板和参数来自后端的 /strategies/catalog，见 docs/domains/strategies.md */
import type {
  Catalog,
  Condition,
  ExitReason,
  IndicatorInfo,
  IndicatorRef,
  Op,
  Operand,
  SignalEntry,
  SignalInfo,
} from "../api/generated/model";

/** 把「{fast} 日均线上穿 {slow} 日均线」中的占位换成参数值 */
export function fillName(template: string, values: Record<string, number | null | undefined>): string {
  return template.replace(/\{(\w+)\}/g, (_, k: string) => {
    const v = values[k];
    return v === null || v === undefined ? k : String(v);
  });
}

/** 信号各参数的默认值 */
export function defaults(info: SignalInfo): Record<string, number> {
  return Object.fromEntries(info.params.map((p) => [p.key, p.default]));
}

/** 信号的名称：参数缺省时用目录里的默认值 */
export function signalName(info: SignalInfo, params?: Record<string, number>): string {
  return fillName(info.name, { ...defaults(info), ...params });
}

/** 指标的名称：周期缺省时用默认值 */
export function indicatorName(info: IndicatorInfo, n?: number | null): string {
  return fillName(info.name, { n: n ?? info.n_default });
}

export const UNIT_SUFFIX: Record<string, string> = {
  price: "元",
  pct: "%",
  ratio: "倍",
  yuan: "元",
  shares: "股",
  index: "",
};

export function isIndicatorOperand(o: Operand): o is IndicatorRef {
  return "id" in o;
}

export function findSignal(catalog: Catalog, id: string) {
  return catalog.signals.find((s) => s.id === id);
}

export function findIndicator(catalog: Catalog, id: string) {
  return catalog.indicators.find((i) => i.id === id);
}

/** 按 group 分组，保持目录里的顺序，用于下拉框的 optgroup */
export function groupBy<T extends { group: string }>(items: T[]): [string, T[]][] {
  const out = new Map<string, T[]>();
  for (const x of items) out.set(x.group, [...(out.get(x.group) ?? []), x]);
  return [...out];
}

/** 新加一条信号条件：默认参数、当天出现 */
export function newSignalCondition(info: SignalInfo): Extract<Condition, { kind: "signal" }> {
  return { kind: "signal", signal: info.id, params: defaults(info), within_days: 1 };
}

export const EXIT_REASONS: Record<ExitReason, string> = {
  signal: "卖出信号",
  stop_loss: "止损",
  take_profit: "止盈",
  trailing_stop: "回撤止损",
  max_hold: "持有到期",
};

/** 六种比较，和后端 Op 的白话一致 */
export const OPS: { value: Op; label: string; hint: string }[] = [
  { value: ">", label: "高于", hint: "当天左边大于右边" },
  { value: ">=", label: "不低于", hint: "当天左边大于或等于右边" },
  { value: "<", label: "低于", hint: "当天左边小于右边" },
  { value: "<=", label: "不高于", hint: "当天左边小于或等于右边" },
  { value: "crosses_above", label: "上穿", hint: "前一天还不高于、当天变成高于：只在穿过去的那一天成立" },
  { value: "crosses_below", label: "下穿", hint: "前一天还不低于、当天变成低于：只在穿过去的那一天成立" },
];

export const OP_WORD = Object.fromEntries(OPS.map((o) => [o.value, o.label])) as Record<Op, string>;

/** 数值加单位，和后端 format_value 一致：5e7 元写成「5000 万元」 */
export function formatValue(v: number, unit: string): string {
  const n = String(v);
  if (unit === "pct") return `${n}%`;
  if (unit === "ratio") return `${n} 倍`;
  if (unit === "yuan" && Math.abs(v) >= 1e8) return `${v / 1e8} 亿元`;
  if (unit === "yuan" && Math.abs(v) >= 1e4) return `${v / 1e4} 万元`;
  if (unit === "yuan" || unit === "price") return `${n} 元`;
  return n;
}

/** 条件的时间范围：当天、最近 N 个交易日内出现过、连续 N 个交易日 */
export type RangeMode = "today" | "within" | "streak";

export function rangeOf(c: Condition): { mode: RangeMode; n: number } {
  const within = c.within_days ?? 1;
  const streak = c.streak_days ?? 1;
  if (streak > 1) return { mode: "streak", n: streak };
  if (within > 1) return { mode: "within", n: within };
  return { mode: "today", n: 1 };
}

export function withRange(c: Condition, mode: RangeMode, n: number): Condition {
  return { ...c, within_days: mode === "within" ? n : 1, streak_days: mode === "streak" ? n : 1 };
}

/** 自定义信号的名称和是否是状态，用来写白话 */
export type MySignals = Map<number, { name: string; state: boolean }>;

export function mySignalsOf(entries: SignalEntry[] | undefined): MySignals {
  return new Map(
    (entries ?? [])
      .filter((e) => !e.builtin && e.signal_id !== null && e.signal_id !== undefined)
      .map((e) => [e.signal_id as number, { name: e.name, state: e.kind === "state" }]),
  );
}

function wrapSignal(name: string, state: boolean, within: number, streak: number): string {
  if (streak > 1) return `连续 ${streak} 个交易日${state ? "处于" : "出现"}「${name}」`;
  if (within <= 1) return `当天${state ? "处于" : "出现"}「${name}」`;
  return `最近 ${within} 个交易日内${state ? "处于过" : "出现过"}「${name}」`;
}

export function indicatorLabel(catalog: Catalog, r: IndicatorRef): string {
  const info = findIndicator(catalog, r.id);
  return info ? indicatorName(info, r.n) : r.id;
}

/** 一条条件的白话，和后端 Condition::describe 的写法一致，编辑时边改边看 */
export function describeCondition(catalog: Catalog, c: Condition, mine: MySignals): string {
  const within = c.within_days ?? 1;
  const streak = c.streak_days ?? 1;
  if (c.kind === "signal") {
    const info = findSignal(catalog, c.signal);
    return wrapSignal(info ? signalName(info, c.params) : c.signal, info?.kind === "state", within, streak);
  }
  if (c.kind === "my_signal") {
    const m = mine.get(c.signal_id);
    return wrapSignal(m?.name ?? `自定义信号 ${c.signal_id}`, m?.state ?? false, within, streak);
  }
  const unit = findIndicator(catalog, c.left.id)?.unit ?? "";
  const right = isIndicatorOperand(c.right) ? indicatorLabel(catalog, c.right) : formatValue(c.right.value, unit);
  const core = `${indicatorLabel(catalog, c.left)}${OP_WORD[c.op]} ${right}`;
  if (within > 1) return `最近 ${within} 个交易日内出现过「${core}」`;
  if (streak > 1) return `连续 ${streak} 个交易日${core}`;
  return core;
}

/** 条件里引用的信号在信号页的地址 */
export function signalHref(c: Condition): string | null {
  if (c.kind === "signal") return `/signals/${c.signal}`;
  if (c.kind === "my_signal") return `/signals/my-${c.signal_id}`;
  return null;
}

/** 巡检最多保存的持有中股票数，和后端 screener::MAX_HOLDINGS 一致；到了这个数说明实际更多 */
export const MAX_HOLDINGS = 300;

export function holdingsText(n: number): string {
  return n >= MAX_HOLDINGS ? `${MAX_HOLDINGS}+` : String(n);
}
