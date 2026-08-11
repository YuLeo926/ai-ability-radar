import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AgentEvidenceResponse,
  Backend,
  Bootstrap,
  DataSettings,
  ManualStep,
  RunDetail,
  RunErrorEvent,
  RunEvent,
  RunRecord,
  TaskResult,
} from "./backend";
import type {
  BatchEstimate,
  BatchAnalysis,
  BatchRetryEstimate,
  NextGuidedMember,
  ScanBatchRecord,
  ScanExecutionAuthorization,
} from "../domain/batch";
import {
  isSafeBatchEstimate,
  isSafeBatchAnalysis,
  isSafeBatchRecord,
  isSafeBatchRecordList,
  isSafeBatchRetryEstimate,
  isSafeAgentEvidenceResponse,
  isSafeClientSelectionDetection,
  isSafeNextGuidedMember,
  isSafeGuidedBatchRunRecord,
  isSafeGuidedBatchTaskResult,
  isSafeScanExecutionAuthorization,
} from "./runtimeValidation";

async function invokeValidated<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  validator: (value: unknown) => value is T,
): Promise<T> {
  const value =
    args === undefined
      ? await invoke<unknown>(command)
      : await invoke<unknown>(command, args);
  if (!validator(value)) {
    throw new Error(`Batch command ${command} returned invalid local data`);
  }
  return value;
}

function batchIdInput(batchId: string) {
  return { input: { batchId } };
}

export const tauriBackend: Backend = {
  getBootstrap: () => invoke<Bootstrap>("get_bootstrap"),
  detectClientSelection: async (target) => {
    const value = await invoke<unknown>("detect_client_selection", {
      input: { target },
    });
    if (!isSafeClientSelectionDetection(value)) {
      throw new Error("本地模型识别返回了无效数据");
    }
    return value;
  },
  startManualRun: (input) =>
    invoke<RunRecord>("start_manual_run", { input }),
  nextManualStep: (runId) =>
    invoke<ManualStep | null>("next_manual_step", { runId }),
  submitManualAnswer: (input) =>
    invoke<TaskResult>("submit_manual_answer", { input }),
  startCliRun: (input) => invoke<RunRecord>("start_cli_run", { input }),
  resumeManualRun: (input) =>
    invoke<RunRecord>("resume_manual_run", { input }),
  resumeCliRun: (input) =>
    invoke<RunRecord>("resume_cli_run", { input }),
  cancelRun: (runId) => invoke<boolean>("cancel_run", { runId }),
  interruptManualRun: (runId) =>
    invoke<boolean>("interrupt_manual_run", { runId }),
  listRuns: () => invoke<RunRecord[]>("list_runs"),
  getRunDetail: (runId) =>
    invoke<RunDetail | null>("get_run_detail", { runId }),
  getAgentExecutionDetail: (runId, taskId) =>
    invokeValidated<AgentEvidenceResponse>(
      "get_agent_execution_detail",
      { input: { runId, taskId } },
      isSafeAgentEvidenceResponse,
    ),
  exportPublicReport: (runId) =>
    invoke<string | null>("export_public_report", { input: { runId } }),
  deleteRawArtifacts: (runId) =>
    invoke<void>("delete_raw_artifacts", { input: { runId } }),
  deleteRun: (runId) =>
    invoke<boolean>("delete_run", { input: { runId } }),
  deleteTargetHistory: (target, expectedRunIds) =>
    invoke<number>("delete_target_history", {
      input: { target, expectedRunIds },
    }),
  getDataSettings: () => invoke<DataSettings>("get_data_settings"),
  setRawRetention: (rawRetentionDays) =>
    invoke<number>("set_raw_retention", { input: { rawRetentionDays } }),
  exportFullBackup: (input) =>
    invoke<boolean>("export_full_backup", { input }),
  estimateBatch: (input) =>
    invokeValidated<BatchEstimate>(
      "estimate_batch",
      { input },
      isSafeBatchEstimate,
    ),
  createAcknowledgedBatch: (input) =>
    invokeValidated<ScanBatchRecord>(
      "create_acknowledged_batch",
      { input },
      isSafeBatchRecord,
    ),
  getBatch: async (batchId) => {
    const value = await invoke<unknown>("get_batch", batchIdInput(batchId));
    if (value !== null && !isSafeBatchRecord(value)) {
      throw new Error("Batch command get_batch returned invalid local data");
    }
    return value;
  },
  getBatchAnalysis: (batchId) =>
    invokeValidated<BatchAnalysis>(
      "get_batch_analysis",
      batchIdInput(batchId),
      isSafeBatchAnalysis,
    ),
  listBatches: () =>
    invokeValidated<ScanBatchRecord[]>(
      "list_batches",
      undefined,
      isSafeBatchRecordList,
    ),
  exportPublicBatchReport: (batchId) =>
    invoke<string | null>("export_public_batch_report", batchIdInput(batchId)),
  deleteBatch: (batchId, deleteOwnedRuns) =>
    invoke<boolean>("delete_batch", { input: { batchId, deleteOwnedRuns } }),
  authorizeBatchExecution: (input) =>
    invokeValidated<ScanExecutionAuthorization>(
      "authorize_batch_execution",
      { input },
      isSafeScanExecutionAuthorization,
    ),
  estimateBatchRetry: (input) =>
    invokeValidated<BatchRetryEstimate>(
      "estimate_batch_retry",
      { input },
      isSafeBatchRetryEstimate,
    ),
  authorizeBatchRetry: (input) =>
    invokeValidated<ScanExecutionAuthorization>(
      "authorize_batch_retry",
      { input },
      isSafeScanExecutionAuthorization,
    ),
  startBatch: (batchId) =>
    invokeValidated<ScanBatchRecord>(
      "start_batch",
      batchIdInput(batchId),
      isSafeBatchRecord,
    ),
  resumeBatch: (batchId) =>
    invokeValidated<ScanBatchRecord>(
      "resume_batch",
      batchIdInput(batchId),
      isSafeBatchRecord,
    ),
  pauseBatch: (batchId) =>
    invokeValidated<ScanBatchRecord>(
      "pause_batch",
      batchIdInput(batchId),
      isSafeBatchRecord,
    ),
  cancelBatch: (batchId) =>
    invokeValidated<ScanBatchRecord>(
      "cancel_batch",
      batchIdInput(batchId),
      isSafeBatchRecord,
    ),
  getNextGuidedMember: (batchId) =>
    invokeValidated<NextGuidedMember>(
      "get_next_guided_member",
      batchIdInput(batchId),
      isSafeNextGuidedMember,
    ),
  beginGuidedBatchMember: (batchId) =>
    invokeValidated<RunRecord>(
      "begin_guided_batch_member",
      batchIdInput(batchId),
      isSafeGuidedBatchRunRecord,
    ),
  submitGuidedBatchAnswer: (input) =>
    invokeValidated<TaskResult>(
      "submit_guided_batch_answer",
      { input },
      isSafeGuidedBatchTaskResult,
    ),
  declineGuidedBatchAttestation: (input) =>
    invokeValidated<ScanBatchRecord>(
      "decline_guided_batch_attestation",
      { input },
      isSafeBatchRecord,
    ),
  onRunEvent: async (listener) =>
    listen<RunEvent>("run://event", ({ payload }) => listener(payload)),
  onRunError: async (listener) =>
    listen<RunErrorEvent>("run://error", ({ payload }) => listener(payload)),
};
