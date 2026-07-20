import { expect, test } from "vitest";
import type { TargetKind } from "../api/backend";
import { formatReportedModel } from "./reportedModel";

test("default is a route sentinel only for the two CLI targets", () => {
  const cases: ReadonlyArray<readonly [TargetKind, string]> = [
    ["chat_gpt_client", "default"],
    ["claude_client", "default"],
    ["codex_cli", "默认路由（未固定）"],
    ["claude_code", "默认路由（未固定）"],
  ];

  for (const [kind, expected] of cases) {
    expect(formatReportedModel(kind, "default"), kind).toBe(expected);
  }
});

test("target-aware formatting preserves safe names and hides unsafe stored names", () => {
  for (const kind of [
    "chat_gpt_client",
    "claude_client",
    "codex_cli",
    "claude_code",
  ] as const) {
    expect(formatReportedModel(kind, "GPT-5.6")).toBe("GPT-5.6");
    expect(formatReportedModel(kind, "GPT\u200b-5")).toBe(
      "模型名称不可显示",
    );
  }
});
