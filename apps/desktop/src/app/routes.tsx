import { Route, Routes } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { HomePage } from "../pages/HomePage";
import {
  CliRunPage,
  HistoryPage,
  ManualRunPage,
  NotFoundPage,
  ResultPage,
} from "../pages/PlaceholderPages";

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
