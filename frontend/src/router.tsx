import { createBrowserRouter, Navigate, useParams, useSearchParams } from "react-router";
import { Layout } from "./components/Layout";
import { IndexDetailPage } from "./pages/IndexDetailPage";
import { IndicesPage } from "./pages/IndicesPage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { RunDetailPage } from "./pages/RunDetailPage";
import { SecurityDetailPage } from "./pages/SecurityDetailPage";
import { SetupPage } from "./pages/SetupPage";
import { StrategiesPage } from "./pages/StrategiesPage";
import { StrategyEditorPage } from "./pages/StrategyEditorPage";
import { StrategyPage } from "./pages/StrategyPage";
import { SyncPage } from "./pages/SyncPage";
import { TodayPage } from "./pages/TodayPage";
import { WatchlistPage } from "./pages/WatchlistPage";

/** 旧的「回测」页：按参数转到个股页或策略页的回测 */
function LegacyBacktest() {
  const [p] = useSearchParams();
  const code = p.get("code");
  const strategy = p.get("strategy");
  if (code) return <Navigate replace to={`/securities/${code}?tab=backtest${strategy ? `&method=${strategy}` : ""}`} />;
  if (strategy) return <Navigate replace to={`/strategies/${strategy}?tab=history`} />;
  return <Navigate replace to="/strategies" />;
}

function LegacyScreener({ edit = false }: { edit?: boolean }) {
  const { id } = useParams();
  return <Navigate replace to={id ? `/strategies/${id}${edit ? "/edit" : ""}` : "/strategies"} />;
}

export const router = createBrowserRouter([
  {
    element: <Layout />,
    children: [
      { path: "/", element: <TodayPage /> },
      { path: "/strategies", element: <StrategiesPage /> },
      { path: "/strategies/new", element: <StrategyEditorPage /> },
      { path: "/strategies/:id", element: <StrategyPage /> },
      { path: "/strategies/:id/edit", element: <StrategyEditorPage /> },
      { path: "/watchlist", element: <WatchlistPage /> },
      { path: "/securities/:thscode", element: <SecurityDetailPage /> },
      { path: "/indices", element: <IndicesPage /> },
      { path: "/indices/:thscode", element: <IndexDetailPage /> },
      { path: "/setup", element: <SetupPage /> },
      { path: "/sync", element: <SyncPage /> },
      { path: "/sync/runs/:runId", element: <RunDetailPage /> },
      // 旧地址
      { path: "/securities", element: <Navigate replace to="/" /> },
      { path: "/signals", element: <Navigate replace to="/strategies" /> },
      { path: "/backtest", element: <LegacyBacktest /> },
      { path: "/screener", element: <LegacyScreener /> },
      { path: "/screener/new", element: <Navigate replace to="/strategies/new" /> },
      { path: "/screener/:id", element: <LegacyScreener /> },
      { path: "/screener/:id/edit", element: <LegacyScreener edit /> },
      { path: "*", element: <NotFoundPage /> },
    ],
  },
]);
