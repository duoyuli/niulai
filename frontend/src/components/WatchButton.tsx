import { clsx } from "clsx";
import { useWatchlist } from "../hooks/useWatchlist";

/** 收藏星标：实心表示已收藏。withLabel 时显示「收藏」「已收藏」文字 */
export function WatchButton({ thscode, withLabel = false }: { thscode: string; withLabel?: boolean }) {
  const { watched, ready, pendingCode, toggle } = useWatchlist();
  const on = watched.has(thscode);
  const title = on ? "已收藏，点击取消" : "收藏到自选，方便以后快速查看";
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      aria-pressed={on}
      disabled={!ready || pendingCode === thscode}
      onClick={(e) => {
        // 放在可点击的行里时不触发行的跳转
        e.preventDefault();
        e.stopPropagation();
        toggle(thscode);
      }}
      className={clsx(
        "inline-flex items-center gap-1 rounded-md text-sm leading-none transition disabled:opacity-50",
        withLabel
          ? "border border-zinc-200 bg-white px-2.5 py-1.5 hover:bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900 dark:hover:bg-zinc-800"
          : "p-1 hover:bg-zinc-100 dark:hover:bg-zinc-800",
        on ? "text-amber-500" : "text-zinc-400 hover:text-amber-500",
      )}
    >
      <span aria-hidden className="text-base">
        {on ? "★" : "☆"}
      </span>
      {withLabel && <span className="text-zinc-700 dark:text-zinc-200">{on ? "已收藏" : "收藏"}</span>}
    </button>
  );
}
