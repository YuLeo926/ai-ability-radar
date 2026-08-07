import type {
  FailureKind,
  TargetKind,
  TargetSelection,
} from "../api/backend";

export type BatchFeatureLevel =
  | "guided_quick_v1"
  | "cli_standard_v1"
  | "reliable_full_v1";

export type BatchMode = "quick_comparison" | "standard" | "full";
export type BatchExecutionSurface = "guided_client" | "automated_cli";
export type BatchStatus =
  | "created"
  | "running"
  | "paused"
  | "completed"
  | "cancelled"
  | "interrupted";
export type BatchMemberStatus =
  | "planned"
  | "reserved"
  | "launching"
  | "running"
  | "deferred"
  | "completed"
  | "invalid"
  | "unavailable"
  | "cancelled";
export type AdapterLaunchKind =
  | "guided_client"
  | "native_exe"
  | "reviewed_npm";
export type SessionIsolationPolicy =
  | "user_attested_fresh_conversation_per_task"
  | "machine_enforced_fresh_session_and_workspace_per_task";

export interface ExecutionAdapterIdentity {
  executionSurface: BatchExecutionSurface;
  providerFamily: string;
  launchKind: AdapterLaunchKind;
  publicVersion: string | null;
  adapterContractVersion: string;
}

export interface BatchTargetInput {
  target: TargetSelection;
  executionSurface: BatchExecutionSurface;
  executionAdapterIdentity: ExecutionAdapterIdentity;
}

export interface BatchPlanInput {
  mode: BatchMode;
  seed: number;
  targets: BatchTargetInput[];
}

export interface CreateAcknowledgedBatchInput {
  plan: BatchPlanInput;
  estimateIssuedAt: string;
  acknowledgementHash: string;
}

export interface AuthorizeBatchExecutionInput {
  batchId: string;
  acknowledgementHash: string;
}

export interface EstimateBatchRetryInput {
  batchId: string;
  memberOrdinal: number;
  expectedFailureKind: FailureKind;
}

export interface AuthorizeBatchRetryInput {
  batchId: string;
  memberOrdinal: number;
  allowedFailureKind: FailureKind;
  estimateCreatedAt: string;
  acknowledgementHash: string;
}

export interface TargetRouteIdentity {
  kind: TargetKind;
  modelOrRoute: string;
  reasoningEffort: string | null;
  executionSurface: BatchExecutionSurface;
  isDefaultRoute: boolean;
}

export interface ScanBatchTarget {
  target: TargetSelection;
  routeIdentity: TargetRouteIdentity;
  executionAdapterIdentity: ExecutionAdapterIdentity;
}

export interface SealedTaskBudget {
  maxTurns: number;
  timeBudgetSecs: number;
}

export interface BatchCostEstimate {
  policyVersion: number;
  executionSurface: BatchExecutionSurface;
  mode: BatchMode;
  targetCount: number;
  repetitionsPerTarget: number;
  tasksPerMemberRun: number;
  plannedMemberRuns: number;
  taskLaunches: number;
  guidedInteractions: number;
  maxProviderTurns: number;
  summedTaskBudgetSecs: number;
  expectedElapsedSecsMin: number;
  expectedElapsedSecsMax: number;
  providerExecutionCeilingSecs: number;
  authorizationWallClockSecs: number;
  issuedAt: string;
  initialAcknowledgementExpiresAt: string;
  tokenQuotaAmount: number | null;
  automaticRetryBudget: number;
}

export interface ScanBatchPlan {
  suiteId: string;
  suiteVersion: string;
  suiteContentSha256: string;
  scoringRuleVersion: string;
  mode: BatchMode;
  seed: number;
  status: "created";
  schedulePolicyVersion: number;
  taskSessionPolicyVersion: number;
  sessionIsolationPolicy: SessionIsolationPolicy;
  targets: ScanBatchTarget[];
  sealedTaskBudgets: SealedTaskBudget[];
  costEstimate: BatchCostEstimate;
  acknowledgementHash: string;
}

export interface ScanBatchMemberRecord {
  ordinal: number;
  targetPosition: number;
  repetitionIndex: number;
  runId: string | null;
  status: BatchMemberStatus;
  failureKind: FailureKind | null;
  attemptNumber: number;
  updatedAt: string;
}

export interface ScanBatchRecord {
  id: string;
  plan: ScanBatchPlan;
  status: BatchStatus;
  cancelRequested: boolean;
  plannedMemberCount: number;
  terminalMemberCount: number;
  createdAt: string;
  updatedAt: string;
  members: ScanBatchMemberRecord[];
}

export interface ScanExecutionAuthorization {
  batchId: string;
  memberOrdinal: number | null;
  attemptNumber: number;
  maxTaskLaunches: number;
  maxProviderTurns: number;
  maxTaskBudgetSecs: number;
  maxGuidedInteractions: number;
  acknowledgementHash: string;
  allowedFailureKind: FailureKind | null;
  expiresAt: string;
  createdAt: string;
}

export interface BatchEstimate {
  plan: ScanBatchPlan;
  capabilities: BatchFeatureLevel[];
}

export interface BatchRetryEstimate {
  authorization: ScanExecutionAuthorization;
}

export type GuidedMemberDecision =
  | "runnable"
  | "blocked_by_active"
  | "exhausted";

export interface NextGuidedMember {
  decision: GuidedMemberDecision;
  member: ScanBatchMemberRecord | null;
  target: ScanBatchTarget | null;
}

export interface SubmitGuidedBatchAnswerInput {
  batchId: string;
  memberOrdinal: number;
  runId: string;
  taskId: string;
  answer: string;
  userAttestedNewConversation: true;
}

export interface DeclineGuidedBatchAttestationInput {
  batchId: string;
  memberOrdinal: number;
  runId: string;
  taskId: string;
}

export const BATCH_RESPONSE_LIMITS = Object.freeze({
  targets: 5,
  members: 25,
  taskBudgets: 8,
  batchList: 256,
});

export function supportsBatchMode(
  capabilities: readonly BatchFeatureLevel[],
  surface: BatchExecutionSurface,
  mode: BatchMode,
): boolean {
  if (surface === "guided_client") {
    return (
      mode === "quick_comparison" &&
      capabilities.includes("guided_quick_v1")
    );
  }
  if (mode === "full") {
    return capabilities.includes("reliable_full_v1");
  }
  return capabilities.includes("cli_standard_v1");
}
