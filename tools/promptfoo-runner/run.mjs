import { pathToFileURL } from "node:url";

import {
  executeProviderRequest,
  normalizeProviderError,
  summarizeProviderResult,
} from "./provider-config.mjs";
import {
  CONTRACT_VERSION,
  MAX_REQUEST_BYTES,
  ProtocolError,
  createErrorResponse,
  createSuccessResponse,
  parseRunnerRequest,
} from "./protocol.mjs";

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
    response = createSuccessResponse(request, summarizeProviderResult(request, result));
    exitCode = 0;
  } catch (error) {
    const protocolFailure = error instanceof ProtocolError;
    const code = protocolFailure ? "runtime" : normalizeProviderError(error);
    response = createErrorResponse({
      runId: request?.run_id ?? null,
      requestedModel: request?.requested_model ?? null,
      providerErrorCode: code,
    });
    stderr.write(`${CONTRACT_VERSION} error=${protocolFailure ? error.code : code}\n`);
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
    process.stderr.write(`${CONTRACT_VERSION} error=request_too_large\n`);
    process.stdout.write(`${JSON.stringify(createErrorResponse({ providerErrorCode: "runtime" }))}\n`);
    process.exitCode = 2;
    return;
  }
  process.exitCode = await runOnce({ inputText });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
