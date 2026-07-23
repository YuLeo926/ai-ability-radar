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

function isSafeDisplayText(value: unknown, maxCharacters: number): boolean {
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

function hasValidEnvironment(value: unknown): boolean {
  if (!isObject(value)) return false;
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
    !hasValidEnvironment(value.environment)
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
