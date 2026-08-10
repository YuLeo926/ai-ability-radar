import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  CONTRACT_VERSION,
  MAX_PROMPT_BYTES,
  ProtocolError,
  createErrorResponse,
  createSuccessResponse,
  parseRunnerRequest,
  validateRunnerResponse,
} from "../protocol.mjs";
import { runOnce } from "../run.mjs";

const workspace = mkdtempSync(join(tmpdir(), "ability-radar-runner-"));

function validRequest(overrides = {}) {
  return {
    provider: "codex",
    workspace,
    prompt: "修复测试失败，不要访问网络。",
    requested_model: "gpt-5.6-codex",
    reasoning_effort: "xhigh",
    time_budget_seconds: 600,
    max_turns: null,
    run_id: "0b1ff095-27a4-4f62-8318-e046328818db",
    ...overrides,
  };
}

test("request parser accepts the full Codex and Claude effort matrices", () => {
  for (const effort of [null, "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]) {
    assert.equal(parseRunnerRequest(JSON.stringify(validRequest({ reasoning_effort: effort }))).reasoning_effort, effort);
  }
  for (const effort of [null, "low", "medium", "high", "xhigh", "max"]) {
    const parsed = parseRunnerRequest(JSON.stringify(validRequest({
      provider: "claude",
      requested_model: "claude-sonnet",
      reasoning_effort: effort,
      max_turns: 40,
    })));
    assert.equal(parsed.reasoning_effort, effort);
  }
});

test("request parser rejects unknown fields and unsupported effort values", () => {
  assert.throws(
    () => parseRunnerRequest(JSON.stringify(validRequest({ api_key: "secret" }))),
    (error) => error instanceof ProtocolError && error.code === "unknown_field",
  );
  assert.throws(
    () => parseRunnerRequest(JSON.stringify(validRequest({ reasoning_effort: "extreme" }))),
    (error) => error instanceof ProtocolError && error.code === "unsupported_effort",
  );
  assert.throws(
    () => parseRunnerRequest(JSON.stringify(validRequest({ provider: "claude", max_turns: null }))),
    (error) => error instanceof ProtocolError && error.code === "invalid_max_turns",
  );
});

test("request parser rejects unsafe workspaces, budgets, identifiers, and text", () => {
  for (const request of [
    validRequest({ workspace: "." }),
    validRequest({ workspace: join(workspace, "missing") }),
    validRequest({ time_budget_seconds: Number.POSITIVE_INFINITY }),
    validRequest({ time_budget_seconds: 0 }),
    validRequest({ requested_model: "x".repeat(121) }),
    validRequest({ run_id: "not-a-uuid" }),
    validRequest({ prompt: "x".repeat(MAX_PROMPT_BYTES + 1) }),
  ]) {
    assert.throws(() => parseRunnerRequest(JSON.stringify(request)), ProtocolError);
  }
});

test("response schema always carries execution evidence and stable errors", () => {
  const success = createSuccessResponse(validRequest(), {
    finalText: "done",
    sessionId: "session-1",
    tokenUsage: { input: 12, output: 4, total: 16 },
    toolSummary: [{ name: "edit", count: 2 }],
    observedModel: "gpt-5.6-codex",
  });
  assert.equal(validateRunnerResponse(success).contract_version, CONTRACT_VERSION);
  assert.deepEqual(success.provider_error_code, null);

  const failure = createErrorResponse({
    runId: validRequest().run_id,
    requestedModel: validRequest().requested_model,
    providerErrorCode: "quota",
  });
  assert.equal(validateRunnerResponse(failure).provider_error_code, "quota");
  assert.throws(
    () => validateRunnerResponse({ ...success, unexpected: true }),
    (error) => error instanceof ProtocolError && error.code === "unknown_field",
  );
});

test("runner writes exactly one result JSON line and redacts diagnostics", async () => {
  const prompt = "PRIVATE_PROMPT_MARKER";
  const output = [];
  const diagnostics = [];
  const exitCode = await runOnce({
    inputText: JSON.stringify(validRequest({ prompt })),
    executeProvider: async () => {
      const error = new Error(`${prompt} ${workspace}`);
      error.code = "RATE_LIMIT_EXCEEDED";
      throw error;
    },
    stdout: { write: (value) => output.push(value) },
    stderr: { write: (value) => diagnostics.push(value) },
  });

  assert.equal(exitCode, 1);
  assert.equal(output.length, 1);
  assert.equal(output[0].trim().split("\n").length, 1);
  assert.equal(JSON.parse(output[0]).provider_error_code, "quota");
  assert.doesNotMatch(diagnostics.join(""), /PRIVATE_PROMPT_MARKER/);
  assert.doesNotMatch(diagnostics.join(""), new RegExp(workspace.replaceAll("\\", "\\\\")));
});
