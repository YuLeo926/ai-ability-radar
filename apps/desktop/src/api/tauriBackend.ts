import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Backend,
  Bootstrap,
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
  cancelRun: (runId) => invoke<boolean>("cancel_run", { runId }),
  listRuns: () => invoke<RunRecord[]>("list_runs"),
  getRunDetail: (runId) =>
    invoke<RunDetail | null>("get_run_detail", { runId }),
  onRunEvent: async (listener) =>
    listen<RunEvent>("run://event", ({ payload }) => listener(payload)),
  onRunError: async (listener) =>
    listen<RunErrorEvent>("run://error", ({ payload }) => listener(payload)),
};
