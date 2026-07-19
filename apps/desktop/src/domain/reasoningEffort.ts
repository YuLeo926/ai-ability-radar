import type { TargetKind } from "../api/backend";

export interface EffortOption {
  value: string;
  label: string;
}

const common = [
  { value: "low", label: "低" },
  { value: "medium", label: "中" },
  { value: "high", label: "高" },
  { value: "xhigh", label: "极高" },
  { value: "max", label: "最高" },
] as const;

const matrices: Record<TargetKind, readonly EffortOption[]> = {
  chat_gpt_client: [
    { value: "low", label: "轻度" },
    ...common.slice(1),
    { value: "ultra", label: "Ultra" },
  ],
  claude_client: common,
  codex_cli: [
    { value: "minimal", label: "最小" },
    ...common,
    { value: "ultra", label: "Ultra" },
  ],
  claude_code: common,
};

const CONTROL_CHARACTER = /\p{Cc}/u;
const SAFE_CLI_EFFORT = /^[A-Za-z0-9_-]{1,32}$/;
const KNOWN_EFFORTS = new Set([
  "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
]);

export function effortOptionsFor(kind: TargetKind): readonly EffortOption[] {
  return matrices[kind];
}

export function formatReasoningEffort(
  kind: TargetKind,
  value?: string | null,
  emptyLabel = "未记录",
): string {
  if (!value) return emptyLabel;
  return matrices[kind].find((option) => option.value === value)?.label ?? value;
}

export function reasoningEffortError(
  kind: TargetKind,
  value: string,
): string | null {
  if (!value) return null;
  if (CONTROL_CHARACTER.test(value)) return "推理档位不能包含控制字符";
  const trimmed = value.trim();
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
