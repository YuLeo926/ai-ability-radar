import type {
  ModelSource,
  ModelVerification,
  TargetSelection,
} from "../api/backend";

export type { ModelSource, ModelVerification };

// Frontend mirror of the committed public-report contract. Rust/schema remain
// authoritative; ResultPage tests pin this reviewed user-facing version.
export const PUBLIC_REPORT_SCHEMA_VERSION = 3;

const sourceLabels: Record<ModelSource, string> = {
  manual: "用户填写",
  windows_accessibility: "Windows 客户端界面",
  cli_requested: "CLI 本次明确指定",
  cli_reported: "CLI 已报告",
  default_route: "CLI 默认路由",
  legacy_unknown: "历史记录，来源未知",
};

const verificationLabels: Record<ModelVerification, string> = {
  user_confirmed: "用户已确认",
  provider_reported: "提供方已报告",
  unverified: "未核验",
  legacy_unknown: "可信状态未知",
};

export function formatModelProvenance(target: TargetSelection): string {
  return `模型来源：${sourceLabels[target.modelSource]} · ${
    verificationLabels[target.modelVerification]
  }`;
}
