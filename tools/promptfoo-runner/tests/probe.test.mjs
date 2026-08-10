import assert from "node:assert/strict";
import test from "node:test";

import { probeRuntime } from "../probe.mjs";

test("runtime probe loads the pinned Promptfoo and Codex SDK without a provider call", async () => {
  assert.deepEqual(await probeRuntime("codex"), {
    contract_version: "promptfoo-agent-v1",
    provider: "codex",
    provider_id: "openai:codex-sdk",
    promptfoo_version: "0.122.0",
    sdk_name: "@openai/codex-sdk",
    sdk_version: "0.147.0",
    runner_ready: true,
  });
});

test("runtime probe loads the pinned Claude SDK without a provider call", async () => {
  assert.deepEqual(await probeRuntime("claude"), {
    contract_version: "promptfoo-agent-v1",
    provider: "claude",
    provider_id: "anthropic:claude-agent-sdk",
    promptfoo_version: "0.122.0",
    sdk_name: "@anthropic-ai/claude-agent-sdk",
    sdk_version: "0.3.226",
    runner_ready: true,
  });
});

test("runtime probe rejects unknown providers", async () => {
  await assert.rejects(probeRuntime("other"), /unsupported provider/);
});
