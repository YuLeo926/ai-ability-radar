import type {
  Category,
  ClientSelectionCandidate,
  ClientSelectionDetection,
  FailureKind,
  ModelSource,
  ModelVerification,
  RunDetail,
  RunRecord,
  RunStatus,
  ScoreSummary,
  TargetKind,
  TaskOutcome,
  TaskResult,
} from "./backend";
import { clientSelectionCandidateKey } from "../domain/clientSelection";
import {
  BATCH_RESPONSE_LIMITS,
  supportsBatchMode,
  type AdapterLaunchKind,
  type BatchEstimate,
  type BatchExecutionSurface,
  type BatchFeatureLevel,
  type BatchMemberStatus,
  type BatchMode,
  type BatchRetryEstimate,
  type BatchStatus,
  type ExecutionAdapterIdentity,
  type NextGuidedMember,
  type ScanBatchMemberRecord,
  type ScanBatchPlan,
  type ScanBatchRecord,
  type ScanBatchTarget,
  type ScanExecutionAuthorization,
} from "../domain/batch";

const targetKinds = new Set<TargetKind>([
  "chat_gpt_client",
  "claude_client",
  "codex_cli",
  "claude_code",
]);
const modelSources = new Set<ModelSource>([
  "manual",
  "windows_accessibility",
  "cli_requested",
  "cli_reported",
  "default_route",
  "legacy_unknown",
]);
const modelVerifications = new Set<ModelVerification>([
  "user_confirmed",
  "provider_reported",
  "unverified",
  "legacy_unknown",
]);
const runStatuses = new Set<RunStatus>([
  "created",
  "running",
  "completed",
  "cancelled",
  "interrupted",
]);
const categoryOrder: Category[] = [
  "instruction_following",
  "logic",
  "code_review",
  "cli_coding",
];
const categories = new Set<Category>(categoryOrder);
const taskOutcomes = new Set<TaskOutcome>([
  "passed",
  "failed",
  "invalid",
  "cancelled",
]);
const failureKinds = new Set<FailureKind>([
  "cli_missing",
  "runtime_missing",
  "auth_expired",
  "quota_exhausted",
  "network",
  "user_cancelled",
  "app_interrupted",
  "infrastructure_timeout",
  "agent_budget_exceeded",
  "verifier_error",
  "wrong_answer",
]);
const infrastructureFailureKinds = new Set<FailureKind>([
  "cli_missing",
  "runtime_missing",
  "auth_expired",
  "quota_exhausted",
  "network",
  "user_cancelled",
  "app_interrupted",
  "infrastructure_timeout",
  "verifier_error",
]);
const clientSelectionStatuses = new Set<ClientSelectionDetection["status"]>([
  "detected",
  "multiple",
  "not_running",
  "not_exposed",
  "unsupported",
  "timed_out",
  "failed",
]);
const clientSelectionSurfaces = new Set<ClientSelectionCandidate["surface"]>([
  "chatgpt",
  "codex_desktop",
  "claude",
]);
const clientSelectionConfidences = new Set<
  ClientSelectionCandidate["confidence"]
>(["visible_selector", "best_effort"]);
const clientSelectionKeys = new Set([
  "model",
  "reasoningEffort",
  "surface",
  "source",
  "confidence",
]);
const clientSelectionDetectionKeys = new Set(["status", "candidates"]);
const forbiddenDisplayCharacter =
  /[\p{Cc}\p{Cf}\p{Default_Ignorable_Code_Point}\uD800-\uDFFF]/u;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function hasExactKeys(
  value: Record<string, unknown>,
  allowed: ReadonlySet<string>,
  required: readonly string[],
): boolean {
  const keys = Object.keys(value);
  return (
    keys.every((key) => allowed.has(key)) &&
    required.every((key) =>
      Object.prototype.hasOwnProperty.call(value, key),
    )
  );
}

function isSafeDisplayText(
  value: unknown,
  maxCharacters: number,
): value is string {
  return (
    typeof value === "string" &&
    value === value.trim() &&
    value.length > 0 &&
    Array.from(value).length <= maxCharacters &&
    !forbiddenDisplayCharacter.test(value)
  );
}

function isSafeClientSelectionCandidate(
  value: unknown,
): value is ClientSelectionCandidate {
  if (
    !isObject(value) ||
    !hasExactKeys(value, clientSelectionKeys, [
      "surface",
      "source",
      "confidence",
    ])
  ) {
    return false;
  }
  const model =
    value.model === undefined || value.model === null
      ? null
      : isSafeDisplayText(value.model, 120)
        ? value.model
        : false;
  const reasoningEffort =
    value.reasoningEffort === undefined || value.reasoningEffort === null
      ? null
      : isSafeDisplayText(value.reasoningEffort, 40)
        ? value.reasoningEffort
        : false;

  return (
    model !== false &&
    reasoningEffort !== false &&
    (model !== null || reasoningEffort !== null) &&
    clientSelectionSurfaces.has(
      value.surface as ClientSelectionCandidate["surface"],
    ) &&
    value.source === "windows_accessibility" &&
    clientSelectionConfidences.has(
      value.confidence as ClientSelectionCandidate["confidence"],
    )
  );
}

export function isSafeClientSelectionDetection(
  value: unknown,
): value is ClientSelectionDetection {
  if (
    !isObject(value) ||
    !hasExactKeys(value, clientSelectionDetectionKeys, [
      "status",
      "candidates",
    ]) ||
    !clientSelectionStatuses.has(
      value.status as ClientSelectionDetection["status"],
    ) ||
    !Array.isArray(value.candidates) ||
    value.candidates.length > 24 ||
    !value.candidates.every(isSafeClientSelectionCandidate)
  ) {
    return false;
  }

  if (value.status === "detected") {
    return value.candidates.length === 1;
  }
  if (value.status === "multiple") {
    return (
      value.candidates.length >= 2 &&
      new Set(value.candidates.map(clientSelectionCandidateKey)).size >= 2
    );
  }
  return value.candidates.length === 0;
}

function isOptionalString(value: unknown): boolean {
  return value === undefined || value === null || typeof value === "string";
}

function isCount(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value >= 0
  );
}

function isScoreValue(value: unknown): value is number {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    value >= 0 &&
    value <= 100
  );
}

function roundOne(value: number): number {
  return Math.round(value * 10) / 10;
}

function isOneDecimalScore(value: unknown): value is number {
  return isScoreValue(value) && roundOne(value) === value;
}

function hasValidScore(
  value: unknown,
  runTotalTasks: number,
  runCompletedTasks: number,
): value is ScoreSummary {
  if (!isObject(value) || !isObject(value.categoryScores)) return false;
  const categoryEntries = Object.entries(value.categoryScores);
  if (
    categoryEntries.length === 0 ||
    !isOneDecimalScore(value.abilityScore) ||
    !isCount(value.passedTasks) ||
    !isCount(value.validTasks) ||
    value.validTasks === 0 ||
    !isCount(value.totalTasks) ||
    value.totalTasks !== runTotalTasks ||
    value.passedTasks > value.validTasks ||
    value.validTasks > runCompletedTasks ||
    value.validTasks > value.totalTasks ||
    categoryEntries.length > value.validTasks ||
    !categoryEntries.every(
      ([category, score]) =>
        categories.has(category as Category) && isOneDecimalScore(score),
    )
  ) {
    return false;
  }

  const categoryMean = roundOne(
    categoryEntries.reduce((sum, [, score]) => sum + (score as number), 0) /
      categoryEntries.length,
  );
  return value.abilityScore === categoryMean;
}

function hasValidEnvironment(value: unknown, targetKind: TargetKind): boolean {
  if (!isObject(value)) return false;
  const expectedSurface: BatchExecutionSurface =
    targetKind === "chat_gpt_client" || targetKind === "claude_client"
      ? "guided_client"
      : "automated_cli";
  return (
    typeof value.osFamily === "string" &&
    typeof value.osVersion === "string" &&
    typeof value.appVersion === "string" &&
    isOptionalString(value.cliVersion) &&
    isOptionalString(value.verifierRuntimeVersion) &&
    typeof value.suiteId === "string" &&
    typeof value.suiteVersion === "string" &&
    typeof value.suiteContentSha256 === "string" &&
    typeof value.scoringRuleVersion === "string" &&
    (value.executionAdapterIdentity === undefined ||
      value.executionAdapterIdentity === null ||
      isSafeAdapterIdentity(
        value.executionAdapterIdentity,
        targetKind,
        expectedSurface,
      )) &&
    typeof value.resumed === "boolean"
  );
}

export function scoreableResultScore(result: TaskResult): number | null {
  const score = result.score;
  if (!isScoreValue(score)) return null;

  if (
    result.outcome === "passed" &&
    result.failureKind == null &&
    score === 100
  ) {
    return score;
  }

  if (
    result.outcome === "failed" &&
    score < 100 &&
    !(
      result.failureKind != null &&
      infrastructureFailureKinds.has(result.failureKind)
    )
  ) {
    return score;
  }

  return null;
}

export function isSafeRunRecord(value: unknown): value is RunRecord {
  if (!isObject(value) || !isObject(value.target)) return false;
  if (
    !isCount(value.totalTasks) ||
    !isCount(value.completedTasks) ||
    value.completedTasks > value.totalTasks
  ) {
    return false;
  }
  if (
    typeof value.id !== "string" ||
    !targetKinds.has(value.target.kind as TargetKind) ||
    typeof value.target.reportedModel !== "string" ||
    !isOptionalString(value.target.reasoningEffort) ||
    !modelSources.has(value.target.modelSource as ModelSource) ||
    !modelVerifications.has(
      value.target.modelVerification as ModelVerification,
    ) ||
    (value.mode !== "quick" && value.mode !== "deep") ||
    typeof value.suiteId !== "string" ||
    typeof value.suiteVersion !== "string" ||
    !runStatuses.has(value.status as RunStatus) ||
    typeof value.startedAt !== "string" ||
    !isOptionalString(value.finishedAt) ||
    !hasValidEnvironment(value.environment, value.target.kind as TargetKind)
  ) {
    return false;
  }
  if (
    value.status === "completed" &&
    value.completedTasks !== value.totalTasks
  ) {
    return false;
  }

  if (value.score === undefined || value.score === null) return true;
  return (
    value.status === "completed" &&
    hasValidScore(
      value.score,
      value.totalTasks,
      value.completedTasks,
    )
  );
}

function isSafeTaskResult(value: unknown): value is TaskResult {
  if (!isObject(value)) return false;
  return (
    typeof value.runId === "string" &&
    typeof value.taskId === "string" &&
    categories.has(value.category as Category) &&
    taskOutcomes.has(value.outcome as TaskOutcome) &&
    isCount(value.durationMs) &&
    (value.score === undefined ||
      value.score === null ||
      isScoreValue(value.score)) &&
    (value.failureKind === undefined ||
      value.failureKind === null ||
      failureKinds.has(value.failureKind as FailureKind)) &&
    isOptionalString(value.answerRelPath)
  );
}

function hasCoherentTaskEvidence(result: TaskResult): boolean {
  const scoreable = scoreableResultScore(result) !== null;

  if (result.outcome === "passed") return scoreable;
  if (result.outcome === "failed") {
    return (
      scoreable ||
      (result.failureKind != null &&
        infrastructureFailureKinds.has(result.failureKind))
    );
  }

  return (
    result.failureKind !== "wrong_answer" &&
    result.failureKind !== "agent_budget_exceeded"
  );
}

function recomputeScore(
  taskResults: TaskResult[],
  totalTasks: number,
): ScoreSummary | null {
  const categoryValues = new Map<Category, number[]>();
  let passedTasks = 0;
  let validTasks = 0;

  for (const result of taskResults) {
    const score = scoreableResultScore(result);
    if (score === null) continue;
    validTasks += 1;
    if (result.outcome === "passed") passedTasks += 1;
    const values = categoryValues.get(result.category);
    if (values) {
      values.push(score);
    } else {
      categoryValues.set(result.category, [score]);
    }
  }

  if (validTasks === 0) return null;

  const categoryScores: Partial<Record<Category, number>> = {};
  for (const category of categoryOrder) {
    const scores = categoryValues.get(category);
    if (!scores) continue;
    categoryScores[category] = roundOne(
      scores.reduce((sum, score) => sum + score, 0) / scores.length,
    );
  }
  const values = Object.values(categoryScores);

  return {
    abilityScore: roundOne(
      values.reduce((sum, score) => sum + score, 0) / values.length,
    ),
    passedTasks,
    validTasks,
    totalTasks,
    categoryScores,
  };
}

function scoreSummariesEqual(
  stored: ScoreSummary | null | undefined,
  recomputed: ScoreSummary | null,
): boolean {
  if (recomputed === null) return stored == null;
  if (stored == null) return false;

  const storedCategories = Object.entries(stored.categoryScores);
  const recomputedCategories = Object.entries(recomputed.categoryScores);
  if (
    stored.abilityScore !== recomputed.abilityScore ||
    stored.passedTasks !== recomputed.passedTasks ||
    stored.validTasks !== recomputed.validTasks ||
    stored.totalTasks !== recomputed.totalTasks ||
    storedCategories.length !== recomputedCategories.length
  ) {
    return false;
  }

  return recomputedCategories.every(
    ([category, score]) =>
      stored.categoryScores[category as Category] === score,
  );
}

export function isSafeRunDetail(value: unknown): value is RunDetail {
  if (
    !isObject(value) ||
    !isSafeRunRecord(value.run) ||
    !Array.isArray(value.taskResults) ||
    !value.taskResults.every(isSafeTaskResult) ||
    !value.taskResults.every(hasCoherentTaskEvidence) ||
    value.taskResults.length !== value.run.completedTasks
  ) {
    return false;
  }

  const taskIds = new Set<string>();
  for (const result of value.taskResults) {
    if (
      result.runId !== value.run.id ||
      taskIds.has(result.taskId)
    ) {
      return false;
    }
    taskIds.add(result.taskId);
  }

  if (value.run.status !== "completed") return true;
  return scoreSummariesEqual(
    value.run.score,
    recomputeScore(value.taskResults, value.run.totalTasks),
  );
}

export function isSafeRunRecordList(
  value: unknown,
): value is RunRecord[] {
  return Array.isArray(value) && value.every(isSafeRunRecord);
}

const batchModes = new Set<BatchMode>([
  "quick_comparison",
  "standard",
  "full",
]);
const batchSurfaces = new Set<BatchExecutionSurface>([
  "guided_client",
  "automated_cli",
]);
const adapterLaunchKinds = new Set<AdapterLaunchKind>([
  "guided_client",
  "native_exe",
  "reviewed_npm",
]);
const batchStatuses = new Set<BatchStatus>([
  "created",
  "running",
  "paused",
  "completed",
  "cancelled",
  "interrupted",
]);
const batchMemberStatuses = new Set<BatchMemberStatus>([
  "planned",
  "reserved",
  "launching",
  "running",
  "deferred",
  "completed",
  "invalid",
  "unavailable",
  "cancelled",
]);
const retryableBatchFailures = new Set<FailureKind>([
  "cli_missing",
  "runtime_missing",
  "auth_expired",
  "quota_exhausted",
  "network",
  "app_interrupted",
  "infrastructure_timeout",
  "verifier_error",
]);
const batchPlanKeys = new Set([
  "suiteId",
  "suiteVersion",
  "suiteContentSha256",
  "scoringRuleVersion",
  "mode",
  "seed",
  "status",
  "schedulePolicyVersion",
  "taskSessionPolicyVersion",
  "sessionIsolationPolicy",
  "targets",
  "sealedTaskBudgets",
  "costEstimate",
  "acknowledgementHash",
]);
const batchTargetKeys = new Set([
  "target",
  "routeIdentity",
  "executionAdapterIdentity",
]);
const targetSelectionKeys = new Set([
  "kind",
  "reportedModel",
  "reasoningEffort",
  "modelSource",
  "modelVerification",
]);
const routeIdentityKeys = new Set([
  "kind",
  "modelOrRoute",
  "reasoningEffort",
  "executionSurface",
  "isDefaultRoute",
]);
const adapterIdentityKeys = new Set([
  "executionSurface",
  "providerFamily",
  "launchKind",
  "publicVersion",
  "adapterContractVersion",
]);
const taskBudgetKeys = new Set(["maxTurns", "timeBudgetSecs"]);
const batchCostKeys = new Set([
  "policyVersion",
  "executionSurface",
  "mode",
  "targetCount",
  "repetitionsPerTarget",
  "tasksPerMemberRun",
  "plannedMemberRuns",
  "taskLaunches",
  "guidedInteractions",
  "maxProviderTurns",
  "summedTaskBudgetSecs",
  "expectedElapsedSecsMin",
  "expectedElapsedSecsMax",
  "providerExecutionCeilingSecs",
  "authorizationWallClockSecs",
  "issuedAt",
  "initialAcknowledgementExpiresAt",
  "tokenQuotaAmount",
  "automaticRetryBudget",
]);
const batchEstimateKeys = new Set(["plan", "capabilities"]);
const batchRecordKeys = new Set([
  "id",
  "plan",
  "status",
  "cancelRequested",
  "plannedMemberCount",
  "terminalMemberCount",
  "createdAt",
  "updatedAt",
  "members",
]);
const batchMemberKeys = new Set([
  "ordinal",
  "targetPosition",
  "repetitionIndex",
  "runId",
  "status",
  "failureKind",
  "attemptNumber",
  "updatedAt",
]);
const authorizationKeys = new Set([
  "batchId",
  "memberOrdinal",
  "attemptNumber",
  "maxTaskLaunches",
  "maxProviderTurns",
  "maxTaskBudgetSecs",
  "maxGuidedInteractions",
  "acknowledgementHash",
  "allowedFailureKind",
  "expiresAt",
  "createdAt",
]);
const retryEstimateKeys = new Set(["authorization"]);
const nextGuidedMemberKeys = new Set(["decision", "member", "target"]);
const lowerSha256 = /^[0-9a-f]{64}$/;
const uuid =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const utcInstant =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/;
const identifier = /^[a-z0-9][a-z0-9._-]*$/;

function isPositiveCount(value: unknown): value is number {
  return isCount(value) && value > 0;
}

function isSha256(value: unknown): value is string {
  return typeof value === "string" && lowerSha256.test(value);
}

function isUuid(value: unknown): value is string {
  return typeof value === "string" && uuid.test(value);
}

function instantMillis(value: unknown): number | null {
  if (typeof value !== "string" || !utcInstant.test(value)) return null;
  const millis = Date.parse(value);
  return Number.isFinite(millis) ? millis : null;
}

function isIdentifier(value: unknown, maxLength: number): value is string {
  return (
    typeof value === "string" &&
    value.length <= maxLength &&
    identifier.test(value)
  );
}

function isBatchTargetSelection(
  value: unknown,
  surface: BatchExecutionSurface,
): value is ScanBatchTarget["target"] {
  if (
    !isObject(value) ||
    !hasExactKeys(value, targetSelectionKeys, [
      "kind",
      "reportedModel",
      "reasoningEffort",
      "modelSource",
      "modelVerification",
    ]) ||
    !targetKinds.has(value.kind as TargetKind) ||
    !isSafeDisplayText(value.reportedModel, 120) ||
    /[\\/:]/.test(value.reportedModel) ||
    !(
      value.reasoningEffort === null ||
      isSafeDisplayText(value.reasoningEffort, 40)
    )
  ) {
    return false;
  }
  const isClient =
    value.kind === "chat_gpt_client" || value.kind === "claude_client";
  const isCli = value.kind === "codex_cli" || value.kind === "claude_code";
  if (
    (surface === "guided_client" && !isClient) ||
    (surface === "automated_cli" && !isCli)
  ) {
    return false;
  }
  if (surface === "guided_client") {
    return (
      (value.modelSource === "manual" ||
        value.modelSource === "windows_accessibility") &&
      value.modelVerification === "user_confirmed" &&
      value.reportedModel.toLowerCase() !== "default"
    );
  }
  if (value.reportedModel === "default") {
    return (
      value.reasoningEffort === null &&
      value.modelSource === "default_route" &&
      value.modelVerification === "unverified"
    );
  }
  return (
    value.modelSource === "cli_requested" &&
    value.modelVerification === "user_confirmed"
  );
}

function expectedAdapter(kind: TargetKind): [string, string] {
  switch (kind) {
    case "chat_gpt_client":
      return ["openai", "guided-client-v1"];
    case "claude_client":
      return ["anthropic", "guided-client-v1"];
    case "codex_cli":
      return ["openai", "codex-cli-v1"];
    case "claude_code":
      return ["anthropic", "claude-code-v1"];
  }
}

function isSafeAdapterIdentity(
  value: unknown,
  kind: TargetKind,
  surface: BatchExecutionSurface,
): value is ExecutionAdapterIdentity {
  if (
    !isObject(value) ||
    !hasExactKeys(value, adapterIdentityKeys, [
      "executionSurface",
      "providerFamily",
      "launchKind",
      "publicVersion",
      "adapterContractVersion",
    ]) ||
    value.executionSurface !== surface ||
    !batchSurfaces.has(value.executionSurface as BatchExecutionSurface) ||
    !adapterLaunchKinds.has(value.launchKind as AdapterLaunchKind) ||
    !isIdentifier(value.providerFamily, 32) ||
    !isIdentifier(value.adapterContractVersion, 64) ||
    !(
      value.publicVersion === null ||
      isSafeDisplayText(value.publicVersion, 96)
    )
  ) {
    return false;
  }
  const [provider, contract] = expectedAdapter(kind);
  if (
    value.providerFamily !== provider ||
    value.adapterContractVersion !== contract
  ) {
    return false;
  }
  if (surface === "guided_client") {
    return value.launchKind === "guided_client" && value.publicVersion === null;
  }
  return value.launchKind === "native_exe" || value.launchKind === "reviewed_npm";
}

function canonicalRouteText(value: string): string {
  return value.trim().toLowerCase().replace(/\s+/g, " ");
}

function isSafeBatchTarget(
  value: unknown,
  expectedSurface?: BatchExecutionSurface,
): value is ScanBatchTarget {
  if (
    !isObject(value) ||
    !hasExactKeys(value, batchTargetKeys, [
      "target",
      "routeIdentity",
      "executionAdapterIdentity",
    ]) ||
    !isObject(value.routeIdentity) ||
    !hasExactKeys(value.routeIdentity, routeIdentityKeys, [
      "kind",
      "modelOrRoute",
      "reasoningEffort",
      "executionSurface",
      "isDefaultRoute",
    ]) ||
    !batchSurfaces.has(
      value.routeIdentity.executionSurface as BatchExecutionSurface,
    )
  ) {
    return false;
  }
  const surface = value.routeIdentity
    .executionSurface as BatchExecutionSurface;
  if (
    (expectedSurface !== undefined && surface !== expectedSurface) ||
    !isBatchTargetSelection(value.target, surface)
  ) {
    return false;
  }
  const target = value.target;
  const route = value.routeIdentity;
  const isDefault = target.modelSource === "default_route";
  if (
    route.kind !== target.kind ||
    route.isDefaultRoute !== isDefault ||
    route.modelOrRoute !==
      (isDefault ? "default_route" : canonicalRouteText(target.reportedModel)) ||
    route.reasoningEffort !==
      (target.reasoningEffort == null
        ? null
        : canonicalRouteText(target.reasoningEffort)) ||
    !isSafeAdapterIdentity(
      value.executionAdapterIdentity,
      target.kind,
      surface,
    )
  ) {
    return false;
  }
  return true;
}

function repetitionsForMode(mode: BatchMode): number {
  if (mode === "quick_comparison") return 1;
  if (mode === "standard") return 3;
  return 5;
}

interface BatchPolicyLimits {
  maxTargets: number;
  memberCap: number;
  launchOrInteractionCap: number;
  turnCap: number;
  taskBudgetCapSecs: number;
  windowSecs: number;
}

function batchPolicyLimits(
  surface: BatchExecutionSurface,
  mode: BatchMode,
): BatchPolicyLimits | null {
  if (surface === "guided_client") {
    return mode === "quick_comparison"
      ? {
          maxTargets: 4,
          memberCap: 4,
          launchOrInteractionCap: 32,
          turnCap: 32,
          taskBudgetCapSecs: 4_320,
          windowSecs: 4 * 60 * 60,
        }
      : null;
  }
  if (mode === "quick_comparison") {
    return {
      maxTargets: 4,
      memberCap: 4,
      launchOrInteractionCap: 8,
      turnCap: 160,
      taskBudgetCapSecs: 14_400,
      windowSecs: 8 * 60 * 60,
    };
  }
  if (mode === "standard") {
    return {
      maxTargets: 4,
      memberCap: 12,
      launchOrInteractionCap: 24,
      turnCap: 480,
      taskBudgetCapSecs: 43_200,
      windowSecs: 24 * 60 * 60,
    };
  }
  return {
    maxTargets: 5,
    memberCap: 25,
    launchOrInteractionCap: 50,
    turnCap: 1_000,
    taskBudgetCapSecs: 90_000,
    windowSecs: 72 * 60 * 60,
  };
}

function isSafeBatchPlan(value: unknown): value is ScanBatchPlan {
  if (
    !isObject(value) ||
    !hasExactKeys(value, batchPlanKeys, [...batchPlanKeys]) ||
    !isSafeDisplayText(value.suiteId, 128) ||
    !isSafeDisplayText(value.suiteVersion, 64) ||
    !isSha256(value.suiteContentSha256) ||
    !isSafeDisplayText(value.scoringRuleVersion, 64) ||
    !batchModes.has(value.mode as BatchMode) ||
    !isCount(value.seed) ||
    value.status !== "created" ||
    value.schedulePolicyVersion !== 1 ||
    value.taskSessionPolicyVersion !== 1 ||
    !Array.isArray(value.targets) ||
    value.targets.length < 2 ||
    value.targets.length > BATCH_RESPONSE_LIMITS.targets ||
    !Array.isArray(value.sealedTaskBudgets) ||
    value.sealedTaskBudgets.length === 0 ||
    value.sealedTaskBudgets.length > BATCH_RESPONSE_LIMITS.taskBudgets ||
    !isSha256(value.acknowledgementHash)
  ) {
    return false;
  }
  const surface = isObject(value.costEstimate)
    ? (value.costEstimate.executionSurface as BatchExecutionSurface)
    : undefined;
  if (
    surface === undefined ||
    !batchSurfaces.has(surface) ||
    !value.targets.every((target) => isSafeBatchTarget(target, surface)) ||
    (surface === "guided_client" &&
      value.sessionIsolationPolicy !==
        "user_attested_fresh_conversation_per_task") ||
    (surface === "automated_cli" &&
      value.sessionIsolationPolicy !==
        "machine_enforced_fresh_session_and_workspace_per_task")
  ) {
    return false;
  }
  const routeKeys = value.targets.map((target) =>
    JSON.stringify(target.routeIdentity),
  );
  if (new Set(routeKeys).size !== routeKeys.length) return false;
  const budgets = value.sealedTaskBudgets;
  if (
    !budgets.every(
      (budget) =>
        isObject(budget) &&
        hasExactKeys(budget, taskBudgetKeys, ["maxTurns", "timeBudgetSecs"]) &&
        isPositiveCount(budget.maxTurns) &&
        isPositiveCount(budget.timeBudgetSecs),
    )
  ) {
    return false;
  }
  const cost = value.costEstimate;
  if (
    !isObject(cost) ||
    !hasExactKeys(cost, batchCostKeys, [...batchCostKeys]) ||
    cost.policyVersion !== 1 ||
    cost.executionSurface !== surface ||
    cost.mode !== value.mode ||
    !batchModes.has(cost.mode as BatchMode)
  ) {
    return false;
  }
  const numericKeys = [
    "targetCount",
    "repetitionsPerTarget",
    "tasksPerMemberRun",
    "plannedMemberRuns",
    "taskLaunches",
    "guidedInteractions",
    "maxProviderTurns",
    "summedTaskBudgetSecs",
    "expectedElapsedSecsMin",
    "expectedElapsedSecsMax",
    "providerExecutionCeilingSecs",
    "authorizationWallClockSecs",
    "automaticRetryBudget",
  ] as const;
  if (!numericKeys.every((key) => isCount(cost[key]))) return false;
  const mode = value.mode as BatchMode;
  const limits = batchPolicyLimits(surface, mode);
  if (limits === null) return false;
  const repetitions = repetitionsForMode(mode);
  const memberRuns = value.targets.length * repetitions;
  const taskCount = budgets.length;
  const taskLaunches = memberRuns * taskCount;
  const turnsPerMember = budgets.reduce(
    (sum, budget) => sum + (budget.maxTurns as number),
    0,
  );
  const secsPerMember = budgets.reduce(
    (sum, budget) => sum + (budget.timeBudgetSecs as number),
    0,
  );
  const elapsedBand =
    surface === "guided_client" ? [600, 900] : [1_800, 3_600];
  const issued = instantMillis(cost.issuedAt);
  const acknowledgementExpiry = instantMillis(
    cost.initialAcknowledgementExpiresAt,
  );
  return (
    issued !== null &&
    acknowledgementExpiry === issued + 15 * 60 * 1_000 &&
    value.targets.length <= limits.maxTargets &&
    cost.targetCount === value.targets.length &&
    cost.repetitionsPerTarget === repetitions &&
    cost.tasksPerMemberRun === taskCount &&
    cost.plannedMemberRuns === memberRuns &&
    memberRuns <= limits.memberCap &&
    cost.taskLaunches === taskLaunches &&
    taskLaunches <= limits.launchOrInteractionCap &&
    cost.guidedInteractions ===
      (surface === "guided_client" ? taskLaunches : 0) &&
    cost.guidedInteractions <= limits.launchOrInteractionCap &&
    cost.maxProviderTurns === memberRuns * turnsPerMember &&
    cost.maxProviderTurns <= limits.turnCap &&
    cost.summedTaskBudgetSecs === memberRuns * secsPerMember &&
    cost.summedTaskBudgetSecs <= limits.taskBudgetCapSecs &&
    cost.expectedElapsedSecsMin === memberRuns * elapsedBand[0] &&
    cost.expectedElapsedSecsMax === memberRuns * elapsedBand[1] &&
    cost.providerExecutionCeilingSecs ===
      memberRuns * secsPerMember + memberRuns * 300 &&
    cost.authorizationWallClockSecs === limits.windowSecs &&
    cost.tokenQuotaAmount === null &&
    cost.automaticRetryBudget === 0
  );
}

export function isSafeBatchEstimate(value: unknown): value is BatchEstimate {
  if (
    !isObject(value) ||
    !hasExactKeys(value, batchEstimateKeys, ["plan", "capabilities"]) ||
    !isSafeBatchPlan(value.plan) ||
    !Array.isArray(value.capabilities) ||
    value.capabilities.length !== 2 ||
    value.capabilities[0] !== "guided_quick_v1" ||
    value.capabilities[1] !== "cli_standard_v1"
  ) {
    return false;
  }
  return supportsBatchMode(
    value.capabilities as BatchFeatureLevel[],
    value.plan.costEstimate.executionSurface,
    value.plan.mode,
  );
}

function isSafeBatchMember(value: unknown): value is ScanBatchMemberRecord {
  if (
    !isObject(value) ||
    !hasExactKeys(value, batchMemberKeys, [...batchMemberKeys]) ||
    !isCount(value.ordinal) ||
    !isCount(value.targetPosition) ||
    !isCount(value.repetitionIndex) ||
    !(value.runId === null || isUuid(value.runId)) ||
    !batchMemberStatuses.has(value.status as BatchMemberStatus) ||
    !(
      value.failureKind === null ||
      failureKinds.has(value.failureKind as FailureKind)
    ) ||
    !isCount(value.attemptNumber) ||
    instantMillis(value.updatedAt) === null
  ) {
    return false;
  }
  if (value.status === "planned") {
    return value.runId === null && value.failureKind === null;
  }
  if (["reserved", "launching", "running"].includes(value.status as string)) {
    return value.runId !== null && value.failureKind === null;
  }
  if (value.status === "deferred") {
    return (
      value.failureKind !== null &&
      retryableBatchFailures.has(value.failureKind as FailureKind)
    );
  }
  return true;
}

function isTerminalBatchMember(status: BatchMemberStatus): boolean {
  return ["completed", "invalid", "unavailable", "cancelled"].includes(
    status,
  );
}

export function isSafeBatchRecord(value: unknown): value is ScanBatchRecord {
  if (
    !isObject(value) ||
    !hasExactKeys(value, batchRecordKeys, [...batchRecordKeys]) ||
    !isUuid(value.id) ||
    !isSafeBatchPlan(value.plan) ||
    !batchStatuses.has(value.status as BatchStatus) ||
    typeof value.cancelRequested !== "boolean" ||
    !isCount(value.plannedMemberCount) ||
    !isCount(value.terminalMemberCount) ||
    instantMillis(value.createdAt) === null ||
    instantMillis(value.updatedAt) === null ||
    !Array.isArray(value.members) ||
    value.members.length > BATCH_RESPONSE_LIMITS.members ||
    value.plannedMemberCount !== value.members.length ||
    !value.members.every(isSafeBatchMember)
  ) {
    return false;
  }
  const plan = value.plan as ScanBatchPlan;
  const members = value.members as ScanBatchMemberRecord[];
  if (
    members.length !== plan.costEstimate.plannedMemberRuns ||
    members.some(
      (member, ordinal) =>
        member.ordinal !== ordinal ||
        member.targetPosition >= plan.targets.length ||
        member.repetitionIndex >=
          plan.costEstimate.repetitionsPerTarget,
    )
  ) {
    return false;
  }
  const coordinateKeys = members.map(
    (member) => `${member.targetPosition}:${member.repetitionIndex}`,
  );
  if (new Set(coordinateKeys).size !== coordinateKeys.length) return false;
  const terminalCount = members.filter((member) =>
    isTerminalBatchMember(member.status),
  ).length;
  if (value.terminalMemberCount !== terminalCount) return false;
  const status = value.status as BatchStatus;
  if (status === "created") {
    return (
      !value.cancelRequested &&
      members.every((member) => member.status === "planned")
    );
  }
  if (status === "completed") {
    return !value.cancelRequested && terminalCount === members.length;
  }
  if (status === "cancelled") {
    return value.cancelRequested && terminalCount === members.length;
  }
  if (status === "paused" || status === "interrupted") {
    return members.some((member) => member.status === "deferred");
  }
  return members.some((member) =>
    ["planned", "reserved", "launching", "running"].includes(member.status),
  );
}

export function isSafeBatchRecordList(
  value: unknown,
): value is ScanBatchRecord[] {
  if (
    !Array.isArray(value) ||
    value.length > BATCH_RESPONSE_LIMITS.batchList ||
    !value.every(isSafeBatchRecord)
  ) {
    return false;
  }
  return new Set(value.map((record) => record.id)).size === value.length;
}

export function isSafeScanExecutionAuthorization(
  value: unknown,
): value is ScanExecutionAuthorization {
  if (
    !isObject(value) ||
    !hasExactKeys(value, authorizationKeys, [...authorizationKeys]) ||
    !isUuid(value.batchId) ||
    !(value.memberOrdinal === null || isCount(value.memberOrdinal)) ||
    !isPositiveCount(value.attemptNumber) ||
    !isPositiveCount(value.maxTaskLaunches) ||
    value.maxTaskLaunches > 50 ||
    !isPositiveCount(value.maxProviderTurns) ||
    value.maxProviderTurns > 1_000 ||
    !isPositiveCount(value.maxTaskBudgetSecs) ||
    value.maxTaskBudgetSecs > 90_000 ||
    !isCount(value.maxGuidedInteractions) ||
    value.maxGuidedInteractions > 32 ||
    !(
      value.maxGuidedInteractions === 0 ||
      value.maxGuidedInteractions === value.maxTaskLaunches
    ) ||
    !isSha256(value.acknowledgementHash) ||
    !(
      value.allowedFailureKind === null ||
      retryableBatchFailures.has(value.allowedFailureKind as FailureKind)
    )
  ) {
    return false;
  }
  const created = instantMillis(value.createdAt);
  const expires = instantMillis(value.expiresAt);
  if (
    created === null ||
    expires === null ||
    expires <= created ||
    expires - created > 72 * 60 * 60 * 1_000
  ) {
    return false;
  }
  if (value.memberOrdinal === null) {
    return value.attemptNumber === 1 && value.allowedFailureKind === null;
  }
  return value.allowedFailureKind !== null;
}

export function isSafeBatchRetryEstimate(
  value: unknown,
): value is BatchRetryEstimate {
  return (
    isObject(value) &&
    hasExactKeys(value, retryEstimateKeys, ["authorization"]) &&
    isSafeScanExecutionAuthorization(value.authorization) &&
    value.authorization.memberOrdinal !== null
  );
}

export function isSafeNextGuidedMember(
  value: unknown,
): value is NextGuidedMember {
  if (
    !isObject(value) ||
    !hasExactKeys(value, nextGuidedMemberKeys, [
      "decision",
      "member",
      "target",
    ]) ||
    !["runnable", "blocked_by_active", "exhausted"].includes(
      value.decision as string,
    )
  ) {
    return false;
  }
  if (value.decision === "exhausted") {
    return value.member === null && value.target === null;
  }
  if (
    !isSafeBatchMember(value.member) ||
    !isSafeBatchTarget(value.target, "guided_client")
  ) {
    return false;
  }
  return value.decision === "runnable"
    ? value.member.status === "planned"
    : ["reserved", "launching", "running"].includes(value.member.status);
}
