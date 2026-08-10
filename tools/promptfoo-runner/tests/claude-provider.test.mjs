import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { buildProviderDescriptor, summarizeProviderResult } from "../provider-config.mjs";

const workspace = mkdtempSync(join(tmpdir(), "ability-radar-claude-"));

function request(overrides = {}) {
  return {
    provider: "claude",
    workspace,
    prompt: "fix the repository",
    requested_model: "claude-sonnet",
    reasoning_effort: "max",
    time_budget_seconds: 900,
    max_turns: 40,
    run_id: "dd87404f-d724-46e1-8ab2-c99dcd0d98ce",
    ...overrides,
  };
}

test("Claude descriptor uses local login with an exact tool and sandbox policy", () => {
  const config = buildProviderDescriptor(request(), {
    PATH: "C:\\runtime",
    USERPROFILE: "C:\\Users\\tester",
    APPDATA: "C:\\Users\\tester\\AppData\\Roaming",
    ANTHROPIC_API_KEY: "must-not-pass",
    CLAUDE_CONFIG_DIR: "C:\\Users\\tester\\.claude",
  }).options.config;

  assert.deepEqual(config, {
    working_dir: workspace,
    model: "claude-sonnet",
    effort: "max",
    max_turns: 40,
    apiKeyRequired: false,
    permission_mode: "dontAsk",
    persist_session: false,
    strict_mcp_config: true,
    custom_allowed_tools: ["Read", "Grep", "Glob", "Edit", "Write", "Bash"],
    disallowed_tools: ["WebSearch", "WebFetch", "AskUserQuestion"],
    setting_sources: [],
    allow_dangerously_skip_permissions: false,
    forward_subagent_text: false,
    include_partial_messages: false,
    enable_file_checkpointing: false,
    sandbox: {
      enabled: true,
      failIfUnavailable: true,
      autoAllowBashIfSandboxed: true,
      allowUnsandboxedCommands: false,
      network: {
        allowedDomains: [],
        strictAllowlist: true,
        allowAllUnixSockets: false,
        allowLocalBinding: false,
      },
      filesystem: {
        allowRead: [workspace],
        allowWrite: [workspace],
      },
    },
    env: {
      APPDATA: "C:\\Users\\tester\\AppData\\Roaming",
      CLAUDE_CONFIG_DIR: "C:\\Users\\tester\\.claude",
      PATH: "C:\\runtime",
      USERPROFILE: "C:\\Users\\tester",
    },
  });
  assert.doesNotMatch(JSON.stringify(config), /must-not-pass|ANTHROPIC_API_KEY/);
});

test("Claude result derives observed model and counts only safe tool names", () => {
  const summary = summarizeProviderResult(request(), {
    output: "done",
    sessionId: "session-456",
    tokenUsage: { prompt: 8, completion: 3, total: 11 },
    metadata: {
      toolCalls: [
        { name: "Read", input: { file: "PRIVATE" } },
        { name: "Bash", output: "PRIVATE" },
        { name: "Read" },
      ],
      modelUsage: {
        "claude-sonnet": { inputTokens: 10, outputTokens: 5 },
        "claude-haiku": { inputTokens: 1, outputTokens: 1 },
      },
      visibleModel: "forged-model",
    },
  });

  assert.equal(summary.observedModel, "claude-sonnet");
  assert.deepEqual(summary.toolSummary, [
    { name: "Bash", count: 1 },
    { name: "Read", count: 2 },
  ]);
  assert.deepEqual(summary.providerSummary.unknown_fields, ["metadata.visibleModel"]);
  assert.doesNotMatch(JSON.stringify(summary), /forged-model|PRIVATE/);
});

test("malformed Claude provider output is a runtime failure", () => {
  assert.throws(
    () => summarizeProviderResult(request(), { output: 42 }),
    (error) => error.code === "MALFORMED_PROVIDER_RESULT",
  );
});

test("Claude result rejects any tool outside the exact allowlist", () => {
  assert.throws(
    () => summarizeProviderResult(request(), {
      output: "done",
      metadata: { toolCalls: [{ name: "WebSearch" }] },
    }),
    (error) => error.code === "TOOL_POLICY_VIOLATION",
  );
});
