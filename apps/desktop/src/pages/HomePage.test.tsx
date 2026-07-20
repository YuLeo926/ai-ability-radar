import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { expect, test, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type { Backend, Bootstrap, TargetKind } from "../api/backend";
import { HomePage } from "./HomePage";

const targetOrder: TargetKind[] = [
  "chat_gpt_client",
  "claude_client",
  "codex_cli",
  "claude_code",
];

function readyBootstrap(): Bootstrap {
  return {
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
    targets: targetOrder.map((kind) => ({
      kind,
      installed: true,
      version:
        kind === "codex_cli"
          ? "codex 1.2.3\nforged"
          : kind === "claude_code"
            ? "claude 2.0.0"
            : null,
      authState: kind === "codex_cli" || kind === "claude_code" ? "ready" : "unknown",
      status: "ready",
      source:
        kind === "codex_cli"
          ? "reviewed_npm"
          : kind === "claude_code"
            ? "native_exe"
            : null,
      prerequisites:
        kind === "codex_cli" || kind === "claude_code"
          ? [{ name: "Node.js 22/24 LTS", available: true, version: "v22.22.0" }]
          : [],
    })),
  };
}

function backendFor(load: () => Promise<Bootstrap>): Backend {
  return {
    getBootstrap: load,
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
}

function renderHome(backend: Backend) {
  return render(
    <MemoryRouter>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/" element={<HomePage />} />
          <Route
            path="/manual/:target"
            element={<h1>已进入客户端体检准备页</h1>}
          />
          <Route path="/cli/:target" element={<h1>已进入 CLI 体检准备页</h1>} />
        </Routes>
      </BackendProvider>
    </MemoryRouter>,
  );
}

test("shows four stable targets in separate client and CLI groups", async () => {
  renderHome(backendFor(async () => readyBootstrap()));

  expect(
    await screen.findByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
  const clients = screen.getByRole("region", { name: "聊天客户端" });
  const clis = screen.getByRole("region", { name: "编程 CLI" });

  expect(
    within(clients).getByRole("heading", { name: "ChatGPT 客户端" }),
  ).toBeInTheDocument();
  expect(
    within(clients).getByRole("heading", { name: "Claude 客户端" }),
  ).toBeInTheDocument();
  expect(
    within(clis).getByRole("heading", { name: "Codex CLI" }),
  ).toBeInTheDocument();
  expect(
    within(clis).getByRole("heading", { name: "Claude Code" }),
  ).toBeInTheDocument();
  expect(within(clients).getByText("8 道任务 · 预计 10–15 分钟")).toBeInTheDocument();
  expect(within(clis).getByText("2 道任务 · 预计 30–60 分钟")).toBeInTheDocument();
  expect(
    within(clis).getByText("版本：codex 1.2.3 forged"),
  ).toBeInTheDocument();
  expect(screen.getAllByRole("status")).toHaveLength(4);
  expect(
    screen.getByRole("status", {
      name: "ChatGPT 客户端状态：可开始手动体检",
    }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("status", {
      name: "Claude Code 状态：本机环境检查通过",
    }),
  ).toBeInTheDocument();
  expect(screen.getByText("npm 安装")).toBeInTheDocument();
});

test.each([
  ["not_found", "未检测到受支持入口"],
  ["runtime_missing", "缺少 Node.js 运行时"],
  ["entry_inaccessible", "入口不可访问"],
  ["version_probe_failed", "版本检测失败"],
  ["needs_login", "需要先在终端登录"],
] as const)("renders %s as %s", async (status, copy) => {
  const bootstrap = readyBootstrap();
  bootstrap.targets = bootstrap.targets.map((target) =>
    target.kind === "codex_cli"
      ? {
          ...target,
          installed: status === "needs_login",
          status,
          authState: status === "needs_login" ? "needs_login" : "unknown",
          version: status === "needs_login" ? "codex-cli 0.142.5" : null,
          source: status === "needs_login" ? "reviewed_npm" : null,
        }
      : target,
  );

  renderHome(backendFor(async () => bootstrap));

  expect(
    await screen.findByRole("status", {
      name: `Codex CLI 状态：${copy}`,
    }),
  ).toBeInTheDocument();
});

test("states estimates, subscription cost, privacy, and measurement limits precisely", async () => {
  renderHome(backendFor(async () => readyBootstrap()));

  await screen.findByRole("heading", { name: "选择要体检的 AI" });
  expect(
    screen.getByText(
      "手动客户端体检和自动 CLI 体检都可能消耗你自己的订阅额度。",
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByText("维护者不会承担这些费用，也不会接收你的登录凭据。"),
  ).toBeInTheDocument();
  expect(
    screen.getByText("原始回答和运行日志只保存在本机。"),
  ).toBeInTheDocument();
  expect(
    screen.getByText("体检衡量端到端产品表现，不直接测量底层模型的“智商”。"),
  ).toBeInTheDocument();
  expect(
    screen.getByText("CLI 自动任务使用专用临时任务目录。"),
  ).toBeInTheDocument();
  expect(screen.queryByText(/安全沙箱|隔离的临时/)).not.toBeInTheDocument();
});

test("an unavailable CLI disables only that target", async () => {
  const bootstrap = readyBootstrap();
  bootstrap.targets = bootstrap.targets.map((target) =>
    target.kind === "codex_cli"
      ? { ...target, installed: false, version: null, status: "not_found" }
      : target,
  );
  const user = userEvent.setup();
  renderHome(backendFor(async () => bootstrap));

  const disabled = await screen.findByRole("button", {
    name: "Codex CLI 暂时无法开始",
  });
  expect(disabled).toBeDisabled();
  expect(
    screen.getByRole("status", {
      name: "Codex CLI 状态：未检测到受支持入口",
    }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "开始 Claude Code 自动体检" }),
  ).toBeInTheDocument();

  await user.click(disabled);
  expect(
    screen.getByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
});

test("re-detects CLI availability and explains inherited PATH limits", async () => {
  const first = readyBootstrap();
  first.targets = first.targets.map((target) =>
    target.kind === "codex_cli"
      ? { ...target, installed: false, version: null, status: "not_found" }
      : target,
  );
  const second = readyBootstrap();
  second.targets = second.targets.map((target) =>
    target.kind === "codex_cli"
      ? { ...target, version: "codex-cli 0.142.5" }
      : target,
  );
  const load = vi
    .fn<() => Promise<Bootstrap>>()
    .mockResolvedValueOnce(first)
    .mockResolvedValueOnce(second);
  const user = userEvent.setup();
  renderHome(backendFor(load));

  expect(await screen.findByText("未检测到受支持入口")).toBeInTheDocument();
  expect(
    screen.getByText(
      /已继承 PATH 目录内的变化可立即重新检测.*新增 PATH 目录.*重启应用.*重新检测/,
    ),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "重新检测 CLI" }));

  expect(await screen.findByText("版本：codex-cli 0.142.5")).toBeInTheDocument();
  expect(load).toHaveBeenCalledTimes(2);
});

test("a CLI detection failure takes precedence over a missing Node prerequisite", async () => {
  const bootstrap = readyBootstrap();
  bootstrap.targets = bootstrap.targets.map((target) =>
    target.kind === "codex_cli"
      ? {
          ...target,
          installed: false,
          status: "runtime_missing",
          prerequisites: [
            { name: "Node.js 22/24 LTS", available: false, version: null },
          ],
        }
      : target,
  );
  renderHome(backendFor(async () => bootstrap));

  expect(
    await screen.findByRole("status", {
      name: "Codex CLI 状态：缺少 Node.js 运行时",
    }),
  ).toBeInTheDocument();
});

test("a missing prerequisite disables only that CLI", async () => {
  const bootstrap = readyBootstrap();
  bootstrap.targets = bootstrap.targets.map((target) =>
    target.kind === "codex_cli"
      ? {
          ...target,
          prerequisites: [
            { name: "Node.js 22/24 LTS", available: false, version: null },
          ],
        }
      : target,
  );
  renderHome(backendFor(async () => bootstrap));

  expect(
    await screen.findByRole("status", {
      name: "Codex CLI 状态：缺少 Node.js 22/24 LTS",
    }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "开始 Claude Code 自动体检" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "开始 ChatGPT 客户端手动体检" }),
  ).toBeInTheDocument();
});

test("a login requirement disables only that CLI", async () => {
  const bootstrap = readyBootstrap();
  bootstrap.targets = bootstrap.targets.map((target) =>
    target.kind === "claude_code"
      ? { ...target, authState: "needs_login" }
      : target,
  );
  renderHome(backendFor(async () => bootstrap));

  expect(
    await screen.findByRole("status", {
      name: "Claude Code 状态：需要先在终端登录",
    }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Claude Code 暂时无法开始" }),
  ).toBeDisabled();
  expect(
    screen.getByRole("link", { name: "开始 Codex CLI 自动体检" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("status", {
      name: "ChatGPT 客户端状态：可开始手动体检",
    }),
  ).toBeInTheDocument();
});

test("an available target navigates with a descriptive link name", async () => {
  const user = userEvent.setup();
  renderHome(backendFor(async () => readyBootstrap()));

  await user.click(
    await screen.findByRole("link", {
      name: "开始 ChatGPT 客户端手动体检",
    }),
  );

  expect(
    screen.getByRole("heading", { name: "已进入客户端体检准备页" }),
  ).toBeInTheDocument();
});

test("announces loading and lets the user retry a bootstrap failure", async () => {
  const load = vi
    .fn<() => Promise<Bootstrap>>()
    .mockRejectedValueOnce(new Error("模拟环境检查失败"))
    .mockResolvedValueOnce(readyBootstrap());
  const user = userEvent.setup();
  renderHome(backendFor(load));

  expect(
    screen.getByRole("status", { name: "正在检查本机环境" }),
  ).toBeInTheDocument();
  const alert = await screen.findByRole("alert");
  expect(
    within(alert).getByRole("heading", { name: "无法读取本机环境" }),
  ).toBeInTheDocument();
  expect(within(alert).getByText("模拟环境检查失败")).toBeInTheDocument();

  await user.click(within(alert).getByRole("button", { name: "重新检查" }));

  expect(
    await screen.findByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
  expect(load).toHaveBeenCalledTimes(2);
});

test("a synchronous bridge failure is announced inside the shell", async () => {
  const backend = backendFor(async () => readyBootstrap());
  backend.getBootstrap = () => {
    throw new Error("模拟同步桥接失败");
  };

  renderHome(backend);

  const alert = await screen.findByRole("alert");
  expect(
    within(alert).getByRole("heading", { name: "无法读取本机环境" }),
  ).toBeInTheDocument();
  expect(within(alert).getByText("模拟同步桥接失败")).toBeInTheDocument();
});

test("a stale bootstrap completion cannot replace newer backend state", async () => {
  let resolveFirst!: (value: Bootstrap) => void;
  const first = new Promise<Bootstrap>((resolve) => {
    resolveFirst = resolve;
  });
  const firstBackend = backendFor(() => first);
  const secondBootstrap = readyBootstrap();
  secondBootstrap.clientPack.title = "新的客户端题包";
  const secondBackend = backendFor(async () => secondBootstrap);
  const view = render(
    <MemoryRouter>
      <BackendProvider backend={firstBackend}>
        <HomePage />
      </BackendProvider>
    </MemoryRouter>,
  );

  view.rerender(
    <MemoryRouter>
      <BackendProvider backend={secondBackend}>
        <HomePage />
      </BackendProvider>
    </MemoryRouter>,
  );
  expect(await screen.findByText("新的客户端题包 · v1.0.0")).toBeInTheDocument();

  const stale = readyBootstrap();
  stale.clientPack.title = "过期题包";
  await act(async () => {
    resolveFirst(stale);
    await first;
  });

  expect(screen.getByText("新的客户端题包 · v1.0.0")).toBeInTheDocument();
  expect(screen.queryByText("过期题包 · v1.0.0")).not.toBeInTheDocument();
});
