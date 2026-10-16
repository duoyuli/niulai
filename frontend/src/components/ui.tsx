import { clsx } from "clsx";
import type { ReactNode } from "react";
import { ApiError } from "../api/fetcher";
import type { Tone } from "../lib/labels";

export function Card({
  title,
  extra,
  children,
  className,
}: {
  title?: ReactNode;
  extra?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={clsx(
        "min-w-0 rounded-xl border border-zinc-200 bg-white dark:border-zinc-800 dark:bg-zinc-900",
        className,
      )}
    >
      {(title || extra) && (
        <header className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-100 px-4 py-3 dark:border-zinc-800">
          <h2 className="text-sm font-semibold">{title}</h2>
          {extra}
        </header>
      )}
      <div className="p-4">{children}</div>
    </section>
  );
}

const toneClass: Record<Tone, string> = {
  ok: "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-500/10 dark:text-emerald-300",
  warn: "bg-amber-50 text-amber-800 ring-amber-600/20 dark:bg-amber-500/10 dark:text-amber-300",
  bad: "bg-red-50 text-red-700 ring-red-600/20 dark:bg-red-500/10 dark:text-red-300",
  info: "bg-blue-50 text-blue-700 ring-blue-600/20 dark:bg-blue-500/10 dark:text-blue-300",
  muted: "bg-zinc-100 text-zinc-600 ring-zinc-500/20 dark:bg-zinc-800 dark:text-zinc-300",
};

export function Badge({ tone = "muted", children }: { tone?: Tone; children: ReactNode }) {
  return (
    <span
      className={clsx(
        "inline-flex items-center rounded-md px-1.5 py-0.5 text-xs font-medium ring-1 ring-inset whitespace-nowrap",
        toneClass[tone],
      )}
    >
      {children}
    </span>
  );
}

/** 金叉 / 死叉标签：金叉用红色（看涨），死叉用绿色（看跌），与 A 股红涨绿跌一致 */
export function SignalBadge({ signal }: { signal: "golden" | "death" }) {
  return signal === "golden" ? (
    <span className="inline-flex items-center gap-1 rounded-md bg-up/10 px-1.5 py-0.5 text-xs font-medium text-up">
      ▲ 金叉
    </span>
  ) : (
    <span className="inline-flex items-center gap-1 rounded-md bg-down/10 px-1.5 py-0.5 text-xs font-medium text-down">
      ▼ 死叉
    </span>
  );
}

export function Spinner({ label = "加载中" }: { label?: string }) {
  return (
    <div className="flex items-center justify-center gap-2 py-10 text-sm text-zinc-500">
      <span className="size-4 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" />
      {label}
    </div>
  );
}

export function errorText(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.status === 404) return "没有找到";
    return err.message;
  }
  if (err instanceof TypeError) return "无法连接后端服务，确认 niulai serve 正在运行";
  return err instanceof Error ? err.message : String(err);
}

export function ErrorState({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  return (
    <div className="flex flex-col items-center gap-3 py-10 text-sm">
      <p className="text-red-600 dark:text-red-400">{errorText(error)}</p>
      {onRetry && (
        <button type="button" onClick={onRetry} className={buttonClass("ghost")}>
          重试
        </button>
      )}
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="py-10 text-center text-sm text-zinc-500">{children}</p>;
}

/** 按查询状态渲染：加载中、出错、有数据 */
export function QueryView<T>({
  query,
  children,
}: {
  query: { data: T | undefined; error: unknown; isPending: boolean; refetch: () => unknown };
  children: (data: T) => ReactNode;
}) {
  if (query.isPending) return <Spinner />;
  if (query.error || query.data === undefined)
    return <ErrorState error={query.error} onRetry={() => query.refetch()} />;
  return <>{children(query.data)}</>;
}

export function buttonClass(variant: "primary" | "ghost" = "primary") {
  return clsx(
    "inline-flex items-center justify-center gap-1 rounded-lg px-3 py-1.5 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-50",
    variant === "primary"
      ? "bg-accent text-white hover:bg-accent/90"
      : "border border-zinc-200 bg-white hover:bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900 dark:hover:bg-zinc-800",
  );
}

export const inputClass =
  "rounded-lg border border-zinc-200 bg-white px-3 py-1.5 text-sm text-zinc-900 outline-none placeholder:text-zinc-400 focus:border-accent focus:ring-2 focus:ring-accent/20 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100";

/** 一组互斥选项（分段按钮） */
export function Segmented<V extends string>({
  value,
  options,
  onChange,
}: {
  value: V;
  options: readonly { value: V; label: string; hint?: string; disabled?: boolean }[];
  onChange: (v: V) => void;
}) {
  return (
    <div className="inline-flex flex-wrap rounded-lg border border-zinc-200 bg-zinc-100 p-0.5 dark:border-zinc-700 dark:bg-zinc-800">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          title={o.hint}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
          className={clsx(
            "rounded-md px-2.5 py-1 text-xs font-medium whitespace-nowrap transition disabled:cursor-not-allowed disabled:opacity-40",
            o.value === value
              ? "bg-white text-zinc-900 shadow-sm dark:bg-zinc-950 dark:text-zinc-100"
              : "text-zinc-500 hover:text-zinc-800 enabled:hover:text-zinc-800 dark:hover:text-zinc-200",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    // biome-ignore lint/a11y/noLabelWithoutControl: 控件通过 children 传入
    <label className="flex flex-col gap-1 text-xs text-zinc-500">
      <span title={hint}>
        {label}
        {hint && <span className="ml-1 cursor-help text-zinc-400">ⓘ</span>}
      </span>
      {children}
    </label>
  );
}

/** 表格外壳：窄屏横向滚动 */
export function Table({ children }: { children: ReactNode }) {
  return (
    <div className="-mx-4 overflow-x-auto">
      <table className="w-full min-w-max border-collapse text-sm [&_td]:px-4 [&_td]:py-2 [&_th]:px-4 [&_th]:py-2 [&_th]:text-left [&_th]:text-xs [&_th]:font-medium [&_th]:text-zinc-500 [&_tbody_tr]:border-t [&_tbody_tr]:border-zinc-100 dark:[&_tbody_tr]:border-zinc-800">
        {children}
      </table>
    </div>
  );
}

export function Stat({ label, value, sub }: { label: string; value: ReactNode; sub?: ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-zinc-500">{label}</span>
      <span className="num text-base font-semibold">{value}</span>
      {sub && <span className="text-xs text-zinc-500">{sub}</span>}
    </div>
  );
}

export function ProgressBar({ done, failed, total }: { done: number; failed: number; total: number }) {
  const pct = (n: number) => (total > 0 ? `${(n / total) * 100}%` : "0%");
  return (
    <div className="flex h-1.5 w-full overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800">
      <div className="bg-emerald-500" style={{ width: pct(done) }} />
      <div className="bg-red-500" style={{ width: pct(failed) }} />
    </div>
  );
}

export function PageHeader({ title, desc, extra }: { title: ReactNode; desc?: ReactNode; extra?: ReactNode }) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-3">
      <div className="min-w-0">
        <h1 className="text-xl font-semibold">{title}</h1>
        {desc && <p className="mt-1 text-sm text-zinc-500">{desc}</p>}
      </div>
      {extra}
    </div>
  );
}
