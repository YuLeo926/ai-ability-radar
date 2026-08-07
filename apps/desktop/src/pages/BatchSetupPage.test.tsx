import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { expect, test, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type { Backend } from "../api/backend";
import { unsupportedBatchBackend } from "../api/backend";
import type {
  BatchEstimate,
  BatchPlanInput,
  ScanBatchRecord,
} from "../domain/batch";
import { BatchSetupPage } from "./BatchSetupPage";

function estimateFor(input: BatchPlanInput): BatchEstimate {
  const interactions = input.targets[0]?.target.reportedModel.includes("NEW")
    ? 18
    : 16;
  const targets = input.targets.map((target) => ({
    target: target.target,
    routeIdentity: {
      kind: target.target.kind,
      modelOrRoute: target.target.reportedModel.trim().toLowerCase(),
      reasoningEffort: target.target.reasoningEffort ?? null,
      executionSurface: target.executionSurface,
      isDefaultRoute: false,
    },
    executionAdapterIdentity: target.executionAdapterIdentity,
  }));
  return {
    capabilities: ["guided_quick_v1", "cli_standard_v1"],
    plan: {
      suiteId: "client-quick",
      suiteVersion: "1.0.0",
      suiteContentSha256: "a".repeat(64),
      scoringRuleVersion: "ability-v1",
      mode: "quick_comparison",
      seed: input.seed,
      status: "created",
      schedulePolicyVersion: 1,
      taskSessionPolicyVersion: 1,
      sessionIsolationPolicy:
        "user_attested_fresh_conversation_per_task",
      targets,
      sealedTaskBudgets: Array.from({ length: 8 }, () => ({
        maxTurns: 1,
        timeBudgetSecs: 270,
      })),
      costEstimate: {
        policyVersion: 1,
        executionSurface: "guided_client",
        mode: "quick_comparison",
        targetCount: 2,
        repetitionsPerTarget: 1,
        tasksPerMemberRun: 8,
        plannedMemberRuns: 2,
        taskLaunches: 16,
        guidedInteractions: interactions,
        maxProviderTurns: 16,
        summedTaskBudgetSecs: 4_320,
        expectedElapsedSecsMin: 1_200,
        expectedElapsedSecsMax: 1_800,
        providerExecutionCeilingSecs: 4_920,
        authorizationWallClockSecs: 14_400,
        issuedAt: "2026-07-31T12:00:00Z",
        initialAcknowledgementExpiresAt: "2026-07-31T12:15:00Z",
        tokenQuotaAmount: null,
        automaticRetryBudget: 0,
      },
      acknowledgementHash: (interactions === 18 ? "c" : "b").repeat(64),
    },
  };
}

function recordFor(estimate: BatchEstimate): ScanBatchRecord {
  return {
    id: "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
    plan: estimate.plan,
    status: "created",
    cancelRequested: false,
    plannedMemberCount: 2,
    terminalMemberCount: 0,
    createdAt: "2026-07-31T12:00:01Z",
    updatedAt: "2026-07-31T12:00:01Z",
    members: [
      {
        ordinal: 0,
        targetPosition: 0,
        repetitionIndex: 0,
        runId: null,
        status: "planned",
        failureKind: null,
        attemptNumber: 0,
        updatedAt: "2026-07-31T12:00:01Z",
      },
      {
        ordinal: 1,
        targetPosition: 1,
        repetitionIndex: 0,
        runId: null,
        status: "planned",
        failureKind: null,
        attemptNumber: 0,
        updatedAt: "2026-07-31T12:00:01Z",
      },
    ],
  };
}

function fakeBackend(overrides: Partial<Backend> = {}): Backend {
  return {
    ...unsupportedBatchBackend,
    getBootstrap: vi.fn(async () => {
      throw new Error("unused");
    }),
    detectClientSelection: vi.fn(async () => ({
      status: "not_running" as const,
      candidates: [],
    })),
    startManualRun: vi.fn(async () => {
      throw new Error("unused");
    }),
    nextManualStep: vi.fn(async () => null),
    submitManualAnswer: vi.fn(async () => {
      throw new Error("unused");
    }),
    startCliRun: vi.fn(async () => {
      throw new Error("unused");
    }),
    resumeManualRun: vi.fn(async () => {
      throw new Error("unused");
    }),
    resumeCliRun: vi.fn(async () => {
      throw new Error("unused");
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

function renderSetup(backend: Backend) {
  return render(
    <MemoryRouter initialEntries={["/batch/setup"]}>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/batch/setup" element={<BatchSetupPage />} />
          <Route path="/batch/:batchId" element={<div>进入批量运行页</div>} />
        </Routes>
      </BackendProvider>
    </MemoryRouter>,
  );
}

function fillModels() {
  const modelFields = screen.getAllByRole("textbox", {
    name: "客户端当前显示的模型",
  });
  fireEvent.change(
    modelFields[0]!,
    { target: { value: "GPT-5.6" } },
  );
  fireEvent.change(modelFields[1]!, {
    target: { value: "Claude Sonnet 4.5" },
  });
}

test("re-estimates on target changes and makes a stale acknowledgement unusable", async () => {
  const pending: Array<{
    input: BatchPlanInput;
    resolve(value: BatchEstimate): void;
  }> = [];
  const estimateBatch = vi.fn(
    (input: BatchPlanInput) =>
      new Promise<BatchEstimate>((resolve) => pending.push({ input, resolve })),
  );
  renderSetup(fakeBackend({ estimateBatch }));

  const modelFields = screen.getAllByRole("textbox", {
    name: "客户端当前显示的模型",
  });
  fireEvent.change(modelFields[0]!, { target: { value: "GPT-5.6" } });
  fireEvent.change(modelFields[1]!, {
    target: { value: "Claude Sonnet 4.5" },
  });
  await waitFor(() => expect(pending).toHaveLength(1));

  fireEvent.change(modelFields[0]!, { target: { value: "GPT-5.6 NEW" } });
  await waitFor(() => expect(pending).toHaveLength(2));
  expect(
    screen.getByRole("button", { name: "确认并建立扫描" }),
  ).toBeDisabled();

  pending[1]!.resolve(estimateFor(pending[1]!.input));
  await screen.findByText("18", { selector: "dd" });
  const acknowledgement = screen.getByRole("checkbox", {
    name: /我已核对这次扫描的目标/,
  });
  await userEvent.setup().click(acknowledgement);
  expect(
    screen.getByRole("button", { name: "确认并建立扫描" }),
  ).toBeEnabled();

  pending[0]!.resolve(estimateFor(pending[0]!.input));
  await waitFor(() =>
    expect(screen.getByText("18", { selector: "dd" })).toBeInTheDocument(),
  );

  fireEvent.change(modelFields[1]!, { target: { value: "Claude Opus 4.5" } });
  expect(
    screen.getByRole("button", { name: "确认并建立扫描" }),
  ).toBeDisabled();
  expect(acknowledgement).not.toBeChecked();
  expect(screen.getByText(/重新计算本地估算/)).toBeInTheDocument();
});

test("a late automatic detection never overwrites text the user already typed", async () => {
  let resolveChat!: (value: Awaited<ReturnType<Backend["detectClientSelection"]>>) => void;
  const detectClientSelection = vi.fn<Backend["detectClientSelection"]>(
    async (target) => {
      if (target === "claude_client") {
        return { status: "not_running", candidates: [] };
      }
      return new Promise((resolve) => {
        resolveChat = resolve;
      });
    },
  );
  renderSetup(fakeBackend({ detectClientSelection }));
  const chatModel = screen.getAllByRole("textbox", {
    name: "客户端当前显示的模型",
  })[0]!;
  await waitFor(() =>
    expect(detectClientSelection).toHaveBeenCalledWith("chat_gpt_client"),
  );
  fireEvent.change(chatModel, { target: { value: "我手动确认的模型" } });

  await act(async () => {
    resolveChat({
      status: "detected",
      candidates: [
        {
          model: "迟到的自动识别模型",
          reasoningEffort: "max",
          surface: "chatgpt",
          source: "windows_accessibility",
          confidence: "visible_selector",
        },
      ],
    });
  });
  await screen.findByRole("button", { name: "应用识别结果" });
  expect(chatModel).toHaveValue("我手动确认的模型");
});

test("exposes only Quick Comparison, explains mixed surfaces, and starts the acknowledged plan", async () => {
  const estimateBatch = vi.fn(async (input: BatchPlanInput) =>
    estimateFor(input),
  );
  const createAcknowledgedBatch = vi.fn<Backend["createAcknowledgedBatch"]>(
    async (input) => recordFor(estimateFor(input.plan)),
  );
  const authorizeBatchExecution = vi.fn<Backend["authorizeBatchExecution"]>(async (input) => ({
    batchId: input.batchId,
    memberOrdinal: null,
    attemptNumber: 1,
    maxTaskLaunches: 16,
    maxProviderTurns: 16,
    maxTaskBudgetSecs: 4_320,
    maxGuidedInteractions: 16,
    acknowledgementHash: input.acknowledgementHash,
    allowedFailureKind: null,
    expiresAt: "2026-07-31T16:00:00Z",
    createdAt: "2026-07-31T12:00:01Z",
  }));
  const startBatch = vi.fn<Backend["startBatch"]>(async () => {
    const input = estimateBatch.mock.calls[estimateBatch.mock.calls.length - 1]?.[0];
    if (!input) throw new Error("missing estimate");
    return { ...recordFor(estimateFor(input)), status: "running" as const };
  });
  renderSetup(
    fakeBackend({
      estimateBatch,
      createAcknowledgedBatch,
      authorizeBatchExecution,
      startBatch,
    }),
  );
  fillModels();
  await screen.findByText("16", { selector: "dd" });

  const modes = screen.getAllByRole("radio");
  expect(modes).toHaveLength(3);
  expect(modes[0]).toBeChecked();
  expect(modes[1]).toBeDisabled();
  expect(modes[2]).toBeDisabled();
  expect(screen.getByText("为什么这里不能混入 CLI？")).toBeInTheDocument();

  const user = userEvent.setup();
  await user.click(
    screen.getByRole("checkbox", { name: /我已核对这次扫描的目标/ }),
  );
  await user.click(screen.getByRole("button", { name: "确认并建立扫描" }));
  await screen.findByText("进入批量运行页");
  expect(createAcknowledgedBatch).toHaveBeenCalledWith(
    expect.objectContaining({
      plan: expect.objectContaining({ mode: "quick_comparison" }),
    }),
  );
  expect(authorizeBatchExecution).toHaveBeenCalledOnce();
  expect(startBatch).toHaveBeenCalledOnce();
});
