import { createBrowserRouter } from "react-router";
import { Layout } from "./components/Layout";
import { BacktestPage } from "./pages/BacktestPage";
import { IndexDetailPage } from "./pages/IndexDetailPage";
import { IndicesPage } from "./pages/IndicesPage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { RunDetailPage } from "./pages/RunDetailPage";
import { ScreenerPage } from "./pages/ScreenerPage";
import { SecuritiesPage } from "./pages/SecuritiesPage";
import { SecurityDetailPage } from "./pages/SecurityDetailPage";
import { SetupPage } from "./pages/SetupPage";
import { SignalsPage } from "./pages/SignalsPage";
import { StrategyEditorPage } from "./pages/StrategyEditorPage";
import { StrategyPage } from "./pages/StrategyPage";
import { SyncPage } from "./pages/SyncPage";
import { WatchlistPage } from "./pages/WatchlistPage";

export const router = createBrowserRouter([
  {
    element: <Layout />,
    children: [
      { path: "/", element: <SecuritiesPage /> },
      { path: "/securities/:thscode", element: <SecurityDetailPage /> },
      { path: "/watchlist", element: <WatchlistPage /> },
      { path: "/screener", element: <ScreenerPage /> },
      { path: "/screener/new", element: <StrategyEditorPage /> },
      { path: "/screener/:id", element: <StrategyPage /> },
      { path: "/screener/:id/edit", element: <StrategyEditorPage /> },
      { path: "/signals", element: <SignalsPage /> },
      { path: "/backtest", element: <BacktestPage /> },
      { path: "/indices", element: <IndicesPage /> },
      { path: "/indices/:thscode", element: <IndexDetailPage /> },
      { path: "/setup", element: <SetupPage /> },
      { path: "/sync", element: <SyncPage /> },
      { path: "/sync/runs/:runId", element: <RunDetailPage /> },
      { path: "*", element: <NotFoundPage /> },
    ],
  },
]);
