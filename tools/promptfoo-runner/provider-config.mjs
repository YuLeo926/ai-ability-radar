import { cache, loadApiProvider } from "promptfoo";

const PROVIDER_IDS = Object.freeze({
  codex: "openai:codex-sdk",
  claude: "anthropic:claude-agent-sdk",
});

export function buildProviderDescriptor(request) {
  const id = PROVIDER_IDS[request.provider];
  if (!id) {
    throw new Error("unsupported_provider");
  }
  return Object.freeze({ id, basePath: request.workspace, cache: false });
}

export function createProviderExecutor({ loadProvider = loadApiProvider, cacheController = cache } = {}) {
  return async function executeProvider(request) {
    cacheController.disableCache();
    const descriptor = buildProviderDescriptor(request);
    const provider = await loadProvider(descriptor.id, { basePath: descriptor.basePath });
    if (!provider || typeof provider.callApi !== "function") {
      const error = new Error("provider runtime did not expose callApi");
      error.code = "PROVIDER_RUNTIME";
      throw error;
    }
    const result = await provider.callApi(request.prompt);
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
  if (/spawn|enoent|executable|runtime|cache.?hit|callapi/.test(evidence)) {
    return "runtime";
  }
  return "unknown";
}

export const executeProviderRequest = createProviderExecutor();
