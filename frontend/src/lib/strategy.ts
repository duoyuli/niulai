/** 策略、信号、指标的展示辅助。名称模板和参数来自后端的 /strategies/catalog，见 docs/domains/strategies.md */
import type {
  Catalog,
  Condition,
  ExitReason,
  IndicatorInfo,
  IndicatorRef,
  Operand,
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
  max_hold: "持有到期",
};
