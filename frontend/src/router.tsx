import { createBrowserRouter } from "react-router";
import { Layout } from "./components/Layout";
import { BacktestPage } from "./pages/BacktestPage";
import { IndexDetailPage } from "./pages/IndexDetailPage";
import { IndicesPage } from "./pages/IndicesPage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { RunDetailPage } from "./pages/RunDetailPage";
import { SecuritiesPage } from "./pages/SecuritiesPage";
import { SecurityDetailPage } from "./pages/SecurityDetailPage";
import { SetupPage } from "./pages/SetupPage";
import { SignalsPage } from "./pages/SignalsPage";
import { SyncPage } from "./pages/SyncPage";
import { WatchlistPage } from "./pages/WatchlistPage";

export const router = createBrowserRouter([
  {
    element: <Layout />,
    children: [
      { path: "/", element: <SecuritiesPage /> },
      { path: "/securities/:thscode", element: <SecurityDetailPage /> },
      { path: "/watchlist", element: <WatchlistPage /> },
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
