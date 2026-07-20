import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFile,
  cp,
  mkdir,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

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

async function filesUnder(root, current = root) {
  const entries = await readdir(current, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) =>
    left.name.localeCompare(right.name, "en"),
  )) {
    const path = join(current, entry.name);
    if (entry.isDirectory()) files.push(...await filesUnder(root, path));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

async function sha256(path) {
  return createHash("sha256").update(await readFile(path)).digest("hex");
}

export async function stagePortable({
  repoRoot,
  targetDir,
  bundleDir,
  version,
}) {
  assertInside(targetDir, bundleDir, "portable bundle directory");
  const executable = join(targetDir, "ability-radar.exe");
  const packs = join(targetDir, "benchmark-packs");
  const readme = join(
    repoRoot,
    "packaging",
    "windows-portable",
    "README.txt",
  );
  for (const required of [
    executable,
    join(packs, "registry.json"),
    join(packs, "client-quick-v1", "manifest.json"),
    join(packs, "cli-quick-v1", "manifest.json"),
    readme,
  ]) {
    if (!(await stat(required)).isFile()) {
      throw new Error(`required portable input is not a file: ${required}`);
    }
  }

  const stageParent = join(bundleDir, ".stage");
  const stageRoot = join(stageParent, "ability-radar-portable");
  assertInside(bundleDir, stageParent, "portable stage directory");
  await rm(stageParent, { recursive: true, force: true });
  await mkdir(stageRoot, { recursive: true });
  await copyFile(executable, join(stageRoot, "ability-radar.exe"));
  await cp(packs, join(stageRoot, "benchmark-packs"), { recursive: true });
  await copyFile(readme, join(stageRoot, "README.txt"));

  const files = await filesUnder(stageRoot);
  const lines = [];
  for (const file of files) {
    const name = relative(stageRoot, file).split(sep).join("/");
    lines.push(`${await sha256(file)}  ${name}`);
  }
  await writeFile(join(stageRoot, "SHA256SUMS.txt"), `${lines.join("\n")}\n`);

  return {
    archivePath: join(
      bundleDir,
      `ability-radar_${version}_windows-x64-portable.zip`,
    ),
    stageRoot,
  };
}

async function main() {
  if (process.platform !== "win32") {
    throw new Error("portable packaging currently supports Windows only");
  }
  const scriptPath = fileURLToPath(import.meta.url);
  const repoRoot = resolve(dirname(scriptPath), "..");
  const packageManifest = JSON.parse(
    await readFile(join(repoRoot, "package.json"), "utf8"),
  );
  const version = packageManifest.version;
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
    throw new Error("root package version must be strict semantic version");
  }
  const targetDir = join(repoRoot, "target", "release");
  const bundleDir = join(targetDir, "bundle", "portable");
  const { archivePath, stageRoot } = await stagePortable({
    repoRoot,
    targetDir,
    bundleDir,
    version,
  });
  const stageParent = dirname(stageRoot);
  assertInside(bundleDir, stageParent, "portable stage directory");
  await rm(archivePath, { force: true });
  try {
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
        archivePath,
      ],
      { cwd: repoRoot, stdio: "inherit" },
    );
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error("portable ZIP compression failed");
    }
  } finally {
    await rm(stageParent, { recursive: true, force: true });
  }
  process.stdout.write(`${archivePath}\n`);
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  });
}
