/**
 * 长任务的提示：预计耗时、已经等了多久、取消按钮。界面文字面向没有金融背景的用户，不用「请求」「中断」这类词。
 * 预计耗时的数字只引用文档里的实测：组合回测 3 年约 6 秒（docs/domains/backtest.md#portfolio），
 * 信号过去表现 3 年第一次约 19 秒、之后约 20 毫秒（docs/domains/strategies.md#signal_stats），
 * 单只股票十年回测约 30 毫秒（docs/domains/backtest.md#limits）。没有单独实测的区间只给范围，不用编出来的数字。
 */
import { useEffect, useState } from "react";
import { buttonClass } from "./ui";

/** 已经算了多少秒；running 每次从 false 变 true 都重新计时 */
export function useElapsedSeconds(running: boolean): number {
  const [seconds, setSeconds] = useState(0);
  useEffect(() => {
    if (!running) {
      setSeconds(0);
      return;
    }
    const start = Date.now();
    setSeconds(0);
    const id = window.setInterval(() => setSeconds(Math.floor((Date.now() - start) / 1000)), 500);
    return () => window.clearInterval(id);
  }, [running]);
  return seconds;
}

/** 「已等待 12 秒」；超过一分钟用分和秒表示，免得数字越来越大 */
export function elapsedText(seconds: number): string {
  if (seconds < 1) return "刚刚开始";
  if (seconds < 60) return `已等待 ${seconds} 秒`;
  const m = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return rest > 0 ? `已等待 ${m} 分 ${rest} 秒` : `已等待 ${m} 分`;
}

/** 计算中的一行：转圈、说明、已等待多久、可以取消。原来「开始」按钮的位置换成本行的「取消」 */
export function TaskRunning({ text, seconds, onCancel }: { text: string; seconds: number; onCancel: () => void }) {
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-lg border border-zinc-200 bg-zinc-50 px-3 py-2 text-sm dark:border-zinc-700 dark:bg-zinc-800/60">
      <span className="size-4 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" />
      <span className="text-zinc-600 dark:text-zinc-400">{text}</span>
      <span className="num text-xs text-zinc-500">{elapsedText(seconds)}，可以取消</span>
      {seconds >= 60 && (
        <span className="text-xs text-amber-600">已经超过 1 分钟，后端可能正忙，可以继续等，也可以取消稍后再试。</span>
      )}
      <button type="button" className={`${buttonClass("ghost")} ml-auto`} onClick={onCancel}>
        取消
      </button>
    </div>
  );
}

/** 组合回测的预计耗时。区间最长 5 年，只有 3 年是文档里的实测值 */
export function portfolioEstimate(range: string): string {
  if (range === "1") return "全市场 1 年，通常几秒";
  if (range === "3") return "全市场 3 年，约 6 秒";
  if (range === "5") return "全市场 5 年，通常十几秒";
  return "全市场，通常几秒到十几秒";
}

export const PORTFOLIO_ESTIMATE_BASIS =
  "依据：文档实测全市场组合回测 3 年约 6 秒（235 MB 内存）。1 年和 5 年没有单独实测，按日线根数推算，所以只写范围。";

/** 信号过去表现的预计耗时。统计最近 1 到 5 年，只有 3 年是文档里的实测值 */
export function signalStatsEstimate(years: string): string {
  if (years === "3") return "第一次约 20 秒，算过一次后很快";
  if (years === "1") return "第一次十几秒，算过一次后很快";
  if (years === "5") return "第一次可能几十秒，算过一次后很快";
  return "第一次十几秒到几十秒，算过一次后很快";
}

export const SIGNAL_STATS_ESTIMATE_BASIS =
  "依据：文档实测信号过去表现 3 年第一次约 19 秒，之后命中缓存约 20 毫秒。1 年和 5 年没有单独实测，按数据多少给范围。";

/** 单只股票回测只要几十毫秒，保留改参数自动计算，不需要取消 */
export const SINGLE_STOCK_ESTIMATE = "只算这一只股票，通常几十毫秒（十年约 30 毫秒），改参数会自动重算。";

export const SINGLE_STOCK_ESTIMATE_BASIS = "依据：文档实测单只股票十年回测约 30 毫秒。";
