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
import { join } from "node:path";

import { assertCacheRootOwned, ensureCacheRoot } from "./cache.mjs";
import { LauncherError } from "./errors.mjs";
import { assertOperationToken } from "./paths.mjs";

const LOCK_OWNER_NAME = "owner.json";
const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_STALE_MS = 10 * 60_000;
const DEFAULT_POLL_MS = 100;

function lockError(code, message) {
  return new LauncherError(code, message);
}

function identity(info) {
  return { dev: info.dev, ino: info.ino, birthtimeNs: info.birthtimeNs };
}

function sameIdentity(left, right) {
  return left.dev === right.dev && left.ino === right.ino && left.birthtimeNs === right.birthtimeNs;
}

async function pathInfo(path) {
  try {
    return await lstat(path, { bigint: true });
  } catch (error) {
    if (error?.code === "ENOENT") return undefined;
    throw lockError("LOCK_LOST", "无法检查版本锁。");
  }
}

function ownerText(paths, token, createdAtMs) {
  return `${JSON.stringify({
    schema_version: 1,
    owner: "ai-ability-radar-launcher",
    version: paths.version,
    token,
    created_at_ms: createdAtMs,
  })}\n`;
}

async function readOwnerAt(paths, directory) {
  const directoryInfo = await pathInfo(directory);
  if (!directoryInfo || !directoryInfo.isDirectory() || directoryInfo.isSymbolicLink()) {
    throw lockError("LOCK_LOST", "版本锁已丢失。");
  }
  const entries = await readdir(directory);
  if (entries.length !== 1 || entries[0] !== LOCK_OWNER_NAME) {
    throw lockError("LOCK_LOST", "版本锁所有权记录无效。");
  }
  const ownerPath = join(directory, LOCK_OWNER_NAME);
  const ownerInfo = await pathInfo(ownerPath);
  if (
    !ownerInfo ||
    !ownerInfo.isFile() ||
    ownerInfo.isSymbolicLink() ||
    ownerInfo.size > 512n
  ) {
    throw lockError("LOCK_LOST", "版本锁所有权记录无效。");
  }
  let value;
  try {
    value = JSON.parse(await readFile(ownerPath, "utf8"));
  } catch {
    throw lockError("LOCK_LOST", "版本锁所有权记录无效。");
  }
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).sort().join(",") !== "created_at_ms,owner,schema_version,token,version" ||
    value.schema_version !== 1 ||
    value.owner !== "ai-ability-radar-launcher" ||
    value.version !== paths.version ||
    !Number.isSafeInteger(value.created_at_ms) ||
    value.created_at_ms < 0
  ) {
    throw lockError("LOCK_LOST", "版本锁所有权记录无效。");
  }
  let token;
  try {
    token = assertOperationToken(value.token);
  } catch {
    throw lockError("LOCK_LOST", "版本锁所有权记录无效。");
  }
  return {
    token,
    createdAtMs: value.created_at_ms,
    identity: identity(directoryInfo),
  };
}

async function removeKnownLockDirectory(paths, directory, expected) {
  const observed = await readOwnerAt(paths, directory);
  if (
    observed.token !== expected.token ||
    !sameIdentity(observed.identity, expected.identity)
  ) {
    throw lockError("LOCK_LOST", "版本锁目录身份已变化。");
  }
  await rm(directory, { recursive: true });
}

async function tryCreateLock(paths, token, createdAtMs) {
  const initialization = join(
    paths.cacheRoot,
    `.lock-init-${paths.versionTag}-${token}`,
  );
  if (await pathInfo(initialization)) {
    throw lockError("LOCK_BUSY", "版本锁初始化目录已存在。");
  }
  await mkdir(initialization);
  let snapshot;
  try {
    await writeFile(
      join(initialization, LOCK_OWNER_NAME),
      ownerText(paths, token, createdAtMs),
      { flag: "wx" },
    );
    snapshot = await readOwnerAt(paths, initialization);
    try {
      await rename(initialization, paths.lockDirectory);
      return snapshot;
    } catch (error) {
      if (!["EEXIST", "ENOTEMPTY", "EPERM"].includes(error?.code)) throw error;
      await removeKnownLockDirectory(paths, initialization, snapshot);
      return null;
    }
  } catch (error) {
    if (snapshot && await pathInfo(initialization)) {
      await removeKnownLockDirectory(paths, initialization, snapshot);
    }
    if (error instanceof LauncherError) throw error;
    throw lockError("LOCK_BUSY", "无法创建版本锁。");
  }
}

async function restoreMovedLock(paths, movedPath) {
  if (!await pathInfo(paths.lockDirectory) && await pathInfo(movedPath)) {
    await rename(movedPath, paths.lockDirectory);
  }
}

async function removeStaleLock(paths, observed, contenderToken) {
  const quarantine = join(
    paths.cacheRoot,
    `.lock-stale-${paths.versionTag}-${contenderToken}`,
  );
  if (await pathInfo(quarantine)) {
    throw lockError("LOCK_BUSY", "版本锁接管目录已存在。");
  }
  try {
    await rename(paths.lockDirectory, quarantine);
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw lockError("LOCK_BUSY", "无法接管已超时的版本锁。");
  }
  let moved;
  try {
    moved = await readOwnerAt(paths, quarantine);
    if (
      moved.token !== observed.token ||
      !sameIdentity(moved.identity, observed.identity)
    ) {
      await restoreMovedLock(paths, quarantine);
      return false;
    }
    await removeKnownLockDirectory(paths, quarantine, moved);
    return true;
  } catch (error) {
    await restoreMovedLock(paths, quarantine);
    if (error instanceof LauncherError) throw error;
    throw lockError("LOCK_BUSY", "无法清理已超时的版本锁。");
  }
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function requireDuration(value, fallback, label) {
  const duration = value ?? fallback;
  if (!Number.isSafeInteger(duration) || duration < 1 || duration > 60 * 60_000) {
    throw lockError("LOCK_BUSY", `${label}无效。`);
  }
  return duration;
}

function createHandle(paths, token, acquiredIdentity) {
  let released = false;
  const assertOwned = async () => {
    if (released) throw lockError("LOCK_LOST", "版本锁已释放。");
    const observed = await readOwnerAt(paths, paths.lockDirectory);
    if (
      observed.token !== token ||
      !sameIdentity(observed.identity, acquiredIdentity)
    ) {
      throw lockError("LOCK_LOST", "版本锁已被其他启动器接管。");
    }
    return true;
  };
  const release = async () => {
    await assertOwned();
    const releasePath = join(
      paths.cacheRoot,
      `.lock-release-${paths.versionTag}-${token}`,
    );
    if (await pathInfo(releasePath)) {
      throw lockError("LOCK_LOST", "版本锁释放目录已存在。");
    }
    await rename(paths.lockDirectory, releasePath);
    try {
      const moved = await readOwnerAt(paths, releasePath);
      if (
        moved.token !== token ||
        !sameIdentity(moved.identity, acquiredIdentity)
      ) {
        await restoreMovedLock(paths, releasePath);
        throw lockError("LOCK_LOST", "版本锁在释放时发生变化。");
      }
      await removeKnownLockDirectory(paths, releasePath, moved);
      released = true;
    } catch (error) {
      await restoreMovedLock(paths, releasePath);
      throw error;
    }
  };
  return Object.freeze({ token, assertOwned, release });
}

export async function acquireVersionLock(
  paths,
  {
    token = randomUUID(),
    timeoutMs,
    staleMs,
    pollMs,
    now = Date.now,
  } = {},
) {
  const operationToken = assertOperationToken(token);
  const waitLimit = requireDuration(timeoutMs, DEFAULT_TIMEOUT_MS, "版本锁等待时间");
  const staleLimit = requireDuration(staleMs, DEFAULT_STALE_MS, "版本锁超时时间");
  const poll = requireDuration(pollMs, DEFAULT_POLL_MS, "版本锁轮询时间");
  if (typeof now !== "function") {
    throw lockError("LOCK_BUSY", "版本锁时钟无效。");
  }
  await ensureCacheRoot(paths, { token: operationToken });
  await assertCacheRootOwned(paths);
  const startedAt = now();
  if (!Number.isSafeInteger(startedAt) || startedAt < 0) {
    throw lockError("LOCK_BUSY", "版本锁时钟无效。");
  }
  const deadline = startedAt + waitLimit;
  if (!Number.isSafeInteger(deadline)) {
    throw lockError("LOCK_BUSY", "版本锁等待时间无效。");
  }

  while (true) {
    const currentTime = now();
    if (!Number.isSafeInteger(currentTime) || currentTime < 0) {
      throw lockError("LOCK_BUSY", "版本锁时钟无效。");
    }
    const created = await tryCreateLock(paths, operationToken, currentTime);
    if (created) {
      return createHandle(paths, operationToken, created.identity);
    }

    let observed;
    try {
      observed = await readOwnerAt(paths, paths.lockDirectory);
    } catch (error) {
      if (!await pathInfo(paths.lockDirectory)) continue;
      if (currentTime >= deadline) {
        throw lockError("LOCK_BUSY", "另一个启动器正在使用此版本缓存。");
      }
      await delay(Math.min(poll, Math.max(1, deadline - currentTime)));
      continue;
    }
    if (currentTime - observed.createdAtMs >= staleLimit) {
      if (await removeStaleLock(paths, observed, operationToken)) continue;
    }
    if (currentTime >= deadline) {
      throw lockError("LOCK_BUSY", "另一个启动器正在使用此版本缓存。");
    }
    await delay(Math.min(poll, Math.max(1, deadline - currentTime)));
  }
}
