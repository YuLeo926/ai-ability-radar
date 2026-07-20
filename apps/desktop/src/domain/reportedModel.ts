const CONTROL_CHARACTER = /\p{Cc}/u;
const FORBIDDEN_INVISIBLE_CHARACTER =
  /[\p{Cf}\p{Default_Ignorable_Code_Point}]/u;

export const INVALID_REPORTED_MODEL_LABEL = "模型名称不可显示";

export function reportedModelError(value: string): string | null {
  if (CONTROL_CHARACTER.test(value)) {
    return "模型名称不能包含控制字符";
  }
  if (FORBIDDEN_INVISIBLE_CHARACTER.test(value)) {
    return "模型名称不能包含格式字符或不可见字符";
  }
  const trimmed = value.trim();
  if (!trimmed || Array.from(trimmed).length > 120) {
    return "模型名称必须是 1–120 个可见字符";
  }
  return null;
}

export function isSafeStoredReportedModel(value: string): boolean {
  return value === value.trim() && reportedModelError(value) === null;
}

export function formatReportedModel(
  value: string,
  defaultLabel = "默认路由（未固定）",
): string {
  if (value === "default") return defaultLabel;
  return isSafeStoredReportedModel(value)
    ? value
    : INVALID_REPORTED_MODEL_LABEL;
}
