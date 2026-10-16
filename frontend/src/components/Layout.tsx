import { clsx } from "clsx";
import { Navigate, NavLink, Outlet, useLocation, useNavigate } from "react-router";
import { useLogout, useMe } from "../api/generated/auth/auth";
import { useGetSetup } from "../api/generated/setup/setup";
import { useReady } from "../api/generated/system/system";
import { SecuritySearch } from "./SecuritySearch";
import { VersionBadge } from "./VersionBadge";

/** 主导航只放三件事：今天看什么、我的选股方法、我关注的股票。个股从页头搜索进入 */
const NAV = [
  { to: "/", label: "今日", end: true },
  { to: "/strategies", label: "策略" },
  { to: "/watchlist", label: "自选" },
];

/** 数据同步、指数板块、设置：不是每天用的，放在右侧 */
const AUX = [
  { to: "/guide", label: "说明", title: "指标与信号是什么、怎么看" },
  { to: "/sync", label: "数据", title: "数据同步与指数板块" },
  { to: "/setup", label: "设置", title: "数据源设置：API Key" },
];

export function Layout() {
  // 没有登录时 fetcher 收到 401 会转到登录页；登录后才检查 API Key
  const me = useMe({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const logout = useLogout({ mutation: { onSuccess: () => window.location.assign("/login") } });
  // 首次登录引导：还没有配置 API Key 时，所有页面都先去配置
  const setup = useGetSetup({ query: { staleTime: 30_000, enabled: me.isSuccess } });
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
          {me.isSuccess && <VersionBadge />}
          <nav className="flex min-w-0 gap-1 overflow-x-auto">
            {NAV.map((n) => (
              <NavLink
                key={n.to}
                to={n.to}
                end={n.end}
                className={({ isActive }) =>
                  clsx(
                    "rounded-md px-2.5 py-1 text-sm whitespace-nowrap transition",
                    isActive
                      ? "bg-zinc-100 font-medium text-zinc-900 dark:bg-zinc-800 dark:text-zinc-100"
                      : "text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100",
                  )
                }
              >
                {n.label}
              </NavLink>
            ))}
          </nav>
          <div className="ml-auto flex min-w-0 items-center gap-3">
            <SecuritySearch onPick={(code) => nav(`/securities/${code}`)} width="w-28 sm:w-56" />
            {AUX.map((a) => (
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
            <button
              type="button"
              title={me.data ? `已登录：${me.data.username}` : undefined}
              onClick={() => logout.mutate()}
              className="hidden text-xs whitespace-nowrap text-zinc-500 hover:text-zinc-900 sm:inline dark:hover:text-zinc-100"
            >
              退出
            </button>
            <ServerStatus />
          </div>
        </div>
      </header>
      <main className="mx-auto flex w-full min-w-0 max-w-7xl flex-1 flex-col gap-4 px-4 py-6">
        {me.data?.default_credentials && (
          <p className="rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
            正在使用默认账号密码 niulai / niulai。请在部署配置中设置 <code>NIULAI_ADMIN_USER</code> 和{" "}
            <code>NIULAI_ADMIN_PASSWORD</code>，重启后生效。
          </p>
        )}
        <Outlet />
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

function ServerStatus() {
  const ready = useReady({ query: { refetchInterval: 30_000, retry: false } });
  const ok = ready.data?.status === "ready";
  return (
    <span
      className="flex items-center gap-1.5 text-xs text-zinc-500"
      title={ok ? "后端和数据库正常" : "后端或数据库不可用"}
    >
      <span
        className={clsx("size-2 rounded-full", ready.isPending ? "bg-zinc-300" : ok ? "bg-emerald-500" : "bg-red-500")}
      />
      <span className="hidden sm:inline">{ready.isPending ? "连接中" : ok ? "服务正常" : "服务异常"}</span>
    </span>
  );
}
