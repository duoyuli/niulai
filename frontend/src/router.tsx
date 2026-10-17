import { createBrowserRouter } from "react-router";
import { AdminOnly } from "./components/AdminOnly";
import { Layout } from "./components/Layout";
import { AccountNewPage } from "./pages/AccountNewPage";
import { AccountPage } from "./pages/AccountPage";
import { AccountsPage } from "./pages/AccountsPage";
import { GuidePage } from "./pages/GuidePage";
import { IndexDetailPage } from "./pages/IndexDetailPage";
import { IndicatorDetailPage, IndicatorsPage } from "./pages/IndicatorsPage";
import { IndicesPage } from "./pages/IndicesPage";
import { LoginPage } from "./pages/LoginPage";
import { MePage } from "./pages/MePage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { RunDetailPage } from "./pages/RunDetailPage";
import { SecurityDetailPage } from "./pages/SecurityDetailPage";
import { SetupPage } from "./pages/SetupPage";
import { SignalEditorPage } from "./pages/SignalEditorPage";
import { SignalDetailPage, SignalsPage } from "./pages/SignalsPage";
import { StrategiesPage } from "./pages/StrategiesPage";
import { StrategyEditorPage } from "./pages/StrategyEditorPage";
import { StrategyPage } from "./pages/StrategyPage";
import { SyncPage } from "./pages/SyncPage";
import { TodayPage } from "./pages/TodayPage";
import { UsersPage } from "./pages/UsersPage";
import { WatchlistPage } from "./pages/WatchlistPage";

export const router = createBrowserRouter([
  { path: "/login", element: <LoginPage /> },
  {
    element: <Layout />,
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
]);
