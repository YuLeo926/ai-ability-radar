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
  ManualStep,
  RunDetail,
  RunRecord,
  TargetKind,
  TaskResult,
} from "../api/backend";
import { ManualRunPage } from "./ManualRunPage";
import { ResultPage } from "./ResultPage";

const RUN_ID = "6c8cce50-bbf3-4bc5-890d-1f3316222a46";
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
    startManualRun: vi.fn(async (input) =>
      makeRun(input.target.kind),
    ),
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
    listRuns: vi.fn(async () => []),
    getRunDetail: vi.fn(async () => null),
    exportPublicReport: vi.fn(async () => null),
    deleteRawArtifacts: vi.fn(async () => undefined),
    deleteRun: vi.fn(async () => false),
    deleteTargetHistory: vi.fn(async () => 0),
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
  const resumed = makeRun("chat_gpt_client");
  resumed.completedTasks = 1;
  resumed.target.reasoningEffort = "high";
  resumed.environment.resumed = true;
  const resumeManualRun = vi.fn(async () => resumed);
  const nextManualStep = vi.fn(async () => makeStep(2));
  const startManualRun = vi.fn(async () => makeRun());
  const backend = fakeBackend({
    getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
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
  expect(
    screen.queryByLabelText("当前显示的模型"),
  ).not.toBeInTheDocument();
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
  ["model", { reportedModel: "changed-model" }],
  ["reasoning effort", { reasoningEffort: "low" }],
])(
  "manual recovery rejects a returned same-kind run with changed %s",
  async (_field, targetChange) => {
    const user = userEvent.setup();
    const preview = makeRun("chat_gpt_client");
    preview.status = "interrupted";
    preview.target.reasoningEffort = "high";
    const changed = makeRun("chat_gpt_client");
    changed.environment.resumed = true;
    changed.target = { ...preview.target, ...targetChange };
    const backend = fakeBackend({
      getRunDetail: vi.fn(async () => ({ run: preview, taskResults: [] })),
      resumeManualRun: vi.fn(async () => changed),
    });

    renderWizard(backend, `/manual/chat_gpt_client?resume=${RUN_ID}`);
    await user.click(
      await screen.findByRole("button", { name: "继续剩余题目" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "原体检配置或本地检查点已经变化",
    );
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
      },
      mode: "quick",
    });
  },
);

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

test("resets on a valid target change and ignores stale async completion", async () => {
  const user = userEvent.setup();
  const startDeferred = deferred<RunRecord>();
  const start = vi
    .fn<Backend["startManualRun"]>()
    .mockImplementationOnce(() => startDeferred.promise)
    .mockResolvedValueOnce(makeRun("claude_client"));
  const backend = fakeBackend({ startManualRun: start });
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

  await completeSetup(user, "Claude Sonnet");
  await user.click(screen.getByRole("button", { name: "开始快速体检" }));
  expect(await screen.findByText("只输出第 1 题答案")).toBeInTheDocument();
  expect(start).toHaveBeenLastCalledWith({
    target: {
      kind: "claude_client",
      reportedModel: "Claude Sonnet",
      reasoningEffort: null,
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
