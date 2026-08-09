import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type { Backend, TargetKind } from "../api/backend";
import type {
  BatchAnalysis,
  BatchExecutionSurface,
  BatchMemberStatus,
  ScanBatchRecord,
  ScanBatchTarget,
} from "../domain/batch";
import { BatchResultPage } from "./BatchResultPage";

function target(
  kind: TargetKind,
  model: string,
  source: ScanBatchTarget["target"]["modelSource"],
  surface: BatchExecutionSurface,
): ScanBatchTarget {
  const provider = kind === "chat_gpt_client" || kind === "codex_cli" ? "openai" : "anthropic";
  const defaultRoute = source === "default_route";
  return {
    target: {
      kind,
      reportedModel: model,
      reasoningEffort: "high",
      modelSource: source,
      modelVerification: defaultRoute ? "unverified" : "user_confirmed",
    },
    routeIdentity: {
      kind,
      modelOrRoute: model,
      reasoningEffort: "high",
      executionSurface: surface,
      isDefaultRoute: defaultRoute,
    },
    executionAdapterIdentity: {
      executionSurface: surface,
      providerFamily: provider,
      launchKind: surface === "guided_client" ? "guided_client" : "native_exe",
      publicVersion: surface === "guided_client" ? null : "1.2.3",
      adapterContractVersion: kind === "claude_code" ? "claude-code-v1" : kind === "codex_cli" ? "codex-cli-v1" : "guided-client-v1",
    },
  };
}

function batch(
  surface: BatchExecutionSurface,
  statuses: BatchMemberStatus[] = ["completed", "invalid"],
): ScanBatchRecord {
  const targets = surface === "guided_client"
    ? [
        target("chat_gpt_client", "GPT-5.6", "windows_accessibility", surface),
        target("claude_client", "Claude Sonnet 4.6", "manual", surface),
      ]
    : [
        target("codex_cli", "gpt-5.6", "cli_requested", surface),
        target("claude_code", "default", "default_route", surface),
      ];
  return {
    id: "10000000-0000-4000-8000-000000000001",
    plan: {
      suiteId: surface === "guided_client" ? "client-quick" : "cli-quick",
      suiteVersion: "1.0.0",
      suiteContentSha256: "a".repeat(64),
      scoringRuleVersion: "ability-v1",
      mode: "full",
      seed: 17,
      status: "created",
      schedulePolicyVersion: 1,
      taskSessionPolicyVersion: 1,
      sessionIsolationPolicy: surface === "guided_client"
        ? "user_attested_fresh_conversation_per_task"
        : "machine_enforced_fresh_session_and_workspace_per_task",
      targets,
      sealedTaskBudgets: [{ maxTurns: 10, timeBudgetSecs: 900 }],
      costEstimate: {
        policyVersion: 1,
        executionSurface: surface,
        mode: "full",
        targetCount: 2,
        repetitionsPerTarget: 5,
        tasksPerMemberRun: 1,
        plannedMemberRuns: statuses.length,
        taskLaunches: statuses.length,
        guidedInteractions: surface === "guided_client" ? statuses.length : 0,
        maxProviderTurns: 100,
        summedTaskBudgetSecs: 9000,
        expectedElapsedSecsMin: 30,
        expectedElapsedSecsMax: 60,
        providerExecutionCeilingSecs: 12000,
        authorizationWallClockSecs: 259200,
        issuedAt: "2026-08-09T02:00:00Z",
        initialAcknowledgementExpiresAt: "2026-08-09T02:15:00Z",
        tokenQuotaAmount: null,
        automaticRetryBudget: 0,
      },
      acknowledgementHash: "b".repeat(64),
    },
    baselineSnapshot: null,
    status: "completed",
    cancelRequested: false,
    plannedMemberCount: statuses.length,
    terminalMemberCount: statuses.filter((status) => ["completed", "invalid", "unavailable", "cancelled"].includes(status)).length,
    createdAt: "2026-08-09T02:00:00Z",
    updatedAt: "2026-08-09T03:00:00Z",
    members: statuses.map((status, index) => ({
      ordinal: index,
      targetPosition: index % 2,
      repetitionIndex: Math.floor(index / 2),
      runId: status === "planned" ? null : `20000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
      status,
      failureKind: status === "invalid" ? "verifier_error" : status === "unavailable" ? "cli_missing" : null,
      attemptNumber: status === "planned" ? 0 : 1,
      updatedAt: "2026-08-09T03:00:00Z",
    })),
  };
}

function analysis(batchRecord: ScanBatchRecord): BatchAnalysis {
  return {
    candidateBatchId: batchRecord.id,
    analysisVersion: 1,
    calibrationPolicyVersion: 1,
    baselineSnapshotSha256: null,
    signal: "watch",
    targets: batchRecord.plan.targets.map((_, position) => ({
      targetPosition: position,
      signal: position === 0 ? "watch" : "insufficient_data",
      candidate: position === 0 ? { count: 5, median: 82, medianAbsoluteDeviation: 2 } : null,
      baseline: position === 0 ? { count: 6, median: 88, medianAbsoluteDeviation: 1.5 } : null,
      baselineBatchCount: position === 0 ? 6 : 0,
      baselineUtcDayCount: position === 0 ? 4 : 0,
      candidateMemberCount: position === 0 ? 5 : 0,
      delta: position === 0 ? -6 : null,
      absoluteDrop: position === 0 ? 6 : null,
      relativeDrop: position === 0 ? 0.068 : null,
      deltaConfidenceInterval: position === 0 ? { lower: -9, upper: -2, confidenceLevel: 0.95 } : null,
      categoryCandidate: {},
      categoryBaseline: {},
      matchedTaskDeltas: position === 0 ? [{ taskId: "logic-grid", category: "logic", candidateMedian: 82, baselineMedian: 88, delta: -6 }] : [],
      excludedCandidateMemberOrdinals: [],
    })),
  };
}

function renderPage(record: ScanBatchRecord, result = analysis(record)) {
  const exportPublicBatchReport = vi.fn(async () => "report-id");
  const backend = {
    getBatch: vi.fn(async () => record),
    getBatchAnalysis: vi.fn(async () => result),
    exportPublicBatchReport,
  } as unknown as Backend;
  const rendered = render(
    <BackendProvider backend={backend}>
      <MemoryRouter initialEntries={[`/batch/${record.id}/result`]}>
        <Routes><Route path="/batch/:batchId/result" element={<BatchResultPage />} /></Routes>
      </MemoryRouter>
    </BackendProvider>,
  );
  return { exportPublicBatchReport, unmount: rendered.unmount };
}

describe("BatchResultPage", () => {
  it("renders a separate client matrix with visible provenance and sample-aware drill-down", async () => {
    const user = userEvent.setup();
    renderPage(batch("guided_client", ["completed", "invalid", "completed", "completed", "completed", "completed"]));
    expect(await screen.findByRole("heading", { name: "客户端证据矩阵" })).toBeInTheDocument();
    expect(screen.getByText("界面可见模型")).toBeInTheDocument();
    expect(screen.getByText("用户确认模型")).toBeInTheDocument();
    expect(screen.getByText(/不会合并排名或直接比较/)).toBeInTheDocument();

    const cells = screen.getAllByRole("button", { name: /打开证据明细/ });
    expect(cells).toHaveLength(8);
    for (const cell of cells) expect(cell).toHaveTextContent(/n = \d+/);
    await user.click(screen.getByRole("button", { name: /ChatGPT 客户端 当前中位数.*样本 5/ }));
    expect(screen.getByRole("region", { name: /ChatGPT 客户端 当前中位数/ })).toHaveTextContent("纳入 5 个完整成员");
  });

  it("labels requested and provider-default CLI routes without mixing the client cohort", async () => {
    renderPage(batch("automated_cli", ["completed", "unavailable"]));
    expect(await screen.findByRole("heading", { name: "CLI 证据矩阵" })).toBeInTheDocument();
    expect(screen.getByText("请求模型")).toBeInTheDocument();
    expect(screen.getByText("提供方默认路由")).toBeInTheDocument();
    expect(screen.getAllByText("目标不可用").length).toBeGreaterThan(0);
    expect(screen.queryByRole("heading", { name: "客户端证据矩阵" })).not.toBeInTheDocument();
  });

  it("uses distinct queued running completed invalid unavailable and insufficient states", async () => {
    const queued = batch("automated_cli", ["planned", "running"]);
    const { unmount } = renderPage(queued, { ...analysis(queued), targets: [] });
    expect((await screen.findAllByText("排队中")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("运行中").length).toBeGreaterThan(0);
    unmount();

    const terminal = batch("automated_cli", ["completed", "invalid"]);
    const terminalRender = renderPage(terminal, { ...analysis(terminal), targets: [] });
    expect((await screen.findAllByText("证据已完成")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("证据无效").length).toBeGreaterThan(0);
    terminalRender.unmount();

    const missing = batch("automated_cli", ["unavailable", "cancelled"]);
    renderPage(missing, { ...analysis(missing), targets: [] });
    expect((await screen.findAllByText("目标不可用")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("证据不足").length).toBeGreaterThan(0);
  });

  it("exports the aggregate report and states the privacy boundary", async () => {
    const user = userEvent.setup();
    const record = batch("automated_cli", ["completed", "completed"]);
    const { exportPublicBatchReport } = renderPage(record);
    expect(await screen.findByText("公开导出不含原始回答")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "导出匿名批次证据" }));
    await waitFor(() => expect(exportPublicBatchReport).toHaveBeenCalledWith(record.id));
    expect(screen.getByRole("status")).toHaveTextContent("匿名批次证据已导出");
  });

  it("supports keyboard activation of matrix evidence", async () => {
    const user = userEvent.setup();
    renderPage(batch("guided_client", ["completed", "completed"]));
    const first = await screen.findByRole("button", { name: /ChatGPT 客户端 当前中位数/ });
    first.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("region", { name: /ChatGPT 客户端 当前中位数/ })).toHaveFocus();
  });
});
