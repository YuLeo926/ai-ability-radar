import { createHash } from "node:crypto";
import { lstat, open, readdir } from "node:fs/promises";
import { join } from "node:path";

import { LauncherError, isLauncherError } from "./errors.mjs";
import { PORTABLE_ROOT_DIRECTORY } from "./manifest.mjs";

const MAX_INTERNAL_CHECKSUM_BYTES = 4 * 1024 * 1024;
const SHA256 = /^[a-f0-9]{64}$/u;
const RESERVED_WINDOWS_NAME = /^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?$/iu;

function treeError(detail, cause) {
  return new LauncherError(
    "INVALID_EXTRACTED_TREE",
    `解压后的便携版无效：${detail}。`,
    cause === undefined ? undefined : { cause },
  );
}

function identity(info) {
  return {
    dev: info.dev,
    ino: info.ino,
    birthtimeNs: info.birthtimeNs,
    size: info.size,
    mtimeNs: info.mtimeNs,
    ctimeNs: info.ctimeNs,
  };
}

function sameIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.birthtimeNs === right.birthtimeNs &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs
  );
}

async function collectTree(root) {
  const rootInfo = await lstat(root, { bigint: true });
  if (!rootInfo.isDirectory() || rootInfo.isSymbolicLink()) {
    throw treeError("校验根目录不是普通目录");
  }
  const directories = new Map([["", identity(rootInfo)]]);
  const files = new Map();

  async function visit(absoluteDirectory, relativeDirectory) {
    const names = await readdir(absoluteDirectory);
    names.sort((left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right)));
    for (const name of names) {
      if (
        name.length === 0 ||
        name === "." ||
        name === ".." ||
        name.includes("/") ||
        name.includes("\\") ||
        name.endsWith(".") ||
        name.endsWith(" ") ||
        /[\u0000-\u001f\u007f<>:"|?*]/u.test(name) ||
        RESERVED_WINDOWS_NAME.test(name) ||
        name.normalize("NFC") !== name
      ) {
        throw treeError("文件树包含不安全的名称");
      }
      const relativePath = relativeDirectory ? `${relativeDirectory}/${name}` : name;
      const absolutePath = join(absoluteDirectory, name);
      const info = await lstat(absolutePath, { bigint: true });
      if (info.isSymbolicLink()) {
        throw treeError("文件树包含链接或重解析点");
      }
      if (info.isDirectory()) {
        directories.set(relativePath, identity(info));
        await visit(absolutePath, relativePath);
      } else if (info.isFile() && info.nlink === 1n) {
        files.set(relativePath, { absolutePath, before: identity(info) });
      } else {
        throw treeError("文件树包含不支持的条目类型");
      }
    }
  }

  await visit(root, "");
  return { directories, files };
}

function expectedDirectories(files) {
  const directories = new Set([PORTABLE_ROOT_DIRECTORY]);
  for (const file of files) {
    const segments = file.path.split("/");
    for (let index = 1; index < segments.length; index += 1) {
      directories.add(segments.slice(0, index).join("/"));
    }
  }
  return directories;
}

async function hashOrdinaryFile(record, expected, collectBytes) {
  let handle;
  try {
    handle = await open(record.absolutePath, "r");
    const before = await handle.stat({ bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      before.nlink !== 1n ||
      !sameIdentity(record.before, identity(before)) ||
      before.size !== BigInt(expected.size)
    ) {
      throw treeError("文件身份或大小与发布清单不一致");
    }
    const digest = createHash("sha256");
    const chunks = collectBytes ? [] : undefined;
    for await (const value of handle.createReadStream({ autoClose: false })) {
      const chunk = Buffer.isBuffer(value) ? value : Buffer.from(value);
      digest.update(chunk);
      chunks?.push(chunk);
    }
    const after = await handle.stat({ bigint: true });
    if (!sameIdentity(identity(before), identity(after))) {
      throw treeError("读取期间文件发生变化");
    }
    const sha256 = digest.digest("hex");
    if (sha256 !== expected.sha256) {
      throw treeError("文件哈希与发布清单不一致");
    }
    return chunks === undefined ? undefined : Buffer.concat(chunks);
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw treeError("无法安全读取解压文件", error);
  } finally {
    await handle?.close().catch(() => {});
  }
}

function parseInternalChecksums(bytes, manifestFiles) {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_INTERNAL_CHECKSUM_BYTES) {
    throw treeError("内部校验文件大小无效");
  }
  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch (error) {
    throw treeError("内部校验文件不是有效 UTF-8", error);
  }
  if (text.startsWith("\uFEFF") || !text.endsWith("\n") || /\r|\u0000/u.test(text)) {
    throw treeError("内部校验文件格式无效");
  }
  const actual = new Map();
  for (const line of text.slice(0, -1).split("\n")) {
    const match = line.match(/^([a-f0-9]{64})  ([^\s].*)$/u);
    if (!match || !SHA256.test(match[1])) {
      throw treeError("内部校验文件行格式无效");
    }
    const path = match[2];
    const segments = path.split("/");
    if (
      path.startsWith("/") ||
      path.includes("\\") ||
      path.includes(":") ||
      segments.some((segment) =>
        segment.length === 0 ||
        segment === "." ||
        segment === ".." ||
        segment.endsWith(".") ||
        segment.endsWith(" ") ||
        /[\u0000-\u001f\u007f<>:"|?*]/u.test(segment) ||
        RESERVED_WINDOWS_NAME.test(segment)
      ) ||
      actual.has(path.toUpperCase())
    ) {
      throw treeError("内部校验文件包含不安全或重复路径");
    }
    actual.set(path.toUpperCase(), { path, sha256: match[1] });
  }

  const expected = manifestFiles
    .filter(({ path }) => path !== `${PORTABLE_ROOT_DIRECTORY}/SHA256SUMS.txt`)
    .map(({ path, sha256 }) => ({
      path: path.slice(`${PORTABLE_ROOT_DIRECTORY}/`.length),
      sha256,
    }));
  if (actual.size !== expected.length) {
    throw treeError("内部校验文件的成员数量不一致");
  }
  for (const item of expected) {
    const found = actual.get(item.path.toUpperCase());
    if (!found || found.path !== item.path || found.sha256 !== item.sha256) {
      throw treeError("内部校验文件与发布清单不一致");
    }
  }
}

export async function verifyExtractedTree(root, manifest) {
  try {
    const expectedFiles = manifest?.assets?.portable?.files;
    if (!Array.isArray(expectedFiles) || expectedFiles.length === 0) {
      throw treeError("发布清单缺少文件列表");
    }
    const collected = await collectTree(root);
    const expectedByPath = new Map(expectedFiles.map((file) => [file.path, file]));
    if (
      expectedByPath.size !== expectedFiles.length ||
      collected.files.size !== expectedFiles.length ||
      [...collected.files.keys()].some((path) => !expectedByPath.has(path))
    ) {
      throw treeError("文件成员与发布清单不一致");
    }
    const directories = expectedDirectories(expectedFiles);
    const actualDirectories = new Set([...collected.directories.keys()].filter(Boolean));
    if (
      actualDirectories.size !== directories.size ||
      [...actualDirectories].some((path) => !directories.has(path))
    ) {
      throw treeError("目录成员与发布清单不一致");
    }

    let checksumBytes;
    let totalBytes = 0;
    for (const expected of expectedFiles) {
      const record = collected.files.get(expected.path);
      const isChecksums = expected.path === `${PORTABLE_ROOT_DIRECTORY}/SHA256SUMS.txt`;
      if (isChecksums && expected.size > MAX_INTERNAL_CHECKSUM_BYTES) {
        throw treeError("内部校验文件过大");
      }
      const bytes = await hashOrdinaryFile(record, expected, isChecksums);
      if (isChecksums) checksumBytes = bytes;
      totalBytes += expected.size;
    }
    parseInternalChecksums(checksumBytes, expectedFiles);

    for (const [path, expectedIdentity] of collected.directories) {
      const current = await lstat(path ? join(root, ...path.split("/")) : root, { bigint: true });
      if (
        !current.isDirectory() ||
        current.isSymbolicLink() ||
        expectedIdentity.dev !== current.dev ||
        expectedIdentity.ino !== current.ino ||
        expectedIdentity.birthtimeNs !== current.birthtimeNs
      ) {
        throw treeError("校验期间目录身份发生变化");
      }
    }
    return { fileCount: expectedFiles.length, totalBytes };
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw treeError("文件树校验失败", error);
  }
}
