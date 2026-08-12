import { createHash, randomUUID } from "node:crypto";
import {
  lstat,
  mkdtemp,
  open,
  readFile,
  readdir,
  rename,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  resolve,
} from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  enumeratePortableArchive,
  extractPortableArchive,
  inspectPortableArchive,
} from "../packages/launcher/lib/archive.mjs";
import { LauncherError, isLauncherError } from "../packages/launcher/lib/errors.mjs";
import {
  MANIFEST_SCHEMA_VERSION,
  PORTABLE_ROOT_DIRECTORY,
  deriveReleaseIdentity,
  validateReleaseManifest,
} from "../packages/launcher/lib/manifest.mjs";
import { verifyExtractedTree } from "../packages/launcher/lib/tree.mjs";

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url));
const LAUNCHER_PACKAGE_PATH = resolve(SCRIPT_DIRECTORY, "../packages/launcher/package.json");
const SHA256 = /^[a-f0-9]{64}$/u;
const SAFE_ASSET_NAME = /^[A-Za-z0-9._-]+$/u;
const MAX_CHECKSUM_BYTES = 64 * 1024;

function generationError(detail, cause) {
  return new LauncherError(
    "MANIFEST_GENERATION_FAILED",
    `启动器发布清单生成失败：${detail}。`,
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

async function requirePlainDirectory(path, detail) {
  try {
    const info = await lstat(path, { bigint: true });
    if (!info.isDirectory() || info.isSymbolicLink()) {
      throw generationError(detail);
    }
    return info;
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw generationError(detail, error);
  }
}

async function hashPlainFile(path, expectedSize) {
  let handle;
  try {
    handle = await open(path, "r");
    const before = await handle.stat({ bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      before.nlink !== 1n ||
      (expectedSize !== undefined && before.size !== BigInt(expectedSize))
    ) {
      throw generationError("资产或解压文件不是普通文件，或大小不一致");
    }
    const digest = createHash("sha256");
    for await (const value of handle.createReadStream({ autoClose: false })) {
      digest.update(value);
    }
    const after = await handle.stat({ bigint: true });
    if (!sameIdentity(identity(before), identity(after))) {
      throw generationError("读取期间文件身份发生变化");
    }
    return { size: Number(before.size), sha256: digest.digest("hex") };
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw generationError("无法安全读取资产或解压文件", error);
  } finally {
    await handle?.close().catch(() => {});
  }
}

async function readLauncherVersion() {
  let value;
  try {
    value = JSON.parse(await readFile(LAUNCHER_PACKAGE_PATH, "utf8"));
  } catch (error) {
    throw generationError("无法读取启动器包版本", error);
  }
  return deriveReleaseIdentity(value?.version);
}

async function validateAssetDirectory(assetsDir, identity) {
  await requirePlainDirectory(assetsDir, "资产目录不是普通目录");
  const expectedNames = [identity.portableFileName, identity.checksumsFileName].sort();
  const actualNames = (await readdir(assetsDir)).sort();
  if (
    actualNames.length !== expectedNames.length ||
    actualNames.some((name, index) => name !== expectedNames[index])
  ) {
    throw generationError("资产目录必须只包含目标便携 ZIP 和 SHA256SUMS.txt");
  }
  for (const name of actualNames) {
    const info = await lstat(join(assetsDir, name), { bigint: true });
    if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1n) {
      throw generationError("资产目录包含非普通文件");
    }
  }
}

function parseOuterChecksums(text, identity, portableSha256) {
  if (
    typeof text !== "string" ||
    text.length === 0 ||
    Buffer.byteLength(text, "utf8") > MAX_CHECKSUM_BYTES ||
    text.startsWith("\uFEFF") ||
    !text.endsWith("\n")
  ) {
    throw generationError("外层 SHA256SUMS.txt 格式无效");
  }
  const lineEnding = text.includes("\r") ? "\r\n" : "\n";
  if (lineEnding === "\r\n" && text.replaceAll("\r\n", "").includes("\r")) {
    throw generationError("外层 SHA256SUMS.txt 使用了混合换行");
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
      throw generationError("外层 SHA256SUMS.txt 包含无效或重复条目");
    }
    entries.set(match[2].toUpperCase(), { name: match[2], sha256: match[1] });
  }
  const target = entries.get(identity.portableFileName.toUpperCase());
  if (
    !target ||
    target.name !== identity.portableFileName ||
    target.sha256 !== portableSha256
  ) {
    throw generationError("外层校验表中的便携 ZIP 哈希不匹配");
  }
}

function manifestFromInventory(identity, archiveFile, files) {
  return validateReleaseManifest(
    {
      schema_version: MANIFEST_SCHEMA_VERSION,
      repository: identity.repository,
      launcher_version: identity.version,
      desktop_version: identity.version,
      tag: identity.tag,
      assets: {
        portable: {
          file_name: identity.portableFileName,
          size: archiveFile.size,
          sha256: archiveFile.sha256,
          root_directory: PORTABLE_ROOT_DIRECTORY,
          executable: `${PORTABLE_ROOT_DIRECTORY}/ability-radar.exe`,
          files,
        },
        checksums: { file_name: identity.checksumsFileName },
      },
    },
    { packageVersion: identity.version },
  );
}

async function atomicallyWriteManifest(outputPath, bytes) {
  const parent = dirname(outputPath);
  await requirePlainDirectory(parent, "输出目录不是普通目录");
  try {
    const existing = await readFile(outputPath);
    const info = await lstat(outputPath);
    if (!info.isFile() || info.isSymbolicLink()) {
      throw generationError("已有输出不是普通文件");
    }
    if (Buffer.compare(existing, bytes) === 0) return;
  } catch (error) {
    if (error?.code !== "ENOENT") {
      if (isLauncherError(error)) throw error;
      throw generationError("无法检查已有输出", error);
    }
  }

  const temporaryPath = join(parent, `.release-manifest-${randomUUID()}.tmp`);
  let handle;
  let renamed = false;
  try {
    handle = await open(temporaryPath, "wx", 0o600);
    await handle.writeFile(bytes);
    await handle.sync();
    await handle.close();
    handle = undefined;
    await rename(temporaryPath, outputPath);
    renamed = true;
  } catch (error) {
    throw generationError("无法原子写入发布清单", error);
  } finally {
    await handle?.close().catch(() => {});
    if (!renamed) await rm(temporaryPath, { force: true }).catch(() => {});
  }
}

async function generateLauncherManifest({ assetsDir, outputPath } = {}) {
  if (
    typeof assetsDir !== "string" ||
    typeof outputPath !== "string" ||
    !isAbsolute(assetsDir) ||
    !isAbsolute(outputPath) ||
    basename(outputPath) !== "release-manifest.json"
  ) {
    throw generationError("必须提供绝对资产目录和明确的 release-manifest.json 输出路径");
  }
  const identity = await readLauncherVersion();
  await validateAssetDirectory(assetsDir, identity);
  const archivePath = join(assetsDir, identity.portableFileName);
  const checksumsPath = join(assetsDir, identity.checksumsFileName);
  const archiveFile = await hashPlainFile(archivePath);
  parseOuterChecksums(await readFile(checksumsPath, "utf8"), identity, archiveFile.sha256);
  const archive = await enumeratePortableArchive(archivePath);

  const extractionDirectory = await mkdtemp(join(tmpdir(), "ability-radar-manifest-extract-"));
  try {
    await extractPortableArchive({ archivePath, destination: extractionDirectory });
    const files = [];
    for (const entry of archive.files) {
      const measured = await hashPlainFile(
        join(extractionDirectory, ...entry.path.split("/")),
        entry.size,
      );
      files.push({ path: entry.path, size: entry.size, sha256: measured.sha256 });
    }
    const manifest = manifestFromInventory(identity, archiveFile, files);
    await inspectPortableArchive(archivePath, manifest);
    await verifyExtractedTree(extractionDirectory, manifest);
    const bytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    await atomicallyWriteManifest(outputPath, bytes);
    return manifest;
  } catch (error) {
    if (isLauncherError(error) && error.code === "MANIFEST_GENERATION_FAILED") throw error;
    throw generationError("资产结构、解压树或内部校验不一致", error);
  } finally {
    await rm(extractionDirectory, { recursive: true, force: true });
  }
}

function parseArguments(args) {
  if (!Array.isArray(args) || args.length !== 4) {
    throw generationError("用法：--assets-dir <绝对目录> --output <输出文件>");
  }
  const values = new Map();
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index];
    if (!["--assets-dir", "--output"].includes(flag) || values.has(flag)) {
      throw generationError("只支持 --assets-dir 和 --output");
    }
    values.set(flag, args[index + 1]);
  }
  const output = values.get("--output");
  return {
    assetsDir: values.get("--assets-dir"),
    outputPath: isAbsolute(output ?? "") ? output : resolve(output ?? ""),
  };
}

export function generateLauncherManifestForTest(options) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw generationError("测试生成入口不可用于生产运行");
  }
  return generateLauncherManifest(options);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  generateLauncherManifest(parseArguments(process.argv.slice(2)))
    .then((manifest) => {
      process.stdout.write(`已生成启动器发布清单 ${manifest.tag}。\n`);
    })
    .catch((error) => {
      const message = isLauncherError(error) ? error.message : "启动器发布清单生成失败。";
      process.stderr.write(`${message}\n`);
      process.exitCode = 1;
    });
}
