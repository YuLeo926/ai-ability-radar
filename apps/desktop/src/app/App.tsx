import "../styles/tokens.css";
import "../styles/app.css";
import { BrowserRouter } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type { Backend } from "../api/backend";
import { tauriBackend } from "../api/tauriBackend";
import { I18nProvider } from "../i18n/I18nContext";
import { AppRoutes } from "./routes";

export function App({
  backend = tauriBackend,
}: {
  backend?: Backend;
} = {}) {
  return (
    <I18nProvider>
      <BackendProvider backend={backend}>
        <BrowserRouter>
          <AppRoutes />
        </BrowserRouter>
      </BackendProvider>
    </I18nProvider>
  );
}
