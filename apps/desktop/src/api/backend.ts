export type TargetKind =
  | "chat_gpt_client"
  | "claude_client"
  | "codex_cli"
  | "claude_code";

export type RunMode = "quick" | "deep";
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
  startManualRun(input: StartRunInput): Promise<RunRecord>;
  nextManualStep(runId: string): Promise<ManualStep | null>;
  submitManualAnswer(input: SubmitManualAnswerInput): Promise<TaskResult>;
  startCliRun(input: StartRunInput): Promise<RunRecord>;
  resumeManualRun(input: ResumeRunInput): Promise<RunRecord>;
  resumeCliRun(input: ResumeRunInput): Promise<RunRecord>;
  cancelRun(runId: string): Promise<boolean>;
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
  onRunEvent(listener: (event: RunEvent) => void): Promise<Unlisten>;
  onRunError(listener: (event: RunErrorEvent) => void): Promise<Unlisten>;
}
