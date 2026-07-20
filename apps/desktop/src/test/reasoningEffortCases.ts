import type { TargetKind } from "../api/backend";

export const CANONICAL_EFFORT_DISPLAY_CASES = [
  ["chat_gpt_client", "none", "无"],
  ["chat_gpt_client", "minimal", "最小"],
  ["chat_gpt_client", "low", "轻度"],
  ["chat_gpt_client", "medium", "中"],
  ["chat_gpt_client", "high", "高"],
  ["chat_gpt_client", "xhigh", "极高"],
  ["chat_gpt_client", "max", "最高"],
  ["chat_gpt_client", "ultra", "Ultra"],
  ["claude_client", "none", "无"],
  ["claude_client", "minimal", "最小"],
  ["claude_client", "low", "低"],
  ["claude_client", "medium", "中"],
  ["claude_client", "high", "高"],
  ["claude_client", "xhigh", "极高"],
  ["claude_client", "max", "最高"],
  ["claude_client", "ultra", "Ultra"],
  ["codex_cli", "none", "无"],
  ["codex_cli", "minimal", "最小"],
  ["codex_cli", "low", "低"],
  ["codex_cli", "medium", "中"],
  ["codex_cli", "high", "高"],
  ["codex_cli", "xhigh", "极高"],
  ["codex_cli", "max", "最高"],
  ["codex_cli", "ultra", "Ultra"],
  ["claude_code", "none", "无"],
  ["claude_code", "minimal", "最小"],
  ["claude_code", "low", "低"],
  ["claude_code", "medium", "中"],
  ["claude_code", "high", "高"],
  ["claude_code", "xhigh", "极高"],
  ["claude_code", "max", "最高"],
  ["claude_code", "ultra", "Ultra"],
] as const satisfies ReadonlyArray<
  readonly [TargetKind, string, string]
>;

export const MANUAL_EFFORT_DISPLAY_CASES =
  CANONICAL_EFFORT_DISPLAY_CASES.filter(
    (
      entry,
    ): entry is Extract<
      (typeof CANONICAL_EFFORT_DISPLAY_CASES)[number],
      readonly ["chat_gpt_client" | "claude_client", string, string]
    > => entry[0] === "chat_gpt_client" || entry[0] === "claude_client",
  );

export const CLI_EFFORT_DISPLAY_CASES =
  CANONICAL_EFFORT_DISPLAY_CASES.filter(
    (
      entry,
    ): entry is Extract<
      (typeof CANONICAL_EFFORT_DISPLAY_CASES)[number],
      readonly ["codex_cli" | "claude_code", string, string]
    > => entry[0] === "codex_cli" || entry[0] === "claude_code",
  );

export const INVALID_LEGACY_EFFORT_CASES = [
  ["U+200B", "\u200b"],
  ["U+202E", "\u202e"],
  ["U+2060", "\u2060"],
  ["a pure invisible sequence", "\u200b\u2060"],
  ["mixed visible and invisible text", "扩\u200b展"],
] as const;

export const DEFAULT_MODEL_DISPLAY_CASES = [
  ["chat_gpt_client", "ChatGPT 客户端", "default", false],
  ["claude_client", "Claude 客户端", "default", false],
  ["codex_cli", "Codex CLI", "默认路由（未固定）", true],
  ["claude_code", "Claude Code", "默认路由（未固定）", true],
] as const satisfies ReadonlyArray<
  readonly [TargetKind, string, string, boolean]
>;
