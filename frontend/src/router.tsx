import { createBrowserRouter } from "react-router";
import { Layout } from "./components/Layout";
import { GuidePage } from "./pages/GuidePage";
import { IndexDetailPage } from "./pages/IndexDetailPage";
import { IndicatorDetailPage, IndicatorsPage } from "./pages/IndicatorsPage";
import { IndicesPage } from "./pages/IndicesPage";
import { LoginPage } from "./pages/LoginPage";
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
import { WatchlistPage } from "./pages/WatchlistPage";

export const router = createBrowserRouter([
  { path: "/login", element: <LoginPage /> },
  {
    element: <Layout />,
    children: [
      { path: "/", element: <TodayPage /> },
      { path: "/strategies", element: <StrategiesPage /> },
      { path: "/strategies/new", element: <StrategyEditorPage /> },
      { path: "/strategies/:id", element: <StrategyPage /> },
      { path: "/strategies/:id/edit", element: <StrategyEditorPage /> },
      { path: "/signals", element: <SignalsPage /> },
      { path: "/signals/new", element: <SignalEditorPage /> },
      { path: "/signals/:key", element: <SignalDetailPage /> },
      { path: "/signals/:key/edit", element: <SignalEditorPage /> },
      { path: "/indicators", element: <IndicatorsPage /> },
      { path: "/indicators/:id", element: <IndicatorDetailPage /> },
      { path: "/watchlist", element: <WatchlistPage /> },
      { path: "/securities/:thscode", element: <SecurityDetailPage /> },
      { path: "/guide", element: <GuidePage /> },
      { path: "/indices", element: <IndicesPage /> },
      { path: "/indices/:thscode", element: <IndexDetailPage /> },
      { path: "/setup", element: <SetupPage /> },
      { path: "/sync", element: <SyncPage /> },
      { path: "/sync/runs/:runId", element: <RunDetailPage /> },
      { path: "*", element: <NotFoundPage /> },
    ],
  },
]);
