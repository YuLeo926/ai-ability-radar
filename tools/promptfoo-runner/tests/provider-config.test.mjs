import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  buildProviderDescriptor,
  createProviderExecutor,
  normalizeProviderError,
} from "../provider-config.mjs";

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
  assert.deepEqual(buildProviderDescriptor(request("codex")), {
    id: "openai:codex-sdk",
    basePath: workspace,
    cache: false,
  });
  assert.deepEqual(buildProviderDescriptor(request("claude")), {
    id: "anthropic:claude-agent-sdk",
    basePath: workspace,
    cache: false,
  });
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
  assert.deepEqual(events[1], ["openai:codex-sdk", { basePath: workspace }]);
  assert.deepEqual(events[2], ["call", "fix it"]);
});

test("provider errors normalize to the six stable public codes", () => {
  const cases = [
    ["authentication failed", "auth"],
    ["RATE_LIMIT_EXCEEDED", "quota"],
    ["ECONNRESET", "network"],
    ["model not found", "model_unavailable"],
    ["spawn ENOENT", "runtime"],
    ["something else", "unknown"],
  ];
  for (const [message, expected] of cases) {
    assert.equal(normalizeProviderError(Object.assign(new Error(message), { code: message })), expected);
  }
});
