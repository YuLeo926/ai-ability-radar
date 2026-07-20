import type { TargetKind } from "../api/backend";
import displayPolicyJson from "../../../../schemas/reasoning-effort-display.json";

export interface EffortOption {
  value: string;
  label: string;
}

const CANONICAL_EFFORTS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;
type CanonicalEffort = (typeof CANONICAL_EFFORTS)[number];

const displayPolicy = displayPolicyJson satisfies Record<
  TargetKind,
  Record<CanonicalEffort, string>
>;
const offeredEfforts: Record<TargetKind, readonly CanonicalEffort[]> = {
  chat_gpt_client: ["low", "medium", "high", "xhigh", "max", "ultra"],
  claude_client: ["low", "medium", "high", "xhigh", "max"],
  codex_cli: ["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
  claude_code: ["low", "medium", "high", "xhigh", "max"],
};

const FORBIDDEN_CUSTOM_CHARACTER =
  /[\p{Cc}\p{Cf}\p{Default_Ignorable_Code_Point}]/u;
const SAFE_CLI_EFFORT = /^[A-Za-z0-9_-]{1,32}$/;
const KNOWN_EFFORTS = new Set<string>(CANONICAL_EFFORTS);

export const INVALID_REASONING_EFFORT_LABEL = "推理档位不可显示";

export function effortOptionsFor(kind: TargetKind): readonly EffortOption[] {
  return offeredEfforts[kind].map((value) => ({
    value,
    label: displayPolicy[kind][value],
  }));
}

export function formatReasoningEffort(
  kind: TargetKind,
  value?: string | null,
  emptyLabel = "未记录",
): string {
  if (!value) return emptyLabel;
  if (KNOWN_EFFORTS.has(value)) {
    return displayPolicy[kind][value as CanonicalEffort];
  }
  return FORBIDDEN_CUSTOM_CHARACTER.test(value)
    ? INVALID_REASONING_EFFORT_LABEL
    : value;
}

export function reasoningEffortError(
  kind: TargetKind,
  value: string,
): string | null {
  if (FORBIDDEN_CUSTOM_CHARACTER.test(value)) {
    return "推理档位不能包含控制字符、格式字符或不可见字符";
  }
  const trimmed = value.trim();
  if (!trimmed) return "请填写自定义推理档位";
  const cli = kind === "codex_cli" || kind === "claude_code";
  if (cli) {
    return SAFE_CLI_EFFORT.test(trimmed)
      ? null
      : "自定义 CLI 档位只能包含 1–32 个 ASCII 字母、数字、下划线或连字符";
  }
  return Array.from(trimmed).length <= 40
    ? null
    : "自定义推理档位不能超过 40 个字符";
}

export function normalizeReasoningEffortForTarget(
  kind: TargetKind,
  value: string,
): string {
  const trimmed = value.trim();
  const lowered = trimmed.toLowerCase();
  if (
    KNOWN_EFFORTS.has(lowered) ||
    kind === "codex_cli" ||
    kind === "claude_code"
  ) {
    return lowered;
  }
  return trimmed;
}
