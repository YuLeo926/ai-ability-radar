import { BrowserRouter } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type { Backend } from "../api/backend";
import { tauriBackend } from "../api/tauriBackend";
import { AppRoutes } from "./routes";

export function App({
  backend = tauriBackend,
}: {
  backend?: Backend;
} = {}) {
  return (
    <BackendProvider backend={backend}>
      <BrowserRouter>
        <AppRoutes />
      </BrowserRouter>
    </BackendProvider>
  );
}
