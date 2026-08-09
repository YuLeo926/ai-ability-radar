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

export type AcceptedProvenanceClass =
  | "guided_manual_confirmed"
  | "guided_accessibility_confirmed"
  | "cli_requested_confirmed"
  | "cli_default_unverified";

export type BaselineExclusionReason =
  | "candidate_batch"
  | "duplicate_evidence_id"
  | "not_completed_full"
  | "missing_or_invalid_snapshot"
  | "not_strictly_before_cutoff"
  | "outside_history_window"
  | "incompatible_identity"
  | "older_batch_on_same_utc_day"
  | "beyond_maximum_historical_batches";

export interface AnalysisTargetIdentity {
  routeIdentity: TargetRouteIdentity;
  provenanceClass: AcceptedProvenanceClass;
  providerFamily: string;
  launchKind: AdapterLaunchKind;
  adapterContractVersion: string;
}

export interface BatchAnalysisIdentity {
  analysisVersion: number;
  suiteId: string;
  suiteVersion: string;
  suiteContentSha256: string;
  scoringRuleVersion: string;
  executionSurface: BatchExecutionSurface;
  targets: AnalysisTargetIdentity[];
}

export interface BaselineExclusion {
  batchId: string;
  reason: BaselineExclusionReason;
}

export interface BaselineSnapshot {
  candidateBatchId: string;
  baselineAsOf: string;
  analysisVersion: number;
  calibrationPolicyVersion: number;
  historyWindowDays: number;
  maximumHistoricalBatches: number;
  bootstrapSeed: number;
  bootstrapResamples: number;
  identity: BatchAnalysisIdentity;
  selectedBatchIds: string[];
  exclusions: BaselineExclusion[];
  contentSha256: string;
}

export type RegressionSignal =
  | "insufficient_data"
  | "stable"
  | "watch"
  | "likely_regression";

export interface DistributionSummary {
  count: number;
  median: number;
  medianAbsoluteDeviation: number;
}

export interface ConfidenceInterval {
  lower: number;
  upper: number;
  confidenceLevel: number;
}

export interface MatchedTaskDelta {
  taskId: string;
  category: "instruction_following" | "logic" | "code_review" | "cli_coding";
  candidateMedian: number;
  baselineMedian: number;
  delta: number;
}

export interface TargetBatchAnalysis {
  targetPosition: number;
  signal: RegressionSignal;
  candidate: DistributionSummary | null;
  baseline: DistributionSummary | null;
  baselineBatchCount: number;
  baselineUtcDayCount: number;
  candidateMemberCount: number;
  delta: number | null;
  absoluteDrop: number | null;
  relativeDrop: number | null;
  deltaConfidenceInterval: ConfidenceInterval | null;
  categoryCandidate: Partial<
    Record<MatchedTaskDelta["category"], DistributionSummary>
  >;
  categoryBaseline: Partial<
    Record<MatchedTaskDelta["category"], DistributionSummary>
  >;
  matchedTaskDeltas: MatchedTaskDelta[];
  excludedCandidateMemberOrdinals: number[];
}

export interface BatchAnalysis {
  candidateBatchId: string;
  analysisVersion: number;
  calibrationPolicyVersion: number;
  baselineSnapshotSha256: string | null;
  signal: RegressionSignal;
  targets: TargetBatchAnalysis[];
}

export function regressionSignalLabel(signal: RegressionSignal): string {
  switch (signal) {
    case "stable":
      return "表现稳定";
    case "watch":
    case "likely_regression":
      // Stronger wording remains calibration-gated in production.
      return "值得复测";
    case "insufficient_data":
      return "证据不足";
  }
}

export interface ScanBatchRecord {
  id: string;
  plan: ScanBatchPlan;
  baselineSnapshot: BaselineSnapshot | null;
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
