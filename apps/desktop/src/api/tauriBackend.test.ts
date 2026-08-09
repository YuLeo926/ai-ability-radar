import { beforeEach, expect, test, vi } from "vitest";
import type { RunErrorEvent, RunEvent, StartRunInput } from "./backend";
import type {
  AuthorizeBatchRetryInput,
  BatchPlanInput,
  CreateAcknowledgedBatchInput,
  EstimateBatchRetryInput,
} from "../domain/batch";

const bridge = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: bridge.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: bridge.listen }));

import { tauriBackend } from "./tauriBackend";

beforeEach(() => {
  bridge.invoke.mockReset();
  bridge.listen.mockReset();
});

test("uses exactly the nineteen reviewed commands and narrow camelCase payloads", async () => {
  bridge.invoke.mockImplementation(async (command: string) =>
    command === "detect_client_selection"
      ? { status: "not_running", candidates: [] }
      : undefined,
  );
  const manualInput: StartRunInput = {
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-5",
      reasoningEffort: null,
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  };
  const cliInput: StartRunInput = {
    target: {
      kind: "codex_cli",
      reportedModel: "default",
      reasoningEffort: "high",
      modelSource: "default_route",
      modelVerification: "unverified",
    },
    mode: "quick",
  };
  const answer = {
    runId: "run-manual",
    taskId: "logic-truth",
    answer: "4",
  };

  await tauriBackend.getBootstrap();
  await tauriBackend.detectClientSelection("chat_gpt_client");
  await tauriBackend.startManualRun(manualInput);
  await tauriBackend.nextManualStep("run-manual");
  await tauriBackend.submitManualAnswer(answer);
  await tauriBackend.startCliRun(cliInput);
  await tauriBackend.resumeManualRun({
    runId: "run-manual",
    expectedTarget: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-5",
      reasoningEffort: "high",
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
  });
  await tauriBackend.resumeCliRun({
    runId: "run-cli",
    expectedTarget: {
      kind: "codex_cli",
      reportedModel: "gpt-5.1-codex",
      reasoningEffort: null,
      modelSource: "cli_requested",
      modelVerification: "user_confirmed",
    },
  });
  await tauriBackend.cancelRun("run-cli");
  await tauriBackend.interruptManualRun("run-manual");
  await tauriBackend.listRuns();
  await tauriBackend.getRunDetail("run-result");
  await tauriBackend.exportPublicReport("run-result");
  await tauriBackend.deleteRawArtifacts("run-result");
  await tauriBackend.deleteRun("run-result");
  await tauriBackend.deleteTargetHistory("codex_cli", [
    "run-cli-1",
    "run-cli-2",
  ]);
  await tauriBackend.getDataSettings();
  await tauriBackend.setRawRetention(7);
  await tauriBackend.exportFullBackup({
    acknowledgedUnencryptedRawData: true,
  });

  expect(bridge.invoke.mock.calls).toEqual([
    ["get_bootstrap"],
    [
      "detect_client_selection",
      { input: { target: "chat_gpt_client" } },
    ],
    ["start_manual_run", { input: manualInput }],
    ["next_manual_step", { runId: "run-manual" }],
    ["submit_manual_answer", { input: answer }],
    ["start_cli_run", { input: cliInput }],
    [
      "resume_manual_run",
      {
        input: {
          runId: "run-manual",
          expectedTarget: {
            kind: "chat_gpt_client",
            reportedModel: "GPT-5",
            reasoningEffort: "high",
            modelSource: "manual",
            modelVerification: "user_confirmed",
          },
        },
      },
    ],
    [
      "resume_cli_run",
      {
        input: {
          runId: "run-cli",
          expectedTarget: {
            kind: "codex_cli",
            reportedModel: "gpt-5.1-codex",
            reasoningEffort: null,
            modelSource: "cli_requested",
            modelVerification: "user_confirmed",
          },
        },
      },
    ],
    ["cancel_run", { runId: "run-cli" }],
    ["interrupt_manual_run", { runId: "run-manual" }],
    ["list_runs"],
    ["get_run_detail", { runId: "run-result" }],
    ["export_public_report", { input: { runId: "run-result" } }],
    ["delete_raw_artifacts", { input: { runId: "run-result" } }],
    ["delete_run", { input: { runId: "run-result" } }],
    [
      "delete_target_history",
      {
        input: {
          target: "codex_cli",
          expectedRunIds: ["run-cli-1", "run-cli-2"],
        },
      },
    ],
    ["get_data_settings"],
    ["set_raw_retention", { input: { rawRetentionDays: 7 } }],
    [
      "export_full_backup",
      { input: { acknowledgedUnencryptedRawData: true } },
    ],
  ]);
  expect(JSON.stringify(bridge.invoke.mock.calls)).not.toMatch(
    /destination|filePath|outputPath|artifactPath|program/i,
  );
});

function batchPlanInput(): BatchPlanInput {
  return {
    mode: "quick_comparison",
    seed: 17,
    targets: [
      {
        target: {
          kind: "chat_gpt_client",
          reportedModel: "GPT-5.6",
          reasoningEffort: "high",
          modelSource: "manual",
          modelVerification: "user_confirmed",
        },
        executionSurface: "guided_client",
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
        executionSurface: "guided_client",
        executionAdapterIdentity: {
          executionSurface: "guided_client",
          providerFamily: "anthropic",
          launchKind: "guided_client",
          publicVersion: null,
          adapterContractVersion: "guided-client-v1",
        },
      },
    ],
  };
}

function batchResponses() {
  const planInput = batchPlanInput();
  const targets = planInput.targets.map((target) => ({
    target: target.target,
    routeIdentity: {
      kind: target.target.kind,
      modelOrRoute: target.target.reportedModel.toLowerCase(),
      reasoningEffort: target.target.reasoningEffort,
      executionSurface: target.executionSurface,
      isDefaultRoute: false,
    },
    executionAdapterIdentity: target.executionAdapterIdentity,
  }));
  const plan = {
    suiteId: "client-quick",
    suiteVersion: "1.0.0",
    suiteContentSha256: "a".repeat(64),
    scoringRuleVersion: "ability-v1",
    mode: "quick_comparison",
    seed: 17,
    status: "created",
    schedulePolicyVersion: 1,
    taskSessionPolicyVersion: 1,
    sessionIsolationPolicy: "user_attested_fresh_conversation_per_task",
    targets,
    sealedTaskBudgets: [
      { maxTurns: 1, timeBudgetSecs: 100 },
      { maxTurns: 1, timeBudgetSecs: 100 },
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
      summedTaskBudgetSecs: 400,
      expectedElapsedSecsMin: 1_200,
      expectedElapsedSecsMax: 1_800,
      providerExecutionCeilingSecs: 1_000,
      authorizationWallClockSecs: 14_400,
      issuedAt: "2026-07-30T02:00:00Z",
      initialAcknowledgementExpiresAt: "2026-07-30T02:15:00Z",
      tokenQuotaAmount: null,
      automaticRetryBudget: 0,
    },
    acknowledgementHash: "b".repeat(64),
  };
  const members = [
    {
      ordinal: 0,
      targetPosition: 1,
      repetitionIndex: 0,
      runId: null,
      status: "planned",
      failureKind: null,
      attemptNumber: 0,
      updatedAt: "2026-07-30T02:00:01Z",
    },
    {
      ordinal: 1,
      targetPosition: 0,
      repetitionIndex: 0,
      runId: null,
      status: "planned",
      failureKind: null,
      attemptNumber: 0,
      updatedAt: "2026-07-30T02:00:01Z",
    },
  ];
  const record = {
    id: "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
    plan,
    baselineSnapshot: null,
    status: "created",
    cancelRequested: false,
    plannedMemberCount: 2,
    terminalMemberCount: 0,
    createdAt: "2026-07-30T02:00:01Z",
    updatedAt: "2026-07-30T02:00:01Z",
    members,
  };
  const authorization = {
    batchId: record.id,
    memberOrdinal: null,
    attemptNumber: 1,
    maxTaskLaunches: 4,
    maxProviderTurns: 4,
    maxTaskBudgetSecs: 400,
    maxGuidedInteractions: 4,
    acknowledgementHash: plan.acknowledgementHash,
    allowedFailureKind: null,
    expiresAt: "2026-07-30T06:00:02Z",
    createdAt: "2026-07-30T02:00:02Z",
  };
  return {
    estimate: {
      plan,
      capabilities: [
        "guided_quick_v1",
        "cli_standard_v1",
        "reliable_full_v1",
      ],
    },
    record,
    analysis: {
      candidateBatchId: record.id,
      analysisVersion: 1,
      calibrationPolicyVersion: 1,
      baselineSnapshotSha256: null,
      signal: "insufficient_data",
      targets: [],
    },
    authorization,
    retryEstimate: {
      authorization: {
        ...authorization,
        memberOrdinal: 0,
        maxTaskLaunches: 2,
        maxProviderTurns: 2,
        maxTaskBudgetSecs: 200,
        maxGuidedInteractions: 2,
        allowedFailureKind: "network",
      },
    },
    next: {
      decision: "runnable",
      member: members[0],
      target: targets[1],
    },
    run: {
      id: "6ca97ed4-1b88-4ee1-9e0e-5ab34f225761",
      target: targets[1].target,
      mode: "quick",
      suiteId: plan.suiteId,
      suiteVersion: plan.suiteVersion,
      status: "running",
      startedAt: "2026-07-30T02:00:04Z",
      finishedAt: null,
      totalTasks: 2,
      completedTasks: 0,
      environment: {
        osFamily: "Windows",
        osVersion: "11",
        appVersion: "0.2.2",
        cliVersion: null,
        verifierRuntimeVersion: null,
        suiteId: plan.suiteId,
        suiteVersion: plan.suiteVersion,
        suiteContentSha256: plan.suiteContentSha256,
        scoringRuleVersion: plan.scoringRuleVersion,
        executionAdapterIdentity: targets[1].executionAdapterIdentity,
        resumed: false,
      },
      score: null,
    },
    taskResult: {
      runId: "6ca97ed4-1b88-4ee1-9e0e-5ab34f225761",
      taskId: "logic-grid",
      category: "logic",
      outcome: "failed",
      score: 0,
      failureKind: "wrong_answer",
      durationMs: 712,
      answerRelPath: "runs/6ca97ed4-1b88-4ee1-9e0e-5ab34f225761/answers/logic-grid.txt",
    },
  };
}

test("uses the sixteen reviewed batch commands with exact nested camelCase payloads", async () => {
  const responses = batchResponses();
  bridge.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "estimate_batch":
        return responses.estimate;
      case "create_acknowledged_batch":
      case "get_batch":
      case "start_batch":
      case "resume_batch":
      case "pause_batch":
      case "cancel_batch":
      case "decline_guided_batch_attestation":
        return responses.record;
      case "get_batch_analysis":
        return responses.analysis;
      case "list_batches":
        return [responses.record];
      case "authorize_batch_execution":
      case "authorize_batch_retry":
        return responses.authorization;
      case "estimate_batch_retry":
        return responses.retryEstimate;
      case "get_next_guided_member":
        return responses.next;
      case "begin_guided_batch_member":
        return responses.run;
      case "submit_guided_batch_answer":
        return responses.taskResult;
      default:
        throw new Error(`unexpected command ${command}`);
    }
  });
  const plan = batchPlanInput();
  const createInput: CreateAcknowledgedBatchInput = {
    plan,
    estimateIssuedAt: "2026-07-30T02:00:00Z",
    acknowledgementHash: "b".repeat(64),
  };
  const batchId = responses.record.id;
  const retryEstimateInput: EstimateBatchRetryInput = {
    batchId,
    memberOrdinal: 0,
    expectedFailureKind: "network",
  };
  const retryInput: AuthorizeBatchRetryInput = {
    batchId,
    memberOrdinal: 0,
    allowedFailureKind: "network",
    estimateCreatedAt: "2026-07-30T02:00:02Z",
    acknowledgementHash: "c".repeat(64),
  };
  const guidedAnswer = {
    batchId,
    memberOrdinal: 0,
    runId: responses.run.id,
    taskId: "logic-grid",
    answer: "local pasted answer",
    userAttestedNewConversation: true as const,
  };
  const declineInput = {
    batchId,
    memberOrdinal: 0,
    runId: responses.run.id,
    taskId: "logic-grid",
  };

  await tauriBackend.estimateBatch(plan);
  await tauriBackend.createAcknowledgedBatch(createInput);
  await tauriBackend.getBatch(batchId);
  await tauriBackend.getBatchAnalysis(batchId);
  await tauriBackend.listBatches();
  await tauriBackend.authorizeBatchExecution({
    batchId,
    acknowledgementHash: "b".repeat(64),
  });
  await tauriBackend.estimateBatchRetry(retryEstimateInput);
  await tauriBackend.authorizeBatchRetry(retryInput);
  await tauriBackend.startBatch(batchId);
  await tauriBackend.resumeBatch(batchId);
  await tauriBackend.pauseBatch(batchId);
  await tauriBackend.cancelBatch(batchId);
  await tauriBackend.getNextGuidedMember(batchId);
  await tauriBackend.beginGuidedBatchMember(batchId);
  await tauriBackend.submitGuidedBatchAnswer(guidedAnswer);
  await tauriBackend.declineGuidedBatchAttestation(declineInput);

  expect(bridge.invoke.mock.calls).toEqual([
    ["estimate_batch", { input: plan }],
    ["create_acknowledged_batch", { input: createInput }],
    ["get_batch", { input: { batchId } }],
    ["get_batch_analysis", { input: { batchId } }],
    ["list_batches"],
    [
      "authorize_batch_execution",
      { input: { batchId, acknowledgementHash: "b".repeat(64) } },
    ],
    ["estimate_batch_retry", { input: retryEstimateInput }],
    ["authorize_batch_retry", { input: retryInput }],
    ["start_batch", { input: { batchId } }],
    ["resume_batch", { input: { batchId } }],
    ["pause_batch", { input: { batchId } }],
    ["cancel_batch", { input: { batchId } }],
    ["get_next_guided_member", { input: { batchId } }],
    ["begin_guided_batch_member", { input: { batchId } }],
    ["submit_guided_batch_answer", { input: guidedAnswer }],
    ["decline_guided_batch_attestation", { input: declineInput }],
  ]);
  expect(JSON.stringify(bridge.invoke.mock.calls)).not.toMatch(
    /program|arguments|destination|filePath|artifactPath/i,
  );
});

test("rejects malformed nested batch responses with a stable local protocol error", async () => {
  const response = batchResponses().estimate;
  bridge.invoke.mockResolvedValue({
    ...response,
    plan: {
      ...response.plan,
      targets: [
        {
          ...response.plan.targets[0],
          executionAdapterIdentity: {
            ...response.plan.targets[0].executionAdapterIdentity,
            program: "C:/private/codex.exe",
          },
        },
        response.plan.targets[1],
      ],
    },
  });

  await expect(tauriBackend.estimateBatch(batchPlanInput())).rejects.toThrow(
    "Batch command estimate_batch returned invalid local data",
  );
});

test("rejects extra fields from guided run and task-result commands", async () => {
  const responses = batchResponses();
  bridge.invoke
    .mockResolvedValueOnce({ ...responses.run, program: "private.exe" })
    .mockResolvedValueOnce({
      ...responses.taskResult,
      detail: "private grader detail",
    });

  await expect(
    tauriBackend.beginGuidedBatchMember(responses.record.id),
  ).rejects.toThrow(
    "Batch command begin_guided_batch_member returned invalid local data",
  );
  await expect(
    tauriBackend.submitGuidedBatchAnswer({
      batchId: responses.record.id,
      memberOrdinal: 0,
      runId: responses.run.id,
      taskId: "logic-grid",
      answer: "local answer",
      userAttestedNewConversation: true,
    }),
  ).rejects.toThrow(
    "Batch command submit_guided_batch_answer returned invalid local data",
  );
});

test.each(["windowTitle", "processPath", "rawControls"])(
  "rejects client selection data containing extra %s with the stable protocol error",
  async (field) => {
    bridge.invoke.mockResolvedValue({
      status: "detected",
      candidates: [
        {
          model: "GPT-5.6",
          reasoningEffort: "max",
          surface: "codex_desktop",
          source: "windows_accessibility",
          confidence: "visible_selector",
          [field]: field === "rawControls" ? ["private"] : "private",
        },
      ],
    });

    await expect(
      tauriBackend.detectClientSelection("chat_gpt_client"),
    ).rejects.toThrow("本地模型识别返回了无效数据");
    expect(bridge.invoke).toHaveBeenCalledWith("detect_client_selection", {
      input: { target: "chat_gpt_client" },
    });
  },
);

test("propagates detect-client invoke failures without replacing them", async () => {
  const invokeError = new Error("模拟本地识别失败");
  bridge.invoke.mockRejectedValue(invokeError);

  await expect(
    tauriBackend.detectClientSelection("claude_client"),
  ).rejects.toBe(invokeError);
  expect(bridge.invoke).toHaveBeenCalledWith("detect_client_selection", {
    input: { target: "claude_client" },
  });
});

test("listens only to reviewed events, forwards payloads, and returns unlisten", async () => {
  const handlers: Array<(event: { payload: unknown }) => void> = [];
  const unlistenRun = vi.fn();
  const unlistenError = vi.fn();
  bridge.listen
    .mockImplementationOnce(
      async (_name: string, handler: (event: { payload: unknown }) => void) => {
        handlers.push(handler);
        return unlistenRun;
      },
    )
    .mockImplementationOnce(
      async (_name: string, handler: (event: { payload: unknown }) => void) => {
        handlers.push(handler);
        return unlistenError;
      },
    );
  const runListener = vi.fn();
  const errorListener = vi.fn();
  const runEvent: RunEvent = {
    runId: "run-1",
    kind: "task_finished",
    taskId: "dedupe-events",
    completedTasks: 1,
    totalTasks: 2,
  };
  const errorEvent: RunErrorEvent = {
    runId: "run-1",
    message: "fake backend failure",
  };

  const returnedRunUnlisten = await tauriBackend.onRunEvent(runListener);
  const returnedErrorUnlisten = await tauriBackend.onRunError(errorListener);
  handlers[0]?.({ payload: runEvent });
  handlers[1]?.({ payload: errorEvent });

  expect(bridge.listen.mock.calls.map(([name]) => name)).toEqual([
    "run://event",
    "run://error",
  ]);
  expect(bridge.listen.mock.calls.every((call) => call.length === 2)).toBe(true);
  expect(runListener).toHaveBeenCalledOnce();
  expect(runListener).toHaveBeenCalledWith(runEvent);
  expect(errorListener).toHaveBeenCalledOnce();
  expect(errorListener).toHaveBeenCalledWith(errorEvent);
  expect(returnedRunUnlisten).toBe(unlistenRun);
  expect(returnedErrorUnlisten).toBe(unlistenError);
});
