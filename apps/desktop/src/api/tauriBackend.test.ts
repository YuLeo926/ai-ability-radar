import { beforeEach, expect, test, vi } from "vitest";
import type { RunErrorEvent, RunEvent, StartRunInput } from "./backend";

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
