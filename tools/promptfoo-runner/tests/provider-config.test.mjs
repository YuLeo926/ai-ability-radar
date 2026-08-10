import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  RUNNER_PROJECT_ROOT,
  buildProviderDescriptor,
  createProviderExecutor,
  normalizeProviderError,
} from "../provider-config.mjs";
import { createFakeProviderHarness } from "./fixtures/fake-provider.mjs";

const workspace = mkdtempSync(join(tmpdir(), "ability-radar-provider-"));

function request(provider = "codex") {
  return {
    provider,
    workspace,
    prompt: "fix it",
    requested_model: provider === "codex" ? "gpt-5.6-codex" : "claude-sonnet",
    reasoning_effort: "high",
    time_budget_seconds: 600,
    max_turns: provider === "claude" ? 40 : null,
    run_id: "146b36ca-baf6-4e17-8c19-80d08dcd98e8",
  };
}

test("provider descriptors use the pinned Promptfoo provider IDs and disable caching", () => {
  const codex = buildProviderDescriptor(request("codex"));
  const claude = buildProviderDescriptor(request("claude"));

  assert.equal(codex.id, "openai:codex-sdk");
  assert.equal(claude.id, "anthropic:claude-agent-sdk");
  assert.equal(codex.basePath, RUNNER_PROJECT_ROOT);
  assert.equal(claude.basePath, RUNNER_PROJECT_ROOT);
  assert.equal(codex.cache, false);
  assert.equal(claude.cache, false);
  assert.equal(codex.options.config.working_dir, workspace);
  assert.equal(claude.options.config.working_dir, workspace);
});

test("provider executor disables Promptfoo cache and calls the loaded provider once", async () => {
  const events = [];
  const execute = createProviderExecutor({
    cacheController: { disableCache: () => events.push("cache-disabled") },
    loadProvider: async (id, options) => {
      events.push([id, options]);
      return {
        callApi: async (prompt) => {
          events.push(["call", prompt]);
          return { output: "done", tokenUsage: { prompt: 3, completion: 2, total: 5 } };
        },
      };
    },
  });

  const result = await execute(request());
  assert.equal(result.output, "done");
  assert.equal(events[0], "cache-disabled");
  assert.equal(events[1][0], "openai:codex-sdk");
  assert.equal(events[1][1].basePath, RUNNER_PROJECT_ROOT);
  assert.equal(events[1][1].options.config.working_dir, workspace);
  assert.deepEqual(events[2], ["call", "fix it"]);
});

test("provider errors normalize to the six stable public codes", () => {
  const cases = [
    ["authentication failed", "auth"],
    ["RATE_LIMIT_EXCEEDED", "quota"],
    ["ECONNRESET", "network"],
    ["model not found", "model_unavailable"],
    ["spawn ENOENT", "runtime"],
    ["AbortError", "runtime"],
    ["something else", "unknown"],
  ];
  for (const [message, expected] of cases) {
    assert.equal(normalizeProviderError(Object.assign(new Error(message), { code: message })), expected);
  }
});

test("fake provider failures cross the executor boundary and keep stable classifications", async () => {
  const cases = [
    [Object.assign(new Error("authentication failed"), { code: "AUTH_FAILED" }), "auth"],
    [Object.assign(new Error("usage quota reached"), { code: "RATE_LIMIT" }), "quota"],
    [Object.assign(new Error("socket disconnected"), { code: "ECONNRESET" }), "network"],
    [Object.assign(new Error("requested model unavailable"), { code: "MODEL_NOT_FOUND" }), "model_unavailable"],
    [Object.assign(new Error("cancelled"), { name: "AbortError" }), "runtime"],
  ];

  for (const [failure, expected] of cases) {
    const harness = createFakeProviderHarness({ error: failure });
    const execute = createProviderExecutor({
      loadProvider: harness.loadProvider,
      cacheController: { disableCache() {} },
      environmentSource: {},
    });
    await assert.rejects(execute(request()), (error) => {
      assert.equal(normalizeProviderError(error), expected);
      return true;
    });
  }
});
