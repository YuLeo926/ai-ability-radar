import { realpathSync, statSync } from "node:fs";
import { isAbsolute } from "node:path";

export const CONTRACT_VERSION = "promptfoo-agent-v1";
export const MAX_REQUEST_BYTES = 256 * 1024;
export const MAX_PROMPT_BYTES = 128 * 1024;
export const MAX_FINAL_TEXT_BYTES = 1024 * 1024;

const REQUEST_FIELDS = new Set([
  "provider",
  "workspace",
  "prompt",
  "requested_model",
  "reasoning_effort",
  "time_budget_seconds",
  "max_turns",
  "run_id",
]);
const RESPONSE_FIELDS = new Set([
  "contract_version",
  "run_id",
  "status",
  "final_text",
  "session_id",
  "tokens",
  "tool_summary",
  "model_evidence",
  "provider_error_code",
]);
const CODEX_EFFORTS = new Set([null, "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]);
const CLAUDE_EFFORTS = new Set([null, "low", "medium", "high", "xhigh", "max"]);
const PROVIDER_ERROR_CODES = new Set([
  "auth",
  "quota",
  "network",
  "model_unavailable",
  "runtime",
  "unknown",
]);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const UNSAFE_LABEL = /[\p{Cc}\p{Cf}\p{Default_Ignorable_Code_Point}]/u;

export class ProtocolError extends Error {
  constructor(code) {
    super(code);
    this.name = "ProtocolError";
    this.code = code;
  }
}

function object(value, code) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new ProtocolError(code);
  }
  return value;
}
function exactFields(value, allowed, required = allowed) {
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw new ProtocolError("unknown_field");
    }
  }
  for (const key of required) {
    if (!Object.hasOwn(value, key)) {
      throw new ProtocolError("missing_field");
    }
  }
}

function safeLabel(value, maxLength, code, { nullable = false } = {}) {
  if (nullable && value === null) {
    return null;
  }
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    [...value].length > maxLength ||
    UNSAFE_LABEL.test(value) ||
    value.trim() !== value
  ) {
    throw new ProtocolError(code);
  }
  return value;
}

function nullableToken(value) {
  if (value === null) {
    return null;
  }
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new ProtocolError("invalid_tokens");
  }
  return value;
}

export function parseRunnerRequest(inputText) {
  if (typeof inputText !== "string" || Buffer.byteLength(inputText, "utf8") > MAX_REQUEST_BYTES) {
    throw new ProtocolError("request_too_large");
  }
  let value;
  try {
    value = JSON.parse(inputText);
  } catch {
    throw new ProtocolError("invalid_json");
  }
  object(value, "invalid_request");
  exactFields(value, REQUEST_FIELDS);

  if (value.provider !== "codex" && value.provider !== "claude") {
    throw new ProtocolError("unsupported_provider");
  }
  if (typeof value.workspace !== "string" || !isAbsolute(value.workspace)) {
    throw new ProtocolError("invalid_workspace");
  }
  let workspace;
  try {
    if (!statSync(value.workspace).isDirectory()) {
      throw new ProtocolError("invalid_workspace");
    }
    workspace = realpathSync(value.workspace);
  } catch (error) {
    if (error instanceof ProtocolError) {
      throw error;
    }
    throw new ProtocolError("invalid_workspace");
  }
  if (
    typeof value.prompt !== "string" ||
    value.prompt.length === 0 ||
    value.prompt.includes("\0") ||
    Buffer.byteLength(value.prompt, "utf8") > MAX_PROMPT_BYTES
  ) {
    throw new ProtocolError("invalid_prompt");
  }
  const requestedModel = safeLabel(value.requested_model, 120, "invalid_model");
  const efforts = value.provider === "codex" ? CODEX_EFFORTS : CLAUDE_EFFORTS;
  if (!efforts.has(value.reasoning_effort)) {
    throw new ProtocolError("unsupported_effort");
  }
  if (
    !Number.isSafeInteger(value.time_budget_seconds) ||
    value.time_budget_seconds < 1 ||
    value.time_budget_seconds > 3600
  ) {
    throw new ProtocolError("invalid_time_budget");
  }
  if (
    (value.provider === "codex" && value.max_turns !== null) ||
    (value.provider === "claude" &&
      (!Number.isSafeInteger(value.max_turns) || value.max_turns < 1 || value.max_turns > 200))
  ) {
    throw new ProtocolError("invalid_max_turns");
  }
  if (typeof value.run_id !== "string" || !UUID.test(value.run_id)) {
    throw new ProtocolError("invalid_run_id");
  }

  return Object.freeze({
    provider: value.provider,
    workspace,
    prompt: value.prompt,
    requested_model: requestedModel,
    reasoning_effort: value.reasoning_effort,
    time_budget_seconds: value.time_budget_seconds,
    max_turns: value.max_turns,
    run_id: value.run_id.toLowerCase(),
  });
}

export function createSuccessResponse(request, result = {}) {
  return validateRunnerResponse({
    contract_version: CONTRACT_VERSION,
    run_id: request.run_id,
    status: "success",
    final_text: result.finalText ?? "",
    session_id: result.sessionId ?? null,
    tokens: {
      input: result.tokenUsage?.input ?? null,
      output: result.tokenUsage?.output ?? null,
      total: result.tokenUsage?.total ?? null,
    },
    tool_summary: result.toolSummary ?? [],
    model_evidence: {
      requested_model: request.requested_model,
      observed_model: result.observedModel ?? null,
      source: result.observedModel ? "provider" : "request_only",
    },
    provider_error_code: null,
  });
}

export function createErrorResponse({ runId = null, requestedModel = null, providerErrorCode = "unknown" } = {}) {
  return validateRunnerResponse({
    contract_version: CONTRACT_VERSION,
    run_id: runId,
    status: "error",
    final_text: "",
    session_id: null,
    tokens: { input: null, output: null, total: null },
    tool_summary: [],
    model_evidence: {
      requested_model: requestedModel,
      observed_model: null,
      source: "unavailable",
    },
    provider_error_code: providerErrorCode,
  });
}

export function validateRunnerResponse(value) {
  object(value, "invalid_response");
  exactFields(value, RESPONSE_FIELDS);
  if (value.contract_version !== CONTRACT_VERSION) {
    throw new ProtocolError("invalid_contract_version");
  }
  if (value.run_id !== null && (typeof value.run_id !== "string" || !UUID.test(value.run_id))) {
    throw new ProtocolError("invalid_run_id");
  }
  if (value.status !== "success" && value.status !== "error") {
    throw new ProtocolError("invalid_status");
  }
  if (
    typeof value.final_text !== "string" ||
    Buffer.byteLength(value.final_text, "utf8") > MAX_FINAL_TEXT_BYTES
  ) {
    throw new ProtocolError("invalid_final_text");
  }
  safeLabel(value.session_id, 256, "invalid_session_id", { nullable: true });

  object(value.tokens, "invalid_tokens");
  exactFields(value.tokens, new Set(["input", "output", "total"]));
  nullableToken(value.tokens.input);
  nullableToken(value.tokens.output);
  nullableToken(value.tokens.total);

  if (!Array.isArray(value.tool_summary) || value.tool_summary.length > 64) {
    throw new ProtocolError("invalid_tool_summary");
  }
  for (const tool of value.tool_summary) {
    object(tool, "invalid_tool_summary");
    exactFields(tool, new Set(["name", "count"]));
    safeLabel(tool.name, 128, "invalid_tool_summary");
    if (!Number.isSafeInteger(tool.count) || tool.count < 1 || tool.count > 10_000) {
      throw new ProtocolError("invalid_tool_summary");
    }
  }

  object(value.model_evidence, "invalid_model_evidence");
  exactFields(value.model_evidence, new Set(["requested_model", "observed_model", "source"]));
  safeLabel(value.model_evidence.requested_model, 120, "invalid_model_evidence", { nullable: true });
  safeLabel(value.model_evidence.observed_model, 120, "invalid_model_evidence", { nullable: true });
  if (!["provider", "request_only", "unavailable"].includes(value.model_evidence.source)) {
    throw new ProtocolError("invalid_model_evidence");
  }
  if (
    (value.status === "success" && value.provider_error_code !== null) ||
    (value.status === "error" && !PROVIDER_ERROR_CODES.has(value.provider_error_code))
  ) {
    throw new ProtocolError("invalid_provider_error_code");
  }
  return value;
}
