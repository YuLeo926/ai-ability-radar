import { createHash, randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFile,
  link,
  lstat,
  mkdir,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual, TextDecoder } from "node:util";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  parse,
  relative,
  resolve,
  sep,
} from "node:path";

const strictSemver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const packId = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const verifierId = /^[a-z0-9-]+$/;
const contentSeal = /^[a-f0-9]{64}$/;
const MAX_MANIFEST_BYTES = 256 * 1024;
const MAX_PROMPT_BYTES = 256 * 1024;
const MAX_PACK_FILE_BYTES = 2 * 1024 * 1024;
const MAX_PACK_BYTES = 32 * 1024 * 1024;
const MAX_PACK_ENTRIES = 4_096;
const packValidatorLeaf = process.platform === "win32"
  ? "ability-pack-validator.exe"
  : "ability-pack-validator";
const expectedPackIdentities = [
  { id: "client-quick", path: "client-quick-v1" },
  { id: "cli-quick", path: "cli-quick-v1" },
];
const targetKinds = new Set([
  "chat_gpt_client",
  "claude_client",
  "codex_cli",
  "claude_code",
]);
const taskCategories = new Set([
  "instruction_following",
  "logic",
  "code_review",
  "cli_coding",
]);
let runtimePackValidatorObserver;

function comparable(path) {
  const absolute = resolve(path);
  return process.platform === "win32" ? absolute.toLowerCase() : absolute;
}

function samePath(left, right) {
  return comparable(left) === comparable(right);
}

function assertInside(root, candidate, label) {
  const from = resolve(root);
  const to = resolve(candidate);
  const child = relative(from, to);
  if (
    !child ||
    child.startsWith(`..${sep}`) ||
    child === ".." ||
    isAbsolute(child)
  ) {
    throw new Error(`${label} must stay inside target directory`);
  }
}

async function pathInfo(path) {
  try {
    return await lstat(path);
  } catch (error) {
    if (error?.code === "ENOENT") return undefined;
    throw error;
  }
}

async function canonicalExisting(path, label) {
  const absolute = resolve(path);
  const root = parse(absolute).root;
  let current = root;
  for (const part of relative(root, absolute).split(sep).filter(Boolean)) {
    current = join(current, part);
    const info = await lstat(current);
    if (info.isSymbolicLink()) {
      throw new Error(`${label} must not contain symbolic link or reparse indirection`);
    }
    const canonical = await realpath(current);
    if (!samePath(canonical, current)) {
      throw new Error(`${label} must not contain filesystem indirection`);
    }
  }
  return realpath(absolute);
}

async function requireDirectory(path, label) {
  const canonical = await canonicalExisting(path, label);
  if (!(await lstat(path)).isDirectory()) {
    throw new Error(`${label} must be a directory`);
  }
  return canonical;
}

async function requireFile(path, label) {
  const canonical = await canonicalExisting(path, label);
  if (!(await lstat(path)).isFile()) {
    throw new Error(`${label} must be a regular file`);
  }
  return canonical;
}

async function ensureDirectory(path, label) {
  const absolute = resolve(path);
  const missing = [];
  let current = absolute;
  while (!(await pathInfo(current))) {
    missing.push(basename(current));
    const parent = dirname(current);
    if (parent === current) {
      throw new Error(`${label} has no existing canonical ancestor`);
    }
    current = parent;
  }
  await requireDirectory(current, label);
  for (const part of missing.reverse()) {
    current = join(current, part);
    try {
      await mkdir(current);
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
    }
    await requireDirectory(current, label);
  }
  return requireDirectory(absolute, label);
}

async function entriesUnder(root, current = root) {
  const canonicalRoot = await requireDirectory(root, "portable tree root");
  const canonicalCurrent = await requireDirectory(
    current,
    "portable tree directory",
  );
  if (!samePath(canonicalRoot, canonicalCurrent)) {
    assertInside(canonicalRoot, canonicalCurrent, "portable tree directory");
  }
  const entries = await readdir(current, { withFileTypes: true });
  entries.sort((left, right) =>
    left.name < right.name ? -1 : left.name > right.name ? 1 : 0,
  );
  const result = [];
  for (const entry of entries) {
    const path = join(current, entry.name);
    const info = await lstat(path);
    if (info.isSymbolicLink()) {
      throw new Error(
        `portable tree entry must not contain symbolic link or reparse indirection: ${path}`,
      );
    }
    const canonical = await canonicalExisting(path, "portable tree entry");
    assertInside(canonicalRoot, canonical, "portable tree entry");
    if (info.isDirectory()) {
      result.push({ path, directory: true });
      result.push(...await entriesUnder(root, path));
    } else if (info.isFile()) {
      result.push({ path, directory: false });
    } else {
      throw new Error(`portable tree entry must be a regular file or directory: ${path}`);
    }
  }
  return result;
}

function plainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireExactKeys(value, expected, label) {
  if (!plainObject(value)) {
    throw new Error(`${label} must be a JSON object`);
  }
  const actual = Object.keys(value).sort();
  const required = [...expected].sort();
  if (!isDeepStrictEqual(actual, required)) {
    throw new Error(`${label} has an invalid schema`);
  }
}

async function readBoundedJson(path, maximumBytes, label) {
  const canonical = await requireFile(path, label);
  const info = await lstat(canonical);
  if (info.size > maximumBytes) {
    throw new Error(`${label} exceeds its size limit`);
  }
  const bytes = await readFile(canonical);
  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`${label} is not UTF-8`);
  }
  try {
    return { value: JSON.parse(text), bytes };
  } catch {
    throw new Error(`${label} is not valid JSON`);
  }
}

function safePackRelativePath(value) {
  return typeof value === "string" &&
    value.length > 0 &&
    !value.startsWith("/") &&
    !value.startsWith("\\") &&
    !value.includes(":") &&
    !value.split(/[\\/]/).includes("..");
}

function validateRegistry(registry, label) {
  requireExactKeys(registry, ["schema_version", "packs"], label);
  if (registry.schema_version !== 1 || !Array.isArray(registry.packs)) {
    throw new Error(`${label} has an unsupported registry schema`);
  }
  if (registry.packs.length !== expectedPackIdentities.length) {
    throw new Error(`${label} must contain exactly two portable packs`);
  }
  for (const [index, entry] of registry.packs.entries()) {
    requireExactKeys(
      entry,
      [
        "bundled",
        "content_sha256",
        "id",
        "license",
        "path",
        "version",
      ],
      `${label} entry`,
    );
    const expected = expectedPackIdentities[index];
    if (
      entry.bundled !== true ||
      entry.id !== expected.id ||
      entry.path !== expected.path ||
      !safePackRelativePath(entry.path) ||
      entry.license !== "Apache-2.0" ||
      !strictSemver.test(entry.version) ||
      !contentSeal.test(entry.content_sha256)
    ) {
      throw new Error(`${label} entry has an invalid identity, path, or seal`);
    }
  }
  return registry;
}

function validateGrader(grader, label) {
  if (!plainObject(grader) || typeof grader.type !== "string") {
    throw new Error(`${label} has an invalid grader`);
  }
  switch (grader.type) {
    case "exact_text":
      requireExactKeys(grader, ["type", "expected"], label);
      if (typeof grader.expected !== "string") {
        throw new Error(`${label} exact text grader is invalid`);
      }
      break;
    case "exact_json":
      requireExactKeys(grader, ["type", "expected"], label);
      break;
    case "json_string_set":
      requireExactKeys(grader, ["type", "expected"], label);
      if (
        !Array.isArray(grader.expected) ||
        grader.expected.some((value) => typeof value !== "string") ||
        new Set(grader.expected).size !== grader.expected.length
      ) {
        throw new Error(`${label} JSON string set grader is invalid`);
      }
      break;
    case "external_verifier":
      requireExactKeys(grader, ["type", "verifier_id"], label);
      if (
        typeof grader.verifier_id !== "string" ||
        !verifierId.test(grader.verifier_id)
      ) {
        throw new Error(`${label} external verifier is invalid`);
      }
      break;
    default:
      throw new Error(`${label} has an unsupported grader`);
  }
}

async function requirePackChild(packRoot, relativePath, kind, label) {
  if (!safePackRelativePath(relativePath)) {
    throw new Error(`${label} contains an unsafe pack path`);
  }
  const child = join(packRoot, ...relativePath.split(/[\\/]/));
  assertInside(packRoot, child, label);
  let canonical;
  try {
    canonical = kind === "file"
      ? await requireFile(child, label)
      : await requireDirectory(child, label);
  } catch (error) {
    throw new Error(`${label} is missing or invalid`, { cause: error });
  }
  assertInside(packRoot, canonical, label);
  return canonical;
}

async function validateManifest(packRoot, expectedEntry, label) {
  const manifestPath = join(packRoot, "manifest.json");
  const { value: manifest } = await readBoundedJson(
    manifestPath,
    MAX_MANIFEST_BYTES,
    `${label} manifest`,
  );
  requireExactKeys(
    manifest,
    ["schema_version", "id", "version", "title", "target_kinds", "tasks"],
    `${label} manifest`,
  );
  if (
    manifest.schema_version !== 1 ||
    manifest.id !== expectedEntry.id ||
    manifest.version !== expectedEntry.version ||
    typeof manifest.title !== "string" ||
    manifest.title.trim().length === 0 ||
    !Array.isArray(manifest.target_kinds) ||
    manifest.target_kinds.length === 0 ||
    manifest.target_kinds.some((kind) => !targetKinds.has(kind)) ||
    !Array.isArray(manifest.tasks) ||
    manifest.tasks.length === 0
  ) {
    throw new Error(`${label} manifest identity or required fields mismatch`);
  }

  const taskIds = new Set();
  for (const task of manifest.tasks) {
    requireExactKeys(
      task,
      [
        "id",
        "category",
        "prompt_file",
        "starter_dir",
        "time_budget_secs",
        "max_turns",
        "grader",
      ],
      `${label} task`,
    );
    if (
      typeof task.id !== "string" ||
      !packId.test(task.id) ||
      taskIds.has(task.id) ||
      !taskCategories.has(task.category) ||
      !Number.isSafeInteger(task.time_budget_secs) ||
      task.time_budget_secs < 1 ||
      task.time_budget_secs > 7_200 ||
      !Number.isSafeInteger(task.max_turns) ||
      task.max_turns < 1 ||
      task.max_turns > 100
    ) {
      throw new Error(`${label} task manifest fields are invalid`);
    }
    taskIds.add(task.id);
    const prompt = await requirePackChild(
      packRoot,
      task.prompt_file,
      "file",
      `${label} prompt`,
    );
    const promptInfo = await lstat(prompt);
    if (promptInfo.size > MAX_PROMPT_BYTES) {
      throw new Error(`${label} prompt exceeds its size limit`);
    }
    try {
      new TextDecoder("utf-8", { fatal: true }).decode(await readFile(prompt));
    } catch {
      throw new Error(`${label} prompt is not UTF-8`);
    }
    if (task.starter_dir !== null) {
      if (typeof task.starter_dir !== "string") {
        throw new Error(`${label} starter directory is invalid`);
      }
      await requirePackChild(
        packRoot,
        task.starter_dir,
        "directory",
        `${label} starter directory`,
      );
    }
    validateGrader(task.grader, `${label} task grader`);
  }
}

async function packDirectoryHash(packRoot, label) {
  const entries = await entriesUnder(packRoot);
  if (entries.length > MAX_PACK_ENTRIES) {
    throw new Error(`${label} exceeds the pack entry limit`);
  }
  const files = [];
  let totalBytes = 0;
  for (const entry of entries.filter(({ directory }) => !directory)) {
    const info = await lstat(entry.path);
    if (info.size > MAX_PACK_FILE_BYTES) {
      throw new Error(`${label} contains an oversized file`);
    }
    totalBytes += info.size;
    if (!Number.isSafeInteger(totalBytes) || totalBytes > MAX_PACK_BYTES) {
      throw new Error(`${label} exceeds the total pack size limit`);
    }
    const name = relative(packRoot, entry.path).split(sep).join("/");
    if (!safePackRelativePath(name)) {
      throw new Error(`${label} contains an unsafe pack path`);
    }
    files.push({ name, path: entry.path, size: info.size });
  }
  files.sort((left, right) =>
    left.name < right.name ? -1 : left.name > right.name ? 1 : 0,
  );
  const digest = createHash("sha256");
  for (const file of files) {
    const name = Buffer.from(file.name, "utf8");
    const nameLength = Buffer.alloc(8);
    const fileLength = Buffer.alloc(8);
    nameLength.writeBigUInt64LE(BigInt(name.length));
    fileLength.writeBigUInt64LE(BigInt(file.size));
    digest.update(nameLength);
    digest.update(name);
    digest.update(fileLength);
    digest.update(await readFile(file.path));
  }
  return digest.digest("hex");
}

async function loadTrustedRegistry(repoRoot) {
  const trustedRoot = join(repoRoot, "benchmark-packs");
  const canonicalRepo = await requireDirectory(repoRoot, "repository root");
  const canonicalTrusted = await requireDirectory(
    trustedRoot,
    "committed benchmark packs",
  );
  assertInside(canonicalRepo, canonicalTrusted, "committed benchmark packs");
  const { value } = await readBoundedJson(
    join(trustedRoot, "registry.json"),
    MAX_MANIFEST_BYTES,
    "committed portable pack registry",
  );
  return validateRegistry(value, "committed portable pack registry");
}

function runRuntimePackValidator(validatorPath, packsRoot, label) {
  runtimePackValidatorObserver?.(label);
  const result = spawnSync(validatorPath, [packsRoot], {
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`${label} was rejected by the runtime pack parser`);
  }
}

async function validatePortablePacks(
  packsRoot,
  trustedRegistry,
  label,
  validatorPath,
) {
  const canonicalPacks = await requireDirectory(packsRoot, label);
  const topLevel = await readdir(canonicalPacks, { withFileTypes: true });
  topLevel.sort((left, right) =>
    left.name < right.name ? -1 : left.name > right.name ? 1 : 0,
  );
  const actualTopLevel = topLevel.map((entry) => ({
    name: entry.name,
    directory: entry.isDirectory(),
    file: entry.isFile(),
  }));
  const expectedTopLevel = [
    { name: "cli-quick-v1", directory: true, file: false },
    { name: "client-quick-v1", directory: true, file: false },
    { name: "registry.json", directory: false, file: true },
  ];
  if (!isDeepStrictEqual(actualTopLevel, expectedTopLevel)) {
    throw new Error(`${label} must contain the exact two portable pack directories`);
  }

  const { value: registry } = await readBoundedJson(
    join(canonicalPacks, "registry.json"),
    MAX_MANIFEST_BYTES,
    `${label} registry`,
  );
  validateRegistry(registry, `${label} registry`);
  if (!isDeepStrictEqual(registry, trustedRegistry)) {
    throw new Error(`${label} registry does not match the committed registry`);
  }
  for (const entry of trustedRegistry.packs) {
    const packRoot = join(canonicalPacks, entry.path);
    await validateManifest(packRoot, entry, `${label} ${entry.id}`);
    const actualHash = await packDirectoryHash(packRoot, `${label} ${entry.id}`);
    if (actualHash !== entry.content_sha256) {
      throw new Error(`${label} content hash does not match the registry seal`);
    }
  }
  runRuntimePackValidator(validatorPath, canonicalPacks, label);
}

function fileIdentity(info) {
  return { dev: info.dev, ino: info.ino };
}

function sameIdentity(info, identity) {
  return info.dev === identity?.dev && info.ino === identity?.ino;
}

async function captureOwnedDirectory(path, canonicalRoot, label) {
  const info = await lstat(path);
  if (info.isSymbolicLink() || !info.isDirectory()) {
    throw new Error(`${label} ownership could not be established`);
  }
  const canonical = await canonicalExisting(path, label);
  assertInside(canonicalRoot, canonical, label);
  return fileIdentity(info);
}

async function requireOwnedDirectoryIdentity(
  path,
  canonicalRoot,
  label,
  identity,
) {
  const info = await lstat(path);
  if (
    info.isSymbolicLink() ||
    !info.isDirectory() ||
    !sameIdentity(info, identity)
  ) {
    throw new Error(`${label} ownership identity changed`);
  }
  const canonical = await canonicalExisting(path, label);
  assertInside(canonicalRoot, canonical, label);
}

async function requireOwnedFileIdentity(path, canonicalRoot, label, identity) {
  const info = await lstat(path);
  if (
    info.isSymbolicLink() ||
    !info.isFile() ||
    !sameIdentity(info, identity)
  ) {
    throw new Error(`${label} ownership identity changed`);
  }
  const canonical = await canonicalExisting(path, label);
  assertInside(canonicalRoot, canonical, label);
}

async function safeRemoveOwnedTree(path, canonicalRoot, label, identity) {
  if (!path) return;
  const info = await pathInfo(path);
  if (!info) return;
  if (
    !identity ||
    info.isSymbolicLink() ||
    !info.isDirectory() ||
    !sameIdentity(info, identity)
  ) {
    throw new Error(`${label} cleanup authority could not be established`);
  }
  const canonicalParent = await requireDirectory(dirname(path), `${label} parent`);
  assertInside(canonicalRoot, resolve(path), label);
  const quarantine = join(
    dirname(path),
    `.${basename(path)}.${randomUUID()}.quarantine`,
  );
  assertInside(canonicalRoot, quarantine, `${label} quarantine`);
  if (await pathInfo(quarantine)) {
    throw new Error(`${label} quarantine unexpectedly exists`);
  }
  await rename(path, quarantine);
  const movedInfo = await lstat(quarantine);
  if (
    movedInfo.isSymbolicLink() ||
    !movedInfo.isDirectory() ||
    !sameIdentity(movedInfo, identity)
  ) {
    throw new Error(`${label} cleanup identity changed`);
  }
  const canonicalQuarantine = await canonicalExisting(quarantine, label);
  assertInside(canonicalParent, canonicalQuarantine, `${label} quarantine`);
  await entriesUnder(quarantine);
  await rm(quarantine, { recursive: true });
}

async function safeRemoveOwnedFile(path, canonicalRoot, label, identity) {
  if (!path) return;
  const info = await pathInfo(path);
  if (!info) return;
  if (
    !identity ||
    info.isSymbolicLink() ||
    !info.isFile() ||
    !sameIdentity(info, identity)
  ) {
    throw new Error(`${label} cleanup authority could not be established`);
  }
  const canonical = await canonicalExisting(path, label);
  assertInside(canonicalRoot, canonical, label);
  await rm(path);
}

async function settlePortableCleanup({
  temporaryArchive,
  temporaryIdentity,
  verificationDirectory,
  verificationIdentity,
  stageParent,
  stageIdentity,
  canonicalBundle,
}) {
  const results = await Promise.allSettled([
    safeRemoveOwnedFile(
      temporaryArchive,
      canonicalBundle,
      "portable temporary archive",
      temporaryIdentity,
    ),
    safeRemoveOwnedTree(
      verificationDirectory,
      canonicalBundle,
      "portable verification directory",
      verificationIdentity,
    ),
    safeRemoveOwnedTree(
      stageParent,
      canonicalBundle,
      "portable stage directory",
      stageIdentity,
    ),
  ]);
  return results.filter(({ status }) => status === "rejected").length;
}

function archiveLeaf(version) {
  if (!strictSemver.test(version)) {
    throw new Error("root package version must be strict semantic version");
  }
  const leaf = `ability-radar_${version}_windows-x64-portable.zip`;
  if (basename(leaf) !== leaf || leaf.includes("/") || leaf.includes("\\")) {
    throw new Error("portable archive name must be one leaf filename");
  }
  return leaf;
}

async function assertArchiveCandidate(canonicalBundle, path, label) {
  if (basename(path) !== basename(resolve(path))) {
    throw new Error(`${label} must be one leaf filename`);
  }
  const canonicalParent = await requireDirectory(dirname(path), `${label} parent`);
  if (!samePath(canonicalBundle, canonicalParent)) {
    throw new Error(`${label} must stay inside target directory`);
  }
  assertInside(canonicalBundle, resolve(path), label);
}

async function copyValidatedTree(sourceRoot, destinationRoot, entries) {
  await ensureDirectory(destinationRoot, "portable staged benchmark packs");
  for (const entry of entries) {
    const name = relative(sourceRoot, entry.path);
    const destination = join(destinationRoot, name);
    assertInside(destinationRoot, destination, "portable staged entry");
    if (entry.directory) {
      await ensureDirectory(destination, "portable staged directory");
    } else {
      await ensureDirectory(dirname(destination), "portable staged file parent");
      await copyFile(entry.path, destination);
      await requireFile(destination, "portable staged file");
    }
  }
}

async function sha256(path) {
  await requireFile(path, "portable checksum input");
  return createHash("sha256").update(await readFile(path)).digest("hex");
}

function rawZipError(detail) {
  throw new Error(`portable raw ZIP validation failed: ${detail}`);
}

function validateZipExtraFields(bytes) {
  let cursor = 0;
  while (cursor < bytes.length) {
    if (cursor + 4 > bytes.length) rawZipError("malformed extra field");
    const headerId = bytes.readUInt16LE(cursor);
    const size = bytes.readUInt16LE(cursor + 2);
    cursor += 4;
    if (cursor + size > bytes.length) rawZipError("malformed extra field");
    if (headerId === 0x0001) rawZipError("ZIP64 is unsupported");
    cursor += size;
  }
}

function normalizeRawZipMember(name) {
  if (
    !name ||
    name.length > 4_096 ||
    name.startsWith("/") ||
    name.startsWith("\\")
  ) {
    rawZipError("unsafe member name");
  }
  const portable = name.replaceAll("\\", "/");
  const directory = portable.endsWith("/");
  const body = directory ? portable.slice(0, -1) : portable;
  const parts = body.split("/");
  if (
    !body ||
    parts.some((part) =>
      !part ||
      part === "." ||
      part === ".." ||
      part.endsWith(".") ||
      part.endsWith(" ") ||
      Buffer.byteLength(part, "utf8") > 255 ||
      /[\u0000-\u001f\u007f<>:"|?*]/u.test(part)
    )
  ) {
    rawZipError("unsafe member component");
  }
  const reserved = /^(?:CON|PRN|AUX|NUL|CLOCK\$|CONIN\$|CONOUT\$|COM[1-9¹²³]|LPT[1-9¹²³])$/u;
  if (
    parts.some((part) =>
      reserved.test(part.split(".", 1)[0].toUpperCase())
    )
  ) {
    rawZipError("reserved member component");
  }
  return {
    directory,
    key: body.normalize("NFC").toUpperCase(),
    name: body,
  };
}

function findRawZipEndRecord(bytes) {
  const minimum = Math.max(0, bytes.length - 22 - 65_535);
  for (let offset = bytes.length - 22; offset >= minimum; offset -= 1) {
    if (
      bytes.readUInt32LE(offset) === 0x06054b50 &&
      offset + 22 + bytes.readUInt16LE(offset + 20) === bytes.length
    ) {
      return offset;
    }
  }
  rawZipError("end record is missing");
}

function expectedRawZipMembers(expectedEntries) {
  const files = new Set();
  const directories = new Set(["ability-radar-portable"]);
  for (const entry of expectedEntries) {
    const member = normalizeRawZipMember(
      `ability-radar-portable/${entry.name}${entry.directory ? "/" : ""}`,
    );
    if (member.directory) directories.add(member.name);
    else files.add(member.name);
  }
  return { directories, files };
}

function validateRawZipCentralDirectory(bytes, expectedEntries) {
  if (!Buffer.isBuffer(bytes) || bytes.length < 22) {
    rawZipError("archive is truncated");
  }
  const endOffset = findRawZipEndRecord(bytes);
  const diskNumber = bytes.readUInt16LE(endOffset + 4);
  const centralDisk = bytes.readUInt16LE(endOffset + 6);
  const entriesOnDisk = bytes.readUInt16LE(endOffset + 8);
  const entryCount = bytes.readUInt16LE(endOffset + 10);
  const centralSize = bytes.readUInt32LE(endOffset + 12);
  const centralOffset = bytes.readUInt32LE(endOffset + 16);
  if (
    diskNumber !== 0 ||
    centralDisk !== 0 ||
    entriesOnDisk !== entryCount
  ) {
    rawZipError("multi-disk archives are unsupported");
  }
  if (
    entryCount === 0xffff ||
    centralSize === 0xffffffff ||
    centralOffset === 0xffffffff ||
    endOffset >= 20 && bytes.readUInt32LE(endOffset - 20) === 0x07064b50
  ) {
    rawZipError("ZIP64 is unsupported");
  }
  const centralEnd = centralOffset + centralSize;
  if (
    !Number.isSafeInteger(centralEnd) ||
    centralOffset > endOffset ||
    centralEnd !== endOffset
  ) {
    rawZipError("central directory bounds are invalid");
  }

  const decodedMembers = [];
  const destinations = new Map();
  const localRanges = [];
  const localOffsets = new Set();
  let cursor = centralOffset;
  for (let index = 0; index < entryCount; index += 1) {
    if (
      cursor + 46 > centralEnd ||
      bytes.readUInt32LE(cursor) !== 0x02014b50
    ) {
      rawZipError("central directory entry is malformed");
    }
    const versionNeeded = bytes.readUInt16LE(cursor + 6);
    const flags = bytes.readUInt16LE(cursor + 8);
    const method = bytes.readUInt16LE(cursor + 10);
    const crc = bytes.readUInt32LE(cursor + 16);
    const compressedSize = bytes.readUInt32LE(cursor + 20);
    const uncompressedSize = bytes.readUInt32LE(cursor + 24);
    const nameLength = bytes.readUInt16LE(cursor + 28);
    const extraLength = bytes.readUInt16LE(cursor + 30);
    const commentLength = bytes.readUInt16LE(cursor + 32);
    const diskStart = bytes.readUInt16LE(cursor + 34);
    const localOffset = bytes.readUInt32LE(cursor + 42);
    const entryEnd =
      cursor + 46 + nameLength + extraLength + commentLength;
    if (
      entryEnd > centralEnd ||
      nameLength === 0 ||
      versionNeeded > 20 ||
      flags & ~0x080e ||
      ![0, 8].includes(method) ||
      compressedSize === 0xffffffff ||
      uncompressedSize === 0xffffffff ||
      localOffset === 0xffffffff ||
      diskStart !== 0
    ) {
      rawZipError("unsupported central directory entry");
    }
    const nameBytes = bytes.subarray(cursor + 46, cursor + 46 + nameLength);
    if (
      !(flags & 0x0800) &&
      nameBytes.some((value) => value > 0x7f)
    ) {
      rawZipError("legacy filename encoding is unsupported");
    }
    let rawName;
    try {
      rawName = new TextDecoder("utf-8", { fatal: true }).decode(nameBytes);
    } catch {
      rawZipError("member name is not UTF-8");
    }
    const member = normalizeRawZipMember(rawName);
    if (destinations.has(member.key)) {
      rawZipError("duplicate normalized destination");
    }
    destinations.set(member.key, member.directory ? "directory" : "file");
    decodedMembers.push(member);
    if (member.directory && (compressedSize !== 0 || uncompressedSize !== 0)) {
      rawZipError("directory member has payload bytes");
    }
    validateZipExtraFields(
      bytes.subarray(
        cursor + 46 + nameLength,
        cursor + 46 + nameLength + extraLength,
      ),
    );

    if (
      localOffset + 30 > centralOffset ||
      bytes.readUInt32LE(localOffset) !== 0x04034b50 ||
      localOffsets.has(localOffset)
    ) {
      rawZipError("local entry header is invalid");
    }
    localOffsets.add(localOffset);
    const localVersion = bytes.readUInt16LE(localOffset + 4);
    const localFlags = bytes.readUInt16LE(localOffset + 6);
    const localMethod = bytes.readUInt16LE(localOffset + 8);
    const localCrc = bytes.readUInt32LE(localOffset + 14);
    const localCompressedSize = bytes.readUInt32LE(localOffset + 18);
    const localUncompressedSize = bytes.readUInt32LE(localOffset + 22);
    const localNameLength = bytes.readUInt16LE(localOffset + 26);
    const localExtraLength = bytes.readUInt16LE(localOffset + 28);
    const localHeaderEnd =
      localOffset + 30 + localNameLength + localExtraLength;
    const dataEnd = localHeaderEnd + compressedSize;
    if (
      localVersion !== versionNeeded ||
      localFlags !== flags ||
      localMethod !== method ||
      localHeaderEnd > centralOffset ||
      dataEnd > centralOffset ||
      localNameLength !== nameLength ||
      Buffer.compare(
        bytes.subarray(localOffset + 30, localOffset + 30 + localNameLength),
        nameBytes,
      ) !== 0 ||
      !(flags & 0x0008) && (
        localCrc !== crc ||
        localCompressedSize !== compressedSize ||
        localUncompressedSize !== uncompressedSize
      ) ||
      flags & 0x0008 && (
        ![0, compressedSize].includes(localCompressedSize) ||
        ![0, uncompressedSize].includes(localUncompressedSize)
      )
    ) {
      rawZipError("local and central entry metadata differ");
    }
    validateZipExtraFields(
      bytes.subarray(
        localOffset + 30 + localNameLength,
        localHeaderEnd,
      ),
    );
    localRanges.push({ start: localOffset, end: dataEnd });
    cursor = entryEnd;
  }
  if (cursor !== centralEnd) {
    rawZipError("central directory size is inconsistent");
  }
  localRanges.sort((left, right) => left.start - right.start);
  for (let index = 1; index < localRanges.length; index += 1) {
    if (localRanges[index - 1].end > localRanges[index].start) {
      rawZipError("local entries overlap");
    }
  }
  for (const member of decodedMembers) {
    const parts = member.name.split("/");
    for (let index = 1; index < parts.length; index += 1) {
      const ancestor = parts.slice(0, index).join("/")
        .normalize("NFC")
        .toUpperCase();
      if (destinations.get(ancestor) === "file") {
        rawZipError("file and directory destinations alias");
      }
    }
  }

  const expected = expectedRawZipMembers(expectedEntries);
  const actualFiles = decodedMembers
    .filter(({ directory }) => !directory)
    .map(({ name }) => name)
    .sort();
  const expectedFiles = [...expected.files].sort();
  if (!isDeepStrictEqual(actualFiles, expectedFiles)) {
    rawZipError("file membership differs from the staged payload");
  }
  if (
    decodedMembers
      .filter(({ directory }) => directory)
      .some(({ name }) => !expected.directories.has(name))
  ) {
    rawZipError("directory membership is not a safe payload subset");
  }
}

export function validateRawZipForTest(bytes, expectedEntries) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw new Error("raw ZIP test seam is available only under node:test");
  }
  validateRawZipCentralDirectory(Buffer.from(bytes), expectedEntries);
}

export function observeRuntimePackValidatorForTest(observer) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw new Error("runtime pack validator observer is available only under node:test");
  }
  runtimePackValidatorObserver = observer;
}

async function validateExtractedArchive(
  verificationDirectory,
  expectedArchive,
  validatorPath,
) {
  const extractedEntries = await entriesUnder(verificationDirectory);
  const actualEntries = extractedEntries.map(({ path, directory }) => ({
    name: relative(verificationDirectory, path).split(sep).join("/"),
    directory,
  }));
  const expectedEntries = [
    { name: "ability-radar-portable", directory: true },
    ...expectedArchive.entries.map(({ name, directory }) => ({
      name: `ability-radar-portable/${name}`,
      directory,
    })),
  ];
  const byName = (left, right) =>
    left.name < right.name ? -1 : left.name > right.name ? 1 : 0;
  actualEntries.sort(byName);
  expectedEntries.sort(byName);
  if (JSON.stringify(actualEntries) !== JSON.stringify(expectedEntries)) {
    throw new Error("portable archive verification found unexpected entries");
  }
  const extractedRoot = join(
    verificationDirectory,
    "ability-radar-portable",
  );
  for (const payload of expectedArchive.payloads) {
    const extractedPath = join(extractedRoot, ...payload.name.split("/"));
    if ((await sha256(extractedPath)) !== payload.sha256) {
      throw new Error("portable archive verification found a payload mismatch");
    }
  }
  const extractedChecksums = await readFile(
    join(extractedRoot, "SHA256SUMS.txt"),
  );
  if (Buffer.compare(extractedChecksums, expectedArchive.checksumManifest) !== 0) {
    throw new Error("portable archive verification found a checksum mismatch");
  }
  await validatePortablePacks(
    join(extractedRoot, "benchmark-packs"),
    expectedArchive.trustedRegistry,
    "extracted portable benchmark packs",
    validatorPath,
  );
}

export async function stagePortable({
  repoRoot,
  targetDir,
  bundleDir,
  version,
  packValidatorPath = join(targetDir, packValidatorLeaf),
}) {
  const archiveName = archiveLeaf(version);
  assertInside(targetDir, bundleDir, "portable bundle directory");
  const canonicalRepo = await requireDirectory(repoRoot, "repository root");
  const canonicalTarget = await requireDirectory(targetDir, "release target");
  const canonicalBundle = await ensureDirectory(
    bundleDir,
    "portable bundle directory",
  );
  assertInside(canonicalTarget, canonicalBundle, "portable bundle directory");

  const executable = join(targetDir, "ability-radar.exe");
  const packs = join(targetDir, "benchmark-packs");
  const readme = join(
    repoRoot,
    "packaging",
    "windows-portable",
    "README.txt",
  );
  const canonicalExecutable = await requireFile(
    executable,
    "portable executable",
  );
  assertInside(canonicalTarget, canonicalExecutable, "portable executable");
  const canonicalPacks = await requireDirectory(packs, "portable benchmark packs");
  assertInside(canonicalTarget, canonicalPacks, "portable benchmark packs");
  const canonicalReadme = await requireFile(readme, "portable README");
  assertInside(canonicalRepo, canonicalReadme, "portable README");
  let canonicalPackValidator;
  try {
    canonicalPackValidator = await requireFile(
      packValidatorPath,
      "runtime pack validator",
    );
  } catch {
    throw new Error("runtime pack validator is missing or invalid");
  }
  assertInside(canonicalTarget, canonicalPackValidator, "runtime pack validator");
  const trustedRegistry = await loadTrustedRegistry(repoRoot);
  await validatePortablePacks(
    canonicalPacks,
    trustedRegistry,
    "source portable benchmark packs",
    canonicalPackValidator,
  );
  const packEntries = await entriesUnder(packs);

  const archivePath = join(bundleDir, archiveName);
  await assertArchiveCandidate(
    canonicalBundle,
    archivePath,
    "portable final archive",
  );
  const stageParent = join(bundleDir, `.stage.${randomUUID()}`);
  const stageRoot = join(stageParent, "ability-radar-portable");
  assertInside(canonicalBundle, stageParent, "portable stage directory");

  let ownsStage = false;
  let stageIdentity;
  try {
    await mkdir(stageParent);
    ownsStage = true;
    stageIdentity = await captureOwnedDirectory(
      stageParent,
      canonicalBundle,
      "portable stage directory",
    );
    await mkdir(stageRoot);
    const canonicalStageRoot = await requireDirectory(
      stageRoot,
      "portable stage root",
    );
    assertInside(canonicalBundle, canonicalStageRoot, "portable stage root");
    await copyFile(executable, join(stageRoot, "ability-radar.exe"));
    await requireFile(
      join(stageRoot, "ability-radar.exe"),
      "staged portable executable",
    );
    await copyValidatedTree(
      packs,
      join(stageRoot, "benchmark-packs"),
      packEntries,
    );
    await validatePortablePacks(
      join(stageRoot, "benchmark-packs"),
      trustedRegistry,
      "staged portable benchmark packs",
      canonicalPackValidator,
    );
    await copyFile(readme, join(stageRoot, "README.txt"));
    await requireFile(join(stageRoot, "README.txt"), "staged portable README");

    const stagedEntries = await entriesUnder(stageRoot);
    const files = stagedEntries
      .filter(({ directory }) => !directory)
      .map(({ path }) => path);
    const lines = [];
    const payloads = [];
    for (const file of files) {
      const name = relative(stageRoot, file).split(sep).join("/");
      const hash = await sha256(file);
      lines.push(`${hash}  ${name}`);
      payloads.push({ name, sha256: hash });
    }
    const checksumManifest = Buffer.from(`${lines.join("\n")}\n`, "utf8");
    await writeFile(
      join(stageRoot, "SHA256SUMS.txt"),
      checksumManifest,
      { flag: "wx" },
    );
    await requireFile(
      join(stageRoot, "SHA256SUMS.txt"),
      "portable checksum manifest",
    );
    const entries = stagedEntries.map(({ path, directory }) => ({
      name: relative(stageRoot, path).split(sep).join("/"),
      directory,
    }));
    entries.push({ name: "SHA256SUMS.txt", directory: false });
    return {
      archivePath,
      canonicalBundle,
      checksumManifest,
      entries,
      payloads,
      packValidatorPath: canonicalPackValidator,
      trustedRegistry,
      stageIdentity,
      stageParent,
      stageRoot,
    };
  } catch (error) {
    let cleanupFailureCount = 0;
    if (ownsStage) {
      const results = await Promise.allSettled([
        safeRemoveOwnedTree(
          stageParent,
          canonicalBundle,
          "portable stage directory",
          stageIdentity,
        ),
      ]);
      cleanupFailureCount = results.filter(
        ({ status }) => status === "rejected",
      ).length;
    }
    if (cleanupFailureCount > 0) {
      throw new Error(
        `portable staging processing failed; cleanup incomplete (${cleanupFailureCount} operation)`,
        { cause: error },
      );
    }
    throw error;
  }
}

async function packagePortableFromBuild(repoRoot, publicationHook) {
  const packageManifest = JSON.parse(
    await readFile(join(repoRoot, "package.json"), "utf8"),
  );
  const version = packageManifest.version;
  const targetDir = join(repoRoot, "target", "release");
  const bundleDir = join(targetDir, "bundle", "portable");
  const expectedArchive = await stagePortable({
    repoRoot,
    targetDir,
    bundleDir,
    version,
  });
  const {
    archivePath,
    canonicalBundle,
    stageIdentity,
    stageParent,
    stageRoot,
    packValidatorPath,
  } = expectedArchive;
  let temporaryArchive;
  let temporaryIdentity;
  let verificationDirectory;
  let verificationIdentity;
  let published = false;
  let primaryError;
  try {
    await publicationHook?.({
      phase: "afterChecksums",
      archivePath,
      stageParent,
      stageRoot,
    });
    await assertArchiveCandidate(
      canonicalBundle,
      archivePath,
      "portable final archive",
    );
    await requireOwnedDirectoryIdentity(
      stageParent,
      canonicalBundle,
      "portable stage directory",
      stageIdentity,
    );
    temporaryArchive = join(
      bundleDir,
      `.${basename(archivePath, ".zip")}.${randomUUID()}.tmp.zip`,
    );
    verificationDirectory = join(bundleDir, `.verify.${randomUUID()}`);
    await assertArchiveCandidate(
      canonicalBundle,
      temporaryArchive,
      "portable temporary archive",
    );
    await assertArchiveCandidate(
      canonicalBundle,
      verificationDirectory,
      "portable verification directory",
    );
    if (await pathInfo(archivePath)) {
      throw new Error("portable final archive already exists; refusing to overwrite");
    }
    if (await pathInfo(temporaryArchive)) {
      throw new Error("portable temporary archive unexpectedly exists");
    }
    if (await pathInfo(verificationDirectory)) {
      throw new Error("portable verification directory unexpectedly exists");
    }
    await ensureDirectory(
      verificationDirectory,
      "portable verification directory",
    );
    verificationIdentity = await captureOwnedDirectory(
      verificationDirectory,
      canonicalBundle,
      "portable verification directory",
    );
    await publicationHook?.({
      phase: "beforeCompression",
      archivePath,
      stageParent,
      stageRoot,
      temporaryArchive,
      verificationDirectory,
    });
    await validatePortablePacks(
      join(stageRoot, "benchmark-packs"),
      expectedArchive.trustedRegistry,
      "pre-compression portable benchmark packs",
      packValidatorPath,
    );
    await requireOwnedDirectoryIdentity(
      stageParent,
      canonicalBundle,
      "portable stage directory",
      stageIdentity,
    );
    if (await pathInfo(temporaryArchive)) {
      throw new Error("portable temporary archive unexpectedly exists");
    }
    const compressionResult = spawnSync(
      "powershell.exe",
      [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        join(repoRoot, "scripts", "compress-portable.ps1"),
        "-Source",
        stageRoot,
        "-Destination",
        temporaryArchive,
      ],
      { cwd: repoRoot, stdio: "inherit" },
    );
    const temporaryInfo = await pathInfo(temporaryArchive);
    if (temporaryInfo) {
      if (
        temporaryInfo.isSymbolicLink() ||
        !temporaryInfo.isFile()
      ) {
        throw new Error("portable ZIP compressor did not produce a regular archive");
      }
      const canonicalTemporary = await canonicalExisting(
        temporaryArchive,
        "portable temporary archive",
      );
      assertInside(
        canonicalBundle,
        canonicalTemporary,
        "portable temporary archive",
      );
      temporaryIdentity = fileIdentity(temporaryInfo);
    }
    if (compressionResult.error) throw compressionResult.error;
    if (compressionResult.status !== 0) {
      throw new Error("portable ZIP compression failed");
    }
    if (
      !temporaryInfo ||
      temporaryInfo.size < 4
    ) {
      throw new Error("portable ZIP compressor did not produce a regular archive");
    }
    const archiveBytes = await readFile(temporaryArchive);
    const signature = archiveBytes.subarray(0, 2);
    if (signature[0] !== 0x50 || signature[1] !== 0x4b) {
      throw new Error("portable ZIP compressor produced an invalid ZIP signature");
    }
    validateRawZipCentralDirectory(archiveBytes, expectedArchive.entries);
    const extractionResult = spawnSync(
      "powershell.exe",
      [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        join(repoRoot, "scripts", "extract-portable.ps1"),
        "-Source",
        temporaryArchive,
        "-Destination",
        verificationDirectory,
      ],
      { cwd: repoRoot, stdio: "inherit" },
    );
    if (extractionResult.error) throw extractionResult.error;
    if (extractionResult.status !== 0) {
      throw new Error("portable ZIP extraction failed");
    }
    await requireOwnedDirectoryIdentity(
      verificationDirectory,
      canonicalBundle,
      "portable verification directory",
      verificationIdentity,
    );
    await validateExtractedArchive(
      verificationDirectory,
      expectedArchive,
      packValidatorPath,
    );
    await requireOwnedFileIdentity(
      temporaryArchive,
      canonicalBundle,
      "portable temporary archive",
      temporaryIdentity,
    );
    await publicationHook?.({
      phase: "beforeLink",
      archivePath,
      stageParent,
      stageRoot,
      temporaryArchive,
      verificationDirectory,
    });
    await requireOwnedFileIdentity(
      temporaryArchive,
      canonicalBundle,
      "portable temporary archive",
      temporaryIdentity,
    );
    await link(temporaryArchive, archivePath);
    published = true;
    await publicationHook?.({
      phase: "afterLink",
      archivePath,
      stageParent,
      stageRoot,
      temporaryArchive,
      verificationDirectory,
    });
    await requireOwnedFileIdentity(
      archivePath,
      canonicalBundle,
      "portable final archive",
      temporaryIdentity,
    );
  } catch (error) {
    primaryError = error;
  }

  const cleanupFailureCount = await settlePortableCleanup({
    temporaryArchive,
    temporaryIdentity,
    verificationDirectory,
    verificationIdentity,
    stageParent,
    stageIdentity,
    canonicalBundle,
  });
  const cleanupSummary = `cleanup incomplete (${cleanupFailureCount} operation${
    cleanupFailureCount === 1 ? "" : "s"
  })`;

  if (published && primaryError) {
    throw new Error(
      `portable archive was published; processing failed; ${
        cleanupFailureCount > 0 ? cleanupSummary : "cleanup completed"
      }`,
      { cause: primaryError },
    );
  }
  if (published && cleanupFailureCount > 0) {
    throw new Error(
      `portable archive was published; ${cleanupSummary}`,
    );
  }
  if (primaryError) {
    throw new Error(
      `portable packaging processing failed${
        cleanupFailureCount > 0 ? `; ${cleanupSummary}` : ""
      }`,
      { cause: primaryError },
    );
  }
  return archivePath;
}

export async function packagePortableFromBuildForTest(repoRoot, publicationHook) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw new Error("portable publication hook is available only under node:test");
  }
  return packagePortableFromBuild(repoRoot, publicationHook);
}

async function main() {
  if (process.platform !== "win32") {
    throw new Error("portable packaging currently supports Windows only");
  }
  const scriptPath = fileURLToPath(import.meta.url);
  const repoRoot = resolve(dirname(scriptPath), "..");
  const archivePath = await packagePortableFromBuild(repoRoot);
  process.stdout.write(`${archivePath}\n`);
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  });
}
