import {
  act,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryRouter,
  Link,
  RouterProvider,
} from "react-router-dom";
import { afterEach, expect, test, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type {
  Backend,
  Bootstrap,
  RunDetail,
  RunErrorEvent,
  RunEvent,
  RunRecord,
  TargetKind,
} from "../api/backend";
import { CliRunPage } from "./CliRunPage";

const RUN_ID = "2cf59f48-f775-47ad-b595-8be91f593474";

function makeBootstrap(
  kind: Extract<TargetKind, "codex_cli" | "claude_code"> = "codex_cli",
  overrides: Partial<Bootstrap["targets"][number]> = {},
): Bootstrap {
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
      taskCount: 4,
      estimatedMinutes: "42–55",
    },
    targets: [
      {
        kind,
        installed: true,
        version: kind === "codex_cli" ? "codex 1.2.3" : "claude 2.0.0",
        authState: "unknown",
        prerequisites: [
          {
            name: "Node.js 22/24 LTS",
            available: true,
            version: "v22.22.0",
          },
        ],
        ...overrides,
      },
    ],
  };
}

function makeRun(
  kind: Extract<TargetKind, "codex_cli" | "claude_code"> = "codex_cli",
  overrides: Partial<RunRecord> = {},
): RunRecord {
  return {
    id: RUN_ID,
    target: {
      kind,
      reportedModel: "default",
      reasoningEffort: null,
    },
    mode: "quick",
    suiteId: "cli-quick-v1",
    suiteVersion: "1.0.0",
    status: "running",
    startedAt: new Date(Date.now() - 65_000).toISOString(),
    finishedAt: null,
    totalTasks: 4,
    completedTasks: 0,
    environment: {
      osFamily: "Windows",
      osVersion: "11",
      appVersion: "0.2.0",
      cliVersion: kind === "codex_cli" ? "codex 1.2.3" : "claude 2.0.0",
      verifierRuntimeVersion: "v22.22.0",
      suiteId: "cli-quick-v1",
      suiteVersion: "1.0.0",
      suiteContentSha256: "c".repeat(64),
      scoringRuleVersion: "ability-v1",
      resumed: false,
    },
    ...overrides,
  };
}

function detail(run: RunRecord): RunDetail {
  return { run, taskResults: [] };
}

function fakeBackend(overrides: Partial<Backend> = {}): Backend {
  return {
    getBootstrap: vi.fn(async () => makeBootstrap()),
    startManualRun: vi.fn(async () => {
      throw new Error("unused fake startManualRun");
    }),
    nextManualStep: vi.fn(async () => null),
    submitManualAnswer: vi.fn(async () => {
      throw new Error("unused fake submitManualAnswer");
    }),
    startCliRun: vi.fn(async (input) =>
      makeRun(input.target.kind as "codex_cli" | "claude_code"),
    ),
    resumeManualRun: vi.fn(async () => {
      throw new Error("unused fake resumeManualRun");
    }),
    resumeCliRun: vi.fn(async () => makeRun()),
    cancelRun: vi.fn(async () => false),
    listRuns: vi.fn(async () => []),
    getRunDetail: vi.fn(async () => detail(makeRun())),
    exportPublicReport: vi.fn(async () => null),
    deleteRawArtifacts: vi.fn(async () => undefined),
    deleteRun: vi.fn(async () => false),
    deleteTargetHistory: vi.fn(async () => 0),
    getDataSettings: vi.fn(async () => ({
      rawRetentionDays: null,
      cleanupPending: false,
    })),
    setRawRetention: vi.fn(async () => 0),
    exportFullBackup: vi.fn(async () => false),
    onRunEvent: vi.fn(async () => () => undefined),
    onRunError: vi.fn(async () => () => undefined),
    ...overrides,
  };
}

test("resume route keeps preflight and cost acknowledgement before continuing the persisted CLI run", async () => {
  const user = userEvent.setup();
  const preview = makeRun("codex_cli", { status: "interrupted" });
  preview.target.reportedModel = "gpt-5.1-codex";
  preview.target.reasoningEffort = "high";
  const resumed = makeRun("codex_cli", {
    completedTasks: 2,
    environment: { ...makeRun().environment, resumed: true },
  });
  resumed.target = { ...preview.target };
  const resumeCliRun = vi.fn(async () => resumed);
  const startCliRun = vi.fn(async () => makeRun());
  const getRunDetail = vi
    .fn()
    .mockResolvedValueOnce(detail(preview))
    .mockResolvedValue(detail(resumed));
  const backend = fakeBackend({
    getRunDetail,
    resumeCliRun,
    startCliRun,
  });
  renderWizard(backend, `/cli/codex_cli?resume=${RUN_ID}`);

  await screen.findByRole("checkbox");
  expect(
    screen.getByRole("heading", { name: "Codex CLI 快速体检" }),
  ).toBeInTheDocument();
  const continueButton = screen.getByRole("button", {
    name: /继续剩余任务/,
  });
  expect(screen.getByText("gpt-5.1-codex")).toBeInTheDocument();
  expect(screen.getByText("高")).toBeInTheDocument();
  expect(continueButton).toBeDisabled();
  expect(resumeCliRun).not.toHaveBeenCalled();

  await user.click(screen.getByRole("checkbox"));
  expect(continueButton).toBeEnabled();
  await user.dblClick(continueButton);

  expect(resumeCliRun).toHaveBeenCalledTimes(1);
  expect(resumeCliRun).toHaveBeenCalledWith({
    runId: RUN_ID,
    expectedTarget: preview.target,
  });
  expect(startCliRun).not.toHaveBeenCalled();
  expect(await screen.findByText(/2 \/ 4/)).toBeInTheDocument();
});

test("same-family CLI route mismatch is rejected before any resume call", async () => {
  const stored = makeRun("claude_code", { status: "interrupted" });
  const resumeCliRun = vi.fn(async () => stored);
  const backend = fakeBackend({
    getRunDetail: vi.fn(async () => detail(stored)),
    resumeCliRun,
  });

  renderWizard(backend, `/cli/codex_cli?resume=${RUN_ID}`);

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "恢复链接与原体检目标不一致",
  );
  expect(resumeCliRun).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: /继续剩余任务/ }))
    .not.toBeInTheDocument();
});

test.each([
  ["model", { reportedModel: "changed-model" }],
  ["reasoning effort", { reasoningEffort: "low" }],
])(
  "CLI recovery rejects a returned same-kind run with changed %s",
  async (_field, targetChange) => {
    const user = userEvent.setup();
    const preview = makeRun("codex_cli", { status: "interrupted" });
    preview.target.reportedModel = "gpt-5.1-codex";
    preview.target.reasoningEffort = "high";
    const changed = makeRun("codex_cli");
    changed.environment.resumed = true;
    changed.target = { ...preview.target, ...targetChange };
    const backend = fakeBackend({
      getRunDetail: vi.fn(async () => detail(preview)),
      resumeCliRun: vi.fn(async () => changed),
    });

    renderWizard(backend, `/cli/codex_cli?resume=${RUN_ID}`);
    await screen.findByRole("checkbox");
    await user.click(screen.getByRole("checkbox"));
    await user.click(
      screen.getByRole("button", { name: /继续剩余任务/ }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "无法恢复这次 CLI 体检",
    );
    expect(backend.getRunDetail).toHaveBeenCalledTimes(1);
  },
);

test("CLI resume failure stays on the review step and never renders backend details", async () => {
  const user = userEvent.setup();
  const preview = makeRun("codex_cli", { status: "interrupted" });
  const resumeCliRun = vi.fn(async () => {
    throw new Error("C:\\Users\\Alice\\.claude\\credentials.json");
  });
  const backend = fakeBackend({
    getRunDetail: vi.fn(async () => detail(preview)),
    resumeCliRun,
  });
  renderWizard(backend, `/cli/codex_cli?resume=${RUN_ID}`);
  await screen.findByRole("checkbox");
  await user.click(screen.getByRole("checkbox"));
  await user.click(
    screen.getByRole("button", { name: /继续剩余任务/ }),
  );

  expect(await screen.findByRole("alert")).toBeInTheDocument();
  expect(document.body.textContent).not.toContain("Alice");
  expect(document.body.textContent).not.toContain("credentials.json");
});

function renderWizard(
  backend: Backend,
  initialPath = "/cli/codex_cli",
  onResultRender?: () => void,
) {
  function ResultMarker() {
    onResultRender?.();
    return <h1>CLI 结果</h1>;
  }

  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: (
          <main>
            <h1>选择要体检的 AI</h1>
            <Link to="/cli/codex_cli">Codex CLI</Link>
          </main>
        ),
      },
      { path: "/cli/:target", element: <CliRunPage /> },
      { path: "/results/:runId", element: <ResultMarker /> },
    ],
    { initialEntries: [initialPath] },
  );

  const view = render(
    <BackendProvider backend={backend}>
      <RouterProvider router={router} />
    </BackendProvider>,
  );
  return { ...view, router };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

async function acknowledgeAndStart(
  user: ReturnType<typeof userEvent.setup>,
) {
  await user.click(
    await screen.findByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    ),
  );
  await user.click(screen.getByRole("button", { name: /开始 4 个任务/ }));
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

test.each([
  ["codex_cli", "Codex CLI"],
  ["claude_code", "Claude Code"],
] as const)(
  "starts the %s Quick flow with authoritative pack data and normalized input",
  async (kind, label) => {
    const user = userEvent.setup();
    const bootstrap = makeBootstrap(kind);
    const backend = fakeBackend({
      getBootstrap: vi.fn(async () => bootstrap),
      startCliRun: vi.fn(async () => makeRun(kind)),
      getRunDetail: vi.fn(async () => detail(makeRun(kind))),
    });
    renderWizard(backend, `/cli/${kind}`);

    expect(
      await screen.findByText("4 个微型项目 · 预计 42–55 分钟（估计）"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: `${label} 快速体检` }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "登录状态将在启动时复核，当前不代表已经登录。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "运行使用你本机 CLI 的认证和计费配置，可能消耗订阅额度或 API 余额；应用无法判断或保证具体扣费来源。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "维护者不会承担费用、提供共享密钥、接收凭据或检查认证文件。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "应用只在自己的数据目录创建隔离的临时微型项目，不会改写你的真实项目。",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "结果衡量模型、CLI、配置和工具共同形成的端到端表现，不是底层模型的“智商”。",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: /深度/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "开始 4 个任务" })).toBeDisabled();

    const model = screen.getByLabelText("固定模型（可选）");
    await user.type(model, "  gpt-5.4/custom:beta_1  ");
    await user.selectOptions(screen.getByLabelText("推理档位（可选）"), "high");
    await acknowledgeAndStart(user);

    expect(backend.startCliRun).toHaveBeenCalledTimes(1);
    expect(backend.startCliRun).toHaveBeenCalledWith({
      target: {
        kind,
        reportedModel: "gpt-5.4/custom:beta_1",
        reasoningEffort: "high",
      },
      mode: "quick",
    });
    expect(await screen.findByText("0 / 4 已完成")).toBeInTheDocument();
    expect(screen.getByText("第 1 / 4 个微型项目")).toBeInTheDocument();
    expect(screen.getByText("已用时 1 分 5 秒")).toBeInTheDocument();
  },
);

test.each(["chat_gpt_client", "claude_client", "unknown"])(
  "rejects invalid CLI target %s before any backend activity",
  async (target) => {
    const user = userEvent.setup();
    const backend = fakeBackend();
    renderWizard(backend, `/cli/${target}`);

    expect(
      screen.getByRole("heading", { name: "不支持的 CLI 体检" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "返回开始页" }));
    expect(
      screen.getByRole("heading", { name: "选择要体检的 AI" }),
    ).toBeInTheDocument();
    expect(backend.getBootstrap).not.toHaveBeenCalled();
    expect(backend.onRunEvent).not.toHaveBeenCalled();
    expect(backend.onRunError).not.toHaveBeenCalled();
    expect(backend.startCliRun).not.toHaveBeenCalled();
    expect(backend.getRunDetail).not.toHaveBeenCalled();
    expect(backend.cancelRun).not.toHaveBeenCalled();
  },
);

test("shows safe bootstrap loading/failure/retry and ignores stale completion", async () => {
  const first = deferred<Bootstrap>();
  const backend = fakeBackend({
    getBootstrap: vi
      .fn<Backend["getBootstrap"]>()
      .mockImplementationOnce(() => first.promise)
      .mockRejectedValueOnce(new Error("SECRET C:\\Users\\zhouy\\auth"))
      .mockResolvedValueOnce(makeBootstrap("claude_code")),
  });
  const { router } = renderWizard(backend);

  expect(
    screen.getByRole("status", { name: "正在检查 Codex CLI 环境" }),
  ).toBeInTheDocument();
  await act(async () => {
    await router.navigate("/cli/claude_code");
  });
  expect(
    await screen.findByRole("alert", { name: "无法检查 Claude Code 环境" }),
  ).toHaveTextContent("请确认本机环境后重试");
  expect(screen.queryByText(/SECRET|zhouy|auth/)).not.toBeInTheDocument();

  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "重新检查" }));
  expect(
    await screen.findByRole("heading", { name: "Claude Code 快速体检" }),
  ).toBeInTheDocument();

  await act(async () => {
    first.resolve(makeBootstrap("codex_cli", { installed: false }));
    await first.promise;
  });
  expect(
    screen.getByRole("heading", { name: "Claude Code 快速体检" }),
  ).toBeInTheDocument();
  expect(screen.queryByText("未检测到 Codex CLI")).not.toBeInTheDocument();
  expect(backend.getBootstrap).toHaveBeenCalledTimes(3);
});

const blockedAvailabilityCases: Array<
  [
    string,
    Partial<Bootstrap["targets"][number]>,
    string,
  ]
> = [
  [
    "not-installed",
    { installed: false, version: null },
    "未检测到 Codex CLI，暂时无法开始。",
  ],
  [
    "needs-login",
    { authState: "needs_login" as const },
    "需要先在终端完成 Codex CLI 登录。",
  ],
  [
    "missing-prerequisite",
    {
      prerequisites: [
        {
          name: "Node.js 22/24 LTS",
          available: false,
          version: null,
        },
      ],
    },
    "缺少 Node.js 22/24 LTS，暂时无法开始。",
  ],
];

test.each(blockedAvailabilityCases)(
  "blocks start for %s while preserving the selected CLI boundary",
  async (_case, availability, message) => {
    const bootstrap = makeBootstrap("codex_cli", availability);
    bootstrap.targets.push({
      kind: "claude_code",
      installed: true,
      version: "claude 2.0.0",
      authState: "ready",
      prerequisites: [],
    });
    const backend = fakeBackend({
      getBootstrap: vi.fn(async () => bootstrap),
    });
    renderWizard(backend);

    expect(await screen.findByText(message)).toBeInTheDocument();
    const checkbox = screen.getByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    );
    fireEvent.click(checkbox);
    expect(screen.getByRole("button", { name: "开始 4 个任务" })).toBeDisabled();
    expect(backend.startCliRun).not.toHaveBeenCalled();
  },
);

test("mirrors CLI model validation, default normalization, and safe start retry", async () => {
  const user = userEvent.setup();
  const start = vi
    .fn<Backend["startCliRun"]>()
    .mockRejectedValueOnce(new Error("SECRET token C:\\Users\\zhouy"))
    .mockResolvedValueOnce(makeRun());
  const backend = fakeBackend({ startCliRun: start });
  renderWizard(backend);

  const model = await screen.findByLabelText("固定模型（可选）");
  fireEvent.change(model, { target: { value: "default\t" } });
  expect(
    screen.getByRole("alert", { name: "模型名称格式不正确" }),
  ).toBeInTheDocument();
  expect(model).toHaveValue("default\t");

  await user.clear(model);
  await user.type(model, "-bad model");
  expect(
    screen.getByRole("alert", { name: "模型名称格式不正确" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "开始 4 个任务" })).toBeDisabled();

  await user.clear(model);
  await user.click(
    screen.getByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    ),
  );
  await user.click(screen.getByRole("button", { name: "开始 4 个任务" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "无法启动 CLI 体检，请检查安装和登录状态后重试。",
  );
  expect(screen.queryByText(/SECRET|token|zhouy/)).not.toBeInTheDocument();
  expect(model).toHaveValue("");
  expect(
    screen.getByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    ),
  ).toBeChecked();

  await user.click(screen.getByRole("button", { name: "开始 4 个任务" }));
  expect(start).toHaveBeenNthCalledWith(1, {
    target: {
      kind: "codex_cli",
      reportedModel: "default",
      reasoningEffort: null,
    },
    mode: "quick",
  });
  expect(start).toHaveBeenNthCalledWith(2, {
    target: {
      kind: "codex_cli",
      reportedModel: "default",
      reasoningEffort: null,
    },
    mode: "quick",
  });
});

test("suppresses duplicate starts before React rerenders", async () => {
  const pending = deferred<RunRecord>();
  const start = vi.fn<Backend["startCliRun"]>(() => pending.promise);
  const backend = fakeBackend({ startCliRun: start });
  renderWizard(backend);

  const checkbox = await screen.findByLabelText(
    "我了解本次运行可能消耗自己的订阅额度或 API 余额",
  );
  fireEvent.click(checkbox);
  const button = screen.getByRole("button", { name: "开始 4 个任务" });
  fireEvent.click(button);
  fireEvent.click(button);
  await act(async () => {
    await Promise.resolve();
  });
  expect(start).toHaveBeenCalledTimes(1);

  await act(async () => {
    pending.resolve(makeRun());
    await pending.promise;
  });
});

test("listener failures fall back to polling and late unlisteners are cleaned up", async () => {
  const late = deferred<() => void>();
  const lateUnlisten = vi.fn();
  const backend = fakeBackend({
    onRunEvent: vi.fn(() => late.promise),
    onRunError: vi.fn(async () => {
      throw new Error("SECRET listener path C:\\Users\\zhouy");
    }),
  });
  const view = renderWizard(backend);

  expect(
    await screen.findByRole("status", {
      name: "实时更新不可用，运行时将使用定时同步",
    }),
  ).toBeInTheDocument();
  expect(screen.queryByText(/SECRET|listener|zhouy/)).not.toBeInTheDocument();
  view.unmount();
  await act(async () => {
    late.resolve(lateUnlisten);
    await late.promise;
  });
  expect(lateUnlisten).toHaveBeenCalledOnce();
  expect(backend.onRunEvent).toHaveBeenCalledOnce();
  expect(backend.onRunError).toHaveBeenCalledOnce();
});

test("recovers a lost first event through immediate non-overlapping polling", async () => {
  vi.useFakeTimers();
  let listener: ((event: RunEvent) => void) | undefined;
  const secondPoll = deferred<RunDetail | null>();
  const running = makeRun();
  const getDetail = vi
    .fn<Backend["getRunDetail"]>()
    .mockResolvedValueOnce(detail(running))
    .mockImplementationOnce(() => secondPoll.promise)
    .mockResolvedValue(detail({ ...running, completedTasks: 2 }));
  const backend = fakeBackend({
    onRunEvent: vi.fn(async (next) => {
      listener = next;
      return () => undefined;
    }),
    startCliRun: vi.fn(async () => {
      listener?.({
        runId: RUN_ID,
        kind: "task_finished",
        taskId: "internal-secret-task-id",
        completedTasks: 1,
        totalTasks: 4,
      });
      return running;
    }),
    getRunDetail: getDetail,
  });
  renderWizard(backend);
  await act(async () => {
    await Promise.resolve();
  });
  fireEvent.click(
    screen.getByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    ),
  );
  fireEvent.click(screen.getByRole("button", { name: "开始 4 个任务" }));
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  expect(getDetail).toHaveBeenCalledTimes(1);
  expect(screen.queryByText("internal-secret-task-id")).not.toBeInTheDocument();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
  expect(getDetail).toHaveBeenCalledTimes(2);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(6_000);
  });
  expect(getDetail).toHaveBeenCalledTimes(2);

  await act(async () => {
    secondPoll.resolve(detail({ ...running, completedTasks: 1 }));
    await secondPoll.promise;
  });
  expect(screen.getByText("1 / 4 已完成")).toBeInTheDocument();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
  expect(getDetail).toHaveBeenCalledTimes(3);
  expect(screen.getByText("2 / 4 已完成")).toBeInTheDocument();
});

test("announces null/failed synchronization and preserves monotonic progress", async () => {
  vi.useFakeTimers();
  let listener: ((event: RunEvent) => void) | undefined;
  const running = makeRun();
  const getDetail = vi
    .fn<Backend["getRunDetail"]>()
    .mockResolvedValueOnce(null)
    .mockRejectedValueOnce(new Error("SECRET C:\\private\\run.log"))
    .mockResolvedValueOnce(detail({ ...running, completedTasks: 0 }));
  const backend = fakeBackend({
    onRunEvent: vi.fn(async (next) => {
      listener = next;
      return () => undefined;
    }),
    getRunDetail: getDetail,
  });
  renderWizard(backend);
  await act(async () => {
    await Promise.resolve();
  });
  fireEvent.click(
    screen.getByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    ),
  );
  fireEvent.click(screen.getByRole("button", { name: "开始 4 个任务" }));
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  expect(
    screen.getByRole("status", {
      name: "同步暂未取得本地记录，正在自动重试",
    }),
  ).toBeInTheDocument();

  act(() => {
    listener?.({
      runId: RUN_ID,
      kind: "task_finished",
      taskId: "do-not-render-me",
      completedTasks: 2,
      totalTasks: 4,
    });
    listener?.({
      runId: "other-run",
      kind: "task_finished",
      taskId: "other-secret",
      completedTasks: 4,
      totalTasks: 4,
    });
  });
  expect(screen.getByText("2 / 4 已完成")).toBeInTheDocument();
  expect(screen.queryByText(/do-not-render|other-secret/)).not.toBeInTheDocument();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
  expect(
    screen.getByRole("status", {
      name: "进度同步暂时失败，正在自动重试",
    }),
  ).toBeInTheDocument();
  expect(screen.queryByText(/SECRET|private|run\.log/)).not.toBeInTheDocument();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2_000);
  });
  expect(screen.getByText("2 / 4 已完成")).toBeInTheDocument();
});

test("handles a run error generically and navigates once after persisted interruption", async () => {
  let errorListener: ((event: RunErrorEvent) => void) | undefined;
  const terminal = deferred<RunDetail | null>();
  const running = makeRun();
  const getDetail = vi
    .fn<Backend["getRunDetail"]>()
    .mockResolvedValueOnce(detail(running))
    .mockImplementationOnce(() => terminal.promise);
  const resultRender = vi.fn();
  const backend = fakeBackend({
    onRunError: vi.fn(async (next) => {
      errorListener = next;
      return () => undefined;
    }),
    getRunDetail: getDetail,
  });
  const user = userEvent.setup();
  renderWizard(backend, "/cli/codex_cli", resultRender);
  await acknowledgeAndStart(user);
  await waitForPoll(getDetail, 1);

  act(() => {
    errorListener?.({
      runId: RUN_ID,
      message: "SECRET token at C:\\Users\\zhouy\\auth.json",
    });
  });
  expect(
    screen.getByRole("alert", {
      name: "运行可能已中断，正在核对本地记录；这次不会按能力失败计分",
    }),
  ).toBeInTheDocument();
  expect(screen.queryByText(/SECRET|token|zhouy|auth\.json/)).not.toBeInTheDocument();

  await act(async () => {
    terminal.resolve(
      detail({
        ...running,
        status: "interrupted",
        finishedAt: new Date().toISOString(),
      }),
    );
    await terminal.promise;
  });
  expect(
    await screen.findByRole("heading", { name: "CLI 结果" }),
  ).toBeInTheDocument();
  expect(resultRender).toHaveBeenCalledTimes(1);
});

test("navigates exactly once when event and persisted completion race", async () => {
  let listener: ((event: RunEvent) => void) | undefined;
  const terminal = deferred<RunDetail | null>();
  const resultRender = vi.fn();
  const backend = fakeBackend({
    onRunEvent: vi.fn(async (next) => {
      listener = next;
      return () => undefined;
    }),
    getRunDetail: vi.fn(() => terminal.promise),
  });
  const user = userEvent.setup();
  renderWizard(backend, "/cli/codex_cli", resultRender);
  await acknowledgeAndStart(user);

  act(() => {
    listener?.({
      runId: RUN_ID,
      kind: "run_finished",
      taskId: null,
      completedTasks: 4,
      totalTasks: 4,
    });
    listener?.({
      runId: RUN_ID,
      kind: "run_finished",
      taskId: null,
      completedTasks: 4,
      totalTasks: 4,
    });
  });
  expect(
    await screen.findByRole("heading", { name: "CLI 结果" }),
  ).toBeInTheDocument();
  await act(async () => {
    terminal.resolve(
      detail({ ...makeRun(), status: "completed", completedTasks: 4 }),
    );
    await terminal.promise;
  });
  expect(resultRender).toHaveBeenCalledTimes(1);
});

test("resets on a valid target change and ignores stale start/event completion", async () => {
  let oldListener: ((event: RunEvent) => void) | undefined;
  const oldUnlisten = vi.fn();
  const pendingStart = deferred<RunRecord>();
  const bootstrap = vi
    .fn<Backend["getBootstrap"]>()
    .mockResolvedValueOnce(makeBootstrap("codex_cli"))
    .mockResolvedValueOnce(makeBootstrap("claude_code"));
  const backend = fakeBackend({
    getBootstrap: bootstrap,
    onRunEvent: vi.fn(async (next) => {
      oldListener = next;
      return oldUnlisten;
    }),
    startCliRun: vi.fn(() => pendingStart.promise),
  });
  const { router } = renderWizard(backend);
  const user = userEvent.setup();
  await screen.findByText("4 个微型项目 · 预计 42–55 分钟（估计）");
  expect(
    screen.getByRole("heading", { name: "Codex CLI 快速体检" }),
  ).toBeInTheDocument();
  await user.type(screen.getByLabelText("固定模型（可选）"), "gpt-5");
  await user.click(
    screen.getByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    ),
  );
  await user.click(screen.getByRole("button", { name: "开始 4 个任务" }));
  await act(async () => {
    await router.navigate("/cli/claude_code");
  });

  expect(
    await screen.findByRole("heading", { name: "Claude Code 快速体检" }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText("固定模型（可选）")).toHaveValue("");
  expect(
    screen.getByLabelText(
      "我了解本次运行可能消耗自己的订阅额度或 API 余额",
    ),
  ).not.toBeChecked();
  expect(oldUnlisten).toHaveBeenCalledOnce();

  act(() => {
    oldListener?.({
      runId: RUN_ID,
      kind: "run_finished",
      completedTasks: 4,
      totalTasks: 4,
    });
  });
  await act(async () => {
    pendingStart.resolve(makeRun("codex_cli", { status: "completed" }));
    await pendingStart.promise;
  });
  expect(
    screen.getByRole("heading", { name: "Claude Code 快速体检" }),
  ).toBeInTheDocument();
  expect(backend.getRunDetail).not.toHaveBeenCalled();
});

test("requires two-step cancellation and waits for a persisted terminal state", async () => {
  const user = userEvent.setup();
  const running = makeRun();
  const cancel = vi
    .fn<Backend["cancelRun"]>()
    .mockResolvedValueOnce(false)
    .mockRejectedValueOnce(new Error("SECRET C:\\Users\\zhouy"))
    .mockResolvedValueOnce(true);
  const backend = fakeBackend({
    cancelRun: cancel,
    getRunDetail: vi.fn(async () => detail(running)),
  });
  renderWizard(backend);
  await acknowledgeAndStart(user);
  await screen.findByText("0 / 4 已完成");

  await user.click(screen.getByRole("button", { name: "停止运行" }));
  const firstConfirm = screen.getByRole("group", { name: "确认停止运行" });
  expect(firstConfirm).toHaveTextContent(
    "停止请求会结束 CLI 进程树，并把本次记录为已取消或无效，不会算作能力失败。",
  );
  await user.click(within(firstConfirm).getByRole("button", { name: "继续运行" }));
  expect(cancel).not.toHaveBeenCalled();

  await user.click(screen.getByRole("button", { name: "停止运行" }));
  await user.click(screen.getByRole("button", { name: "确认停止" }));
  expect(
    await screen.findByRole("status", {
      name: "没有找到活动的停止登记，正在重新同步",
    }),
  ).toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "再次请求停止" }));
  await user.click(screen.getByRole("button", { name: "确认停止" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "无法请求安全停止，请重试或继续运行。",
  );
  expect(screen.queryByText(/SECRET|zhouy/)).not.toBeInTheDocument();

  const confirm = screen.getByRole("group", { name: "确认停止运行" });
  await user.click(within(confirm).getByRole("button", { name: "确认停止" }));
  expect(
    await screen.findByRole("status", { name: "正在安全停止" }),
  ).toHaveTextContent("等待本地记录确认");
  expect(screen.queryByRole("heading", { name: "CLI 结果" })).not.toBeInTheDocument();
  expect(cancel).toHaveBeenCalledTimes(3);
});

test("duplicate-suppresses a confirmed cancellation and navigates on terminal event", async () => {
  let listener: ((event: RunEvent) => void) | undefined;
  const pendingCancel = deferred<boolean>();
  const cancel = vi.fn<Backend["cancelRun"]>(() => pendingCancel.promise);
  const backend = fakeBackend({
    onRunEvent: vi.fn(async (next) => {
      listener = next;
      return () => undefined;
    }),
    cancelRun: cancel,
  });
  const user = userEvent.setup();
  renderWizard(backend);
  await acknowledgeAndStart(user);
  await screen.findByText("0 / 4 已完成");
  await user.click(screen.getByRole("button", { name: "停止运行" }));
  const confirm = screen.getByRole("button", { name: "确认停止" });
  fireEvent.click(confirm);
  fireEvent.click(confirm);
  await act(async () => {
    await Promise.resolve();
  });
  expect(cancel).toHaveBeenCalledTimes(1);

  await act(async () => {
    pendingCancel.resolve(true);
    await pendingCancel.promise;
  });
  expect(
    screen.getByRole("status", { name: "正在安全停止" }),
  ).toBeInTheDocument();
  act(() => {
    listener?.({
      runId: RUN_ID,
      kind: "run_finished",
      taskId: null,
      completedTasks: 0,
      totalTasks: 4,
    });
  });
  expect(
    await screen.findByRole("heading", { name: "CLI 结果" }),
  ).toBeInTheDocument();
});

async function waitForPoll(
  poll: ReturnType<typeof vi.fn<Backend["getRunDetail"]>>,
  count: number,
) {
  await act(async () => {
    while (poll.mock.calls.length < count) {
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    }
  });
}
