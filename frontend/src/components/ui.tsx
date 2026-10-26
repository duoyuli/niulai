import { clsx } from "clsx";
import { type CSSProperties, type ReactNode, useEffect, useRef, useState } from "react";
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

/** 小问号：鼠标移上去、键盘聚焦或点一下显示一段说明；点别处、按 Esc 或滚动页面时收起。
 * 说明框用 fixed 定位，放在会横向滚动的表格里也不会被裁掉 */
export function HelpTip({ children, label = "说明" }: { children: ReactNode; label?: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [pos, setPos] = useState<CSSProperties | null>(null);
  const [pinned, setPinned] = useState(false);
  const show = () => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    const width = Math.min(320, window.innerWidth * 0.8);
    const left = Math.max(8, Math.min(r.left, window.innerWidth - width - 8));
    // 下方放不下时放到上方
    setPos(
      r.bottom + 200 > window.innerHeight
        ? { left, width, bottom: window.innerHeight - r.top, paddingBottom: 4 }
        : { left, width, top: r.bottom, paddingTop: 4 },
    );
  };
  const hide = () => {
    setPinned(false);
    setPos(null);
  };
  const open = pos !== null;
  useEffect(() => {
    if (!open) return;
    const hide = () => {
      setPinned(false);
      setPos(null);
    };
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) hide();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && hide();
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", hide, true);
    window.addEventListener("resize", hide);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("resize", hide);
    };
  }, [open]);
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: 悬停只是辅助，键盘和点击由里面的按钮处理
    <span
      ref={ref}
      className="relative inline-flex align-middle"
      onMouseEnter={show}
      onMouseLeave={() => !pinned && setPos(null)}
    >
      <button
        type="button"
        aria-label={label}
        aria-expanded={open}
        className="ml-1 flex size-4 cursor-help items-center justify-center rounded-full border border-zinc-300 text-[10px] leading-none text-zinc-500 hover:border-zinc-500 hover:text-zinc-800 dark:border-zinc-600 dark:hover:text-zinc-200"
        onFocus={show}
        onBlur={() => !pinned && setPos(null)}
        onClick={(e) => {
          // 在 label 里时不要把点击转给输入框
          e.preventDefault();
          if (pinned) hide();
          else {
            show();
            setPinned(true);
          }
        }}
      >
        ?
      </button>
      {pos && (
        <span role="tooltip" className="fixed z-50 block" style={pos}>
          <span className="block rounded-lg border border-zinc-200 bg-white p-3 text-left text-xs leading-relaxed font-normal whitespace-normal text-zinc-700 shadow-lg dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-300">
            {children}
          </span>
        </span>
      )}
    </span>
  );
}

/** 带小问号的文字，用在表头、指标名等处：名词后面跟一段说明 */
export function Term({ children, help }: { children: ReactNode; help: ReactNode }) {
  return (
    <span className="inline-flex items-center whitespace-nowrap">
      {children}
      <HelpTip label={typeof children === "string" ? `${children}的说明` : undefined}>{help}</HelpTip>
    </span>
  );
}

/** 表单项：标签和控件。hint 是一句简短提示，help 是较长的说明，都显示成标签后面的小问号 */
export function Field({
  label,
  children,
  hint,
  help,
}: {
  label: string;
  children: ReactNode;
  hint?: string;
  help?: ReactNode;
}) {
  return (
    // biome-ignore lint/a11y/noLabelWithoutControl: 控件通过 children 传入
    <label className="flex flex-col gap-1 text-xs text-zinc-500">
      <span className="inline-flex items-center">
        {label}
        {(help ?? hint) && <HelpTip label={`${label}的说明`}>{help ?? hint}</HelpTip>}
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

export function Stat({ label, value, sub }: { label: ReactNode; value: ReactNode; sub?: ReactNode }) {
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
