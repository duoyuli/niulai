import { useCallback, useRef } from "react";
import { useSearchParams } from "react-router";

/** 把筛选条件放在 URL 查询参数里：刷新、分享链接、前进后退都能保留。等于默认值的参数不写进 URL */
export function useSearchState<K extends string>(defaults: Record<K, string>) {
  const [params, setParams] = useSearchParams();
  // 调用方每次渲染都传新的字面量；放进 ref，set 函数保持稳定
  const defaultsRef = useRef(defaults);
  defaultsRef.current = defaults;
  const values = {} as Record<K, string>;
  for (const k of Object.keys(defaults) as K[]) values[k] = params.get(k) ?? defaults[k];
  const set = useCallback(
    (patch: Partial<Record<K, string>>) =>
      setParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          for (const [k, v] of Object.entries(patch) as [K, string][]) {
            if (v === defaultsRef.current[k] || v === "") next.delete(k);
            else next.set(k, v);
          }
          return next;
        },
        { replace: true },
      ),
    [setParams],
  );
  return [values, set] as const;
}
