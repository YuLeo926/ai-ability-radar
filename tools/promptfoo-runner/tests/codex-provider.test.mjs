import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  RUNNER_PROJECT_ROOT,
  buildProviderDescriptor,
  createProviderExecutor,
  summarizeProviderResult,
} from "../provider-config.mjs";
import { createFakeProviderHarness } from "./fixtures/fake-provider.mjs";

const workspace = mkdtempSync(join(tmpdir(), "ability-radar-codex-"));

function request(overrides = {}) {
  return {
    provider: "codex",
    workspace,
    prompt: "fix the repository",
    requested_model: "gpt-5.6-terra",
    reasoning_effort: "ultra",
    time_budget_seconds: 900,
    max_turns: null,
    run_id: "45c1c4a5-67ad-422f-9022-7fe4b6838f2e",
    ...overrides,
  };
}

test("Codex descriptor locks workspace, model, full effort, permissions, and network", () => {
  const descriptor = buildProviderDescriptor(request(), {
    PATH: "C:\\runtime",
    USERPROFILE: "C:\\Users\\tester",
    CODEX_HOME: "C:\\Users\\tester\\.codex",
    ABILITY_RADAR_CODEX_ENTRY: "C:\\runtime\\codex.js",
    ABILITY_RADAR_CODEX_WRAPPER: "C:\\runtime\\ability-codex-wrapper.exe",
    ABILITY_RADAR_NODE_PROGRAM: "C:\\runtime\\node.exe",
    OPENAI_API_KEY: "must-not-pass",
    NODE_OPTIONS: "--require malicious.js",
    PRIVATE_PROVIDER_SECRET: "must-not-pass",
  });

  assert.equal(descriptor.id, "openai:codex-sdk");
  assert.equal(descriptor.basePath, RUNNER_PROJECT_ROOT);
  assert.equal(descriptor.cache, false);
  assert.deepEqual(descriptor.options.config, {
    working_dir: workspace,
    additional_directories: [],
    skip_git_repo_check: true,
    codex_path_override: "C:\\runtime\\ability-codex-wrapper.exe",
    model: "gpt-5.6-terra",
    model_reasoning_effort: "ultra",
    sandbox_mode: "workspace-write",
    network_access_enabled: false,
    web_search_enabled: false,
    web_search_mode: "disabled",
    collaboration_mode: "coding",
    approval_policy: "never",
    persist_threads: false,
    inherit_process_env: false,
    enable_streaming: false,
    deep_tracing: false,
    cli_env: {
      ABILITY_RADAR_CODEX_ENTRY: "C:\\runtime\\codex.js",
      ABILITY_RADAR_CODEX_WRAPPER: "C:\\runtime\\ability-codex-wrapper.exe",
      ABILITY_RADAR_NODE_PROGRAM: "C:\\runtime\\node.exe",
      CODEX_HOME: "C:\\Users\\tester\\.codex",
      PATH: "C:\\runtime",
      USERPROFILE: "C:\\Users\\tester",
    },
    cli_config: {
      agents: { enabled: false },
      allow_login_shell: false,
      analytics: { enabled: false },
      check_for_update_on_startup: false,
      feedback: { enabled: false },
      features: {
        apps: false,
        hooks: false,
        memories: false,
        multi_agent: false,
        remote_plugin: false,
        skill_mcp_dependency_install: false,
        web_search: false,
      },
      history: { persistence: "none" },
      project_doc_max_bytes: 0,
      sandbox_workspace_write: { network_access: false },
      shell_environment_policy: {
        inherit: "none",
        ignore_default_excludes: false,
        set: { PATH: "C:\\runtime" },
      },
      tools: { view_image: false, web_search: false },
      web_search: "disabled",
    },
  });
  assert.doesNotMatch(JSON.stringify(descriptor), /must-not-pass|NODE_OPTIONS|OPENAI_API_KEY/);
});

test("Codex default route omits model and effort overrides", () => {
  const config = buildProviderDescriptor(request({
    requested_model: "default",
    reasoning_effort: null,
  }), {
    ABILITY_RADAR_CODEX_ENTRY: "C:\\runtime\\codex.js",
    ABILITY_RADAR_CODEX_WRAPPER: "C:\\runtime\\ability-codex-wrapper.exe",
    ABILITY_RADAR_NODE_PROGRAM: "C:\\runtime\\node.exe",
  }).options.config;
  assert.equal(Object.hasOwn(config, "model"), false);
  assert.equal(Object.hasOwn(config, "model_reasoning_effort"), false);
});

test("Codex execution uses a sanitized process environment and rejects cache hits", async () => {
  const harness = createFakeProviderHarness({ result: { output: "cached", cached: true } });
  const previous = process.env.PRIVATE_PROVIDER_SECRET;
  const previousNodeOptions = process.env.NODE_OPTIONS;
  process.env.PRIVATE_PROVIDER_SECRET = "restore-after-call";
  process.env.NODE_OPTIONS = "--require malicious.js";
  process.env.ABILITY_RADAR_CODEX_ENTRY = "C:\\runtime\\codex.js";
  process.env.ABILITY_RADAR_CODEX_WRAPPER = "C:\\runtime\\ability-codex-wrapper.exe";
  process.env.ABILITY_RADAR_NODE_PROGRAM = "C:\\runtime\\node.exe";
  try {
    const execute = createProviderExecutor({
      loadProvider: harness.loadProvider,
      cacheController: { disableCache() {} },
      environmentSource: process.env,
    });
    await assert.rejects(execute(request()), (error) => error.code === "CACHE_HIT");
    assert.deepEqual(harness.events[1].context, { bustCache: true });
    assert.equal(harness.events[1].privateEnvironmentVisible, false);
    assert.equal(harness.events[1].nodeOptionsVisible, false);
    assert.equal(process.env.PRIVATE_PROVIDER_SECRET, "restore-after-call");
  } finally {
    if (previous === undefined) {
      delete process.env.PRIVATE_PROVIDER_SECRET;
    } else {
      process.env.PRIVATE_PROVIDER_SECRET = previous;
    }
    if (previousNodeOptions === undefined) {
      delete process.env.NODE_OPTIONS;
    } else {
      process.env.NODE_OPTIONS = previousNodeOptions;
    }
    delete process.env.ABILITY_RADAR_CODEX_ENTRY;
    delete process.env.ABILITY_RADAR_CODEX_WRAPPER;
    delete process.env.ABILITY_RADAR_NODE_PROGRAM;
  }
});

test("Codex result keeps bounded execution evidence without retaining commands or paths", () => {
  const summary = summarizeProviderResult(request(), {
    output: `done at ${workspace} with Bearer PRIVATE_TOKEN`,
    sessionId: "thread-123",
    tokenUsage: { prompt: 10, completion: 4, total: 14 },
    raw: JSON.stringify({
      items: [
        { type: "command_execution", command: "PRIVATE COMMAND", status: "completed", exit_code: 0 },
        { type: "file_change", patch: "PRIVATE PATCH", status: "completed" },
        { type: "command_execution", output: "PRIVATE OUTPUT", status: "failed", exit_code: 7 },
        { type: "error", message: `failure in ${workspace} api_key=PRIVATE_KEY` },
      ],
    }),
    metadata: { futureModel: "forged-visible-model" },
    futureTopLevel: { secret: "PRIVATE UNKNOWN VALUE" },
  });

  assert.equal(summary.observedModel, null);
  assert.deepEqual(summary.toolSummary, [
    { name: "command_execution", count: 2 },
    { name: "error", count: 1 },
    { name: "file_change", count: 1 },
  ]);
  assert.equal(summary.sessionPresent, true);
  assert.deepEqual(summary.commandSummary, {
    succeeded: 1,
    failed: 1,
    unknown: 0,
    exit_codes: [{ code: 0, count: 1 }, { code: 7, count: 1 }],
  });
  assert.equal(summary.fileChangeCount, 1);
  assert.deepEqual(summary.toolErrorSummary, [
    { kind: "command_execution", message: "Command exited with code 7" },
    { kind: "error", message: "failure in <path> api_key=<redacted>" },
  ]);
  assert.deepEqual(summary.providerSummary.unknown_fields, [
    "futureTopLevel",
    "metadata.futureModel",
  ]);
  assert.doesNotMatch(JSON.stringify(summary), /PRIVATE COMMAND|PRIVATE PATCH|PRIVATE OUTPUT|PRIVATE UNKNOWN VALUE|forged-visible-model|PRIVATE_TOKEN|PRIVATE_KEY|thread-123/);
});
