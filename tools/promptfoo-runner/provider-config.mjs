import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { cache, loadApiProvider } from "promptfoo";

export const RUNNER_PROJECT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

const PROVIDER_IDS = Object.freeze({
  codex: "openai:codex-sdk",
  claude: "anthropic:claude-agent-sdk",
});
const SAFE_ENVIRONMENT_KEYS = new Set([
  "ABILITY_RADAR_CODEX_ENTRY",
  "ABILITY_RADAR_CODEX_WRAPPER",
  "ABILITY_RADAR_NODE_PROGRAM",
  "APPDATA",
  "CLAUDE_CONFIG_DIR",
  "CODEX_HOME",
  "COMSPEC",
  "HOME",
  "HOMEDRIVE",
  "HOMEPATH",
  "LANG",
  "LC_ALL",
  "LOCALAPPDATA",
  "NODE_EXTRA_CA_CERTS",
  "PATH",
  "PATHEXT",
  "SHELL",
  "SSL_CERT_DIR",
  "SSL_CERT_FILE",
  "SYSTEMROOT",
  "TEMP",
  "TERM",
  "TMP",
  "TMPDIR",
  "USER",
  "USERNAME",
  "USERPROFILE",
]);
const SAFE_SHELL_ENVIRONMENT_KEYS = Object.freeze([
  "COMSPEC",
  "LANG",
  "LC_ALL",
  "PATH",
  "PATHEXT",
  "SHELL",
  "SYSTEMROOT",
  "TEMP",
  "TERM",
  "TMP",
  "TMPDIR",
]);
const CLAUDE_ALLOWED_TOOLS = Object.freeze(["Read", "Grep", "Glob", "Edit", "Write", "Bash"]);
const CLAUDE_ALLOWED_TOOL_SET = new Set(CLAUDE_ALLOWED_TOOLS);
const CLAUDE_DISALLOWED_TOOLS = Object.freeze(["WebSearch", "WebFetch", "AskUserQuestion"]);
const KNOWN_TOP_LEVEL_RESULT_FIELDS = new Set([
  "output",
  "tokenUsage",
  "cost",
  "metadata",
  "raw",
  "sessionId",
  "cached",
  "error",
]);
const KNOWN_METADATA_FIELDS = new Set([
  "toolCalls",
  "modelUsage",
  "numTurns",
  "durationMs",
  "durationApiMs",
  "permissionDenials",
  "skillCalls",
  "terminalReason",
  "skills",
]);
const SAFE_SUMMARY_FIELD = /^[A-Za-z][A-Za-z0-9_.-]{0,127}$/;
const SAFE_TOOL_NAME = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/;
const SAFE_MODEL = /^[^\p{Cc}\p{Cf}\p{Default_Ignorable_Code_Point}]{1,120}$/u;
const MAX_RAW_EVIDENCE_BYTES = 4 * 1024 * 1024;

export function sanitizeProviderEnvironment(source = {}) {
  const safe = {};
  for (const key of Object.keys(source).sort()) {
    const value = source[key];
    if (
      SAFE_ENVIRONMENT_KEYS.has(key.toUpperCase()) &&
      typeof value === "string" &&
      value.length > 0 &&
      !value.includes("\0")
    ) {
      safe[key] = value;
    }
  }
  return safe;
}

function codexConfig(request, environment) {
  const wrapper = environment.ABILITY_RADAR_CODEX_WRAPPER;
  const entry = environment.ABILITY_RADAR_CODEX_ENTRY;
  const nodeProgram = environment.ABILITY_RADAR_NODE_PROGRAM;
  if (!wrapper || !entry || !nodeProgram) {
    throw new Error("Codex isolation runtime is unavailable");
  }
  const shellEnvironment = Object.fromEntries(
    SAFE_SHELL_ENVIRONMENT_KEYS.flatMap((key) =>
      typeof environment[key] === "string" ? [[key, environment[key]]] : [],
    ),
  );
  return {
    working_dir: request.workspace,
    additional_directories: [],
    skip_git_repo_check: true,
    codex_path_override: wrapper,
    ...(request.requested_model === "default" ? {} : { model: request.requested_model }),
    ...(request.reasoning_effort === null ? {} : { model_reasoning_effort: request.reasoning_effort }),
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
    cli_env: environment,
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
        set: shellEnvironment,
      },
      tools: { view_image: false, web_search: false },
      web_search: "disabled",
    },
  };
}

function claudeConfig(request, environment) {
  return {
    working_dir: request.workspace,
    ...(request.requested_model === "default" ? {} : { model: request.requested_model }),
    ...(request.reasoning_effort === null ? {} : { effort: request.reasoning_effort }),
    max_turns: request.max_turns,
    apiKeyRequired: false,
    permission_mode: "dontAsk",
    persist_session: false,
    strict_mcp_config: true,
    custom_allowed_tools: [...CLAUDE_ALLOWED_TOOLS],
    disallowed_tools: [...CLAUDE_DISALLOWED_TOOLS],
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
        allowRead: [request.workspace],
        allowWrite: [request.workspace],
      },
    },
    env: environment,
  };
}

export function buildProviderDescriptor(request, environmentSource = process.env) {
  const id = PROVIDER_IDS[request.provider];
  if (!id) {
    const error = new Error("unsupported_provider");
    error.code = "PROVIDER_RUNTIME";
    throw error;
  }
  const environment = sanitizeProviderEnvironment(environmentSource);
  const config = request.provider === "codex"
    ? codexConfig(request, environment)
    : claudeConfig(request, environment);
  return Object.freeze({
    id,
    basePath: RUNNER_PROJECT_ROOT,
    cache: false,
    options: { config },
  });
}

async function withSanitizedProcessEnvironment(environment, operation) {
  const original = { ...process.env };
  for (const key of Object.keys(process.env)) {
    delete process.env[key];
  }
  Object.assign(process.env, environment);
  try {
    return await operation();
  } finally {
    for (const key of Object.keys(process.env)) {
      delete process.env[key];
    }
    Object.assign(process.env, original);
  }
}

async function closeProvider(provider) {
  if (typeof provider?.shutdown === "function") {
    await provider.shutdown();
  } else if (typeof provider?.cleanup === "function") {
    await provider.cleanup();
  }
}

export function createProviderExecutor({
  loadProvider = loadApiProvider,
  cacheController = cache,
  environmentSource = process.env,
} = {}) {
  return async function executeProvider(request) {
    cacheController.disableCache();
    const safeEnvironment = sanitizeProviderEnvironment(environmentSource);
    const descriptor = buildProviderDescriptor(request, safeEnvironment);
    return withSanitizedProcessEnvironment(safeEnvironment, async () => {
      const provider = await loadProvider(descriptor.id, {
        basePath: descriptor.basePath,
        options: descriptor.options,
      });
      if (!provider || typeof provider.callApi !== "function") {
        const error = new Error("provider runtime did not expose callApi");
        error.code = "PROVIDER_RUNTIME";
        throw error;
      }
      try {
        const result = await provider.callApi(request.prompt, { bustCache: true });
        if (result?.cached) {
          const error = new Error("cached provider result rejected");
          error.code = "CACHE_HIT";
          throw error;
        }
        if (result?.error) {
          const error = new Error("provider call failed");
          error.code = result.error;
          throw error;
        }
        return result ?? {};
      } finally {
        await closeProvider(provider);
      }
    });
  };
}

function safeNonnegativeInteger(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function summarizeUnknownFields(result, metadata, discardedFieldCount) {
  const unknown = [];
  for (const key of Object.keys(result)) {
    if (!KNOWN_TOP_LEVEL_RESULT_FIELDS.has(key)) {
      if (SAFE_SUMMARY_FIELD.test(key)) {
        unknown.push(key);
      } else {
        discardedFieldCount.count += 1;
      }
    }
  }
  for (const key of Object.keys(metadata)) {
    if (!KNOWN_METADATA_FIELDS.has(key)) {
      const qualified = `metadata.${key}`;
      if (SAFE_SUMMARY_FIELD.test(qualified)) {
        unknown.push(qualified);
      } else {
        discardedFieldCount.count += 1;
      }
    }
  }
  return [...new Set(unknown)].sort((left, right) => left.localeCompare(right, "en")).slice(0, 64);
}

function countTools(names, discardedFieldCount) {
  const counts = new Map();
  for (const name of names) {
    if (typeof name !== "string" || !SAFE_TOOL_NAME.test(name)) {
      discardedFieldCount.count += 1;
      continue;
    }
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort(([left], [right]) => left.localeCompare(right, "en"))
    .slice(0, 64)
    .map(([name, count]) => ({ name, count }));
}

function codexToolNames(raw, discardedFieldCount) {
  if (typeof raw !== "string" || Buffer.byteLength(raw, "utf8") > MAX_RAW_EVIDENCE_BYTES) {
    if (raw !== undefined) {
      discardedFieldCount.count += 1;
    }
    return [];
  }
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed?.items)) {
      return [];
    }
    return parsed.items.map((item) => item?.type);
  } catch {
    discardedFieldCount.count += 1;
    return [];
  }
}

function claudeObservedModel(modelUsage) {
  if (modelUsage === null || typeof modelUsage !== "object" || Array.isArray(modelUsage)) {
    return null;
  }
  let observed = null;
  let topUsage = -1;
  for (const [model, usage] of Object.entries(modelUsage)) {
    if (!SAFE_MODEL.test(model) || model.trim() !== model || model === "undefined") {
      continue;
    }
    const input = safeNonnegativeInteger(usage?.inputTokens) ?? 0;
    const output = safeNonnegativeInteger(usage?.outputTokens) ?? 0;
    if (input + output > topUsage) {
      observed = model;
      topUsage = input + output;
    }
  }
  return observed;
}

function enforceClaudeToolPolicy(toolNames) {
  if (toolNames.some((name) => typeof name !== "string" || !CLAUDE_ALLOWED_TOOL_SET.has(name))) {
    const error = new Error("Claude provider returned a tool outside the allowlist");
    error.code = "TOOL_POLICY_VIOLATION";
    throw error;
  }
}

export function summarizeProviderResult(request, result) {
  if (result === null || typeof result !== "object" || Array.isArray(result) || typeof result.output !== "string") {
    const error = new Error("malformed provider result");
    error.code = "MALFORMED_PROVIDER_RESULT";
    throw error;
  }
  const metadata = result.metadata !== null && typeof result.metadata === "object" && !Array.isArray(result.metadata)
    ? result.metadata
    : {};
  const discardedFieldCount = { count: 0 };
  const toolNames = request.provider === "claude"
    ? Array.isArray(metadata.toolCalls) ? metadata.toolCalls.map((tool) => tool?.name) : []
    : codexToolNames(result.raw, discardedFieldCount);
  if (request.provider === "claude") {
    enforceClaudeToolPolicy(toolNames);
  }
  const tokenUsage = result.tokenUsage !== null && typeof result.tokenUsage === "object"
    ? result.tokenUsage
    : {};
  const sessionId = typeof result.sessionId === "string" && result.sessionId.length > 0 && result.sessionId.length <= 256
    ? result.sessionId
    : null;

  return {
    finalText: result.output,
    sessionId,
    tokenUsage: {
      input: safeNonnegativeInteger(tokenUsage.input) ?? safeNonnegativeInteger(tokenUsage.prompt),
      output: safeNonnegativeInteger(tokenUsage.output) ?? safeNonnegativeInteger(tokenUsage.completion),
      total: safeNonnegativeInteger(tokenUsage.total),
    },
    toolSummary: countTools(toolNames, discardedFieldCount),
    observedModel: request.provider === "claude" ? claudeObservedModel(metadata.modelUsage) : null,
    providerSummary: {
      unknown_fields: summarizeUnknownFields(result, metadata, discardedFieldCount),
      discarded_field_count: discardedFieldCount.count,
    },
  };
}

export function normalizeProviderError(error) {
  const evidence = [error?.code, error?.name, error?.message]
    .filter((part) => typeof part === "string")
    .join(" ")
    .toLowerCase();
  if (/rate.?limit|quota|credit|billing|usage.?limit/.test(evidence)) {
    return "quota";
  }
  if (/auth|unauthori[sz]ed|forbidden|invalid.?key|login|credential/.test(evidence)) {
    return "auth";
  }
  if (/model.*(?:not.?found|unavailable|unsupported)|unknown.?model/.test(evidence)) {
    return "model_unavailable";
  }
  if (/econn|network|fetch.?failed|dns|socket|timed?.?out|timeout/.test(evidence)) {
    return "network";
  }
  if (/spawn|enoent|executable|runtime|cache.?hit|callapi|malformed|abort/.test(evidence)) {
    return "runtime";
  }
  return "unknown";
}

export const executeProviderRequest = createProviderExecutor();
