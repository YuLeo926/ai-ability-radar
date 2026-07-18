import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { expect, test } from "vitest";
import RootApp from "../App";
import { BackendProvider } from "../api/BackendContext";
import type { Backend } from "../api/backend";
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
  cancelRun: async () => false,
  listRuns: async () => [],
  getRunDetail: async () => null,
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
    screen.getByRole("heading", { name: "历史记录" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "历史记录" })).toHaveAttribute(
    "aria-current",
    "page",
  );
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

test("result routes retain the requested run identifier", () => {
  renderRoute("/results/run-42");

  expect(
    screen.getByRole("heading", { name: "体检结果" }),
  ).toBeInTheDocument();
  expect(screen.getByText("测试编号：run-42")).toBeInTheDocument();
});

test("manual target routes show the selected client", () => {
  renderRoute("/manual/claude_client");

  expect(
    screen.getByRole("heading", { name: "Claude 客户端快速体检" }),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/一次只处理一道题/),
  ).toBeInTheDocument();
});

test("CLI target routes show the selected command-line tool", () => {
  renderRoute("/cli/codex_cli");

  expect(
    screen.getByRole("heading", { name: "Codex CLI 体检" }),
  ).toBeInTheDocument();
  expect(screen.getByText("自动运行流程将在后续任务中接入。")).toBeInTheDocument();
});

test("unknown routes explain the problem and offer a way home", async () => {
  const user = userEvent.setup();
  renderRoute("/missing/place");

  expect(
    screen.getByRole("heading", { name: "没有找到这个页面" }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("link", { name: "返回开始页" }));

  expect(
    await screen.findByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
});
