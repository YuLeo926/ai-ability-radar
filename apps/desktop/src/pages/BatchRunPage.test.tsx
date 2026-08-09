import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { expect, test, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type { Backend, ManualStep, RunRecord } from "../api/backend";
import { unsupportedBatchBackend } from "../api/backend";
import type {
  NextGuidedMember,
  ScanBatchMemberRecord,
  ScanBatchRecord,
} from "../domain/batch";
import { BatchRunPage } from "./BatchRunPage";

const batchId = "39d9f772-2e12-4b2d-af13-94c32d36f2d3";
const runId = "6ca97ed4-1b88-4ee1-9e0e-5ab34f225761";

function members(
  firstStatus: ScanBatchMemberRecord["status"] = "running",
): ScanBatchMemberRecord[] {
  return [
    {
      ordinal: 0,
      targetPosition: 0,
      repetitionIndex: 0,
      runId: firstStatus === "planned" ? null : runId,
      status: firstStatus,
      failureKind: firstStatus === "invalid" ? "user_cancelled" : null,
      attemptNumber: firstStatus === "planned" ? 0 : 1,
      updatedAt: "2026-07-31T12:00:04Z",
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
  ];
}

function batchRecord(
  firstStatus: ScanBatchMemberRecord["status"] = "running",
): ScanBatchRecord {
  const batchMembers = members(firstStatus);
  return {
    id: batchId,
    plan: {
      suiteId: "client-quick",
      suiteVersion: "1.0.0",
      suiteContentSha256: "a".repeat(64),
      scoringRuleVersion: "ability-v1",
      mode: "quick_comparison",
      seed: 17,
      status: "created",
      schedulePolicyVersion: 1,
      taskSessionPolicyVersion: 1,
      sessionIsolationPolicy:
        "user_attested_fresh_conversation_per_task",
      targets: [
        {
          target: {
            kind: "chat_gpt_client",
            reportedModel: "GPT-5.6",
            reasoningEffort: "max",
            modelSource: "windows_accessibility",
            modelVerification: "user_confirmed",
          },
          routeIdentity: {
            kind: "chat_gpt_client",
            modelOrRoute: "gpt-5.6",
            reasoningEffort: "max",
            executionSurface: "guided_client",
            isDefaultRoute: false,
          },
          executionAdapterIdentity: {
            executionSurface: "guided_client",
            providerFamily: "openai",
            launchKind: "guided_client",
            publicVersion: null,
            adapterContractVersion: "guided-client-v1",
          },
        },
        {
          target: {
            kind: "claude_client",
            reportedModel: "Claude Sonnet 4.5",
            reasoningEffort: "high",
            modelSource: "manual",
            modelVerification: "user_confirmed",
          },
          routeIdentity: {
            kind: "claude_client",
            modelOrRoute: "claude sonnet 4.5",
            reasoningEffort: "high",
            executionSurface: "guided_client",
            isDefaultRoute: false,
          },
          executionAdapterIdentity: {
            executionSurface: "guided_client",
            providerFamily: "anthropic",
            launchKind: "guided_client",
            publicVersion: null,
            adapterContractVersion: "guided-client-v1",
          },
        },
      ],
      sealedTaskBudgets: [
        { maxTurns: 1, timeBudgetSecs: 270 },
        { maxTurns: 1, timeBudgetSecs: 270 },
      ],
      costEstimate: {
        policyVersion: 1,
        executionSurface: "guided_client",
        mode: "quick_comparison",
        targetCount: 2,
        repetitionsPerTarget: 1,
        tasksPerMemberRun: 2,
        plannedMemberRuns: 2,
        taskLaunches: 4,
        guidedInteractions: 4,
        maxProviderTurns: 4,
        summedTaskBudgetSecs: 1_080,
        expectedElapsedSecsMin: 1_200,
        expectedElapsedSecsMax: 1_800,
        providerExecutionCeilingSecs: 1_680,
        authorizationWallClockSecs: 14_400,
        issuedAt: "2026-07-31T12:00:00Z",
        initialAcknowledgementExpiresAt: "2026-07-31T12:15:00Z",
        tokenQuotaAmount: null,
        automaticRetryBudget: 0,
      },
      acknowledgementHash: "b".repeat(64),
    },
    status: "running",
    cancelRequested: false,
    plannedMemberCount: 2,
    terminalMemberCount: ["completed", "invalid", "unavailable", "cancelled"].includes(
      firstStatus,
    )
      ? 1
      : 0,
    createdAt: "2026-07-31T12:00:01Z",
    updatedAt: "2026-07-31T12:00:04Z",
    members: batchMembers,
  };
}

function completedBatch(): ScanBatchRecord {
  const base = batchRecord("completed");
  return {
    ...base,
    status: "completed",
    terminalMemberCount: 2,
    members: [
      { ...base.members[0]!, status: "completed", failureKind: null },
      {
        ...base.members[1]!,
        runId: "6eac7183-954c-426d-9c69-86a96772da12",
        status: "completed",
        attemptNumber: 1,
      },
    ],
  };
}

function cliBatchRecord(paused = false): ScanBatchRecord {
  const base = batchRecord();
  return {
    ...base,
    status: paused ? "paused" : "running",
    terminalMemberCount: paused ? 1 : 0,
    plan: {
      ...base.plan,
      suiteId: "cli-quick",
      mode: "standard",
      sessionIsolationPolicy:
        "machine_enforced_fresh_session_and_workspace_per_task",
      targets: [
        {
          target: {
            kind: "codex_cli",
            reportedModel: "gpt-5.6-sol",
            reasoningEffort: "max",
            modelSource: "cli_requested",
            modelVerification: "user_confirmed",
          },
          routeIdentity: {
            kind: "codex_cli",
            modelOrRoute: "gpt-5.6-sol",
            reasoningEffort: "max",
            executionSurface: "automated_cli",
            isDefaultRoute: false,
          },
          executionAdapterIdentity: {
            executionSurface: "automated_cli",
            providerFamily: "openai",
            launchKind: "native_exe",
            publicVersion: "codex-cli 1.2.3",
            adapterContractVersion: "codex-cli-v1",
          },
        },
        {
          target: {
            kind: "claude_code",
            reportedModel: "claude-sonnet-4-5",
            reasoningEffort: "high",
            modelSource: "cli_requested",
            modelVerification: "user_confirmed",
          },
          routeIdentity: {
            kind: "claude_code",
            modelOrRoute: "claude-sonnet-4-5",
            reasoningEffort: "high",
            executionSurface: "automated_cli",
            isDefaultRoute: false,
          },
          executionAdapterIdentity: {
            executionSurface: "automated_cli",
            providerFamily: "anthropic",
            launchKind: "native_exe",
            publicVersion: "1.2.3 (claude code)",
            adapterContractVersion: "claude-code-v1",
          },
        },
      ],
      costEstimate: {
        ...base.plan.costEstimate,
        executionSurface: "automated_cli",
        mode: "standard",
        guidedInteractions: 0,
        maxProviderTurns: 240,
        summedTaskBudgetSecs: 21_600,
        authorizationWallClockSecs: 86_400,
      },
    },
    members: paused
      ? [
          {
            ...base.members[0]!,
            status: "deferred",
            failureKind: "network",
            attemptNumber: 1,
          },
          {
            ...base.members[1]!,
            runId: "6eac7183-954c-426d-9c69-86a96772da12",
            status: "completed",
            failureKind: null,
            attemptNumber: 1,
          },
        ]
      : base.members,
  };
}

function activeDecision(
  record = batchRecord(),
): NextGuidedMember {
  return {
    decision: "blocked_by_active",
    member: record.members[0]!,
    target: record.plan.targets[0]!,
  };
}

function manualStep(): ManualStep {
  return {
    runId,
    taskId: "logic-grid",
    taskNumber: 1,
    totalTasks: 2,
    prompt: "只输出最终答案：1 + 1 = ?",
  };
}

function runRecord(): RunRecord {
  const record = batchRecord();
  return {
    id: runId,
    target: record.plan.targets[0]!.target,
    mode: "quick",
    suiteId: record.plan.suiteId,
    suiteVersion: record.plan.suiteVersion,
    status: "running",
    startedAt: "2026-07-31T12:00:04Z",
    finishedAt: null,
    totalTasks: 2,
    completedTasks: 0,
    environment: {
      osFamily: "Windows",
      osVersion: "11",
      appVersion: "0.2.2",
      cliVersion: null,
      verifierRuntimeVersion: null,
      suiteId: record.plan.suiteId,
      suiteVersion: record.plan.suiteVersion,
      suiteContentSha256: record.plan.suiteContentSha256,
      scoringRuleVersion: record.plan.scoringRuleVersion,
      executionAdapterIdentity:
        record.plan.targets[0]!.executionAdapterIdentity,
      resumed: false,
    },
    score: null,
  };
}

function fakeBackend(overrides: Partial<Backend> = {}): Backend {
  const record = batchRecord();
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
    nextManualStep: vi.fn(async () => manualStep()),
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
    getBatch: vi.fn(async () => record),
    getNextGuidedMember: vi.fn(async () => activeDecision(record)),
    beginGuidedBatchMember: vi.fn(async () => runRecord()),
    submitGuidedBatchAnswer: vi.fn<Backend["submitGuidedBatchAnswer"]>(
      async (input) => ({
        runId: input.runId,
        taskId: input.taskId,
        category: "logic",
        outcome: "passed",
        score: 100,
        failureKind: null,
        durationMs: 123,
        answerRelPath: "runs/local/answers/logic-grid.txt",
      }),
    ),
    declineGuidedBatchAttestation: vi.fn(async () => batchRecord("invalid")),
    onRunEvent: vi.fn(async () => () => undefined),
    onRunError: vi.fn(async () => () => undefined),
    ...overrides,
  };
}

function renderRun(backend: Backend, path = `/batch/${batchId}`) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/batch/:batchId" element={<BatchRunPage />} />
          <Route
            path="/batch/:batchId/result"
            element={<BatchRunPage resultMode />}
          />
        </Routes>
      </BackendProvider>
    </MemoryRouter>,
  );
}

test("requires a fresh-conversation attestation and keeps target provenance visible", async () => {
  const writeText = vi.fn();
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
  const submitGuidedBatchAnswer = vi.fn<Backend["submitGuidedBatchAnswer"]>(
    async (input) => ({
      runId: input.runId,
      taskId: input.taskId,
      category: "logic",
      outcome: "passed",
      score: 100,
      failureKind: null,
      durationMs: 123,
      answerRelPath: "runs/local/answers/logic-grid.txt",
    }),
  );
  renderRun(fakeBackend({ submitGuidedBatchAnswer }));

  await screen.findByRole("heading", { name: "在新的空白对话中完成本题" });
  expect(screen.getAllByText("GPT-5.6").length).toBeGreaterThan(0);
  expect(screen.getByText("最高")).toBeInTheDocument();
  expect(screen.getByText(/Windows 客户端界面/)).toBeInTheDocument();
  expect(screen.getByText(/不是机器验证/)).toBeInTheDocument();
  expect(writeText).not.toHaveBeenCalled();

  fireEvent.change(screen.getByRole("textbox", { name: "粘贴 AI 的完整原始回答" }), {
    target: { value: "2" },
  });
  const submit = screen.getByRole("button", {
    name: "保存本题并读取下一步",
  });
  expect(submit).toBeDisabled();
  await userEvent.setup().click(
    screen.getByRole("checkbox", { name: /这道题是在刚新建的空白对话中完成/ }),
  );
  expect(submit).toBeEnabled();
  await userEvent.setup().click(submit);
  await waitFor(() => expect(submitGuidedBatchAnswer).toHaveBeenCalledOnce());
  expect(submitGuidedBatchAnswer).toHaveBeenCalledWith({
    batchId,
    memberOrdinal: 0,
    runId,
    taskId: "logic-grid",
    answer: "2",
    userAttestedNewConversation: true,
  });
  expect(writeText).not.toHaveBeenCalled();
});

test("refreshes the exact running member but never auto-opens ambiguous reserved work", async () => {
  const exactNextManualStep = vi.fn(async () => manualStep());
  const runningView = renderRun(
    fakeBackend({ nextManualStep: exactNextManualStep }),
  );
  await screen.findByRole("heading", { name: "在新的空白对话中完成本题" });
  expect(exactNextManualStep).toHaveBeenCalledWith(runId);
  runningView.unmount();

  const reserved = batchRecord("reserved");
  const beginGuidedBatchMember = vi.fn(async () => runRecord());
  const reservedNextStep = vi.fn(async () => manualStep());
  renderRun(
    fakeBackend({
      getBatch: vi.fn(async () => reserved),
      getNextGuidedMember: vi.fn(async () => activeDecision(reserved)),
      beginGuidedBatchMember,
      nextManualStep: reservedNextStep,
    }),
    `/batch/${batchId}?reserved=1`,
  );
  await screen.findByRole("heading", { name: "没有自动重开或重复" });
  expect(beginGuidedBatchMember).not.toHaveBeenCalled();
  expect(reservedNextStep).not.toHaveBeenCalled();
});

test("declining the attestation marks the exact member invalid", async () => {
  const declineGuidedBatchAttestation = vi.fn<
    Backend["declineGuidedBatchAttestation"]
  >(async () => batchRecord("invalid"));
  renderRun(fakeBackend({ declineGuidedBatchAttestation }));
  await screen.findByRole("heading", { name: "在新的空白对话中完成本题" });

  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "无法确认新空白对话" }),
  );
  await user.click(screen.getByRole("button", { name: "确认标记为无效" }));
  await waitFor(() =>
    expect(declineGuidedBatchAttestation).toHaveBeenCalledWith({
      batchId,
      memberOrdinal: 0,
      runId,
      taskId: "logic-grid",
    }),
  );
});

test("completion navigates to an insufficient_data result without a degradation conclusion", async () => {
  const running = batchRecord();
  const complete = completedBatch();
  const getBatch = vi
    .fn<Backend["getBatch"]>()
    .mockResolvedValueOnce(running)
    .mockResolvedValue(complete);
  renderRun(fakeBackend({ getBatch }));
  await screen.findByRole("heading", { name: "在新的空白对话中完成本题" });

  fireEvent.change(screen.getByRole("textbox", { name: "粘贴 AI 的完整原始回答" }), {
    target: { value: "2" },
  });
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("checkbox", { name: /这道题是在刚新建的空白对话中完成/ }),
  );
  await user.click(
    screen.getByRole("button", { name: "保存本题并读取下一步" }),
  );

  await screen.findByRole("heading", { name: "证据不足，暂不判断是否降智" });
  expect(screen.getByText("insufficient_data")).toBeInTheDocument();
  expect(screen.getByText(/明确没有下“降智”结论/)).toBeInTheDocument();
  expect(screen.queryByText(/确认降智|已经降智/)).not.toBeInTheDocument();
});

test("renders an automated CLI batch without calling guided-member commands", async () => {
  const record = cliBatchRecord();
  const getNextGuidedMember = vi.fn(async () => activeDecision());
  const cancelBatch = vi.fn(async () => ({
    ...record,
    status: "cancelled" as const,
    cancelRequested: true,
    terminalMemberCount: 2,
    members: record.members.map((member) => ({
      ...member,
      status: "cancelled" as const,
      failureKind: "user_cancelled" as const,
    })),
  }));
  renderRun(
    fakeBackend({
      getBatch: vi.fn(async () => record),
      getNextGuidedMember,
      cancelBatch,
    }),
  );

  await screen.findByRole("heading", { name: "串行执行每个模型坐标" });
  expect(screen.getByText(/全新临时会话与独立工作区/)).toBeInTheDocument();
  expect(screen.getByText(/任一时刻最多只有一个成员/)).toBeInTheDocument();
  expect(getNextGuidedMember).not.toHaveBeenCalled();
  await userEvent.setup().click(
    screen.getByRole("button", { name: "取消并阻止后续启动" }),
  );
  await waitFor(() => expect(cancelBatch).toHaveBeenCalledWith(batchId));
});

test("requires a fresh bounded authorization before retrying one deferred CLI member", async () => {
  const record = cliBatchRecord(true);
  const retryAuthorization = {
    batchId,
    memberOrdinal: 0,
    attemptNumber: 2,
    maxTaskLaunches: 2,
    maxProviderTurns: 40,
    maxTaskBudgetSecs: 3_600,
    maxGuidedInteractions: 0,
    acknowledgementHash: "c".repeat(64),
    allowedFailureKind: "network" as const,
    expiresAt: "2026-08-01T12:00:00Z",
    createdAt: "2026-07-31T12:05:00Z",
  };
  const estimateBatchRetry = vi.fn<Backend["estimateBatchRetry"]>(async () => ({
    authorization: retryAuthorization,
  }));
  const authorizeBatchRetry = vi.fn<Backend["authorizeBatchRetry"]>(
    async () => retryAuthorization,
  );
  const resumeBatch = vi.fn(async () => ({ ...record, status: "running" as const }));
  renderRun(
    fakeBackend({
      getBatch: vi.fn(async () => record),
      estimateBatchRetry,
      authorizeBatchRetry,
      resumeBatch,
    }),
  );

  await screen.findByRole("heading", { name: "扫描已安全暂停" });
  expect(screen.getByText("网络故障")).toBeInTheDocument();
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "生成最新重试预算" }),
  );
  expect(await screen.findByText("60 分钟")).toBeInTheDocument();
  const authorize = screen.getByRole("button", {
    name: "授权同一运行继续",
  });
  expect(authorize).toBeDisabled();
  await user.click(
    screen.getByRole("checkbox", { name: /只恢复这一成员/ }),
  );
  expect(authorize).toBeEnabled();
  await user.click(authorize);

  await waitFor(() =>
    expect(authorizeBatchRetry).toHaveBeenCalledWith({
      batchId,
      memberOrdinal: 0,
      allowedFailureKind: "network",
      estimateCreatedAt: "2026-07-31T12:05:00Z",
      acknowledgementHash: "c".repeat(64),
    }),
  );
  expect(resumeBatch).toHaveBeenCalledWith(batchId);
});
