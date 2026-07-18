import { Route, Routes } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { CliRunPage } from "../pages/CliRunPage";
import { HistoryPage } from "../pages/HistoryPage";
import { HomePage } from "../pages/HomePage";
import { ManualRunPage } from "../pages/ManualRunPage";
import { NotFoundPage } from "../pages/NotFoundPage";
import { ResultPage } from "../pages/ResultPage";

export function AppRoutes() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<HomePage />} />
        <Route path="manual/:target" element={<ManualRunPage />} />
        <Route path="cli/:target" element={<CliRunPage />} />
        <Route path="history" element={<HistoryPage />} />
        <Route path="results/:runId" element={<ResultPage />} />
        <Route path="*" element={<NotFoundPage />} />
      </Route>
    </Routes>
  );
}
