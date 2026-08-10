import { pathToFileURL } from "node:url";

import { executeProviderRequest, normalizeProviderError } from "./provider-config.mjs";
import {
  MAX_REQUEST_BYTES,
  ProtocolError,
  createErrorResponse,
  createSuccessResponse,
  parseRunnerRequest,
} from "./protocol.mjs";

function nullableString(value) {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function summarizeResult(result) {
  const tokenUsage = result?.tokenUsage ?? {};
  return {
    finalText: typeof result?.output === "string" ? result.output : "",
    sessionId: nullableString(result?.metadata?.sessionId),
    tokenUsage: {
      input: Number.isSafeInteger(tokenUsage.input) ? tokenUsage.input :
        Number.isSafeInteger(tokenUsage.prompt) ? tokenUsage.prompt : null,
      output: Number.isSafeInteger(tokenUsage.output) ? tokenUsage.output :
        Number.isSafeInteger(tokenUsage.completion) ? tokenUsage.completion : null,
      total: Number.isSafeInteger(tokenUsage.total) ? tokenUsage.total : null,
    },
    toolSummary: Array.isArray(result?.metadata?.toolSummary) ? result.metadata.toolSummary : [],
    observedModel: nullableString(result?.metadata?.model),
  };
}

export async function runOnce({
  inputText,
  executeProvider = executeProviderRequest,
  stdout = process.stdout,
  stderr = process.stderr,
}) {
  let request;
  let response;
  let exitCode;
  try {
    request = parseRunnerRequest(inputText);
    const result = await executeProvider(request);
    response = createSuccessResponse(request, summarizeResult(result));
    exitCode = 0;
  } catch (error) {
    const protocolFailure = error instanceof ProtocolError;
    const code = protocolFailure ? "runtime" : normalizeProviderError(error);
    response = createErrorResponse({
      runId: request?.run_id ?? null,
      requestedModel: request?.requested_model ?? null,
      providerErrorCode: code,
    });
    stderr.write(`promptfoo-agent-v1 error=${protocolFailure ? error.code : code}\n`);
    exitCode = protocolFailure ? 2 : 1;
  }
  stdout.write(`${JSON.stringify(response)}\n`);
  return exitCode;
}

export async function readBoundedStdin(input = process.stdin) {
  const chunks = [];
  let bytes = 0;
  for await (const chunk of input) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    bytes += buffer.length;
    if (bytes > MAX_REQUEST_BYTES) {
      throw new ProtocolError("request_too_large");
    }
    chunks.push(buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

export async function main() {
  let inputText = "";
  try {
    inputText = await readBoundedStdin();
  } catch (error) {
    process.stderr.write("promptfoo-agent-v1 error=request_too_large\n");
    process.stdout.write(`${JSON.stringify(createErrorResponse({ providerErrorCode: "runtime" }))}\n`);
    process.exitCode = 2;
    return;
  }
  process.exitCode = await runOnce({ inputText });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
