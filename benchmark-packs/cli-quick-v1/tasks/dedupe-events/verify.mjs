import { randomBytes } from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const workspace = process.argv[2];
if (!workspace) {
  emit("VERIFIER_ERROR", 2);
} else {
  runCandidate("dedupe-events", workspace);
}

function runCandidate(taskId, workspaceRoot) {
  const runner = fileURLToPath(
    new URL("../../candidate-runner.mjs", import.meta.url),
  );
  const nonce = randomBytes(32).toString("base64url");
  const environment =
    process.platform === "win32" && process.env.SystemRoot
      ? { SystemRoot: process.env.SystemRoot }
      : {};
  const result = spawnSync(
    process.execPath,
    [
      "--no-warnings",
      "--experimental-permission",
      `--allow-fs-read=${runner}`,
      `--allow-fs-read=${workspaceRoot}`,
      runner,
      workspaceRoot,
      taskId,
    ],
    {
      cwd: workspaceRoot,
      encoding: "utf8",
      env: environment,
      input: nonce,
      maxBuffer: 1024 * 1024,
      shell: false,
      timeout: 10_000,
      windowsHide: true,
    },
  );
  const ready = `RUNNER_READY ${nonce}\n`;
  const passed = `${ready}RUNNER_PASSED ${nonce}\n`;
  const failed = `${ready}RUNNER_FAILED ${nonce}\n`;
  if (
    !result.error &&
    result.status === 0 &&
    result.stdout === passed &&
    result.stderr === ""
  ) {
    emit("TASK_PASSED", 0);
  } else if (
    !result.error &&
    result.status === 1 &&
    result.stdout === failed &&
    result.stderr === ""
  ) {
    emit("TASK_FAILED", 1);
  } else if (typeof result.stdout === "string" && result.stdout.startsWith(ready)) {
    emit("TASK_FAILED", 1);
  } else {
    emit("VERIFIER_ERROR", 2);
  }
}

function emit(marker, exitCode) {
  if (exitCode === 0) {
    console.log(marker);
  } else {
    console.error(marker);
  }
  process.exitCode = exitCode;
}
