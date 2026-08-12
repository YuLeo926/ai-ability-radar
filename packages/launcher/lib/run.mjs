import { createHash, randomUUID } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  copyFile,
  lstat,
  mkdir,
  open,
  readFile,
  readdir,
  unlink,
} from "node:fs/promises";
import { join } from "node:path";

import { inspectPortableArchive, extractPortableArchive } from "./archive.mjs";
import {
  CACHE_ENTRY_MARKER_NAME,
  clearCacheRoot,
  createVersionStaging,
  discardVersionStaging,
  assertVersionCacheOwned,
  publishVersionStaging,
  recoverVersionPublication,
} from "./cache.mjs";
import { downloadChecksums, downloadPortable } from "./download.mjs";
import { LauncherError, isLauncherError } from "./errors.mjs";
import { launchVerifiedExecutable } from "./launch.mjs";
import {
  deriveReleaseIdentity,
  validateReleaseManifest,
} from "./manifest.mjs";
import { acquireVersionLock } from "./lock.mjs";
import { resolveCachePaths } from "./paths.mjs";
import { assertSupportedRuntime } from "./runtime.mjs";
import { verifyExtractedTree } from "./tree.mjs";

const PAYLOAD_DIRECTORY = "app";
const REMOTE_CHECKSUMS_FILE = ".release-checksums.txt";
const SAFE_ASSET_NAME = /^[A-Za-z0-9._-]+$/u;
const SHA256 = /^[a-f0-9]{64}$/u;

const PRODUCTION_DEPENDENCIES = Object.freeze({
  downloadChecksums,
  downloadPortable,
  launchApplication: launchVerifiedExecutable,
});

function runError(code, message, cause) {
  return new LauncherError(
    code,
    message,
    cause === undefined ? undefined : { cause },
  );
}

function fileIdentity(info) {
  return {
    dev: info.dev,
    ino: info.ino,
    birthtimeNs: info.birthtimeNs,
    size: info.size,
    mtimeNs: info.mtimeNs,
    ctimeNs: info.ctimeNs,
  };
}

function sameFileIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.birthtimeNs === right.birthtimeNs &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs
  );
}

async function pathExists(path) {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

async function verifyPortableAsset(archivePath, portable) {
  let handle;
  try {
    handle = await open(archivePath, "r");
    const before = await handle.stat({ bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      before.nlink !== 1n ||
      before.size !== BigInt(portable.size)
    ) {
      throw runError("CACHE_INVALID", "便携版 ZIP 的文件身份或大小无效。");
    }
    const digest = createHash("sha256");
    for await (const value of handle.createReadStream({ autoClose: false })) {
      digest.update(value);
    }
    const after = await handle.stat({ bigint: true });
    if (
      !sameFileIdentity(fileIdentity(before), fileIdentity(after)) ||
      digest.digest("hex") !== portable.sha256
    ) {
      throw runError("CACHE_INVALID", "便携版 ZIP 的完整性校验失败。");
    }
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw runError("CACHE_INVALID", "无法安全验证便携版 ZIP。", error);
  } finally {
    await handle?.close().catch(() => {});
  }
  await inspectPortableArchive(archivePath, { assets: { portable } });
}

function candidateLocations(directory, portable) {
  const payloadRoot = join(directory, PAYLOAD_DIRECTORY);
  const portableRoot = join(payloadRoot, portable.root_directory);
  return {
    archivePath: join(directory, portable.file_name),
    payloadRoot,
    portableRoot,
    executable: join(payloadRoot, ...portable.executable.split("/")),
  };
}

async function verifyCandidate(directory, portable) {
  let names;
  try {
    names = (await readdir(directory)).sort();
  } catch (error) {
    throw runError("CACHE_INVALID", "无法读取版本缓存。", error);
  }
  const expectedNames = [
    CACHE_ENTRY_MARKER_NAME,
    PAYLOAD_DIRECTORY,
    portable.file_name,
  ].sort();
  if (
    names.length !== expectedNames.length ||
    names.some((name, index) => name !== expectedNames[index])
  ) {
    throw runError("CACHE_INVALID", "版本缓存成员不完整或包含额外文件。");
  }
  const locations = candidateLocations(directory, portable);
  await verifyPortableAsset(locations.archivePath, portable);
  await verifyExtractedTree(locations.payloadRoot, { assets: { portable } });
  return locations;
}

async function parseRemoteChecksums(path, identity, portable) {
  let text;
  try {
    const info = await lstat(path, { bigint: true });
    if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1n || info.size > 64n * 1024n) {
      throw runError("DOWNLOAD_INTEGRITY", "远程校验表的文件类型或大小无效。");
    }
    text = await readFile(path, "utf8");
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw runError("DOWNLOAD_INTEGRITY", "无法读取远程校验表。", error);
  }
  if (text.startsWith("\uFEFF") || !text.endsWith("\n")) {
    throw runError("DOWNLOAD_INTEGRITY", "远程校验表格式无效。");
  }
  const lineEnding = text.includes("\r") ? "\r\n" : "\n";
  if (lineEnding === "\r\n" && text.replaceAll("\r\n", "").includes("\r")) {
    throw runError("DOWNLOAD_INTEGRITY", "远程校验表换行格式无效。");
  }
  const entries = new Map();
  for (const line of text.slice(0, -lineEnding.length).split(lineEnding)) {
    const match = line.match(/^([a-f0-9]{64})  ([A-Za-z0-9._-]+)$/u);
    if (
      !match ||
      !SHA256.test(match[1]) ||
      !SAFE_ASSET_NAME.test(match[2]) ||
      entries.has(match[2].toUpperCase())
    ) {
      throw runError("DOWNLOAD_INTEGRITY", "远程校验表包含无效或重复条目。");
    }
    entries.set(match[2].toUpperCase(), { fileName: match[2], sha256: match[1] });
  }
  const target = entries.get(identity.portableFileName.toUpperCase());
  if (
    !target ||
    target.fileName !== identity.portableFileName ||
    target.sha256 !== portable.sha256
  ) {
    throw runError("DOWNLOAD_INTEGRITY", "远程校验表与 npm 内置哈希不一致。");
  }
}

function networkRequired(error) {
  if (
    isLauncherError(error) &&
    [
      "DOWNLOAD_FAILED",
      "DOWNLOAD_TIMEOUT",
      "TOO_MANY_REDIRECTS",
      "INVALID_DOWNLOAD_URL",
    ].includes(error.code)
  ) {
    return runError(
      "NETWORK_REQUIRED",
      "首次运行或修复缓存需要联网，请检查网络后重试。",
      error,
    );
  }
  return error;
}

async function prepareCandidate({
  paths,
  lock,
  identity,
  portable,
  localArchivePath,
  dependencies,
}) {
  const token = lock.token;
  let stagingDirectory = await createVersionStaging(paths, { token });
  const locations = candidateLocations(stagingDirectory, portable);
  let source;
  try {
    if (localArchivePath) {
      await copyFile(localArchivePath, locations.archivePath, fsConstants.COPYFILE_EXCL);
      await verifyPortableAsset(locations.archivePath, portable);
      source = "repaired";
    } else {
      const checksumsPath = join(stagingDirectory, REMOTE_CHECKSUMS_FILE);
      try {
        await dependencies.downloadChecksums({ identity, destination: checksumsPath });
      } catch (error) {
        throw networkRequired(error);
      }
      await parseRemoteChecksums(checksumsPath, identity, portable);
      await unlink(checksumsPath);
      try {
        await dependencies.downloadPortable({
          identity,
          portable,
          destination: locations.archivePath,
        });
      } catch (error) {
        throw networkRequired(error);
      }
      await verifyPortableAsset(locations.archivePath, portable);
      source = "downloaded";
    }

    await mkdir(locations.payloadRoot);
    await extractPortableArchive({
      archivePath: locations.archivePath,
      destination: locations.payloadRoot,
    });
    await verifyExtractedTree(locations.payloadRoot, { assets: { portable } });
    await publishVersionStaging({
      paths,
      stagingDirectory,
      token,
      lock,
      validateCandidate: (directory) => verifyCandidate(directory, portable),
    });
    stagingDirectory = undefined;
    return source;
  } finally {
    if (stagingDirectory && await pathExists(stagingDirectory)) {
      await discardVersionStaging(paths, { stagingDirectory, token });
    }
  }
}

async function launchFromCandidate(directory, portable, dependencies) {
  const locations = await verifyCandidate(directory, portable);
  await dependencies.launchApplication({
    executable: locations.executable,
    cwd: locations.portableRoot,
  });
}

async function launchFromPublishedCache(paths, portable, dependencies) {
  await assertVersionCacheOwned(paths);
  return launchFromCandidate(paths.versionDirectory, portable, dependencies);
}

async function runCore(options, dependencies) {
  const { command, version, localAppData, runtime } = options ?? {};
  if (!command || !["launch", "clear-cache"].includes(command.kind)) {
    throw runError("INVALID_COMMAND", "启动器命令无效。");
  }
  assertSupportedRuntime(runtime);
  const identity = deriveReleaseIdentity(version);
  const paths = resolveCachePaths({ localAppData, version });
  if (command.kind === "clear-cache") {
    const { removed } = await clearCacheRoot(paths, { token: randomUUID() });
    return { kind: "cache-cleared", removed, exitCode: 0, stdout: removed ? "启动器缓存已清理。\n" : "启动器缓存为空。\n", stderr: "" };
  }

  const manifest = validateReleaseManifest(options.manifest, { packageVersion: version });
  const portable = manifest.assets.portable;
  const lock = await acquireVersionLock(paths);
  try {
    if (await pathExists(paths.versionDirectory)) {
      try {
        await launchFromPublishedCache(paths, portable, dependencies);
        return { kind: "launched", source: "cache", exitCode: 0, stdout: "正在启动 AI 能力雷达…\n", stderr: "" };
      } catch (error) {
        if (
          isLauncherError(error) &&
          ["LAUNCH_FAILED", "CACHE_OWNERSHIP"].includes(error.code)
        ) throw error;
      }
    } else {
      const recovered = await recoverVersionPublication({
        paths,
        lock,
        validateCandidate: (directory) => verifyCandidate(directory, portable),
      });
      if (recovered) {
        await launchFromPublishedCache(paths, portable, dependencies);
        return { kind: "launched", source: "cache", exitCode: 0, stdout: "正在启动 AI 能力雷达…\n", stderr: "" };
      }
    }

    let localArchivePath;
    if (await pathExists(paths.versionDirectory)) {
      const candidate = candidateLocations(paths.versionDirectory, portable).archivePath;
      try {
        await assertVersionCacheOwned(paths);
        await verifyPortableAsset(candidate, portable);
        localArchivePath = candidate;
      } catch {
        localArchivePath = undefined;
      }
    }
    const source = await prepareCandidate({
      paths,
      lock,
      identity,
      portable,
      localArchivePath,
      dependencies,
    });
    await launchFromPublishedCache(paths, portable, dependencies);
    const action = source === "repaired" ? "已从本地缓存修复，正在启动 AI 能力雷达…\n" : "已下载并验证，正在启动 AI 能力雷达…\n";
    return { kind: "launched", source, exitCode: 0, stdout: action, stderr: "" };
  } finally {
    await lock.release();
  }
}

export function runLauncherCommand({ command, version, manifest } = {}) {
  return runCore(
    {
      command,
      version,
      manifest,
      localAppData: process.env.LOCALAPPDATA,
      runtime: {
        platform: process.platform,
        arch: process.arch,
        nodeVersion: process.versions.node,
      },
    },
    PRODUCTION_DEPENDENCIES,
  );
}

export function runLauncherCommandForTest(options, dependencies) {
  if (
    !process.env.NODE_TEST_CONTEXT ||
    !dependencies ||
    typeof dependencies.downloadChecksums !== "function" ||
    typeof dependencies.downloadPortable !== "function" ||
    typeof dependencies.launchApplication !== "function"
  ) {
    throw runError("INVALID_COMMAND", "测试运行入口不可用于生产运行。");
  }
  return runCore(options, Object.freeze({ ...dependencies }));
}
