/** 长任务的提示：已经等了多久、取消按钮。界面文字面向没有金融背景的用户，不用「请求」「中断」这类词 */
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
