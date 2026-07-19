import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
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

export const tauriBackend: Backend = {
  getBootstrap: () => invoke<Bootstrap>("get_bootstrap"),
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
  listRuns: () => invoke<RunRecord[]>("list_runs"),
  getRunDetail: (runId) =>
    invoke<RunDetail | null>("get_run_detail", { runId }),
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
  onRunEvent: async (listener) =>
    listen<RunEvent>("run://event", ({ payload }) => listener(payload)),
  onRunError: async (listener) =>
    listen<RunErrorEvent>("run://error", ({ payload }) => listener(payload)),
};
