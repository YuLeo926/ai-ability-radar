import { Link, NavLink, Outlet } from "react-router-dom";

function navClassName({ isActive }: { isActive: boolean }): string {
  return isActive ? "nav-link nav-link-active" : "nav-link";
}

export function AppShell() {
  return (
    <div className="app-shell">
      <header className="topbar">
        <Link className="brand" to="/">
          <span aria-hidden="true" className="brand-mark">
            ◉
          </span>
          <span>AI 能力雷达</span>
        </Link>
        <nav aria-label="主导航" className="main-navigation">
          <NavLink className={navClassName} to="/" end>
            开始体检
          </NavLink>
          <NavLink className={navClassName} to="/history">
            历史记录
          </NavLink>
        </nav>
      </header>
      <div className="app-content">
        <Outlet />
      </div>
    </div>
  );
}
