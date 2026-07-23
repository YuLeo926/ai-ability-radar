import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { expect, test } from "vitest";
import RootApp from "../App";
import { BackendProvider } from "../api/BackendContext";
import type { Backend } from "../api/backend";
import { I18nProvider } from "../i18n/I18nContext";
import { App } from "./App";
import { AppRoutes } from "./routes";

const backend: Backend = {
  getBootstrap: async () => ({
    clientPack: {
      id: "client-quick-v1",
      version: "1.0.0",
      title: "客户端快速体检",
      taskCount: 8,
      estimatedMinutes: "10–15",
    },
    cliPack: {
      id: "cli-quick-v1",
      version: "1.0.0",
      title: "CLI 快速体检",
      taskCount: 2,
      estimatedMinutes: "30–60",
    },
    targets: [],
  }),
  detectClientSelection: async () => ({
    status: "not_running",
    candidates: [],
  }),
  startManualRun: async () => {
    throw new Error("unused fake startManualRun");
  },
  nextManualStep: async () => null,
  submitManualAnswer: async () => {
    throw new Error("unused fake submitManualAnswer");
  },
  startCliRun: async () => {
    throw new Error("unused fake startCliRun");
  },
  resumeManualRun: async () => {
    throw new Error("unused fake resumeManualRun");
  },
  resumeCliRun: async () => {
    throw new Error("unused fake resumeCliRun");
  },
  cancelRun: async () => false,
  interruptManualRun: async () => false,
  listRuns: async () => [],
  getRunDetail: async () => null,
  exportPublicReport: async () => null,
  deleteRawArtifacts: async () => undefined,
  deleteRun: async () => false,
  deleteTargetHistory: async () => 0,
  getDataSettings: async () => ({
    rawRetentionDays: null,
    cleanupPending: false,
  }),
  setRawRetention: async () => 0,
  exportFullBackup: async () => false,
  onRunEvent: async () => () => undefined,
  onRunError: async () => () => undefined,
};

function renderRoute(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <BackendProvider backend={backend}>
        <AppRoutes />
      </BackendProvider>
    </MemoryRouter>,
  );
}

test("the root entry remains the single default application export", () => {
  expect(RootApp).toBe(App);
});

test("the application entry retires the Vite starter stylesheet", () => {
  const retiredStarterStyles = import.meta.glob("../App.css", {
    eager: true,
    import: "default",
    query: "?raw",
  });

  expect(Object.keys(retiredStarterStyles)).toEqual([]);
});

test("the real application root installs the typed i18n provider", () => {
  const root = App({ backend });
  expect(root.type).toBe(I18nProvider);
});

test("main navigation marks the current page and reaches history", async () => {
  const user = userEvent.setup();
  renderRoute("/");

  const start = screen.getByRole("link", { name: "开始体检" });
  expect(start).toHaveAttribute("aria-current", "page");
  expect(
    screen.getByRole("link", { name: "AI 能力雷达" }),
  ).not.toHaveAttribute("aria-current");
  await user.click(screen.getByRole("link", { name: "历史记录" }));

  expect(
    await screen.findByRole("heading", { name: "还没有体检记录" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "历史记录" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("the topbar constrains shared navigation inside its grid wrapper", async () => {
  renderRoute("/");
  await screen.findByRole("heading", { name: "选择要体检的 AI" });

  const topbar = document.querySelector("header.topbar");
  expect(topbar).not.toBeNull();
  const inner = topbar?.querySelector(":scope > .topbar-inner");
  expect(inner).not.toBeNull();
  expect(inner?.children).toHaveLength(3);
  expect(inner?.children[0]).toHaveClass("brand");
  expect(inner?.children[1]).toHaveClass("main-navigation");
  expect(inner?.children[2]).toHaveClass("theme-control");
});

test("history navigation is exact and is not active on unrelated child paths", () => {
  renderRoute("/history/unrelated");

  expect(screen.getByRole("link", { name: "历史记录" })).not.toHaveAttribute(
    "aria-current",
  );
  expect(
    screen.getByRole("heading", { name: "没有找到这个页面" }),
  ).toBeInTheDocument();
});

test("result routes fetch persisted detail and do not expose the technical run identifier", async () => {
  renderRoute("/results/run-42");

  expect(
    await screen.findByRole("heading", { name: "没有找到这次体检" }),
  ).toBeInTheDocument();
  expect(screen.queryByText(/run-42/)).not.toBeInTheDocument();
});

test("manual target routes show the selected client", () => {
  renderRoute("/manual/claude_client");

  expect(
    screen.getByRole("heading", { name: "Claude 客户端快速体检" }),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/一次只处理一道题/),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "开始体检" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("CLI target routes show the selected command-line tool", () => {
  renderRoute("/cli/codex_cli");

  expect(
    screen.getByRole("status", { name: "正在检查 Codex CLI 环境" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "开始体检" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("result routes belong to history navigation", async () => {
  renderRoute("/results/run-42");
  await screen.findByRole("heading", { name: "没有找到这次体检" });
  expect(screen.getByRole("link", { name: "历史记录" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  expect(screen.getByRole("link", { name: "开始体检" })).not.toHaveAttribute(
    "aria-current",
  );
});

test("unknown routes explain the problem and offer a way home", async () => {
  const user = userEvent.setup();
  renderRoute("/missing/place");

  expect(
    screen.getByRole("heading", { name: "没有找到这个页面" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "开始体检" })).not.toHaveAttribute(
    "aria-current",
  );
  expect(screen.getByRole("link", { name: "历史记录" })).not.toHaveAttribute(
    "aria-current",
  );
  await user.click(screen.getByRole("link", { name: "返回开始页" }));

  expect(
    await screen.findByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
});
