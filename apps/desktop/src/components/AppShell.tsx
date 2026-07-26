import { useEffect, useRef } from "react";
import {
  Link,
  Outlet,
  useLocation,
  useNavigationType,
} from "react-router-dom";
import { useT } from "../i18n/I18nContext";
import { ThemeToggle } from "./ThemeToggle";

function navClassName(isActive: boolean): string {
  return isActive ? "nav-link nav-link-active" : "nav-link";
}

function RouteScrollReset() {
  const { pathname } = useLocation();
  const navigationType = useNavigationType();
  const isInitialLocation = useRef(true);

  useEffect(() => {
    if (isInitialLocation.current) {
      isInitialLocation.current = false;
      return;
    }

    if (navigationType !== "POP") {
      window.scrollTo(0, 0);
    }
  }, [navigationType, pathname]);

  return null;
}

export function AppShell() {
  const t = useT();
  const { pathname } = useLocation();
  const startActive =
    pathname === "/" ||
    pathname.startsWith("/manual/") ||
    pathname.startsWith("/cli/");
  const historyActive =
    pathname === "/history" || pathname.startsWith("/results/");

  return (
    <div className="app-shell">
      <RouteScrollReset />
      <a
        className="skip-link button"
        href="#page-content"
        onClick={() => {
          document.getElementById("page-content")?.focus();
        }}
      >
        {t("skip.main")}
      </a>
      <header className="topbar">
        <div className="topbar-inner">
          <Link className="brand" to="/">
            <span aria-hidden="true" className="brand-mark" />
            <span>{t("app.name")}</span>
          </Link>
          <nav aria-label={t("nav.label")} className="main-navigation">
            <Link
              aria-current={startActive ? "page" : undefined}
              className={navClassName(startActive)}
              to="/"
            >
              {t("nav.start")}
            </Link>
            <Link
              aria-current={historyActive ? "page" : undefined}
              className={navClassName(historyActive)}
              to="/history"
            >
              {t("nav.history")}
            </Link>
          </nav>
          <ThemeToggle />
        </div>
      </header>
      <div className="app-content">
        <Outlet />
      </div>
    </div>
  );
}
