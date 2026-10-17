import { useMe } from "../api/generated/auth/auth";

/** 当前用户是不是管理员。普通用户只能查看策略、信号，不能改，也看不到数据同步、数据源设置和用户管理；
 *  后端同样拦截，这里只是不显示用不了的按钮 */
export function useIsAdmin(): boolean {
  const me = useMe({ query: { staleTime: Number.POSITIVE_INFINITY } });
  return me.data?.role === "admin";
}
