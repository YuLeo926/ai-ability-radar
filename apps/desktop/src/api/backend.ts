import type {
  AuthorizeBatchExecutionInput,
  AuthorizeBatchRetryInput,
  BatchEstimate,
  BatchAnalysis,
  BatchFeatureLevel,
  BatchPlanInput,
  BatchRetryEstimate,
  CreateAcknowledgedBatchInput,
  DeclineGuidedBatchAttestationInput,
  EstimateBatchRetryInput,
  ExecutionAdapterIdentity,
  NextGuidedMember,
  ScanBatchRecord,
  ScanExecutionAuthorization,
  SubmitGuidedBatchAnswerInput,
} from "../domain/batch";

export type TargetKind =
  | "chat_gpt_client"
  | "claude_client"
  | "codex_cli"
  | "claude_code";

export type ModelSource =
  | "manual"
  | "windows_accessibility"
  | "cli_requested"
  | "cli_reported"
  | "default_route"
  | "legacy_unknown";

export type ModelVerification =
  | "user_confirmed"
  | "provider_reported"
  | "unverified"
  | "legacy_unknown";

export interface ClientSelectionCandidate {
  model?: string | null;
  reasoningEffort?: string | null;
  surface: "chatgpt" | "codex_desktop" | "claude";
  source: "windows_accessibility";
  confidence: "visible_selector" | "best_effort";
}

export interface ClientSelectionDetection {
  status:
    | "detected"
    | "multiple"
    | "not_running"
    | "not_exposed"
    | "unsupported"
    | "timed_out"
    | "failed";
  candidates: ClientSelectionCandidate[];
}

export type RunMode = "quick" | "deep";
export type AvailabilityStatus =
  | "ready"
  | "needs_login"
  | "not_found"
  | "runtime_missing"
  | "entry_inaccessible"
  | "version_probe_failed";

export type LaunchSource = "native_exe" | "reviewed_npm";

export type RunStatus =
  | "created"
  | "running"
  | "completed"
  | "cancelled"
  | "interrupted";
export type TaskOutcome = "passed" | "failed" | "invalid" | "cancelled";
export type FailureKind =
  | "cli_missing"
  | "runtime_missing"
  | "auth_expired"
  | "quota_exhausted"
  | "network"
  | "user_cancelled"
  | "app_interrupted"
  | "infrastructure_timeout"
  | "agent_budget_exceeded"
  | "verifier_error"
  | "wrong_answer";
export type Category =
  | "instruction_following"
  | "logic"
  | "code_review"
  | "cli_coding";

export interface TargetSelection {
  kind: TargetKind;
  reportedModel: string;
  reasoningEffort?: string | null;
  modelSource: ModelSource;
  modelVerification: ModelVerification;
}

export interface PrerequisiteStatus {
  name: string;
  available: boolean;
  version?: string | null;
}

export interface TargetAvailability {
  kind: TargetKind;
  installed: boolean;
  version?: string | null;
  authState: "unknown" | "ready" | "needs_login";
  status: AvailabilityStatus;
  source?: LaunchSource | null;
  prerequisites: PrerequisiteStatus[];
}

export interface PackSummary {
  id: string;
  version: string;
  title: string;
  taskCount: number;
  estimatedMinutes: string;
}

export interface Bootstrap {
  targets: TargetAvailability[];
  clientPack: PackSummary;
  cliPack: PackSummary;
  batchCapabilities: BatchFeatureLevel[];
}

export interface ScoreSummary {
  abilityScore: number;
  passedTasks: number;
  validTasks: number;
  totalTasks: number;
  categoryScores: Partial<Record<Category, number>>;
}

export interface EnvironmentFingerprint {
  osFamily: string;
  osVersion: string;
  appVersion: string;
  cliVersion?: string | null;
  verifierRuntimeVersion?: string | null;
  suiteId: string;
  suiteVersion: string;
  suiteContentSha256: string;
  scoringRuleVersion: string;
  executionAdapterIdentity?: ExecutionAdapterIdentity | null;
  resumed: boolean;
}

export interface RunRecord {
  id: string;
  target: TargetSelection;
  mode: RunMode;
  suiteId: string;
  suiteVersion: string;
  status: RunStatus;
  startedAt: string;
  finishedAt?: string | null;
  totalTasks: number;
  completedTasks: number;
  environment: EnvironmentFingerprint;
  score?: ScoreSummary | null;
}

export interface TaskResult {
  runId: string;
  taskId: string;
  category: Category;
  outcome: TaskOutcome;
  score?: number | null;
  failureKind?: FailureKind | null;
  durationMs: number;
  answerRelPath?: string | null;
}

export interface ManualStep {
  runId: string;
  taskId: string;
  taskNumber: number;
  totalTasks: number;
  prompt: string;
}

export interface RunDetail {
  run: RunRecord;
  taskResults: TaskResult[];
}

export interface StartRunInput {
  target: TargetSelection;
  mode: "quick";
}

export interface SubmitManualAnswerInput {
  runId: string;
  taskId: string;
  answer: string;
}

export interface ResumeRunInput {
  runId: string;
  expectedTarget: {
    kind: TargetKind;
    reportedModel: string;
    reasoningEffort: string | null;
    modelSource: ModelSource;
    modelVerification: ModelVerification;
  };
}

export interface RunEvent {
  runId: string;
  kind: "task_started" | "task_finished" | "run_finished";
  taskId?: string | null;
  completedTasks: number;
  totalTasks: number;
}

export interface RunErrorEvent {
  runId: string;
  message: string;
}

export interface DataSettings {
  rawRetentionDays: number | null;
  cleanupPending: boolean;
}

export interface FullBackupInput {
  acknowledgedUnencryptedRawData: true;
}

export type Unlisten = () => void;

export interface Backend {
  getBootstrap(): Promise<Bootstrap>;
  detectClientSelection(
    target: "chat_gpt_client" | "claude_client",
  ): Promise<ClientSelectionDetection>;
  startManualRun(input: StartRunInput): Promise<RunRecord>;
  nextManualStep(runId: string): Promise<ManualStep | null>;
  submitManualAnswer(input: SubmitManualAnswerInput): Promise<TaskResult>;
  startCliRun(input: StartRunInput): Promise<RunRecord>;
  resumeManualRun(input: ResumeRunInput): Promise<RunRecord>;
  resumeCliRun(input: ResumeRunInput): Promise<RunRecord>;
  cancelRun(runId: string): Promise<boolean>;
  interruptManualRun(runId: string): Promise<boolean>;
  listRuns(): Promise<RunRecord[]>;
  getRunDetail(runId: string): Promise<RunDetail | null>;
  exportPublicReport(runId: string): Promise<string | null>;
  deleteRawArtifacts(runId: string): Promise<void>;
  deleteRun(runId: string): Promise<boolean>;
  deleteTargetHistory(
    target: TargetKind,
    expectedRunIds: string[],
  ): Promise<number>;
  getDataSettings(): Promise<DataSettings>;
  setRawRetention(rawRetentionDays: number | null): Promise<number>;
  exportFullBackup(input: FullBackupInput): Promise<boolean>;
  estimateBatch(input: BatchPlanInput): Promise<BatchEstimate>;
  createAcknowledgedBatch(
    input: CreateAcknowledgedBatchInput,
  ): Promise<ScanBatchRecord>;
  getBatch(batchId: string): Promise<ScanBatchRecord | null>;
  getBatchAnalysis(batchId: string): Promise<BatchAnalysis>;
  listBatches(): Promise<ScanBatchRecord[]>;
  authorizeBatchExecution(
    input: AuthorizeBatchExecutionInput,
  ): Promise<ScanExecutionAuthorization>;
  estimateBatchRetry(
    input: EstimateBatchRetryInput,
  ): Promise<BatchRetryEstimate>;
  authorizeBatchRetry(
    input: AuthorizeBatchRetryInput,
  ): Promise<ScanExecutionAuthorization>;
  startBatch(batchId: string): Promise<ScanBatchRecord>;
  resumeBatch(batchId: string): Promise<ScanBatchRecord>;
  pauseBatch(batchId: string): Promise<ScanBatchRecord>;
  cancelBatch(batchId: string): Promise<ScanBatchRecord>;
  getNextGuidedMember(batchId: string): Promise<NextGuidedMember>;
  beginGuidedBatchMember(batchId: string): Promise<RunRecord>;
  submitGuidedBatchAnswer(
    input: SubmitGuidedBatchAnswerInput,
  ): Promise<TaskResult>;
  declineGuidedBatchAttestation(
    input: DeclineGuidedBatchAttestationInput,
  ): Promise<ScanBatchRecord>;
  onRunEvent(listener: (event: RunEvent) => void): Promise<Unlisten>;
  onRunError(listener: (event: RunErrorEvent) => void): Promise<Unlisten>;
}

type BatchBackendMethods = Pick<
  Backend,
  | "estimateBatch"
  | "createAcknowledgedBatch"
  | "getBatch"
  | "getBatchAnalysis"
  | "listBatches"
  | "authorizeBatchExecution"
  | "estimateBatchRetry"
  | "authorizeBatchRetry"
  | "startBatch"
  | "resumeBatch"
  | "pauseBatch"
  | "cancelBatch"
  | "getNextGuidedMember"
  | "beginGuidedBatchMember"
  | "submitGuidedBatchAnswer"
  | "declineGuidedBatchAttestation"
>;

async function unsupportedBatchCall(): Promise<never> {
  throw new Error("batch backend is not configured in this test");
}

export const unsupportedBatchBackend: BatchBackendMethods = {
  estimateBatch: unsupportedBatchCall,
  createAcknowledgedBatch: unsupportedBatchCall,
  getBatch: unsupportedBatchCall,
  getBatchAnalysis: unsupportedBatchCall,
  listBatches: unsupportedBatchCall,
  authorizeBatchExecution: unsupportedBatchCall,
  estimateBatchRetry: unsupportedBatchCall,
  authorizeBatchRetry: unsupportedBatchCall,
  startBatch: unsupportedBatchCall,
  resumeBatch: unsupportedBatchCall,
  pauseBatch: unsupportedBatchCall,
  cancelBatch: unsupportedBatchCall,
  getNextGuidedMember: unsupportedBatchCall,
  beginGuidedBatchMember: unsupportedBatchCall,
  submitGuidedBatchAnswer: unsupportedBatchCall,
  declineGuidedBatchAttestation: unsupportedBatchCall,
};
