import { spawn } from "node:child_process";
import { lstat, open, readdir } from "node:fs/promises";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { LauncherError, isLauncherError } from "./errors.mjs";
import {
  MAX_EXTRACTED_FILE_BYTES,
  MAX_EXTRACTED_TOTAL_BYTES,
  MAX_PORTABLE_BYTES,
  PORTABLE_ROOT_DIRECTORY,
} from "./manifest.mjs";

const END_RECORD_BYTES = 22;
const CENTRAL_HEADER_BYTES = 46;
const LOCAL_HEADER_BYTES = 30;
const MAX_ENTRY_COUNT = 10_000;
const UTF8_FLAG = 0x0800;
const DEFLATE_OPTION_FLAGS = 0x0006;
const ENCRYPTED_FLAG = 0x0001;
const DATA_DESCRIPTOR_FLAG = 0x0008;
const WINDOWS_REPARSE_POINT = 0x0400;
const RESERVED_WINDOWS_NAME = /^(?:CON|PRN|AUX|NUL|CLOCK\$|CONIN\$|CONOUT\$|COM[1-9]|LPT[1-9])$/iu;

function archiveError(detail, cause) {
  return new LauncherError(
    "INVALID_ARCHIVE",
    `便携版压缩包无效：${detail}。`,
    cause === undefined ? undefined : { cause },
  );
}

function extractionError(detail, cause) {
  return new LauncherError(
    "EXTRACTION_FAILED",
    `便携版解压失败：${detail}。`,
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

async function readOrdinaryArchive(archivePath) {
  let handle;
  try {
    handle = await open(archivePath, "r");
    const before = await handle.stat({ bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      before.size < BigInt(END_RECORD_BYTES) ||
      before.size > BigInt(MAX_PORTABLE_BYTES)
    ) {
      throw archiveError("文件类型或大小不受支持");
    }
    const bytes = await handle.readFile();
    const after = await handle.stat({ bigint: true });
    if (!sameFileIdentity(fileIdentity(before), fileIdentity(after))) {
      throw archiveError("读取期间文件发生变化");
    }
    return bytes;
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw archiveError("无法安全读取文件", error);
  } finally {
    await handle?.close().catch(() => {});
  }
}

function normalizeMemberName(rawName) {
  if (
    typeof rawName !== "string" ||
    rawName.length === 0 ||
    rawName.length > 4_096 ||
    rawName.startsWith("/") ||
    rawName.startsWith("\\") ||
    rawName.includes("\\") ||
    rawName.normalize("NFC") !== rawName
  ) {
    throw archiveError("成员名称不安全");
  }
  const directory = rawName.endsWith("/");
  const path = directory ? rawName.slice(0, -1) : rawName;
  const segments = path.split("/");
  if (
    path.length === 0 ||
    segments.some((segment) =>
      segment.length === 0 ||
      segment === "." ||
      segment === ".." ||
      segment.endsWith(".") ||
      segment.endsWith(" ") ||
      Buffer.byteLength(segment, "utf8") > 255 ||
      /[\u0000-\u001f\u007f<>:"|?*]/u.test(segment) ||
      RESERVED_WINDOWS_NAME.test(segment.split(".", 1)[0])
    )
  ) {
    throw archiveError("成员路径包含不安全的 Windows 名称");
  }
  if (segments[0] !== PORTABLE_ROOT_DIRECTORY) {
    throw archiveError("成员不在固定便携版根目录中");
  }
  return {
    directory,
    path,
    key: path.toUpperCase(),
  };
}

function decodeMemberName(nameBytes, flags) {
  if (!(flags & UTF8_FLAG) && nameBytes.some((value) => value > 0x7f)) {
    throw archiveError("旧式文件名编码不受支持");
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(nameBytes);
  } catch (error) {
    throw archiveError("成员名称不是有效 UTF-8", error);
  }
}

function validateAttributes(versionMadeBy, externalAttributes, member) {
  if ((externalAttributes & WINDOWS_REPARSE_POINT) !== 0) {
    throw archiveError("成员包含链接或重解析点属性");
  }
  const creatorSystem = versionMadeBy >>> 8;
  if (creatorSystem === 3) {
    const unixType = (externalAttributes >>> 16) & 0xf000;
    const expectedType = member.directory ? 0x4000 : 0x8000;
    if (unixType !== expectedType) {
      throw archiveError("Unix 成员类型不是普通文件或目录");
    }
  }
}

function findEndRecord(bytes) {
  if (!Buffer.isBuffer(bytes) || bytes.length < END_RECORD_BYTES) {
    throw archiveError("文件已截断");
  }
  const offset = bytes.length - END_RECORD_BYTES;
  if (bytes.readUInt32LE(offset) !== 0x06054b50) {
    throw archiveError("末尾记录位置不正确");
  }
  if (bytes.readUInt16LE(offset + 20) !== 0) {
    throw archiveError("不支持压缩包注释");
  }
  return offset;
}

function parseArchive(bytes) {
  const endOffset = findEndRecord(bytes);
  const diskNumber = bytes.readUInt16LE(endOffset + 4);
  const centralDisk = bytes.readUInt16LE(endOffset + 6);
  const entriesOnDisk = bytes.readUInt16LE(endOffset + 8);
  const entryCount = bytes.readUInt16LE(endOffset + 10);
  const centralSize = bytes.readUInt32LE(endOffset + 12);
  const centralOffset = bytes.readUInt32LE(endOffset + 16);
  if (
    diskNumber !== 0 ||
    centralDisk !== 0 ||
    entriesOnDisk !== entryCount ||
    entryCount === 0 ||
    entryCount > MAX_ENTRY_COUNT
  ) {
    throw archiveError("不支持多磁盘或空压缩包");
  }
  if (
    entryCount === 0xffff ||
    centralSize === 0xffffffff ||
    centralOffset === 0xffffffff ||
    (endOffset >= 20 && bytes.readUInt32LE(endOffset - 20) === 0x07064b50)
  ) {
    throw archiveError("不支持 ZIP64");
  }
  if (
    !Number.isSafeInteger(centralOffset + centralSize) ||
    centralOffset + centralSize !== endOffset
  ) {
    throw archiveError("中央目录边界不正确");
  }

  const destinations = new Map();
  const entries = [];
  const localRanges = [];
  const localOffsets = new Set();
  let cursor = centralOffset;
  let totalUncompressedBytes = 0;
  for (let index = 0; index < entryCount; index += 1) {
    if (
      cursor + CENTRAL_HEADER_BYTES > endOffset ||
      bytes.readUInt32LE(cursor) !== 0x02014b50
    ) {
      throw archiveError("中央目录成员已损坏");
    }
    const versionMadeBy = bytes.readUInt16LE(cursor + 4);
    const versionNeeded = bytes.readUInt16LE(cursor + 6);
    const flags = bytes.readUInt16LE(cursor + 8);
    const method = bytes.readUInt16LE(cursor + 10);
    const crc32 = bytes.readUInt32LE(cursor + 16);
    const compressedSize = bytes.readUInt32LE(cursor + 20);
    const uncompressedSize = bytes.readUInt32LE(cursor + 24);
    const nameLength = bytes.readUInt16LE(cursor + 28);
    const extraLength = bytes.readUInt16LE(cursor + 30);
    const commentLength = bytes.readUInt16LE(cursor + 32);
    const diskStart = bytes.readUInt16LE(cursor + 34);
    const externalAttributes = bytes.readUInt32LE(cursor + 38);
    const localOffset = bytes.readUInt32LE(cursor + 42);
    const entryEnd = cursor + CENTRAL_HEADER_BYTES + nameLength + extraLength + commentLength;
    if (
      entryEnd > endOffset ||
      nameLength === 0 ||
      extraLength !== 0 ||
      commentLength !== 0 ||
      diskStart !== 0 ||
      versionNeeded > 20 ||
      (flags & (ENCRYPTED_FLAG | DATA_DESCRIPTOR_FLAG)) !== 0 ||
      (flags & ~(UTF8_FLAG | DEFLATE_OPTION_FLAGS)) !== 0 ||
      (method === 0 && (flags & DEFLATE_OPTION_FLAGS) !== 0) ||
      ![0, 8].includes(method) ||
      compressedSize === 0xffffffff ||
      uncompressedSize === 0xffffffff ||
      localOffset === 0xffffffff
    ) {
      throw archiveError("中央目录成员使用了不支持的 ZIP 功能");
    }
    const nameBytes = bytes.subarray(cursor + CENTRAL_HEADER_BYTES, cursor + CENTRAL_HEADER_BYTES + nameLength);
    const member = normalizeMemberName(decodeMemberName(nameBytes, flags));
    validateAttributes(versionMadeBy, externalAttributes, member);
    if (destinations.has(member.key)) {
      throw archiveError("成员目标重复或仅大小写不同");
    }
    destinations.set(member.key, member.directory ? "directory" : "file");
    if (
      (member.directory && (compressedSize !== 0 || uncompressedSize !== 0)) ||
      (!member.directory && uncompressedSize > MAX_EXTRACTED_FILE_BYTES) ||
      (method === 0 && compressedSize !== uncompressedSize)
    ) {
      throw archiveError("成员大小不受支持");
    }
    totalUncompressedBytes += uncompressedSize;
    if (
      !Number.isSafeInteger(totalUncompressedBytes) ||
      totalUncompressedBytes > MAX_EXTRACTED_TOTAL_BYTES
    ) {
      throw archiveError("解压后总大小超过限制");
    }

    if (
      localOffset + LOCAL_HEADER_BYTES > centralOffset ||
      bytes.readUInt32LE(localOffset) !== 0x04034b50 ||
      localOffsets.has(localOffset)
    ) {
      throw archiveError("本地成员头无效");
    }
    localOffsets.add(localOffset);
    const localVersion = bytes.readUInt16LE(localOffset + 4);
    const localFlags = bytes.readUInt16LE(localOffset + 6);
    const localMethod = bytes.readUInt16LE(localOffset + 8);
    const localCrc32 = bytes.readUInt32LE(localOffset + 14);
    const localCompressedSize = bytes.readUInt32LE(localOffset + 18);
    const localUncompressedSize = bytes.readUInt32LE(localOffset + 22);
    const localNameLength = bytes.readUInt16LE(localOffset + 26);
    const localExtraLength = bytes.readUInt16LE(localOffset + 28);
    const localHeaderEnd = localOffset + LOCAL_HEADER_BYTES + localNameLength + localExtraLength;
    const dataEnd = localHeaderEnd + compressedSize;
    if (
      localVersion !== versionNeeded ||
      localFlags !== flags ||
      localMethod !== method ||
      localCrc32 !== crc32 ||
      localCompressedSize !== compressedSize ||
      localUncompressedSize !== uncompressedSize ||
      localNameLength !== nameLength ||
      localExtraLength !== 0 ||
      localHeaderEnd > centralOffset ||
      dataEnd > centralOffset ||
      Buffer.compare(
        bytes.subarray(localOffset + LOCAL_HEADER_BYTES, localOffset + LOCAL_HEADER_BYTES + localNameLength),
        nameBytes,
      ) !== 0
    ) {
      throw archiveError("本地成员信息与中央目录不一致");
    }
    localRanges.push({ start: localOffset, end: dataEnd });
    entries.push({
      path: member.path,
      directory: member.directory,
      size: uncompressedSize,
      compressedSize,
      crc32,
    });
    cursor = entryEnd;
  }
  if (cursor !== endOffset) {
    throw archiveError("中央目录大小不一致");
  }
  localRanges.sort((left, right) => left.start - right.start);
  if (localRanges[0]?.start !== 0 || localRanges.at(-1)?.end !== centralOffset) {
    throw archiveError("本地成员之间包含未知数据");
  }
  for (let index = 1; index < localRanges.length; index += 1) {
    if (localRanges[index - 1].end !== localRanges[index].start) {
      throw archiveError("本地成员重叠或包含间隙");
    }
  }
  for (const entry of entries) {
    const segments = entry.path.split("/");
    for (let index = 1; index < segments.length; index += 1) {
      const ancestor = segments.slice(0, index).join("/").toUpperCase();
      if (destinations.get(ancestor) === "file") {
        throw archiveError("文件与目录目标发生别名冲突");
      }
    }
  }

  const files = entries
    .filter(({ directory }) => !directory)
    .map(({ path, size, compressedSize, crc32 }) => ({ path, size, compressedSize, crc32 }))
    .sort((left, right) => Buffer.compare(Buffer.from(left.path), Buffer.from(right.path)));
  const derivedDirectories = new Set([PORTABLE_ROOT_DIRECTORY]);
  for (const file of files) {
    const segments = file.path.split("/");
    for (let index = 1; index < segments.length; index += 1) {
      derivedDirectories.add(segments.slice(0, index).join("/"));
    }
  }
  const directories = entries
    .filter(({ directory }) => directory)
    .map(({ path }) => path);
  if (
    files.length === 0 ||
    directories.some((path) => !derivedDirectories.has(path))
  ) {
    throw archiveError("目录成员不属于文件清单的父目录");
  }
  return {
    files,
    directories: [...derivedDirectories].sort(),
    totalUncompressedBytes,
  };
}

export async function enumeratePortableArchive(archivePath) {
  return parseArchive(await readOrdinaryArchive(archivePath));
}

export async function inspectPortableArchive(archivePath, manifest) {
  const inspected = await enumeratePortableArchive(archivePath);
  const expected = manifest?.assets?.portable?.files;
  if (!Array.isArray(expected) || expected.length !== inspected.files.length) {
    throw archiveError("文件数量与发布清单不一致");
  }
  for (let index = 0; index < expected.length; index += 1) {
    const actualFile = inspected.files[index];
    const expectedFile = expected[index];
    if (actualFile.path !== expectedFile?.path || actualFile.size !== expectedFile?.size) {
      throw archiveError("文件路径或大小与发布清单不一致");
    }
  }
  return inspected;
}

async function requireExtractionPaths(archivePath, destination) {
  try {
    const archiveInfo = await lstat(archivePath, { bigint: true });
    const destinationInfo = await lstat(destination, { bigint: true });
    if (
      !archiveInfo.isFile() ||
      archiveInfo.isSymbolicLink() ||
      archiveInfo.size < BigInt(END_RECORD_BYTES) ||
      archiveInfo.size > BigInt(MAX_PORTABLE_BYTES) ||
      extname(archivePath) !== ".zip" ||
      !destinationInfo.isDirectory() ||
      destinationInfo.isSymbolicLink() ||
      (await readdir(destination)).length !== 0
    ) {
      throw extractionError("源文件或目标目录不符合要求");
    }
    return {
      archive: fileIdentity(archiveInfo),
      destination: fileIdentity(destinationInfo),
    };
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw extractionError("无法安全检查源文件或目标目录", error);
  }
}

function runExtractor(powershellPath, scriptPath, archivePath, destination) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      powershellPath,
      [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        scriptPath,
        "-Source",
        archivePath,
        "-Destination",
        destination,
      ],
      { shell: false, stdio: "ignore", windowsHide: true },
    );
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (code === 0 && signal === null) resolve();
      else reject(new Error("extractor process failed"));
    });
  });
}

export async function extractPortableArchive({ archivePath, destination } = {}) {
  if (typeof archivePath !== "string" || typeof destination !== "string") {
    throw extractionError("参数无效");
  }
  const before = await requireExtractionPaths(archivePath, destination);
  const systemRoot = process.env.SystemRoot;
  if (typeof systemRoot !== "string" || systemRoot.length === 0) {
    throw extractionError("找不到 Windows PowerShell");
  }
  const powershellPath = join(systemRoot, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const scriptPath = fileURLToPath(new URL("../extract.ps1", import.meta.url));
  try {
    const powershellInfo = await lstat(powershellPath);
    const scriptInfo = await lstat(scriptPath);
    if (
      !powershellInfo.isFile() ||
      powershellInfo.isSymbolicLink() ||
      !scriptInfo.isFile() ||
      scriptInfo.isSymbolicLink()
    ) {
      throw extractionError("固定解压程序不可用");
    }
    await runExtractor(powershellPath, scriptPath, archivePath, destination);
    const archiveAfter = await lstat(archivePath, { bigint: true });
    const destinationAfter = await lstat(destination, { bigint: true });
    if (
      !sameFileIdentity(before.archive, fileIdentity(archiveAfter)) ||
      before.destination.dev !== destinationAfter.dev ||
      before.destination.ino !== destinationAfter.ino ||
      before.destination.birthtimeNs !== destinationAfter.birthtimeNs ||
      !destinationAfter.isDirectory() ||
      destinationAfter.isSymbolicLink()
    ) {
      throw extractionError("解压期间路径身份发生变化");
    }
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw extractionError("安全解压程序执行失败", error);
  }
}
