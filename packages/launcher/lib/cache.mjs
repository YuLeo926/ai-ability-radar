import { randomUUID } from "node:crypto";
import {
  lstat,
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { basename, join } from "node:path";

import { LauncherError } from "./errors.mjs";
import {
  assertOperationToken,
  versionOldDirectory,
  versionStagingDirectory,
} from "./paths.mjs";

export const CACHE_ROOT_MARKER_NAME = ".launcher-owner.json";
export const CACHE_ENTRY_MARKER_NAME = ".cache-entry.json";

const CACHE_ROOT_MARKER = `${JSON.stringify({
  schema_version: 1,
  owner: "ai-ability-radar-launcher",
  cache_format: 1,
})}\n`;

function cacheError(code, message) {
  return new LauncherError(code, message);
}

async function pathInfo(path) {
  try {
    return await lstat(path, { bigint: true });
  } catch (error) {
    if (error?.code === "ENOENT") return undefined;
    throw cacheError("CACHE_OWNERSHIP", "无法检查缓存路径。");
  }
}

function identity(info) {
  return { dev: info.dev, ino: info.ino, birthtimeNs: info.birthtimeNs };
}

function sameIdentity(left, right) {
  return left.dev === right.dev && left.ino === right.ino && left.birthtimeNs === right.birthtimeNs;
}

async function requirePlainDirectory(path, label) {
  const info = await pathInfo(path);
  if (!info || !info.isDirectory() || info.isSymbolicLink()) {
    throw cacheError("CACHE_OWNERSHIP", `${label}不是受信任的普通目录。`);
  }
  return info;
}

async function requirePlainFile(path, label) {
  const info = await pathInfo(path);
  if (!info || !info.isFile() || info.isSymbolicLink()) {
    throw cacheError("CACHE_OWNERSHIP", `${label}不是受信任的普通文件。`);
  }
  return info;
}

async function requireTreeWithoutLinks(root) {
  const info = await pathInfo(root);
  if (!info || info.isSymbolicLink()) {
    throw cacheError("CACHE_OWNERSHIP", "缓存树包含链接或重解析点。");
  }
  if (info.isDirectory()) {
    for (const entry of await readdir(root)) {
      await requireTreeWithoutLinks(join(root, entry));
    }
    return;
  }
  if (!info.isFile()) {
    throw cacheError("CACHE_OWNERSHIP", "缓存树包含不支持的文件类型。");
  }
}

async function removeDirectoryWithIdentity(path, expectedIdentity) {
  const current = await requirePlainDirectory(path, "待清理缓存目录");
  if (!sameIdentity(expectedIdentity, identity(current))) {
    throw cacheError("CACHE_OWNERSHIP", "待清理缓存目录的身份已变化。");
  }
  await requireTreeWithoutLinks(path);
  const rechecked = await requirePlainDirectory(path, "待清理缓存目录");
  if (!sameIdentity(expectedIdentity, identity(rechecked))) {
    throw cacheError("CACHE_OWNERSHIP", "待清理缓存目录的身份已变化。");
  }
  await rm(path, { recursive: true });
}

async function ensurePlainDirectory(path, label) {
  try {
    await mkdir(path);
  } catch (error) {
    if (error?.code !== "EEXIST") {
      throw cacheError("CACHE_OWNERSHIP", `无法创建${label}。`);
    }
  }
  return requirePlainDirectory(path, label);
}

function entryMarker(paths, token) {
  return `${JSON.stringify({
    schema_version: 1,
    owner: "ai-ability-radar-launcher",
    version: paths.version,
    token: assertOperationToken(token),
  })}\n`;
}

async function readEntryOwner(paths, directory, expectedToken) {
  await requirePlainDirectory(directory, "版本缓存目录");
  const markerPath = join(directory, CACHE_ENTRY_MARKER_NAME);
  const markerInfo = await requirePlainFile(markerPath, "版本缓存标记");
  if (markerInfo.size > 512n) {
    throw cacheError("CACHE_OWNERSHIP", "版本缓存标记过大。");
  }
  let value;
  try {
    value = JSON.parse(await readFile(markerPath, "utf8"));
  } catch {
    throw cacheError("CACHE_OWNERSHIP", "版本缓存标记无效。");
  }
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).sort().join(",") !== "owner,schema_version,token,version" ||
    value.schema_version !== 1 ||
    value.owner !== "ai-ability-radar-launcher" ||
    value.version !== paths.version
  ) {
    throw cacheError("CACHE_OWNERSHIP", "版本缓存标记无效。");
  }
  let token;
  try {
    token = assertOperationToken(value.token);
  } catch {
    throw cacheError("CACHE_OWNERSHIP", "版本缓存标记无效。");
  }
  if (expectedToken !== undefined && token !== assertOperationToken(expectedToken)) {
    throw cacheError("CACHE_OWNERSHIP", "版本缓存不属于本次操作。");
  }
  return { token };
}

export async function assertCacheRootOwned(paths) {
  await requirePlainDirectory(paths.localAppData, "LOCALAPPDATA");
  await requirePlainDirectory(paths.appRoot, "应用缓存父目录");
  await requirePlainDirectory(paths.cacheRoot, "启动器缓存根目录");
  const markerPath = join(paths.cacheRoot, CACHE_ROOT_MARKER_NAME);
  const markerInfo = await requirePlainFile(markerPath, "启动器缓存所有权标记");
  if (markerInfo.size !== BigInt(Buffer.byteLength(CACHE_ROOT_MARKER))) {
    throw cacheError("CACHE_OWNERSHIP", "启动器缓存所有权标记无效。");
  }
  if (await readFile(markerPath, "utf8") !== CACHE_ROOT_MARKER) {
    throw cacheError("CACHE_OWNERSHIP", "启动器缓存所有权标记无效。");
  }
  return paths.cacheRoot;
}

export async function assertVersionCacheOwned(paths) {
  await assertCacheRootOwned(paths);
  return readEntryOwner(paths, paths.versionDirectory);
}

export async function ensureCacheRoot(paths, { token = randomUUID() } = {}) {
  const operationToken = assertOperationToken(token);
  await requirePlainDirectory(paths.localAppData, "LOCALAPPDATA");
  await ensurePlainDirectory(paths.appRoot, "应用缓存父目录");
  if (await pathInfo(paths.cacheRoot)) {
    return assertCacheRootOwned(paths);
  }

  const initialization = join(paths.appRoot, `.launcher-init-${operationToken}`);
  let initializationIdentity;
  try {
    await mkdir(initialization);
    initializationIdentity = identity(
      await requirePlainDirectory(initialization, "缓存初始化目录"),
    );
    await writeFile(
      join(initialization, CACHE_ROOT_MARKER_NAME),
      CACHE_ROOT_MARKER,
      { flag: "wx" },
    );
    try {
      await rename(initialization, paths.cacheRoot);
      initializationIdentity = undefined;
    } catch (error) {
      if (!["EEXIST", "ENOTEMPTY", "EPERM"].includes(error?.code)) throw error;
    }
  } catch (error) {
    if (error instanceof LauncherError) throw error;
    throw cacheError("CACHE_OWNERSHIP", "无法初始化启动器缓存根目录。");
  } finally {
    if (initializationIdentity && await pathInfo(initialization)) {
      await removeDirectoryWithIdentity(initialization, initializationIdentity);
    }
  }
  return assertCacheRootOwned(paths);
}

export async function clearCacheRoot(paths, { token = randomUUID() } = {}) {
  const operationToken = assertOperationToken(token);
  if (!await pathInfo(paths.cacheRoot)) return { removed: false };
  await assertCacheRootOwned(paths);
  await requireTreeWithoutLinks(paths.cacheRoot);
  const original = identity(await requirePlainDirectory(paths.cacheRoot, "启动器缓存根目录"));
  const deletionPath = join(paths.appRoot, `.launcher-delete-${operationToken}`);
  if (await pathInfo(deletionPath)) {
    throw cacheError("CACHE_OWNERSHIP", "缓存清理临时目录已存在。");
  }
  await rename(paths.cacheRoot, deletionPath);
  await removeDirectoryWithIdentity(deletionPath, original);
  return { removed: true };
}

export async function createVersionStaging(paths, { token = randomUUID() } = {}) {
  const operationToken = assertOperationToken(token);
  await ensureCacheRoot(paths, { token: operationToken });
  const directory = versionStagingDirectory(paths, operationToken);
  try {
    await mkdir(directory);
  } catch {
    throw cacheError("CACHE_TRANSACTION", "版本暂存目录已存在或无法创建。");
  }
  const directoryIdentity = identity(
    await requirePlainDirectory(directory, "版本暂存目录"),
  );
  try {
    await writeFile(
      join(directory, CACHE_ENTRY_MARKER_NAME),
      entryMarker(paths, operationToken),
      { flag: "wx" },
    );
  } catch (error) {
    await removeDirectoryWithIdentity(directory, directoryIdentity);
    throw error;
  }
  return directory;
}

export async function discardVersionStaging(
  paths,
  { stagingDirectory, token } = {},
) {
  const operationToken = assertOperationToken(token);
  if (stagingDirectory !== versionStagingDirectory(paths, operationToken)) {
    throw cacheError("CACHE_TRANSACTION", "待清理暂存目录参数无效。");
  }
  await assertCacheRootOwned(paths);
  await readEntryOwner(paths, stagingDirectory, operationToken);
  const stagingIdentity = identity(
    await requirePlainDirectory(stagingDirectory, "待清理暂存目录"),
  );
  await removeDirectoryWithIdentity(stagingDirectory, stagingIdentity);
  return { removed: true };
}

async function publishVersionStagingCore(
  { paths, stagingDirectory, token, lock, validateCandidate },
  hook,
) {
  const operationToken = assertOperationToken(token);
  if (
    stagingDirectory !== versionStagingDirectory(paths, operationToken) ||
    !lock ||
    typeof lock.assertOwned !== "function" ||
    typeof validateCandidate !== "function"
  ) {
    throw cacheError("CACHE_TRANSACTION", "版本缓存发布参数无效。");
  }
  await assertCacheRootOwned(paths);
  await lock.assertOwned();
  await readEntryOwner(paths, stagingDirectory, operationToken);
  await requireTreeWithoutLinks(stagingDirectory);
  await validateCandidate(stagingDirectory);
  await lock.assertOwned();

  let oldDirectory;
  let oldIdentity;
  let published = false;
  try {
    if (await pathInfo(paths.versionDirectory)) {
      const owner = await readEntryOwner(paths, paths.versionDirectory);
      oldDirectory = versionOldDirectory(paths, owner.token);
      if (await pathInfo(oldDirectory)) {
        throw cacheError("CACHE_TRANSACTION", "旧版本隔离目录已存在。");
      }
      oldIdentity = identity(
        await requirePlainDirectory(paths.versionDirectory, "当前版本缓存"),
      );
      await rename(paths.versionDirectory, oldDirectory);
      await hook?.({ phase: "afterQuarantine" });
    }
    await lock.assertOwned();
    await rename(stagingDirectory, paths.versionDirectory);
    published = true;
    await hook?.({ phase: "afterPublish" });
  } catch (error) {
    if (
      !published &&
      oldDirectory &&
      !await pathInfo(paths.versionDirectory) &&
      await pathInfo(oldDirectory)
    ) {
      await rename(oldDirectory, paths.versionDirectory);
      oldDirectory = undefined;
      oldIdentity = undefined;
    }
    throw error;
  }

  if (oldDirectory && oldIdentity) {
    await readEntryOwner(paths, oldDirectory);
    await removeDirectoryWithIdentity(oldDirectory, oldIdentity);
  }
  return paths.versionDirectory;
}

export function publishVersionStaging(options) {
  return publishVersionStagingCore(options);
}

export function publishVersionStagingForTest(options, hook) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw cacheError("CACHE_TRANSACTION", "测试发布入口不可用于生产运行。");
  }
  return publishVersionStagingCore(options, hook);
}

export async function recoverVersionPublication({ paths, lock, validateCandidate }) {
  if (!lock || typeof lock.assertOwned !== "function" || typeof validateCandidate !== "function") {
    throw cacheError("CACHE_TRANSACTION", "版本缓存恢复参数无效。");
  }
  await assertCacheRootOwned(paths);
  await lock.assertOwned();
  if (await pathInfo(paths.versionDirectory)) {
    const owner = await readEntryOwner(paths, paths.versionDirectory);
    try {
      await requireTreeWithoutLinks(paths.versionDirectory);
      await validateCandidate(paths.versionDirectory);
      return paths.versionDirectory;
    } catch (error) {
      if (error instanceof LauncherError && error.code === "CACHE_OWNERSHIP") throw error;
      const old = versionOldDirectory(paths, owner.token);
      if (await pathInfo(old)) {
        throw cacheError("CACHE_TRANSACTION", "恢复时旧版本隔离目录冲突。");
      }
      await rename(paths.versionDirectory, old);
    }
  }

  const stagePrefix = `.stage-${paths.versionTag}-`;
  const oldPrefix = `.old-${paths.versionTag}-`;
  const candidates = [];
  for (const name of await readdir(paths.cacheRoot)) {
    let prefix;
    let priority;
    if (name.startsWith(stagePrefix)) {
      prefix = stagePrefix;
      priority = 0;
    } else if (name.startsWith(oldPrefix)) {
      prefix = oldPrefix;
      priority = 1;
    } else {
      continue;
    }
    const token = name.slice(prefix.length);
    try {
      assertOperationToken(token);
      const directory = join(paths.cacheRoot, name);
      if (basename(directory) !== name) continue;
      await readEntryOwner(paths, directory, token);
      await requireTreeWithoutLinks(directory);
      await validateCandidate(directory);
      candidates.push({ directory, priority, name });
    } catch (error) {
      if (error instanceof LauncherError && error.code === "INVALID_CACHE_PATH") continue;
      if (error instanceof LauncherError && error.code === "CACHE_OWNERSHIP") throw error;
      // An owned but invalid candidate remains isolated for a later safe cleanup.
    }
  }
  candidates.sort((left, right) => left.priority - right.priority || left.name.localeCompare(right.name, "en"));
  const selected = candidates[0];
  if (!selected) return null;
  await lock.assertOwned();
  if (await pathInfo(paths.versionDirectory)) {
    throw cacheError("CACHE_TRANSACTION", "恢复目标已被其他操作创建。");
  }
  await rename(selected.directory, paths.versionDirectory);
  return paths.versionDirectory;
}
