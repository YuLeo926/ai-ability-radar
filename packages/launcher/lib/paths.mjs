import {
  isAbsolute,
  join,
  relative,
  resolve,
} from "node:path";

import { LauncherError } from "./errors.mjs";
import { deriveReleaseIdentity } from "./manifest.mjs";

const OPERATION_TOKEN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu;

export function assertOperationToken(token) {
  if (typeof token !== "string" || !OPERATION_TOKEN.test(token)) {
    throw new LauncherError("INVALID_CACHE_PATH", "缓存操作标识无效。");
  }
  return token.toLowerCase();
}

export function resolveCachePaths({ localAppData, version } = {}) {
  const identity = deriveReleaseIdentity(version);
  if (
    typeof localAppData !== "string" ||
    localAppData.length === 0 ||
    localAppData.includes("\0") ||
    !isAbsolute(localAppData)
  ) {
    throw new LauncherError("INVALID_CACHE_PATH", "LOCALAPPDATA 缓存路径无效。");
  }
  const canonicalLocalAppData = resolve(localAppData);
  const appRoot = join(canonicalLocalAppData, "AI Ability Radar");
  const cacheRoot = join(appRoot, "launcher");
  const versionTag = identity.tag;
  for (const candidate of [appRoot, cacheRoot]) {
    const child = relative(canonicalLocalAppData, candidate);
    if (child === "" || child === ".." || child.startsWith(`..\\`) || isAbsolute(child)) {
      throw new LauncherError("INVALID_CACHE_PATH", "缓存路径越出 LOCALAPPDATA。");
    }
  }
  return Object.freeze({
    localAppData: canonicalLocalAppData,
    appRoot,
    cacheRoot,
    version: identity.version,
    versionTag,
    versionDirectory: join(cacheRoot, versionTag),
    lockDirectory: join(cacheRoot, `.lock-${versionTag}`),
  });
}

export function versionStagingDirectory(paths, token) {
  return join(
    paths.cacheRoot,
    `.stage-${paths.versionTag}-${assertOperationToken(token)}`,
  );
}

export function versionOldDirectory(paths, token) {
  return join(
    paths.cacheRoot,
    `.old-${paths.versionTag}-${assertOperationToken(token)}`,
  );
}
