import { type ComponentType, lazy } from "react";
import { createBrowserRouter } from "react-router";
import { AdminOnly } from "./components/AdminOnly";
import { Layout } from "./components/Layout";
import { RouteError } from "./components/RouteError";
import { LoginPage } from "./pages/LoginPage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { TodayPage } from "./pages/TodayPage";

/** 按页面分包：打开哪个页面才下载哪个页面的代码（K 线图等大的依赖只在用到的页面加载） */
function page<M extends Record<string, unknown>>(load: () => Promise<M>, name: keyof M & string) {
  return lazy(async () => ({ default: (await load())[name] as ComponentType }));
}

const AccountNewPage = page(() => import("./pages/AccountNewPage"), "AccountNewPage");
const AccountPage = page(() => import("./pages/AccountPage"), "AccountPage");
const AccountsPage = page(() => import("./pages/AccountsPage"), "AccountsPage");
const GuidePage = page(() => import("./pages/GuidePage"), "GuidePage");
const HotPage = page(() => import("./pages/HotPage"), "HotPage");
const IndexDetailPage = page(() => import("./pages/IndexDetailPage"), "IndexDetailPage");
const IndicatorDetailPage = page(() => import("./pages/IndicatorsPage"), "IndicatorDetailPage");
const IndicatorsPage = page(() => import("./pages/IndicatorsPage"), "IndicatorsPage");
const IndicesPage = page(() => import("./pages/IndicesPage"), "IndicesPage");
const MePage = page(() => import("./pages/MePage"), "MePage");
const RunDetailPage = page(() => import("./pages/RunDetailPage"), "RunDetailPage");
const SecurityDetailPage = page(() => import("./pages/SecurityDetailPage"), "SecurityDetailPage");
const SetupPage = page(() => import("./pages/SetupPage"), "SetupPage");
const SignalEditorPage = page(() => import("./pages/SignalEditorPage"), "SignalEditorPage");
const SignalDetailPage = page(() => import("./pages/SignalsPage"), "SignalDetailPage");
const SignalsPage = page(() => import("./pages/SignalsPage"), "SignalsPage");
const StrategiesPage = page(() => import("./pages/StrategiesPage"), "StrategiesPage");
const StrategyEditorPage = page(() => import("./pages/StrategyEditorPage"), "StrategyEditorPage");
const StrategyPage = page(() => import("./pages/StrategyPage"), "StrategyPage");
const SyncPage = page(() => import("./pages/SyncPage"), "SyncPage");
const UsersPage = page(() => import("./pages/UsersPage"), "UsersPage");
const WatchlistPage = page(() => import("./pages/WatchlistPage"), "WatchlistPage");

export const router = createBrowserRouter([
  { path: "/login", element: <LoginPage /> },
  {
    element: <Layout />,
    children: [
      {
        // 页面出错（包括发布新版本后旧的分包已经不存在）时在页框里显示，页头和导航还能用
        errorElement: <RouteError />,
        children: [
          { path: "/", element: <TodayPage /> },
          { path: "/accounts", element: <AccountsPage /> },
          { path: "/accounts/new", element: <AccountNewPage /> },
          { path: "/accounts/:id", element: <AccountPage /> },
          { path: "/strategies", element: <StrategiesPage /> },
          {
            path: "/strategies/new",
            element: (
              <AdminOnly>
                <StrategyEditorPage />
              </AdminOnly>
            ),
          },
          { path: "/strategies/:id", element: <StrategyPage /> },
          {
            path: "/strategies/:id/edit",
            element: (
              <AdminOnly>
                <StrategyEditorPage />
              </AdminOnly>
            ),
          },
          { path: "/signals", element: <SignalsPage /> },
          {
            path: "/signals/new",
            element: (
              <AdminOnly>
                <SignalEditorPage />
              </AdminOnly>
            ),
          },
          { path: "/signals/:key", element: <SignalDetailPage /> },
          {
            path: "/signals/:key/edit",
            element: (
              <AdminOnly>
                <SignalEditorPage />
              </AdminOnly>
            ),
          },
          { path: "/indicators", element: <IndicatorsPage /> },
          { path: "/indicators/:id", element: <IndicatorDetailPage /> },
          { path: "/watchlist", element: <WatchlistPage /> },
          { path: "/securities/:thscode", element: <SecurityDetailPage /> },
          { path: "/hot", element: <HotPage /> },
          { path: "/guide", element: <GuidePage /> },
          { path: "/indices", element: <IndicesPage /> },
          { path: "/indices/:thscode", element: <IndexDetailPage /> },
          {
            path: "/setup",
            element: (
              <AdminOnly>
                <SetupPage />
              </AdminOnly>
            ),
          },
          {
            path: "/sync",
            element: (
              <AdminOnly>
                <SyncPage />
              </AdminOnly>
            ),
          },
          {
            path: "/sync/runs/:runId",
            element: (
              <AdminOnly>
                <RunDetailPage />
              </AdminOnly>
            ),
          },
          {
            path: "/users",
            element: (
              <AdminOnly>
                <UsersPage />
              </AdminOnly>
            ),
          },
          { path: "/me", element: <MePage /> },
          { path: "*", element: <NotFoundPage /> },
        ],
      },
    ],
  },
]);
