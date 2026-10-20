import { clsx } from "clsx";
import { Suspense } from "react";
import { Navigate, NavLink, Outlet, useLocation, useNavigate } from "react-router";
import { useListAccounts } from "../api/generated/accounts/accounts";
import { useMe } from "../api/generated/auth/auth";
import { useGetSetup } from "../api/generated/setup/setup";
import { todayCn } from "../lib/format";
import { SecuritySearch } from "./SecuritySearch";
import { UserMenu } from "./UserMenu";
import { Spinner } from "./ui";
import { BackupAlert, VersionBadge } from "./VersionBadge";

/** 主导航：今天看什么、我的账户（每天的调仓建议）、选股方法、我关注的股票。个股从页头搜索进入 */
const NAV = [
  { to: "/", label: "首页", end: true },
  { to: "/accounts", label: "账户" },
  { to: "/strategies", label: "策略" },
  { to: "/watchlist", label: "自选" },
];

/** 不是每天用的，放在右侧：说明和数据。个人与管理员功能收在用户名的菜单里 */
const AUX = [
  { to: "/guide", label: "说明", title: "指标、信号、策略是什么，怎么用", who: "all" },
  { to: "/sync", label: "数据", title: "数据同步与指数板块", who: "admin" },
] as const;

export function Layout() {
  // 没有登录时 fetcher 收到 401 会转到登录页；登录后才检查 API Key
  const me = useMe({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const admin = me.data?.role === "admin";
  // 首次登录引导：还没有配置 API Key 时，管理员的所有页面都先去配置
  const setup = useGetSetup({ query: { staleTime: 30_000, enabled: admin } });
  const { pathname } = useLocation();
  const nav = useNavigate();
  if (setup.data && !setup.data.api_key_configured && pathname !== "/setup") return <Navigate to="/setup" replace />;
  return (
    <div className="flex min-h-dvh flex-col">
      <header className="sticky top-0 z-10 border-b border-zinc-200 bg-white/85 backdrop-blur dark:border-zinc-800 dark:bg-zinc-950/85">
        <div className="mx-auto flex h-12 max-w-7xl items-center gap-3 px-4 sm:gap-6">
          <NavLink to="/" className="flex items-center gap-2 font-semibold">
            <img src="/favicon.svg" alt="" className="size-6" />
            <span className="hidden sm:inline">牛来</span>
          </NavLink>
          {admin && <VersionBadge />}
          <nav className="flex min-w-0 gap-1 overflow-x-auto">
            {NAV.map((n) => (
              <NavLink
                key={n.to}
                to={n.to}
                end={n.end}
                className={({ isActive }) =>
                  clsx(
                    "rounded-md px-2.5 py-1 text-sm whitespace-nowrap transition",
                    isActive ||
                      (n.to === "/strategies" &&
                        (pathname.startsWith("/signals") || pathname.startsWith("/indicators")))
                      ? "bg-zinc-100 font-medium text-zinc-900 dark:bg-zinc-800 dark:text-zinc-100"
                      : "text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100",
                  )
                }
              >
                {n.label}
                {n.to === "/accounts" && <PendingBadge />}
              </NavLink>
            ))}
          </nav>
          <div className="ml-auto flex min-w-0 items-center gap-3">
            <SecuritySearch onPick={(code) => nav(`/securities/${code}`)} width="w-28 sm:w-56" />
            {AUX.filter((a) => a.who === "all" || admin).map((a) => (
              <NavLink
                key={a.to}
                to={a.to}
                title={a.title}
                className={({ isActive }) =>
                  clsx(
                    "hidden text-xs whitespace-nowrap sm:inline",
                    isActive || (a.to === "/sync" && pathname.startsWith("/indices"))
                      ? "text-zinc-900 dark:text-zinc-100"
                      : "text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100",
                  )
                }
              >
                {a.label}
              </NavLink>
            ))}
            {me.data && (
              <div className="border-l border-zinc-200 pl-3 dark:border-zinc-700">
                <UserMenu me={me.data} />
              </div>
            )}
          </div>
        </div>
      </header>
      <main className="mx-auto flex w-full min-w-0 max-w-7xl flex-1 flex-col gap-4 px-4 py-6">
        {admin && <BackupAlert />}
        {me.data?.must_change_password && pathname !== "/me" && (
          <p className="rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
            现在的密码是默认的或管理员设置的，请
            <NavLink to="/me" className="mx-1 underline">
              改成只有你自己知道的密码
            </NavLink>
            。
          </p>
        )}
        <Suspense fallback={<Spinner />}>
          <Outlet />
        </Suspense>
      </main>
      <footer className="mx-auto w-full max-w-7xl px-4 py-4 text-xs text-zinc-400">
        数据来自同花顺金融数据服务，仅供研究，不构成投资建议。K 线图由{" "}
        <a href="https://www.tradingview.com/" target="_blank" rel="noreferrer" className="underline">
          TradingView Lightweight Charts
        </a>{" "}
        绘制。
      </footer>
    </div>
  );
}

/** 「账户」后面的数字：运营建议里还没处理（已执行或不做）的条数，操作日已经过去的不算 */
function PendingBadge() {
  const q = useListAccounts({ query: { staleTime: 60_000 } });
  const today = todayCn();
  const n = (q.data ?? []).reduce(
    (sum, a) => sum + (a.advice && a.advice.next_date >= today ? a.advice.pending : 0),
    0,
  );
  if (n === 0) return null;
  return (
    <span
      className="ml-1 inline-flex min-w-4 items-center justify-center rounded-full bg-up px-1 text-[10px] leading-4 text-white"
      title={`运营建议里还有 ${n} 条没处理`}
    >
      {n}
    </span>
  );
}
