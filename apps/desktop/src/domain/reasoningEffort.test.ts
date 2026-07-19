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

test("known labels are localized and custom labels are preserved", () => {
  expect(formatReasoningEffort("chat_gpt_client", "low")).toBe("轻度");
  expect(formatReasoningEffort("codex_cli", "xhigh")).toBe("极高");
  expect(formatReasoningEffort("claude_code", "max")).toBe("最高");
  expect(formatReasoningEffort("claude_client", "扩展思考")).toBe("扩展思考");
  expect(formatReasoningEffort("codex_cli", null, "CLI 默认")).toBe("CLI 默认");
});

test("custom validation mirrors the Rust family rules", () => {
  expect(reasoningEffortError("chat_gpt_client", "扩展思考")).toBeNull();
  expect(reasoningEffortError("chat_gpt_client", "想".repeat(41))).toMatch(/40/);
  expect(reasoningEffortError("codex_cli", "frontier_2")).toBeNull();
  expect(reasoningEffortError("codex_cli", "high value")).toMatch(/ASCII/);
  expect(reasoningEffortError("claude_code", "极高")).toMatch(/ASCII/);
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
