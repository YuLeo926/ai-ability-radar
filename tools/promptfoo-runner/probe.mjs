import { readFile, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

import { CONTRACT_VERSION } from "./protocol.mjs";

const PROJECT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const PACKAGES = Object.freeze({
  codex: {
    providerId: "openai:codex-sdk",
    sdkName: "@openai/codex-sdk",
    manifest: resolve(PROJECT_ROOT, "node_modules", "@openai", "codex-sdk", "package.json"),
  },
  claude: {
    providerId: "anthropic:claude-agent-sdk",
    sdkName: "@anthropic-ai/claude-agent-sdk",
    manifest: resolve(
      PROJECT_ROOT,
      "node_modules",
      "@anthropic-ai",
      "claude-agent-sdk",
      "package.json",
    ),
  },
});

async function packageVersion(path) {
  const value = JSON.parse(await readFile(path, "utf8"));
  if (typeof value.version !== "string") {
    throw new Error("missing package version");
  }
  return value.version;
}

export async function probeRuntime(provider) {
  const selected = PACKAGES[provider];
  if (!selected) {
    throw new Error("unsupported provider");
  }
  await Promise.all([import("promptfoo"), import(selected.sdkName)]);
  const runner = await stat(resolve(PROJECT_ROOT, "tools", "promptfoo-runner", "run.mjs"));
  return {
    contract_version: CONTRACT_VERSION,
    provider,
    provider_id: selected.providerId,
    promptfoo_version: await packageVersion(resolve(PROJECT_ROOT, "node_modules", "promptfoo", "package.json")),
    sdk_name: selected.sdkName,
    sdk_version: await packageVersion(selected.manifest),
    runner_ready: runner.isFile(),
  };
}

async function main() {
  try {
    const result = await probeRuntime(process.argv[2]);
    process.stdout.write(`${JSON.stringify(result)}\n`);
  } catch {
    process.stderr.write("promptfoo-agent-v1 probe failed\n");
    process.exitCode = 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
