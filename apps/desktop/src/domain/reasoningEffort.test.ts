import { describe, expect, test } from "vitest";
import {
  effortOptionsFor,
  formatReasoningEffort,
  normalizeReasoningEffortForTarget,
  reasoningEffortError,
} from "./reasoningEffort";

describe("provider effort matrices", () => {
  test("ChatGPT exposes the current UI levels and Ultra", () => {
    expect(effortOptionsFor("chat_gpt_client").map(({ value }) => value)).toEqual([
      "low", "medium", "high", "xhigh", "max", "ultra",
    ]);
  });

  test("Claude exposes the complete effort set without ultracode", () => {
    for (const kind of ["claude_client", "claude_code"] as const) {
      expect(effortOptionsFor(kind).map(({ value }) => value)).toEqual([
        "low", "medium", "high", "xhigh", "max",
      ]);
    }
  });

  test("Codex exposes model-dependent lower and upper levels", () => {
    expect(effortOptionsFor("codex_cli").map(({ value }) => value)).toEqual([
      "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ]);
  });
});

test("every canonical label is localized under every provider family", () => {
  const expectedLabels = {
    none: "无",
    minimal: "最小",
    low: "低",
    medium: "中",
    high: "高",
    xhigh: "极高",
    max: "最高",
    ultra: "Ultra",
  } as const;
  const kinds = [
    "chat_gpt_client",
    "claude_client",
    "codex_cli",
    "claude_code",
  ] as const;

  for (const kind of kinds) {
    for (const [value, label] of Object.entries(expectedLabels)) {
      expect(formatReasoningEffort(kind, value), `${kind}/${value}`).toBe(label);
    }
    expect(formatReasoningEffort(kind, "扩展思考")).toBe("扩展思考");
  }

  expect(formatReasoningEffort("codex_cli", null, "CLI 默认")).toBe("CLI 默认");
});

test("custom validation mirrors the Rust family rules", () => {
  expect(reasoningEffortError("chat_gpt_client", "扩展思考")).toBeNull();
  expect(reasoningEffortError("chat_gpt_client", "想".repeat(41))).toMatch(/40/);
  expect(reasoningEffortError("codex_cli", "frontier_2")).toBeNull();
  expect(reasoningEffortError("codex_cli", "high value")).toMatch(/ASCII/);
  expect(reasoningEffortError("claude_code", "极高")).toMatch(/ASCII/);
});

test("manual custom validation requires visible Unicode text", () => {
  for (const value of [
    "\u0000",
    "\u200b",
    "\u202e",
    "\u2060",
    " \u200b ",
    "可\u200b见",
  ]) {
    expect(reasoningEffortError("chat_gpt_client", value), JSON.stringify(value)).not.toBeNull();
  }

  expect(reasoningEffortError("chat_gpt_client", "扩展思考（实验）")).toBeNull();
  expect(reasoningEffortError("chat_gpt_client", "想".repeat(40))).toBeNull();
  expect(reasoningEffortError("chat_gpt_client", "想".repeat(41))).toMatch(/40/);
});

test("known values normalize for every target and custom CLI values lowercase", () => {
  expect(normalizeReasoningEffortForTarget("chat_gpt_client", " XHIGH ")).toBe(
    "xhigh",
  );
  expect(normalizeReasoningEffortForTarget("claude_client", " 扩展思考 ")).toBe(
    "扩展思考",
  );
  expect(normalizeReasoningEffortForTarget("codex_cli", " Frontier_2 ")).toBe(
    "frontier_2",
  );
});
