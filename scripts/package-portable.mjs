import { createHash, randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFile,
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

async function safeRemoveTree(path, canonicalRoot, label) {
  if (!(await pathInfo(path))) return;
  const canonical = await requireDirectory(path, label);
  assertInside(canonicalRoot, canonical, label);
  await entriesUnder(path);
  await rm(path, { recursive: true });
}

async function safeRemoveFile(path, canonicalRoot, label) {
  const info = await pathInfo(path);
  if (!info) return;
  if (info.isSymbolicLink() || !info.isFile()) {
    throw new Error(`${label} must be a non-indirect regular file`);
  }
  const canonical = await canonicalExisting(path, label);
  assertInside(canonicalRoot, canonical, label);
  await rm(path);
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
  try {
    if (await pathInfo(stageParent)) {
      await safeRemoveTree(
        stageParent,
        canonicalBundle,
        "portable stage directory",
      );
    }
    await ensureDirectory(stageRoot, "portable stage root");
    ownsStage = true;
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
    for (const file of files) {
      const name = relative(stageRoot, file).split(sep).join("/");
      lines.push(`${await sha256(file)}  ${name}`);
    }
    await writeFile(
      join(stageRoot, "SHA256SUMS.txt"),
      `${lines.join("\n")}\n`,
      { flag: "wx" },
    );
    await requireFile(
      join(stageRoot, "SHA256SUMS.txt"),
      "portable checksum manifest",
    );
    return { archivePath, stageRoot };
  } catch (error) {
    if (ownsStage) {
      await safeRemoveTree(
        stageParent,
        canonicalBundle,
        "portable stage directory",
      );
    }
    throw error;
  }
}

async function packagePortableFromBuild(repoRoot) {
  const packageManifest = JSON.parse(
    await readFile(join(repoRoot, "package.json"), "utf8"),
  );
  const version = packageManifest.version;
  const targetDir = join(repoRoot, "target", "release");
  const bundleDir = join(targetDir, "bundle", "portable");
  const { archivePath, stageRoot } = await stagePortable({
    repoRoot,
    targetDir,
    bundleDir,
    version,
  });
  const canonicalBundle = await requireDirectory(
    bundleDir,
    "portable bundle directory",
  );
  await assertArchiveCandidate(
    canonicalBundle,
    archivePath,
    "portable final archive",
  );
  const stageParent = dirname(stageRoot);
  const canonicalStageParent = await requireDirectory(
    stageParent,
    "portable stage directory",
  );
  assertInside(canonicalBundle, canonicalStageParent, "portable stage directory");
  const temporaryArchive = join(
    bundleDir,
    `.${basename(archivePath, ".zip")}.${randomUUID()}.tmp.zip`,
  );
  await assertArchiveCandidate(
    canonicalBundle,
    temporaryArchive,
    "portable temporary archive",
  );

  try {
    if (await pathInfo(archivePath)) {
      throw new Error("portable final archive already exists; refusing to overwrite");
    }
    if (await pathInfo(temporaryArchive)) {
      throw new Error("portable temporary archive unexpectedly exists");
    }
    const result = spawnSync(
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
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error("portable ZIP compression failed");
    }
    const temporaryInfo = await lstat(temporaryArchive);
    if (
      temporaryInfo.isSymbolicLink() ||
      !temporaryInfo.isFile() ||
      temporaryInfo.size < 4
    ) {
      throw new Error("portable ZIP compressor did not produce a regular archive");
    }
    const canonicalTemporary = await canonicalExisting(
      temporaryArchive,
      "portable temporary archive",
    );
    assertInside(canonicalBundle, canonicalTemporary, "portable temporary archive");
    const signature = (await readFile(temporaryArchive)).subarray(0, 2);
    if (signature[0] !== 0x50 || signature[1] !== 0x4b) {
      throw new Error("portable ZIP compressor produced an invalid ZIP signature");
    }
    if (await pathInfo(archivePath)) {
      throw new Error("portable final archive appeared during compression");
    }
    await rename(temporaryArchive, archivePath);
    const canonicalFinal = await requireFile(
      archivePath,
      "portable final archive",
    );
    assertInside(canonicalBundle, canonicalFinal, "portable final archive");
  } finally {
    await safeRemoveFile(
      temporaryArchive,
      canonicalBundle,
      "portable temporary archive",
    );
    await safeRemoveTree(
      stageParent,
      canonicalBundle,
      "portable stage directory",
    );
  }
  process.stdout.write(`${archivePath}\n`);
}

async function main() {
  if (process.platform !== "win32") {
    throw new Error("portable packaging currently supports Windows only");
  }
  const scriptPath = fileURLToPath(import.meta.url);
  const repoRoot = resolve(dirname(scriptPath), "..");
  await packagePortableFromBuild(repoRoot);
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  });
}
