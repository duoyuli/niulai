import type { AdviceAction, TradeKind, TradeSource, WeakMarket } from "../api/generated/model";

/** 金额：¥12,345；digits 为 2 时保留到分 */
export function yuan(v: number | null | undefined, digits = 0): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  const s = Math.abs(v).toLocaleString("zh-CN", { minimumFractionDigits: digits, maximumFractionDigits: digits });
  return `${v < 0 ? "-" : ""}¥${s}`;
}

/** 建议的动作：卖出、减仓用绿色（卖出），买入用红色（买入），与 A 股红涨绿跌一致 */
export const ACTIONS: Record<AdviceAction, { label: string; chip: string; dot: string }> = {
  sell: { label: "卖出", chip: "bg-down/10 text-down", dot: "bg-down" },
  reduce: { label: "减仓", chip: "bg-down/10 text-down", dot: "bg-down" },
  buy: { label: "买入", chip: "bg-up/10 text-up", dot: "bg-up" },
  hold: { label: "继续拿", chip: "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300", dot: "bg-zinc-400" },
  wait: { label: "不动", chip: "bg-amber-500/10 text-amber-700 dark:text-amber-400", dot: "bg-amber-500" },
};

export const TRADE_KINDS: Record<TradeKind, string> = {
  buy: "买入",
  sell: "卖出",
  dividend: "分红",
  bonus: "送转",
  rights: "配股认购",
  cash_in: "转入现金",
  cash_out: "转出现金",
  adjust: "校准",
};

export const TRADE_SOURCES: Record<TradeSource, string> = {
  user: "手动",
  advice: "按建议",
  corp_action: "自动记账",
};

export const WEAK_MARKET: Record<WeakMarket, string> = {
  pause: "不买新股票",
  half: "新买入的金额减半",
  normal: "照常买入",
};

/** 理由的种类 */
export const NOTE_KINDS: Record<string, string> = {
  rule: "规则",
  price: "价格",
  position: "持仓",
  diagnosis: "诊断",
  strategy: "策略",
  stat: "过去表现",
  history: "买入前回看",
  market: "市场",
  risk: "风险",
};
