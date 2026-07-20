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
    await mkdir(current);
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

async function validateExtractedArchive(
  verificationDirectory,
  expectedArchive,
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
}

export async function stagePortable({
  repoRoot,
  targetDir,
  bundleDir,
  version,
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
  for (const required of [
    join(packs, "registry.json"),
    join(packs, "client-quick-v1", "manifest.json"),
    join(packs, "cli-quick-v1", "manifest.json"),
  ]) {
    const canonical = await requireFile(required, "required portable input");
    assertInside(canonicalPacks, canonical, "required portable input");
  }
  const packEntries = await entriesUnder(packs);

  const archivePath = join(bundleDir, archiveName);
  await assertArchiveCandidate(
    canonicalBundle,
    archivePath,
    "portable final archive",
  );
  const stageParent = join(bundleDir, ".stage");
  const stageRoot = join(stageParent, "ability-radar-portable");
  assertInside(canonicalBundle, stageParent, "portable stage directory");

  let ownsStage = false;
  let stageIdentity;
  try {
    if (await pathInfo(stageParent)) {
      const previousIdentity = await captureOwnedDirectory(
        stageParent,
        canonicalBundle,
        "portable preexisting stage directory",
      );
      await safeRemoveOwnedTree(
        stageParent,
        canonicalBundle,
        "portable preexisting stage directory",
        previousIdentity,
      );
    }
    await ensureDirectory(stageRoot, "portable stage root");
    ownsStage = true;
    stageIdentity = await captureOwnedDirectory(
      stageParent,
      canonicalBundle,
      "portable stage directory",
    );
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
    const signature = (await readFile(temporaryArchive)).subarray(0, 2);
    if (signature[0] !== 0x50 || signature[1] !== 0x4b) {
      throw new Error("portable ZIP compressor produced an invalid ZIP signature");
    }
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
    await validateExtractedArchive(verificationDirectory, expectedArchive);
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
