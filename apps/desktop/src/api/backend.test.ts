import { expect, test } from "vitest";
import type { RunMode, StartRunInput, TaskResult } from "./backend";

test("TaskResult exposes only the safe Task 13 wire fields", () => {
  const result: TaskResult = {
    runId: "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
    taskId: "dedupe-events",
    category: "cli_coding",
    outcome: "failed",
    score: 0,
    failureKind: "wrong_answer",
    durationMs: 321,
    answerRelPath: "runs/39d9f772/logs/dedupe-events.log",
  };

  expect(Object.keys(result).sort()).toEqual([
    "answerRelPath",
    "category",
    "durationMs",
    "failureKind",
    "outcome",
    "runId",
    "score",
    "taskId",
  ]);
  // @ts-expect-error Task 13 deliberately removed raw repository detail.
  expect(result.detail).toBeUndefined();
  // @ts-expect-error Raw process output must never cross the WebView boundary.
  expect(result.stdout).toBeUndefined();
  // @ts-expect-error Raw process output must never cross the WebView boundary.
  expect(result.stderr).toBeUndefined();
  // @ts-expect-error Absolute artifact paths are not public DTO fields.
  expect(result.absolutePath).toBeUndefined();
  // @ts-expect-error Generic metadata bags are not public DTO fields.
  expect(result.metadata).toBeUndefined();
});

test("new runs are quick-only while persisted records retain the stable mode union", () => {
  const input: StartRunInput = {
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-5",
      reasoningEffort: null,
      modelSource: "manual",
      modelVerification: "user_confirmed",
    },
    mode: "quick",
  };
  const persistedMode: RunMode = "deep";

  expect(input.mode).toBe("quick");
  expect(persistedMode).toBe("deep");

  // @ts-expect-error v0.2 has no public deep-run start workflow.
  const unsupported: StartRunInput = { ...input, mode: "deep" };
  expect(unsupported.mode).toBe("deep");
});
