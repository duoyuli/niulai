import { useSyncExternalStore } from "react";

const query = "(prefers-color-scheme: dark)";

function subscribe(cb: () => void) {
  const mql = window.matchMedia(query);
  mql.addEventListener("change", cb);
  return () => mql.removeEventListener("change", cb);
}

/** 跟随系统的深色模式 */
export function useDarkMode(): boolean {
  return useSyncExternalStore(subscribe, () => window.matchMedia(query).matches);
}
