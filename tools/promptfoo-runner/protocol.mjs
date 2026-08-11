import { realpathSync, statSync } from "node:fs";
import { isAbsolute } from "node:path";

export const CONTRACT_VERSION = "promptfoo-agent-v2";
export const MAX_REQUEST_BYTES = 256 * 1024;
export const MAX_PROMPT_BYTES = 128 * 1024;
// Leave room for the evidence envelope under the Rust runner's 1 MiB stdout cap.
export const MAX_FINAL_TEXT_BYTES = 64 * 1024;
export const MAX_TOOL_ERROR_TEXT_BYTES = 512;
export const MAX_TOOL_ERRORS = 32;

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
  "session_present",
  "tokens",
  "tool_summary",
  "command_summary",
  "tool_error_summary",
  "file_change_count",
  "model_evidence",
  "provider_summary",
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
const UNSAFE_DIAGNOSTIC_CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f\p{Cf}\p{Default_Ignorable_Code_Point}]/gu;
const SECRET_PATTERNS = [
  /\b(?:Bearer|Basic)\s+[^\s,;]+/giu,
  /\bsk-[A-Za-z0-9_-]{8,}\b/gu,
  /\bAKIA[0-9A-Z]{12,}\b/gu,
  /\b(api[_-]?key|access[_-]?token|auth[_-]?token|secret|password)\s*[:=]\s*([^\s,;]+)/giu,
];
const WINDOWS_PATH = /(?:\\\\[^\\\s]+\\[^\s"'<>|]+|\b[A-Za-z]:\\[^\s"'<>|]+)/gu;
const UNIX_PATH = /(^|[\s("'=])\/(?!\/)[^\s"'<>]+/gu;

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

function truncateUtf8(value, maxBytes) {
  if (Buffer.byteLength(value, "utf8") <= maxBytes) {
    return value;
  }
  let bytes = 0;
  let output = "";
  for (const character of value) {
    const length = Buffer.byteLength(character, "utf8");
    if (bytes + length > maxBytes) break;
    output += character;
    bytes += length;
  }
  return output;
}

export function sanitizeDiagnosticText(value, { workspace = null, maxBytes = MAX_FINAL_TEXT_BYTES } = {}) {
  if (typeof value !== "string") {
    return "";
  }
  let sanitized = value.replace(UNSAFE_DIAGNOSTIC_CONTROL, "");
  if (typeof workspace === "string" && workspace.length > 0) {
    sanitized = sanitized.replaceAll(workspace, "<path>");
    sanitized = sanitized.replaceAll(workspace.replaceAll("\\", "/"), "<path>");
  }
  for (const pattern of SECRET_PATTERNS) {
    sanitized = sanitized.replace(pattern, (_match, label) =>
      typeof label === "string" && /key|token|secret|password/i.test(label)
        ? `${label}=<redacted>`
        : "<redacted>",
    );
  }
  sanitized = sanitized.replace(WINDOWS_PATH, "<path>");
  sanitized = sanitized.replace(UNIX_PATH, (_match, prefix) => `${prefix}<path>`);
  return truncateUtf8(sanitized, maxBytes);
}

function nullableCount(value, code) {
  if (value === null) return null;
  if (!Number.isSafeInteger(value) || value < 0 || value > 1_000_000) {
    throw new ProtocolError(code);
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
    final_text: sanitizeDiagnosticText(result.finalText ?? "", { workspace: request.workspace }),
    session_present: result.sessionPresent ?? Boolean(result.sessionId),
    tokens: {
      input: result.tokenUsage?.input ?? null,
      output: result.tokenUsage?.output ?? null,
      total: result.tokenUsage?.total ?? null,
    },
    tool_summary: result.toolSummary ?? [],
    command_summary: result.commandSummary ?? {
      succeeded: null,
      failed: null,
      unknown: null,
      exit_codes: [],
    },
    tool_error_summary: (result.toolErrorSummary ?? []).map((error) => ({
      kind: error.kind,
      message: sanitizeDiagnosticText(error.message, {
        workspace: request.workspace,
        maxBytes: MAX_TOOL_ERROR_TEXT_BYTES,
      }),
    })),
    file_change_count: result.fileChangeCount ?? null,
    model_evidence: {
      requested_model: request.requested_model,
      observed_model: result.observedModel ?? null,
      source: result.observedModel ? "provider" : "request_only",
    },
    provider_summary: result.providerSummary ?? {
      unknown_fields: [],
      discarded_field_count: 0,
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
    session_present: false,
    tokens: { input: null, output: null, total: null },
    tool_summary: [],
    command_summary: { succeeded: null, failed: null, unknown: null, exit_codes: [] },
    tool_error_summary: [],
    file_change_count: null,
    model_evidence: {
      requested_model: requestedModel,
      observed_model: null,
      source: "unavailable",
    },
    provider_summary: {
      unknown_fields: [],
      discarded_field_count: 0,
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
    Buffer.byteLength(value.final_text, "utf8") > MAX_FINAL_TEXT_BYTES ||
    sanitizeDiagnosticText(value.final_text) !== value.final_text
  ) {
    throw new ProtocolError("invalid_final_text");
  }
  if (typeof value.session_present !== "boolean") {
    throw new ProtocolError("invalid_session_present");
  }

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

  object(value.command_summary, "invalid_command_summary");
  exactFields(
    value.command_summary,
    new Set(["succeeded", "failed", "unknown", "exit_codes"]),
  );
  nullableCount(value.command_summary.succeeded, "invalid_command_summary");
  nullableCount(value.command_summary.failed, "invalid_command_summary");
  nullableCount(value.command_summary.unknown, "invalid_command_summary");
  const commandCounts = [
    value.command_summary.succeeded,
    value.command_summary.failed,
    value.command_summary.unknown,
  ];
  if (commandCounts.some((count) => count === null) && commandCounts.some((count) => count !== null)) {
    throw new ProtocolError("invalid_command_summary");
  }
  if (!Array.isArray(value.command_summary.exit_codes) || value.command_summary.exit_codes.length > 64) {
    throw new ProtocolError("invalid_command_summary");
  }
  const seenExitCodes = new Set();
  for (const exitCode of value.command_summary.exit_codes) {
    object(exitCode, "invalid_command_summary");
    exactFields(exitCode, new Set(["code", "count"]));
    if (
      !Number.isSafeInteger(exitCode.code) ||
      exitCode.code < -2_147_483_648 ||
      exitCode.code > 2_147_483_647 ||
      !Number.isSafeInteger(exitCode.count) ||
      exitCode.count < 1 ||
      exitCode.count > 1_000_000 ||
      seenExitCodes.has(exitCode.code)
    ) {
      throw new ProtocolError("invalid_command_summary");
    }
    seenExitCodes.add(exitCode.code);
  }
  if (commandCounts[0] === null && value.command_summary.exit_codes.length > 0) {
    throw new ProtocolError("invalid_command_summary");
  }

  if (!Array.isArray(value.tool_error_summary) || value.tool_error_summary.length > MAX_TOOL_ERRORS) {
    throw new ProtocolError("invalid_tool_error_summary");
  }
  for (const error of value.tool_error_summary) {
    object(error, "invalid_tool_error_summary");
    exactFields(error, new Set(["kind", "message"]));
    safeLabel(error.kind, 64, "invalid_tool_error_summary");
    if (
      typeof error.message !== "string" ||
      error.message.length === 0 ||
      Buffer.byteLength(error.message, "utf8") > MAX_TOOL_ERROR_TEXT_BYTES ||
      sanitizeDiagnosticText(error.message, { maxBytes: MAX_TOOL_ERROR_TEXT_BYTES }) !== error.message
    ) {
      throw new ProtocolError("invalid_tool_error_summary");
    }
  }
  nullableCount(value.file_change_count, "invalid_file_change_count");

  object(value.model_evidence, "invalid_model_evidence");
  exactFields(value.model_evidence, new Set(["requested_model", "observed_model", "source"]));
  safeLabel(value.model_evidence.requested_model, 120, "invalid_model_evidence", { nullable: true });
  safeLabel(value.model_evidence.observed_model, 120, "invalid_model_evidence", { nullable: true });
  if (!["provider", "request_only", "unavailable"].includes(value.model_evidence.source)) {
    throw new ProtocolError("invalid_model_evidence");
  }
  object(value.provider_summary, "invalid_provider_summary");
  exactFields(value.provider_summary, new Set(["unknown_fields", "discarded_field_count"]));
  if (
    !Array.isArray(value.provider_summary.unknown_fields) ||
    value.provider_summary.unknown_fields.length > 64 ||
    value.provider_summary.unknown_fields.some(
      (field) => typeof field !== "string" || !/^[A-Za-z][A-Za-z0-9_.-]{0,127}$/.test(field),
    ) ||
    !Number.isSafeInteger(value.provider_summary.discarded_field_count) ||
    value.provider_summary.discarded_field_count < 0
  ) {
    throw new ProtocolError("invalid_provider_summary");
  }
  if (
    (value.status === "success" && value.provider_error_code !== null) ||
    (value.status === "error" && !PROVIDER_ERROR_CODES.has(value.provider_error_code))
  ) {
    throw new ProtocolError("invalid_provider_error_code");
  }
  if (
    value.status === "error" &&
    (value.final_text !== "" ||
      value.session_present ||
      value.tokens.input !== null ||
      value.tokens.output !== null ||
      value.tokens.total !== null ||
      value.tool_summary.length > 0 ||
      value.command_summary.succeeded !== null ||
      value.command_summary.failed !== null ||
      value.command_summary.unknown !== null ||
      value.command_summary.exit_codes.length > 0 ||
      value.tool_error_summary.length > 0 ||
      value.file_change_count !== null ||
      value.model_evidence.source !== "unavailable")
  ) {
    throw new ProtocolError("invalid_error_evidence");
  }
  return value;
}
