import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
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
  ClientSelectionDetection,
  ManualStep,
  RunDetail,
  RunRecord,
  TargetKind,
  TaskResult,
} from "../api/backend";
import {
  DEFAULT_MODEL_DISPLAY_CASES,
  INVALID_LEGACY_EFFORT_CASES,
  MANUAL_EFFORT_DISPLAY_CASES,
} from "../test/reasoningEffortCases";
import { ManualRunPage } from "./ManualRunPage";
import { ResultPage } from "./ResultPage";

const RUN_ID = "6c8cce50-bbf3-4bc5-890d-1f3316222a46";
const UNTRUSTED_RUN_ID = "d857ee26-0c55-47a1-a8e9-4077e2884920";
const ANSWER_LIMIT_BYTES = 256 * 1024;
const originalClipboard = Object.getOwnPropertyDescriptor(
  navigator,
  "clipboard",
);

function makeRun(
  kind: TargetKind = "chat_gpt_client",
  totalTasks = 2,
): RunRecord {
  return {
    id: RUN_ID,
    target: {
      kind,
      reportedModel: kind === "claude_client" ? "Claude Sonnet" : "GPT-5",
      reasoningEffort: null,
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
    suiteId: "client-quick-v1",
    suiteVersion: "1.0.0",
    status: "running",
    startedAt: "2026-07-17T00:00:00Z",
    finishedAt: null,
    totalTasks,
    completedTasks: 0,
    environment: {
      osFamily: "Windows",
      osVersion: "11",
      appVersion: "0.2.0",
      suiteId: "client-quick-v1",
      suiteVersion: "1.0.0",
      suiteContentSha256: "b".repeat(64),
      scoringRuleVersion: "ability-v1",
      resumed: false,
    },
  };
}

function makeStep(taskNumber: number, totalTasks = 2): ManualStep {
  return {
    runId: RUN_ID,
    taskId: `task-${taskNumber}`,
    taskNumber,
    totalTasks,
    prompt: `只输出第 ${taskNumber} 题答案`,
  };
}

function makeResult(taskId = "task-1"): TaskResult {
  return {
    runId: RUN_ID,
    taskId,
    category: "instruction_following",
    outcome: "passed",
    score: 100,
    failureKind: null,
    durationMs: 100,
    answerRelPath: null,
  };
}

function fakeBackend(overrides: Partial<Backend> = {}): Backend {
  return {
    getBootstrap: vi.fn(async () => {
      throw new Error("unused fake getBootstrap");
    }),
    detectClientSelection: vi.fn<Backend["detectClientSelection"]>(
      async () => ({
        status: "not_running",
        candidates: [],
      }),
    ),
    startManualRun: vi.fn(async (input) => ({
      ...makeRun(input.target.kind),
      target: input.target,
    })),
    nextManualStep: vi.fn(async () => makeStep(1)),
    submitManualAnswer: vi.fn(async (input) => makeResult(input.taskId)),
    startCliRun: vi.fn(async () => {
      throw new Error("unused fake startCliRun");
    }),
    resumeManualRun: vi.fn(async () => {
      throw new Error("unused fake resumeManualRun");
    }),
    resumeCliRun: vi.fn(async () => {
      throw new Error("unused fake resumeCliRun");
    }),
    cancelRun: vi.fn(async () => false),
    interruptManualRun: vi.fn(async () => false),
    listRuns: vi.fn(async () => []),
    getRunDetail: vi.fn(async () => null),
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

test("resume preview shows the persisted target snapshot before continuing it exactly", async () => {
  const user = userEvent.setup();
  const preview = makeRun("chat_gpt_client");
  preview.status = "interrupted";
  preview.target.reasoningEffort = "high";
  preview.target.modelSource = "windows_accessibility";
  const resumed = makeRun("chat_gpt_client");
  resumed.completedTasks = 1;
  resumed.target = { ...preview.target };
  resumed.environment.resumed = true;
  const resumeManualRun = vi.fn(async () => resumed);
  const nextManualStep = vi.fn(async () => makeStep(2));
  const startManualRun = vi.fn(async () => makeRun());
  const detectClientSelection = vi.fn<Backend["detectClientSelection"]>(
    async () => ({
      status: "detected",
      candidates: [
        {
          model: "GPT-Should-Not-Replace",
          reasoningEffort: "max",
          surface: "chatgpt",
          source: "windows_accessibility",
          confidence: "visible_selector",
        },
      ],
    }),
  );
  const backend = fakeBackend({
    getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
    detectClientSelection,
    resumeManualRun,
    nextManualStep,
    startManualRun,
  });

  renderWizard(backend, `/manual/chat_gpt_client?resume=${RUN_ID}`);

  expect(
    await screen.findByRole("heading", { name: "确认恢复原体检" }),
  ).toBeInTheDocument();
  expect(screen.getByText("GPT-5")).toBeInTheDocument();
  expect(screen.getByText("高")).toBeInTheDocument();
  expect(
    screen.getByText("模型来源：Windows 客户端界面 · 用户已确认"),
  ).toBeInTheDocument();
  expect(resumeManualRun).not.toHaveBeenCalled();
  await user.click(
    screen.getByRole("button", { name: "继续剩余题目" }),
  );
  expect(
    await screen.findByText("只输出第 2 题答案"),
  ).toBeInTheDocument();
  expect(resumeManualRun).toHaveBeenCalledTimes(1);
  expect(resumeManualRun).toHaveBeenCalledWith({
    runId: RUN_ID,
    expectedTarget: preview.target,
  });
  expect(nextManualStep).toHaveBeenCalledWith(RUN_ID);
  expect(startManualRun).not.toHaveBeenCalled();
  expect(detectClientSelection).not.toHaveBeenCalled();
  expect(
    screen.queryByLabelText("当前显示的模型"),
  ).not.toBeInTheDocument();
});

test.each(MANUAL_EFFORT_DISPLAY_CASES)(
  "manual resume displays %s/%s as %s",
  async (kind, effort, expectedLabel) => {
    const preview = makeRun(kind);
    preview.status = "interrupted";
    preview.target.reasoningEffort = effort;
    const backend = fakeBackend({
      getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
    });

    renderWizard(backend, `/manual/${kind}?resume=${RUN_ID}`);

    await screen.findByRole("heading", { name: "确认恢复原体检" });
    const effortTerm = screen.getByText("原推理档位");
    expect(effortTerm.parentElement).toHaveTextContent(expectedLabel);
  },
);

test.each(
  DEFAULT_MODEL_DISPLAY_CASES.filter(
    ([kind]) => kind === "chat_gpt_client" || kind === "claude_client",
  ),
)(
  "manual resume treats default as a literal model for %s",
  async (kind, _targetLabel, expectedLabel) => {
    const preview = makeRun(kind);
    preview.status = "interrupted";
    preview.target.reportedModel = "default";
    const backend = fakeBackend({
      getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
    });

    renderWizard(backend, `/manual/${kind}?resume=${RUN_ID}`);

    await screen.findByRole("heading", { name: "确认恢复原体检" });
    const modelTerm = screen.getByText("原模型");
    expect(modelTerm.parentElement).toHaveTextContent(expectedLabel);
  },
);

test.each(INVALID_LEGACY_EFFORT_CASES)(
  "manual resume safely hides legacy effort containing %s",
  async (_name, effort) => {
    const preview = makeRun("chat_gpt_client");
    preview.status = "interrupted";
    preview.target.reasoningEffort = effort;
    const backend = fakeBackend({
      getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
    });

    renderWizard(
      backend,
      `/manual/chat_gpt_client?resume=${RUN_ID}`,
    );

    await screen.findByRole("heading", { name: "确认恢复原体检" });
    const effortTerm = screen.getByText("原推理档位");
    expect(effortTerm.parentElement).toHaveTextContent("推理档位不可显示");
    expect(effortTerm.parentElement?.textContent).not.toContain(effort);
  },
);

test("a resumed run returned after unmount is interrupted exactly without reading a step", async () => {
  const user = userEvent.setup();
  const preview = makeRun("chat_gpt_client");
  preview.status = "interrupted";
  const resumed = makeRun("chat_gpt_client");
  resumed.environment.resumed = true;
  const resumeDeferred = deferred<RunRecord>();
  const resumeManualRun = vi.fn<Backend["resumeManualRun"]>(
    () => resumeDeferred.promise,
  );
  const interruptManualRun = vi.fn(async () => true);
  const nextManualStep = vi.fn(async () => makeStep(1));
  const backend = fakeBackend({
    getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
    resumeManualRun,
    interruptManualRun,
    nextManualStep,
  });
  const { router } = renderWizard(
    backend,
    `/manual/chat_gpt_client?resume=${RUN_ID}`,
  );

  await user.click(
    await screen.findByRole("button", { name: "继续剩余题目" }),
  );
  expect(resumeManualRun).toHaveBeenCalledTimes(1);
  await act(async () => {
    await router.navigate("/");
  });
  await act(async () => {
    resumeDeferred.resolve(resumed);
    await resumeDeferred.promise;
  });

  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
  expect(interruptManualRun).toHaveBeenCalledWith(RUN_ID);
  expect(backend.cancelRun).not.toHaveBeenCalled();
  expect(nextManualStep).not.toHaveBeenCalled();
});

test("same-family route mismatch is rejected from the preview without a resume call", async () => {
  const stored = makeRun("claude_client");
  stored.status = "interrupted";
  const resumeManualRun = vi.fn(async () => stored);
  const backend = fakeBackend({
    getRunDetail: vi.fn(async () => ({ run: stored, taskResults: [] })),
    resumeManualRun,
  });

  renderWizard(backend, `/manual/chat_gpt_client?resume=${RUN_ID}`);

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "恢复链接与原体检目标不一致",
  );
  expect(resumeManualRun).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", { name: "继续剩余题目" }))
    .not.toBeInTheDocument();
});

test.each([
  [
    "model",
    (preview: RunRecord) => ({
      ...makeRun("chat_gpt_client"),
      environment: { ...makeRun().environment, resumed: true },
      target: { ...preview.target, reportedModel: "changed-model" },
    }),
  ],
  [
    "reasoning effort",
    (preview: RunRecord) => ({
      ...makeRun("chat_gpt_client"),
      environment: { ...makeRun().environment, resumed: true },
      target: { ...preview.target, reasoningEffort: "low" as const },
    }),
  ],
  [
    "model source",
    (preview: RunRecord) => ({
      ...makeRun("chat_gpt_client"),
      environment: { ...makeRun().environment, resumed: true },
      target: { ...preview.target, modelSource: "windows_accessibility" },
    }),
  ],
  [
    "model verification",
    (preview: RunRecord) => ({
      ...makeRun("chat_gpt_client"),
      environment: { ...makeRun().environment, resumed: true },
      target: { ...preview.target, modelVerification: "unverified" },
    }),
  ],
  [
    "status",
    (preview: RunRecord) => ({
      ...makeRun("chat_gpt_client"),
      status: "interrupted" as const,
      environment: { ...makeRun().environment, resumed: true },
      target: preview.target,
    }),
  ],
  [
    "id",
    (preview: RunRecord) => ({
      ...makeRun("chat_gpt_client"),
      id: UNTRUSTED_RUN_ID,
      environment: { ...makeRun().environment, resumed: true },
      target: preview.target,
    }),
  ],
  [
    "unsafe response",
    () => ({ id: UNTRUSTED_RUN_ID, status: "running" }) as RunRecord,
  ],
] satisfies Array<[string, (preview: RunRecord) => RunRecord]>)(
  "manual recovery rejects a returned run with changed or invalid %s and interrupts only the known input run",
  async (_field, response) => {
    const user = userEvent.setup();
    const preview = makeRun("chat_gpt_client");
    preview.status = "interrupted";
    preview.target.reasoningEffort = "high";
    const interruptManualRun = vi.fn(async () => true);
    const backend = fakeBackend({
      getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
      resumeManualRun: vi.fn(async () => response(preview)),
      interruptManualRun,
    });

    renderWizard(backend, `/manual/chat_gpt_client?resume=${RUN_ID}`);
    await user.click(
      await screen.findByRole("button", { name: "继续剩余题目" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "原体检配置或本地检查点已经变化",
    );
    await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
    expect(interruptManualRun).toHaveBeenCalledWith(RUN_ID);
    expect(interruptManualRun).not.toHaveBeenCalledWith(UNTRUSTED_RUN_ID);
    expect(backend.cancelRun).not.toHaveBeenCalled();
    expect(backend.nextManualStep).not.toHaveBeenCalled();
  },
);

test("resume failure never exposes backend paths and does not create a replacement run", async () => {
  const preview = makeRun("chat_gpt_client");
  preview.status = "interrupted";
  const resumeManualRun = vi.fn(async () => {
    throw new Error("C:\\Users\\Alice\\.codex\\secret");
  });
  const startManualRun = vi.fn(async () => makeRun());
  const backend = fakeBackend({
    getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
    resumeManualRun,
    startManualRun,
  });

  renderWizard(backend, `/manual/chat_gpt_client?resume=${RUN_ID}`);
  await userEvent.click(
    await screen.findByRole("button", { name: "继续剩余题目" }),
  );

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "无法恢复这次体检",
  );
  expect(document.body.textContent).not.toContain("Alice");
  expect(startManualRun).not.toHaveBeenCalled();
  await waitFor(() =>
    expect(backend.interruptManualRun).toHaveBeenCalledTimes(1),
  );
  expect(backend.interruptManualRun).toHaveBeenCalledWith(RUN_ID);
  expect(backend.cancelRun).not.toHaveBeenCalled();
  expect(backend.nextManualStep).not.toHaveBeenCalled();
});

function renderWizard(
  backend: Backend,
  initialPath = "/manual/chat_gpt_client",
) {
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: (
          <main>
            <h1>选择要体检的 AI</h1>
            <Link to="/manual/chat_gpt_client">ChatGPT</Link>
          </main>
        ),
      },
      { path: "/manual/:target", element: <ManualRunPage /> },
      { path: "/results/:runId", element: <ResultPage /> },
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

async function completeSetup(
  user: ReturnType<typeof userEvent.setup>,
  model = "GPT-5",
) {
  await user.type(screen.getByLabelText("当前显示的模型"), model);
  await user.click(screen.getByLabelText("我会为每道题新建空白对话"));
}

function setClipboard(value: unknown) {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value,
  });
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

afterEach(() => {
  vi.restoreAllMocks();
  if (originalClipboard) {
    Object.defineProperty(navigator, "clipboard", originalClipboard);
  } else {
    Reflect.deleteProperty(navigator, "clipboard");
  }
});

test("setup mode exposes the precision hero, panel, model field, and start action", async () => {
  renderWizard(fakeBackend());

  const heading = await screen.findByRole("heading", {
    name: "ChatGPT 客户端快速体检",
  });
  const setupRoot = screen.getByRole("main");
  const hero = screen.queryByTestId("manual-setup-hero");
  expect.soft(hero, "manual setup hero contract").not.toBeNull();
  if (hero) {
    expect.soft(hero).toHaveClass("manual-setup-hero");
    expect.soft(hero).toContainElement(heading);
  }
  const panel = screen.queryByTestId("manual-setup-panel");
  expect.soft(panel, "manual setup panel contract").not.toBeNull();
  if (panel) {
    expect.soft(panel).toHaveClass("manual-setup-panel");
  }
  if (hero && panel) {
    expect.soft(hero.parentElement).toBe(setupRoot);
    expect.soft(panel.parentElement).toBe(setupRoot);
    expect.soft(hero).not.toContainElement(panel);
    expect.soft(panel).not.toContainElement(hero);
    const childOrder = Array.from(setupRoot.children);
    expect
      .soft(childOrder.indexOf(hero))
      .toBeLessThan(childOrder.indexOf(panel));
  }
  expect(screen.getByLabelText("当前显示的模型")).toHaveAttribute(
    "autocomplete",
    "off",
  );
  expect(
    screen.getByRole("button", { name: "开始快速体检" }),
  ).toBeInTheDocument();
});

test.each([
  ["chat_gpt_client", "ChatGPT 客户端", "GPT-5"],
  ["claude_client", "Claude 客户端", "Claude Sonnet"],
] as const)(
  "starts the %s Quick flow with normalized safe input",
  async (kind, label, model) => {
    const user = userEvent.setup();
    const backend = fakeBackend();
    renderWizard(backend, `/manual/${kind}`);

    expect(
      screen.getByRole("heading", { name: `${label}快速体检` }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "开始快速体检" })).toBeDisabled();
    expect(screen.queryByRole("option", { name: /深度/ })).not.toBeInTheDocument();
    expect(
      screen.getByText("客户端使用可能消耗你自己的订阅额度。"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("维护者不会承担费用，也不会接收你的登录凭据。"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "这里测量端到端客户端表现，不是底层模型的“智商”。",
      ),
    ).toBeInTheDocument();

    await completeSetup(user, `  ${model}  `);
    await user.selectOptions(
      screen.getByLabelText("推理档位（没有显示可留空）"),
      "high",
    );
    await user.click(
      screen.getByRole("button", { name: "开始快速体检" }),
    );

    expect(await screen.findByText("只输出第 1 题答案")).toBeInTheDocument();
    expect(backend.startManualRun).toHaveBeenCalledTimes(1);
    expect(backend.startManualRun).toHaveBeenCalledWith({
      target: {
        kind,
        reportedModel: model,
        reasoningEffort: "high",
        modelSource: "manual",
        modelVerification: "user_confirmed",
      },
      mode: "quick",
    });
  },
);

test("automatically applies a complete detected selection with Windows provenance", async () => {
  const user = userEvent.setup();
  const backend = fakeBackend({
    detectClientSelection: vi.fn<Backend["detectClientSelection"]>(
      async () => ({
        status: "detected",
        candidates: [
          {
            model: "GPT-5.6",
            reasoningEffort: "max",
            surface: "codex_desktop",
            source: "windows_accessibility",
            confidence: "visible_selector",
          },
        ],
      }),
    ),
  });
  renderWizard(backend);

  expect(await screen.findByLabelText("当前显示的模型")).toHaveValue(
    "GPT-5.6",
  );
  expect(
    screen.getByLabelText("推理档位（没有显示可留空）"),
  ).toHaveValue("max");
  await user.click(screen.getByLabelText("我会为每道题新建空白对话"));
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));

  await screen.findByText("只输出第 1 题答案");
  expect(backend.startManualRun).toHaveBeenCalledWith({
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-5.6",
      reasoningEffort: "max",
      modelSource: "windows_accessibility",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  });
});

test("an effort-only result preserves the manual model and manual provenance", async () => {
  const pending = deferred<ClientSelectionDetection>();
  const backend = fakeBackend({
    detectClientSelection: vi.fn(() => pending.promise),
  });
  const user = userEvent.setup();
  renderWizard(backend);

  await user.type(screen.getByLabelText("当前显示的模型"), "Manual GPT");
  await act(async () => {
    pending.resolve({
      status: "detected",
      candidates: [
        {
          model: null,
          reasoningEffort: "high",
          surface: "chatgpt",
          source: "windows_accessibility",
          confidence: "visible_selector",
        },
      ],
    });
    await pending.promise;
  });
  expect(screen.getByLabelText("当前显示的模型")).toHaveValue("Manual GPT");
  await user.click(
    await screen.findByRole("button", { name: "应用识别结果" }),
  );
  expect(screen.getByLabelText("当前显示的模型")).toHaveValue("Manual GPT");
  expect(
    screen.getByLabelText("推理档位（没有显示可留空）"),
  ).toHaveValue("high");

  await user.click(screen.getByLabelText("我会为每道题新建空白对话"));
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");
  expect(backend.startManualRun).toHaveBeenCalledWith({
    target: {
      kind: "chat_gpt_client",
      reportedModel: "Manual GPT",
      reasoningEffort: "high",
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  });
});

test("a model-only result preserves the user's effort and records model provenance", async () => {
  const pending = deferred<ClientSelectionDetection>();
  const backend = fakeBackend({
    detectClientSelection: vi.fn(() => pending.promise),
  });
  const user = userEvent.setup();
  renderWizard(backend);

  await user.selectOptions(
    screen.getByLabelText("推理档位（没有显示可留空）"),
    "high",
  );
  await act(async () => {
    pending.resolve({
      status: "detected",
      candidates: [
        {
          model: "GPT-5.6",
          reasoningEffort: null,
          surface: "codex_desktop",
          source: "windows_accessibility",
          confidence: "best_effort",
        },
      ],
    });
    await pending.promise;
  });
  await user.click(
    await screen.findByRole("button", { name: "应用识别结果" }),
  );
  expect(screen.getByLabelText("当前显示的模型")).toHaveValue("GPT-5.6");
  expect(
    screen.getByLabelText("推理档位（没有显示可留空）"),
  ).toHaveValue("high");

  await user.click(screen.getByLabelText("我会为每道题新建空白对话"));
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");
  expect(backend.startManualRun).toHaveBeenCalledWith({
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-5.6",
      reasoningEffort: "high",
      modelSource: "windows_accessibility",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  });
});

test.each([
  ["model", "GPT-Manual", "max"],
  ["reasoning effort", "GPT-5.6", "high"],
] as const)(
  "editing %s after automatic apply marks it edited and returns to manual provenance",
  async (field, expectedModel, expectedEffort) => {
    const backend = fakeBackend({
      detectClientSelection: vi.fn<Backend["detectClientSelection"]>(
        async () => ({
          status: "detected",
          candidates: [
            {
              model: "GPT-5.6",
              reasoningEffort: "max",
              surface: "codex_desktop",
              source: "windows_accessibility",
              confidence: "visible_selector",
            },
          ],
        }),
      ),
    });
    const user = userEvent.setup();
    renderWizard(backend);
    const model = await screen.findByLabelText("当前显示的模型");
    expect(model).toHaveValue("GPT-5.6");

    if (field === "model") {
      await user.clear(model);
      await user.type(model, expectedModel);
    } else {
      await user.selectOptions(
        screen.getByLabelText("推理档位（没有显示可留空）"),
        expectedEffort,
      );
    }
    expect(
      screen.getByText("用户已修改，请确认当前填写值"),
    ).toBeInTheDocument();
    await user.click(screen.getByLabelText("我会为每道题新建空白对话"));
    await user.click(
      screen.getByRole("button", { name: "开始快速体检" }),
    );
    await screen.findByText("只输出第 1 题答案");

    expect(backend.startManualRun).toHaveBeenCalledWith({
      target: {
        kind: "chat_gpt_client",
        reportedModel: expectedModel,
        reasoningEffort: expectedEffort,
        modelSource: "manual",
        modelVerification: "user_confirmed",
      },
      mode: "quick",
    });
  },
);

test("detection failure leaves manual fields usable and start behavior unchanged", async () => {
  const backend = fakeBackend({
    detectClientSelection: vi.fn(async () => {
      throw new Error("模拟识别失败");
    }),
  });
  const user = userEvent.setup();
  renderWizard(backend);

  expect(await screen.findByRole("status")).toHaveTextContent("可手动填写");
  await completeSetup(user, "Manual Model");
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");
  expect(backend.startManualRun).toHaveBeenCalledWith({
    target: {
      kind: "chat_gpt_client",
      reportedModel: "Manual Model",
      reasoningEffort: null,
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  });
});

test.each([
  ["chat_gpt_client", "ChatGPT 客户端"],
  ["claude_client", "Claude 客户端"],
] as const)(
  "manual setup preserves default as a literal model for %s",
  async (kind, label) => {
    const user = userEvent.setup();
    const backend = fakeBackend();
    renderWizard(backend, `/manual/${kind}`);

    expect(
      screen.getByRole("heading", { name: `${label}快速体检` }),
    ).toBeInTheDocument();
    await completeSetup(user, "default");
    await user.click(
      screen.getByRole("button", { name: "开始快速体检" }),
    );

    expect(backend.startManualRun).toHaveBeenCalledWith({
      target: {
        kind,
        reportedModel: "default",
        reasoningEffort: null,
        modelSource: "manual",
        modelVerification: "user_confirmed",
      },
      mode: "quick",
    });
  },
);

test("sends ChatGPT xhigh and preserves a Claude custom effort", async () => {
  const user = userEvent.setup();
  const chatBackend = fakeBackend();
  const chat = renderWizard(chatBackend, "/manual/chat_gpt_client");
  expect(screen.getByRole("option", { name: "极高" })).toHaveValue("xhigh");
  expect(screen.getByRole("option", { name: "最高" })).toHaveValue("max");
  await completeSetup(user);
  await user.selectOptions(
    screen.getByLabelText("推理档位（没有显示可留空）"),
    "xhigh",
  );
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  expect(chatBackend.startManualRun).toHaveBeenCalledWith({
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-5",
      reasoningEffort: "xhigh",
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  });
  chat.unmount();

  const claudeBackend = fakeBackend();
  renderWizard(claudeBackend, "/manual/claude_client");
  await completeSetup(user, "Claude Sonnet");
  await user.selectOptions(
    screen.getByLabelText("推理档位（没有显示可留空）"),
    "__custom__",
  );
  await user.type(screen.getByLabelText("按界面原样填写"), "扩展思考");
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  expect(claudeBackend.startManualRun).toHaveBeenCalledWith({
    target: {
      kind: "claude_client",
      reportedModel: "Claude Sonnet",
      reasoningEffort: "扩展思考",
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  });
});

test.each(["codex_cli", "claude_code", "unknown"])(
  "rejects invalid manual target %s before any backend call",
  async (target) => {
    const user = userEvent.setup();
    const backend = fakeBackend();
    renderWizard(backend, `/manual/${target}`);

    expect(
      screen.getByRole("heading", { name: "不支持的客户端体检" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("这个地址不是 ChatGPT 或 Claude 客户端体检。"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("link", { name: "返回开始页" }));
    expect(
      screen.getByRole("heading", { name: "选择要体检的 AI" }),
    ).toBeInTheDocument();
    expect(backend.startManualRun).not.toHaveBeenCalled();
  },
);

test("validates the model locally and preserves setup after a start failure", async () => {
  const user = userEvent.setup();
  const backend = fakeBackend({
    startManualRun: vi.fn(async () => {
      throw new Error("模拟创建失败");
    }),
  });
  renderWizard(backend);

  const modelInput = screen.getByLabelText("当前显示的模型");
  fireEvent.change(modelInput, { target: { value: "GPT-5\u0001" } });
  expect(
    screen.getByRole("alert", {
      name: "模型名称不能包含控制字符",
    }),
  ).toBeInTheDocument();
  await user.clear(modelInput);
  await user.type(modelInput, "a".repeat(121));
  expect(modelInput).toHaveAttribute("aria-invalid", "true");
  expect(
    screen.getByRole("alert", {
      name: "模型名称必须是 1–120 个可见字符",
    }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "开始快速体检" })).toBeDisabled();

  await user.clear(modelInput);
  await user.type(modelInput, " GPT-5 ");
  await user.click(screen.getByLabelText("我会为每道题新建空白对话"));
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("模拟创建失败");
  expect(modelInput).toHaveValue(" GPT-5 ");
  expect(screen.getByLabelText("我会为每道题新建空白对话")).toBeChecked();
  expect(screen.getByRole("button", { name: "开始快速体检" })).toBeEnabled();
});

test.each([
  ["zero-width space", "\u200B"],
  ["right-to-left override", "\u202E"],
  ["word joiner", "\u2060"],
  ["invisible-only text", "\u200B\u2060"],
  ["mixed visible and invisible text", "GPT\u200B-5"],
] as const)("rejects %s in a reported model before start", (_, value) => {
  const backend = fakeBackend();
  renderWizard(backend);

  const modelInput = screen.getByLabelText("当前显示的模型");
  fireEvent.change(modelInput, { target: { value } });

  expect(modelInput).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("button", { name: "开始快速体检" })).toBeDisabled();
  expect(backend.startManualRun).not.toHaveBeenCalled();
});

test.each([
  ["valid Unicode", "模型-α"],
  ["exactly 120 visible characters", "模".repeat(120)],
] as const)("accepts a %s reported model", (_, value) => {
  const backend = fakeBackend();
  renderWizard(backend);

  const modelInput = screen.getByLabelText("当前显示的模型");
  fireEvent.change(modelInput, { target: { value } });

  expect(modelInput).not.toHaveAttribute("aria-invalid", "true");
});

test("retains a created run and retries its first step without starting again", async () => {
  const user = userEvent.setup();
  const backend = fakeBackend({
    nextManualStep: vi
      .fn<Backend["nextManualStep"]>()
      .mockRejectedValueOnce(new Error("模拟读取第一题失败"))
      .mockResolvedValueOnce(makeStep(1)),
  });
  renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));

  expect(
    await screen.findByRole("heading", { name: "体检已经创建" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("alert")).toHaveTextContent("模拟读取第一题失败");
  expect(
    screen.queryByRole("button", { name: "开始快速体检" }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "重试读取第一题" }));

  expect(await screen.findByText("只输出第 1 题答案")).toBeInTheDocument();
  expect(backend.startManualRun).toHaveBeenCalledTimes(1);
  expect(backend.nextManualStep).toHaveBeenCalledTimes(2);
  expect(backend.nextManualStep).toHaveBeenNthCalledWith(1, RUN_ID);
  expect(backend.nextManualStep).toHaveBeenNthCalledWith(2, RUN_ID);
});

test("uses only best-effort Web Clipboard and explains manual fallback", async () => {
  const user = userEvent.setup();
  const writeText = vi
    .fn<(text: string) => Promise<void>>()
    .mockResolvedValueOnce()
    .mockRejectedValueOnce(new Error("permission denied"));
  setClipboard({ writeText });
  const backend = fakeBackend();
  renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));

  await user.click(await screen.findByRole("button", { name: "复制题目" }));
  expect(writeText).toHaveBeenCalledWith("只输出第 1 题答案");
  expect(screen.getByRole("status")).toHaveTextContent(
    "题目已复制，请粘贴到新的空白对话。",
  );

  await user.click(screen.getByRole("button", { name: "再次复制题目" }));
  expect(screen.getByRole("status")).toHaveTextContent(
    "自动复制不可用，请选中题目文字后手动复制。",
  );
  expect(screen.getByLabelText("当前题目，可选中后手动复制")).toHaveTextContent(
    "只输出第 1 题答案",
  );

  setClipboard(undefined);
  await user.click(screen.getByRole("button", { name: "复制题目" }));
  expect(screen.getByRole("status")).toHaveTextContent(
    "自动复制不可用，请选中题目文字后手动复制。",
  );
});

test("preserves the raw answer after submit failure and enforces UTF-8 bytes", async () => {
  const user = userEvent.setup();
  const submit = vi
    .fn<Backend["submitManualAnswer"]>()
    .mockRejectedValueOnce(new Error("模拟保存失败"))
    .mockResolvedValueOnce(makeResult());
  const backend = fakeBackend({ submitManualAnswer: submit });
  renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));

  const textarea = await screen.findByLabelText("粘贴 AI 的完整回答");
  const exactLimit = "😀".repeat(ANSWER_LIMIT_BYTES / 4);
  fireEvent.change(textarea, { target: { value: exactLimit } });
  expect(
    screen.getByText(`${ANSWER_LIMIT_BYTES} / ${ANSWER_LIMIT_BYTES} 字节`),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  ).toBeEnabled();

  const overLimit = `${exactLimit}😀`;
  fireEvent.change(textarea, { target: { value: overLimit } });
  expect(
    screen.getByText(
      `${ANSWER_LIMIT_BYTES + 4} / ${ANSWER_LIMIT_BYTES} 字节`,
    ),
  ).toBeInTheDocument();
  expect(textarea).toHaveValue(overLimit);
  expect(textarea).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("alert")).toHaveTextContent(
    "回答超过 256 KiB，请删减后再提交。",
  );
  expect(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  ).toBeDisabled();
  expect(submit).not.toHaveBeenCalled();

  const rawAnswer = "  完整回答，不应修剪  ";
  fireEvent.change(textarea, { target: { value: rawAnswer } });
  await user.click(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("模拟保存失败");
  expect(screen.getByLabelText("粘贴 AI 的完整回答")).toHaveValue(rawAnswer);

  await user.click(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  );
  expect(submit).toHaveBeenNthCalledWith(1, {
    runId: RUN_ID,
    taskId: "task-1",
    answer: rawAnswer,
  });
  expect(submit).toHaveBeenNthCalledWith(2, {
    runId: RUN_ID,
    taskId: "task-1",
    answer: rawAnswer,
  });
});

test("resets answer and copy state only after a successful multi-step checkpoint", async () => {
  const user = userEvent.setup();
  const writeText = vi.fn(async () => undefined);
  setClipboard({ writeText });
  const backend = fakeBackend({
    nextManualStep: vi
      .fn<Backend["nextManualStep"]>()
      .mockResolvedValueOnce(makeStep(1))
      .mockResolvedValueOnce(makeStep(2)),
  });
  renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await user.click(await screen.findByRole("button", { name: "复制题目" }));
  await user.type(
    screen.getByLabelText("粘贴 AI 的完整回答"),
    "第一题回答",
  );
  await user.click(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  );

  expect(await screen.findByText("只输出第 2 题答案")).toBeInTheDocument();
  expect(screen.getByLabelText("粘贴 AI 的完整回答")).toHaveValue("");
  expect(screen.getByRole("button", { name: "复制题目" })).toBeInTheDocument();
  expect(screen.getByText("第 2 / 2 题 · 已完成 1 题")).toBeInTheDocument();
  const progress = screen.getByRole("progressbar");
  expect(progress).toHaveAttribute("value", "1");
  expect(progress).toHaveAttribute("max", "2");
});

test("retries reading after a successful checkpoint without resubmitting", async () => {
  const user = userEvent.setup();
  const next = vi
    .fn<Backend["nextManualStep"]>()
    .mockResolvedValueOnce(makeStep(1))
    .mockRejectedValueOnce(new Error("模拟读取下一题失败"))
    .mockResolvedValueOnce(makeStep(2));
  const submit = vi.fn<Backend["submitManualAnswer"]>(async () => makeResult());
  const backend = fakeBackend({
    nextManualStep: next,
    submitManualAnswer: submit,
  });
  renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await user.type(
    await screen.findByLabelText("粘贴 AI 的完整回答"),
    "已提交回答",
  );
  await user.click(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  );

  expect(
    await screen.findByRole("heading", { name: "上一题已经提交" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("alert")).toHaveTextContent("模拟读取下一题失败");
  expect(
    screen.queryByRole("button", { name: "提交并进入下一题" }),
  ).not.toBeInTheDocument();
  await user.click(
    screen.getByRole("button", { name: "继续读取下一题" }),
  );

  expect(await screen.findByText("只输出第 2 题答案")).toBeInTheDocument();
  expect(submit).toHaveBeenCalledTimes(1);
  expect(next).toHaveBeenCalledTimes(3);
  expect(next).toHaveBeenLastCalledWith(RUN_ID);
});

test("navigates after the final checkpoint and announces completion", async () => {
  const user = userEvent.setup();
  const completedDetail: RunDetail = {
    run: {
      ...makeRun("chat_gpt_client", 1),
      status: "completed",
      completedTasks: 1,
      finishedAt: "2026-07-17T00:01:00Z",
      score: {
        abilityScore: 100,
        passedTasks: 1,
        validTasks: 1,
        totalTasks: 1,
        categoryScores: { instruction_following: 100 },
      },
    },
    taskResults: [makeResult()],
  };
  const backend = fakeBackend({
    nextManualStep: vi
      .fn<Backend["nextManualStep"]>()
      .mockResolvedValueOnce(makeStep(1, 1))
      .mockResolvedValueOnce(null),
    startManualRun: vi.fn(async () => makeRun("chat_gpt_client", 1)),
    getRunDetail: vi.fn(async () => completedDetail),
  });
  renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await user.type(
    await screen.findByLabelText("粘贴 AI 的完整回答"),
    "最终回答",
  );
  await user.click(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  );

  expect(
    await screen.findByRole("heading", { name: "本次客观结果" }),
  ).toBeInTheDocument();
  expect(screen.getByText("100.0")).toBeInTheDocument();
  expect(backend.cancelRun).not.toHaveBeenCalled();
});

test("explicit manual cancellation requires confirmation and navigates only after success", async () => {
  const user = userEvent.setup();
  const cancelRun = vi.fn(async () => true);
  const backend = fakeBackend({ cancelRun });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");

  await user.click(screen.getByRole("button", { name: "取消本次体检" }));
  expect(cancelRun).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "确认取消" }));

  await waitFor(() => expect(cancelRun).toHaveBeenCalledTimes(1));
  expect(cancelRun).toHaveBeenCalledWith(RUN_ID);
  expect(backend.interruptManualRun).not.toHaveBeenCalled();
  expect(router.state.location.pathname).toBe(`/results/${RUN_ID}`);
});

test("manual cancellation failure stays on the task and reports an error", async () => {
  const user = userEvent.setup();
  const backend = fakeBackend({
    cancelRun: vi.fn(async () => false),
  });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");

  await user.click(screen.getByRole("button", { name: "取消本次体检" }));
  await user.click(screen.getByRole("button", { name: "确认取消" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("无法取消");
  expect(router.state.location.pathname).toBe("/manual/chat_gpt_client");
});

test("a rejected pending explicit cancel retries exact interruption after unmount", async () => {
  const user = userEvent.setup();
  const cancelDeferred = deferred<boolean>();
  const cancelRun = vi.fn(() => cancelDeferred.promise);
  const interruptManualRun = vi
    .fn<Backend["interruptManualRun"]>()
    .mockRejectedValueOnce(new Error("run operation is busy"))
    .mockResolvedValueOnce(true);
  const backend = fakeBackend({ cancelRun, interruptManualRun });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");
  await user.click(screen.getByRole("button", { name: "取消本次体检" }));
  await user.click(screen.getByRole("button", { name: "确认取消" }));
  expect(cancelRun).toHaveBeenCalledWith(RUN_ID);

  await act(async () => {
    await router.navigate("/");
  });
  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
  await act(async () => {
    cancelDeferred.reject(new Error("cancel failed"));
    await cancelDeferred.promise.catch(() => undefined);
  });

  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(2));
  expect(interruptManualRun.mock.calls).toEqual([[RUN_ID], [RUN_ID]]);
  expect(cancelRun).toHaveBeenCalledTimes(1);
});

test("a successful pending explicit cancel does not retry interruption after unmount", async () => {
  const user = userEvent.setup();
  const cancelDeferred = deferred<boolean>();
  const cancelRun = vi.fn(() => cancelDeferred.promise);
  const interruptManualRun = vi
    .fn<Backend["interruptManualRun"]>()
    .mockRejectedValueOnce(new Error("run operation is busy"));
  const backend = fakeBackend({ cancelRun, interruptManualRun });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");
  await user.click(screen.getByRole("button", { name: "取消本次体检" }));
  await user.click(screen.getByRole("button", { name: "确认取消" }));

  await act(async () => {
    await router.navigate("/");
  });
  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
  await act(async () => {
    cancelDeferred.resolve(true);
    await cancelDeferred.promise;
  });

  expect(interruptManualRun).toHaveBeenCalledTimes(1);
  expect(cancelRun).toHaveBeenCalledTimes(1);
});

test("a false pending explicit cancel retries exact interruption after unmount", async () => {
  const user = userEvent.setup();
  const cancelDeferred = deferred<boolean>();
  const cancelRun = vi.fn(() => cancelDeferred.promise);
  const interruptManualRun = vi
    .fn<Backend["interruptManualRun"]>()
    .mockRejectedValueOnce(new Error("run operation is busy"))
    .mockResolvedValueOnce(true);
  const backend = fakeBackend({ cancelRun, interruptManualRun });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");
  await user.click(screen.getByRole("button", { name: "取消本次体检" }));
  await user.click(screen.getByRole("button", { name: "确认取消" }));

  await act(async () => {
    await router.navigate("/");
  });
  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
  await act(async () => {
    cancelDeferred.resolve(false);
    await cancelDeferred.promise;
  });

  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(2));
  expect(interruptManualRun.mock.calls).toEqual([[RUN_ID], [RUN_ID]]);
  expect(cancelRun).toHaveBeenCalledTimes(1);
  expect(cancelRun).toHaveBeenCalledWith(RUN_ID);
});

test("leaving an active manual run best-effort interrupts only that exact run", async () => {
  const user = userEvent.setup();
  const interruptManualRun = vi.fn(async () => true);
  const backend = fakeBackend({ interruptManualRun });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");

  await act(async () => {
    await router.navigate("/");
  });

  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
  expect(interruptManualRun).toHaveBeenCalledWith(RUN_ID);
  expect(backend.cancelRun).not.toHaveBeenCalled();
});

test("best-effort unmount cleanup safely swallows interrupt rejection", async () => {
  const user = userEvent.setup();
  const interruptManualRun = vi.fn(async () => {
    throw new Error("simulated interrupt failure");
  });
  const backend = fakeBackend({ interruptManualRun });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await screen.findByText("只输出第 1 题答案");

  await act(async () => {
    await router.navigate("/");
  });

  await waitFor(() =>
    expect(interruptManualRun).toHaveBeenCalledWith(RUN_ID),
  );
  expect(backend.cancelRun).not.toHaveBeenCalled();
  expect(
    screen.getByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
});

test("a pending non-final submit retries exact interruption after its run claim settles", async () => {
  const user = userEvent.setup();
  const submitDeferred = deferred<TaskResult>();
  const interruptManualRun = vi
    .fn<Backend["interruptManualRun"]>()
    .mockRejectedValueOnce(new Error("run operation is busy"))
    .mockResolvedValueOnce(true);
  const nextManualStep = vi.fn(async () => makeStep(1));
  const backend = fakeBackend({
    interruptManualRun,
    nextManualStep,
    submitManualAnswer: vi.fn(() => submitDeferred.promise),
  });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await user.type(
    await screen.findByLabelText("粘贴 AI 的完整回答"),
    "非最终题回答",
  );
  await user.click(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  );

  await act(async () => {
    await router.navigate("/");
  });
  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
  await act(async () => {
    submitDeferred.resolve(makeResult());
    await submitDeferred.promise;
  });

  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(2));
  expect(interruptManualRun.mock.calls).toEqual([[RUN_ID], [RUN_ID]]);
  expect(backend.cancelRun).not.toHaveBeenCalled();
  expect(nextManualStep).toHaveBeenCalledTimes(1);
});

test("a pending next-step read retries exact interruption after it settles", async () => {
  const user = userEvent.setup();
  const nextDeferred = deferred<ManualStep>();
  const nextManualStep = vi
    .fn<Backend["nextManualStep"]>()
    .mockResolvedValueOnce(makeStep(1))
    .mockImplementationOnce(() => nextDeferred.promise);
  const interruptManualRun = vi
    .fn<Backend["interruptManualRun"]>()
    .mockRejectedValueOnce(new Error("run operation is busy"))
    .mockResolvedValueOnce(true);
  const backend = fakeBackend({ interruptManualRun, nextManualStep });
  const { router } = renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await user.type(
    await screen.findByLabelText("粘贴 AI 的完整回答"),
    "非最终题回答",
  );
  await user.click(
    screen.getByRole("button", { name: "提交并进入下一题" }),
  );
  expect(nextManualStep).toHaveBeenCalledTimes(2);

  await act(async () => {
    await router.navigate("/");
  });
  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(1));
  await act(async () => {
    nextDeferred.resolve(makeStep(2));
    await nextDeferred.promise;
  });

  await waitFor(() => expect(interruptManualRun).toHaveBeenCalledTimes(2));
  expect(interruptManualRun.mock.calls).toEqual([[RUN_ID], [RUN_ID]]);
  expect(backend.cancelRun).not.toHaveBeenCalled();
});

test("suppresses double starts and submits while each operation is pending", async () => {
  const user = userEvent.setup();
  const startDeferred = deferred<RunRecord>();
  const submitDeferred = deferred<TaskResult>();
  const start = vi.fn<Backend["startManualRun"]>(
    () => startDeferred.promise,
  );
  const submit = vi.fn<Backend["submitManualAnswer"]>(
    () => submitDeferred.promise,
  );
  const backend = fakeBackend({
    startManualRun: start,
    submitManualAnswer: submit,
  });
  renderWizard(backend);
  await completeSetup(user);

  const startButton = screen.getByRole("button", { name: "开始快速体检" });
  await user.dblClick(startButton);
  expect(start).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("status")).toHaveTextContent("正在创建本地体检…");

  await act(async () => {
    startDeferred.resolve(makeRun());
    await startDeferred.promise;
  });
  const textarea = await screen.findByLabelText("粘贴 AI 的完整回答");
  await user.type(textarea, "回答");
  const submitButton = screen.getByRole("button", {
    name: "提交并进入下一题",
  });
  await user.dblClick(submitButton);
  expect(submit).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("status")).toHaveTextContent("正在保存本题回答…");

  await act(async () => {
    submitDeferred.resolve(makeResult());
    await submitDeferred.promise;
  });
});

test("an invalid start response never interrupts or advances an untrusted returned run id", async () => {
  const user = userEvent.setup();
  const invalid = makeRun("chat_gpt_client");
  invalid.id = UNTRUSTED_RUN_ID;
  invalid.status = "interrupted";
  const interruptManualRun = vi.fn(async () => true);
  const backend = fakeBackend({
    startManualRun: vi.fn(async () => invalid),
    interruptManualRun,
  });
  renderWizard(backend);
  await completeSetup(user);
  await user.click(
    screen.getByRole("button", { name: "开始快速体检" }),
  );

  expect(await screen.findByRole("alert")).toBeInTheDocument();
  expect(interruptManualRun).not.toHaveBeenCalled();
  expect(backend.cancelRun).not.toHaveBeenCalled();
  expect(backend.nextManualStep).not.toHaveBeenCalled();
});

test.each([
  [
    "mismatched target",
    () => makeRun("claude_client"),
  ],
  [
    "non-quick mode",
    () => ({ ...makeRun(), mode: "deep" as const }),
  ],
  [
    "progressed running state",
    () => ({ ...makeRun(), completedTasks: 1 }),
  ],
  [
    "terminal metadata",
    () => ({ ...makeRun(), finishedAt: "2026-07-19T00:00:00Z" }),
  ],
  [
    "non-running status",
    () => ({ ...makeRun(), status: "interrupted" as const }),
  ],
  [
    "resumed environment",
    () => ({
      ...makeRun(),
      environment: { ...makeRun().environment, resumed: true },
    }),
  ],
  [
    "unsafe shape",
    () => ({ id: UNTRUSTED_RUN_ID, status: "running" }) as RunRecord,
  ],
] satisfies Array<[string, () => RunRecord]>)(
  "an unmounted start response with %s never interrupts its returned id",
  async (_case, makeResponse) => {
    const user = userEvent.setup();
    const startDeferred = deferred<RunRecord>();
    const response = makeResponse();
    response.id = UNTRUSTED_RUN_ID;
    const interruptManualRun = vi.fn(async () => true);
    const backend = fakeBackend({
      startManualRun: vi.fn(() => startDeferred.promise),
      interruptManualRun,
    });
    const { router } = renderWizard(backend);
    await completeSetup(user);
    await user.click(screen.getByRole("button", { name: "开始快速体检" }));
    await act(async () => {
      await router.navigate("/");
    });
    await act(async () => {
      startDeferred.resolve(response);
      await startDeferred.promise;
    });

    expect(interruptManualRun).not.toHaveBeenCalled();
    expect(backend.cancelRun).not.toHaveBeenCalled();
    expect(backend.nextManualStep).not.toHaveBeenCalled();
  },
);

test("a valid matching start returned after navigation is interrupted exactly", async () => {
  const user = userEvent.setup();
  const startDeferred = deferred<RunRecord>();
  const start = vi
    .fn<Backend["startManualRun"]>()
    .mockImplementationOnce(() => startDeferred.promise)
    .mockResolvedValueOnce(makeRun("claude_client"));
  const interruptManualRun = vi.fn(async () => true);
  const backend = fakeBackend({ startManualRun: start, interruptManualRun });
  const { router } = renderWizard(backend);

  await completeSetup(user, "GPT-5");
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  await act(async () => {
    await router.navigate("/manual/claude_client");
  });

  expect(
    screen.getByRole("heading", { name: "Claude 客户端快速体检" }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText("当前显示的模型")).toHaveValue("");
  expect(screen.getByLabelText("我会为每道题新建空白对话")).not.toBeChecked();

  await act(async () => {
    startDeferred.resolve(makeRun("chat_gpt_client"));
    await startDeferred.promise;
  });
  expect(
    screen.getByRole("heading", { name: "Claude 客户端快速体检" }),
  ).toBeInTheDocument();
  expect(backend.nextManualStep).not.toHaveBeenCalled();
  expect(interruptManualRun).toHaveBeenCalledTimes(1);
  expect(interruptManualRun).toHaveBeenCalledWith(RUN_ID);
  expect(backend.cancelRun).not.toHaveBeenCalled();

  await completeSetup(user, "Claude Sonnet");
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  expect(await screen.findByText("只输出第 1 题答案")).toBeInTheDocument();
  expect(start).toHaveBeenLastCalledWith({
    target: {
      kind: "claude_client",
      reportedModel: "Claude Sonnet",
      reasoningEffort: null,
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  });
});

test("every task shows fresh-chat and no-tools reminders with real progress", async () => {
  const user = userEvent.setup();
  const backend = fakeBackend({
    startManualRun: vi.fn(async () => makeRun("chat_gpt_client", 7)),
    nextManualStep: vi.fn(async () => ({
      ...makeStep(3, 7),
      taskId: "task-3",
    })),
  });
  renderWizard(backend);
  await completeSetup(user);
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));

  const task = await screen.findByRole("region", { name: "当前体检题目" });
  expect(
    within(task).getByText("请为这道题新建空白对话。"),
  ).toBeInTheDocument();
  expect(
    within(task).getByText(
      "除非题目明确允许，否则不要使用联网搜索、工具或连接器。",
    ),
  ).toBeInTheDocument();
  expect(screen.getByText("第 3 / 7 题 · 已完成 2 题")).toBeInTheDocument();
  expect(screen.getByRole("progressbar")).toHaveAttribute("max", "7");
  expect(screen.getByRole("progressbar")).toHaveAttribute("value", "2");
});
