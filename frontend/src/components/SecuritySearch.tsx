import { type ReactNode, useState } from "react";
import { useListSecurities } from "../api/generated/market/market";
import { useDebounced } from "../hooks/useDebounced";
import { Field, inputClass } from "./ui";

/** 按代码或名称搜索证券的下拉框。onPick 时选中后关闭；action 为每一行前面加一个操作（例如收藏星标） */
export function SecuritySearch({
  label,
  onPick,
  action,
  width = "w-64",
}: {
  /** 为空时不显示标签（页头的搜索框） */
  label?: string;
  width?: string;
  onPick?: (thscode: string) => void;
  action?: (thscode: string) => ReactNode;
}) {
  const [text, setText] = useState("");
  const q = useDebounced(text).trim();
  const found = useListSecurities({ q, limit: 8 }, { query: { enabled: q !== "" } });
  const input = (
    <input
      className={`${inputClass} ${width}`}
      placeholder={label ? "代码或名称，例如 600519 或 沪深300ETF" : "搜索证券"}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Escape") setText("");
        const first = found.data?.items[0];
        if (e.key === "Enter" && onPick && first) {
          onPick(first.thscode);
          setText("");
        }
      }}
    />
  );
  return (
    <div className="relative">
      {label ? <Field label={label}>{input}</Field> : input}
      {q !== "" && found.data && (
        <ul className="absolute right-0 z-20 mt-1 w-72 sm:right-auto overflow-hidden rounded-lg border border-zinc-200 bg-white text-sm shadow-lg dark:border-zinc-700 dark:bg-zinc-900">
          {found.data.items.length === 0 ? (
            <li className="px-3 py-2 text-zinc-500">没有找到</li>
          ) : (
            found.data.items.map((x) => (
              <li
                key={x.thscode}
                className="flex items-center gap-2 px-2 py-1.5 hover:bg-zinc-50 dark:hover:bg-zinc-800"
              >
                {action?.(x.thscode)}
                {onPick ? (
                  <button
                    type="button"
                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    onClick={() => {
                      onPick(x.thscode);
                      setText("");
                    }}
                  >
                    <span className="truncate">{x.name}</span>
                    <span className="num ml-auto font-mono text-xs text-zinc-500">{x.thscode}</span>
                  </button>
                ) : (
                  <>
                    <span className="truncate">{x.name}</span>
                    <span className="num ml-auto font-mono text-xs text-zinc-500">{x.thscode}</span>
                  </>
                )}
              </li>
            ))
          )}
        </ul>
      )}
    </div>
  );
}
