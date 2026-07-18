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

test("uses exactly the eight reviewed commands and camelCase payloads", async () => {
  bridge.invoke.mockResolvedValue(undefined);
  const manualInput: StartRunInput = {
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-5",
      reasoningEffort: null,
    },
    mode: "quick",
  };
  const cliInput: StartRunInput = {
    target: {
      kind: "codex_cli",
      reportedModel: "default",
      reasoningEffort: "high",
    },
    mode: "quick",
  };
  const answer = {
    runId: "run-manual",
    taskId: "logic-truth",
    answer: "4",
  };

  await tauriBackend.getBootstrap();
  await tauriBackend.startManualRun(manualInput);
  await tauriBackend.nextManualStep("run-manual");
  await tauriBackend.submitManualAnswer(answer);
  await tauriBackend.startCliRun(cliInput);
  await tauriBackend.cancelRun("run-cli");
  await tauriBackend.listRuns();
  await tauriBackend.getRunDetail("run-result");

  expect(bridge.invoke.mock.calls).toEqual([
    ["get_bootstrap"],
    ["start_manual_run", { input: manualInput }],
    ["next_manual_step", { runId: "run-manual" }],
    ["submit_manual_answer", { input: answer }],
    ["start_cli_run", { input: cliInput }],
    ["cancel_run", { runId: "run-cli" }],
    ["list_runs"],
    ["get_run_detail", { runId: "run-result" }],
  ]);
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
