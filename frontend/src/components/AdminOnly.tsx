import type { ReactNode } from "react";
import { useMe } from "../api/generated/auth/auth";
import { Empty, Spinner } from "./ui";

/** 只有管理员能打开的页面：普通用户直接打开地址时给一句说明，不发出会被拒绝的请求 */
export function AdminOnly({ children }: { children: ReactNode }) {
  const me = useMe({ query: { staleTime: Number.POSITIVE_INFINITY } });
  if (me.isPending) return <Spinner />;
  if (me.data?.role !== "admin") return <Empty>这个页面只有管理员可以使用。</Empty>;
  return <>{children}</>;
}
