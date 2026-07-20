import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, extname, join, normalize, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  actionSteps,
  exactPermissions,
  hasRunCommand,
  parseWorkflow,
  runSteps,
} from "./workflow-contracts.mjs";

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = process.env.REPOSITORY_ROOT
  ? resolve(process.env.REPOSITORY_ROOT)
  : defaultRoot;
const errors = [];

function fail(message) {
  errors.push(message);
}

function read(path) {
  const absolute = join(root, path);
  if (!existsSync(absolute)) {
    fail(`missing required file: ${path}`);
    return "";
  }
  return readFileSync(absolute, "utf8").replace(/^\uFEFF/, "");
}

function json(path) {
  const source = read(path);
  if (!source) return {};
  try {
    return JSON.parse(source);
  } catch (error) {
    fail(`${path} is not valid JSON: ${error.message}`);
    return {};
  }
}

function requireText(path, patterns) {
  const source = read(path);
  for (const [label, pattern] of patterns) {
    if (!pattern.test(source)) fail(`${path} is missing ${label}`);
  }
  return source;
}

function stripTomlComment(line) {
  let quote;
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (quote === '"') {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        quote = undefined;
      }
      continue;
    }
    if (quote === "'") {
      if (character === "'") quote = undefined;
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
    } else if (character === "#") {
      return line.slice(0, index);
    }
  }
  return line;
}

function tomlSection(source, sectionName) {
  const lines = [];
  let active = false;
  for (const rawLine of source.replace(/\r\n?/g, "\n").split("\n")) {
    const line = stripTomlComment(rawLine);
    const header = line.trim().match(/^\[([^\]]+)\]$/);
    if (header) {
      active = header[1] === sectionName;
      continue;
    }
    if (active) lines.push(line);
  }
  return lines.join("\n");
}

function tomlBasicKey(source, start) {
  let value = "";
  let cursor = start + 1;
  const escapes = new Map([
    ['"', '"'],
    ["\\", "\\"],
    ["b", "\b"],
    ["t", "\t"],
    ["n", "\n"],
    ["f", "\f"],
    ["r", "\r"],
  ]);
  while (cursor < source.length) {
    const character = source[cursor];
    if (character === '"') {
      return { valid: true, value, cursor: cursor + 1 };
    }
    if (character === "\\") {
      const escape = source[cursor + 1];
      if (escapes.has(escape)) {
        value += escapes.get(escape);
        cursor += 2;
        continue;
      }
      if (escape === "u" || escape === "U") {
        const digits = escape === "u" ? 4 : 8;
        const hex = source.slice(cursor + 2, cursor + 2 + digits);
        if (
          hex.length !== digits ||
          !/^[0-9A-Fa-f]+$/.test(hex)
        ) {
          return { valid: false };
        }
        const codePoint = Number.parseInt(hex, 16);
        if (
          codePoint > 0x10ffff ||
          (codePoint >= 0xd800 && codePoint <= 0xdfff)
        ) {
          return { valid: false };
        }
        value += String.fromCodePoint(codePoint);
        cursor += digits + 2;
        continue;
      }
      return { valid: false };
    }
    const codePoint = source.codePointAt(cursor);
    if ((codePoint < 0x20 && codePoint !== 0x09) || codePoint === 0x7f) {
      return { valid: false };
    }
    value += String.fromCodePoint(codePoint);
    cursor += codePoint > 0xffff ? 2 : 1;
  }
  return { valid: false };
}

function tomlLiteralKey(source, start) {
  let value = "";
  let cursor = start + 1;
  while (cursor < source.length) {
    const character = source[cursor];
    if (character === "'") {
      return { valid: true, value, cursor: cursor + 1 };
    }
    const codePoint = source.codePointAt(cursor);
    if ((codePoint < 0x20 && codePoint !== 0x09) || codePoint === 0x7f) {
      return { valid: false };
    }
    value += String.fromCodePoint(codePoint);
    cursor += codePoint > 0xffff ? 2 : 1;
  }
  return { valid: false };
}

function tomlDottedKey(source) {
  const segments = [];
  let cursor = 0;
  const skipWhitespace = () => {
    while (source[cursor] === " " || source[cursor] === "\t") cursor += 1;
  };
  skipWhitespace();
  while (cursor < source.length) {
    let segment;
    if (source[cursor] === '"') {
      segment = tomlBasicKey(source, cursor);
    } else if (source[cursor] === "'") {
      segment = tomlLiteralKey(source, cursor);
    } else {
      const match = source.slice(cursor).match(/^[A-Za-z0-9_-]+/);
      if (!match) return { valid: false };
      segment = {
        valid: true,
        value: match[0],
        cursor: cursor + match[0].length,
      };
    }
    if (!segment.valid) return { valid: false };
    segments.push(segment.value);
    cursor = segment.cursor;
    skipWhitespace();
    if (cursor === source.length) {
      return { valid: segments.length > 0, segments };
    }
    if (source[cursor] !== ".") return { valid: false };
    cursor += 1;
    skipWhitespace();
    if (cursor === source.length) return { valid: false };
  }
  return { valid: false };
}

function tomlTableHeader(line) {
  const trimmed = stripTomlComment(line).trim();
  if (!trimmed.startsWith("[")) return { present: false };
  let array = false;
  let keySource;
  if (trimmed.startsWith("[[")) {
    if (
      trimmed.startsWith("[[[") ||
      !trimmed.endsWith("]]") ||
      trimmed.endsWith("]]]")
    ) {
      return { present: true, valid: false, raw: trimmed };
    }
    array = true;
    keySource = trimmed.slice(2, -2);
  } else {
    if (!trimmed.endsWith("]") || trimmed.endsWith("]]")) {
      return { present: true, valid: false, raw: trimmed };
    }
    keySource = trimmed.slice(1, -1);
  }
  const parsed = tomlDottedKey(keySource);
  if (!parsed.valid) {
    return { present: true, valid: false, raw: trimmed };
  }
  return {
    present: true,
    valid: true,
    raw: trimmed,
    array,
    segments: parsed.segments,
  };
}

function tomlTables(source) {
  const tables = [];
  const invalidHeaders = [];
  let current;
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    const rawLine = lines[index];
    const parsedHeader = tomlTableHeader(rawLine);
    if (parsedHeader.present) {
      if (!parsedHeader.valid) {
        invalidHeaders.push({ line: index + 1, raw: parsedHeader.raw });
        current = undefined;
        continue;
      }
      current = {
        name: parsedHeader.segments.join("."),
        array: parsedHeader.array,
        segments: parsedHeader.segments,
        entries: [],
      };
      tables.push(current);
      continue;
    }
    const line = stripTomlComment(rawLine);
    if (current && line.trim()) current.entries.push(line.trim());
  }
  return { tables, invalidHeaders };
}

function tomlStringArray(source, sectionName, key) {
  const section = tomlSection(source, sectionName);
  const match = section.match(
    new RegExp(`(?:^|\\n)\\s*${key}\\s*=\\s*\\[([\\s\\S]*?)\\]`),
  );
  if (!match) return undefined;
  const values = match[1]
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
  if (values.some((value) => !/^"[^"]+"$/.test(value))) return undefined;
  return values.map((value) => value.slice(1, -1));
}

function tomlStringArrayValue(source, key) {
  const assignment = source.match(new RegExp(`^${key}\\s*=\\s*\\[`, "m"));
  if (!assignment) return { present: false, valid: true, values: undefined };
  const start = assignment.index + assignment[0].length;
  let quoted = false;
  let end = -1;
  for (let index = start; index < source.length; index += 1) {
    const character = source[index];
    if (character === '"' && source[index - 1] !== "\\") quoted = !quoted;
    if (character === "]" && !quoted) {
      end = index;
      break;
    }
  }
  if (end < 0) return { present: true, valid: false, values: [] };

  const body = source.slice(start, end);
  const values = [];
  let cursor = 0;
  while (cursor < body.length) {
    while (cursor < body.length && /\s/.test(body[cursor])) cursor += 1;
    if (cursor >= body.length) break;
    const value = body.slice(cursor).match(/^"([^"\\]*(?:\\.[^"\\]*)*)"/);
    if (!value) return { present: true, valid: false, values };
    values.push(value[1]);
    cursor += value[0].length;
    while (cursor < body.length && /\s/.test(body[cursor])) cursor += 1;
    if (cursor >= body.length) break;
    if (body[cursor] !== ",") {
      return { present: true, valid: false, values };
    }
    cursor += 1;
  }
  return { present: true, valid: true, values };
}

function cargoPackages(source) {
  return [...source.matchAll(/\[\[package\]\]\r?\n([\s\S]*?)(?=\r?\n\[\[package\]\]|\s*$)/g)]
    .map((match) => {
      const dependencyArray = tomlStringArrayValue(
        match[1],
        "dependencies",
      );
      return {
        name: match[1].match(/^name = "([^"]+)"$/m)?.[1],
        version: match[1].match(/^version = "([^"]+)"$/m)?.[1],
        source: match[1].match(/^source = "([^"]+)"$/m)?.[1],
        dependencies: dependencyArray.values,
        dependenciesValid: dependencyArray.valid,
      };
    })
    .filter(({ name, version }) => name && version);
}

function npmLockPackages(lock) {
  const packages = [];
  for (const [path, metadata] of Object.entries(lock.packages ?? {})) {
    const marker = "node_modules/";
    const index = path.lastIndexOf(marker);
    if (index < 0 || !metadata.version) continue;
    const name = path.slice(index + marker.length);
    packages.push({
      name,
      version: metadata.version,
      resolved: metadata.resolved,
      integrity: metadata.integrity,
    });
  }
  return packages;
}

function packageKey({ name, version }) {
  return `${name}@${version}`;
}

function normalizedSourceSha256(source) {
  const canonical = source.replace(/\r\n?/g, "\n");
  return createHash("sha256").update(canonical, "utf8").digest("hex");
}

function sha256(path) {
  return normalizedSourceSha256(readFileSync(join(root, path), "utf8"));
}

const requiredFiles = [
  ".github/workflows/ci.yml",
  ".github/workflows/release.yml",
  ".github/workflows/pages.yml",
  ".github/ISSUE_TEMPLATE/bug.yml",
  ".github/pull_request_template.md",
  "README.md",
  "CONTRIBUTING.md",
  "SECURITY.md",
  "THIRD_PARTY_NOTICES.md",
  "docs/privacy.md",
  "docs/security.md",
  "docs/troubleshooting.md",
  "docs/methodology.md",
  "docs/licenses/npm-dependencies.json",
  "docs/licenses/rust-dependencies.json",
  "docs/release-checklist.md",
  "docs/test-matrix.md",
  "tools/fake-cli/Cargo.toml",
  "tools/fake-cli/src/main.rs",
  "tools/fake-cli/tests/fake_cli.rs",
  "apps/desktop/src-tauri/tests/fake_cli_e2e.rs",
  "scripts/package-portable.mjs",
  "scripts/package-portable.test.mjs",
  "scripts/compress-portable.ps1",
  "packaging/windows-portable/README.txt",
  "site/index.html",
  "site/.nojekyll",
];
for (const path of requiredFiles) read(path);

const expectedVersion = "0.2.1";
const rootPackage = json("package.json");
const desktopPackage = json("apps/desktop/package.json");
const tauriConfig = json("apps/desktop/src-tauri/tauri.conf.json");
const npmLock = json("package-lock.json");
if (rootPackage.scripts?.tauri !== "npm run tauri --workspace apps/desktop --") {
  fail("package.json tauri script must preserve the argument separator for workspace forwarding");
}
const expectedPortableScripts = {
  start: "npm run tauri -- dev",
  "package:portable":
    "npm run tauri -- build --no-bundle && npm run package:portable:from-build",
  "package:portable:from-build": "node scripts/package-portable.mjs",
};
for (const [name, expected] of Object.entries(expectedPortableScripts)) {
  if (rootPackage.scripts?.[name] !== expected) {
    fail(`package.json ${name} script must be exactly: ${expected}`);
  }
}
if (
  !rootPackage.scripts?.test
    ?.split(" && ")
    .includes("node --test scripts/package-portable.test.mjs")
) {
  fail("package.json test script must run scripts/package-portable.test.mjs");
}

const portableSources = new Map([
  ["scripts/package-portable.mjs", read("scripts/package-portable.mjs")],
  ["scripts/compress-portable.ps1", read("scripts/compress-portable.ps1")],
]);
const portableNodeSource = portableSources.get("scripts/package-portable.mjs");
const portablePowerShellSource = portableSources.get(
  "scripts/compress-portable.ps1",
);

function canonicalStatement(source) {
  return source.replace(/\s+/g, " ").trim();
}

const portableNodeImports =
  portableNodeSource.match(/import\s+[\s\S]*?\s+from\s+"[^"]+";/g) ?? [];
const expectedPortableNodeImports = [
  'import { createHash, randomUUID } from "node:crypto";',
  'import { spawnSync } from "node:child_process";',
  'import { copyFile, link, lstat, mkdir, readFile, readdir, realpath, rm, writeFile, } from "node:fs/promises";',
  'import { fileURLToPath } from "node:url";',
  'import { basename, dirname, isAbsolute, join, parse, relative, resolve, sep, } from "node:path";',
].map(canonicalStatement).sort();
const actualPortableNodeImports =
  portableNodeImports.map(canonicalStatement).sort();
if (
  JSON.stringify(actualPortableNodeImports) !==
    JSON.stringify(expectedPortableNodeImports) ||
  /\bimport\s*\(|\brequire\s*\(|\bprocess\.binding\s*\(/.test(
    portableNodeSource,
  ) ||
  /\b(?:fetch|WebSocket|XMLHttpRequest|EventSource)\s*\(|\bsendBeacon\s*\(/.test(
    portableNodeSource,
  )
) {
  fail(
    "portable Node import allowlist permits only reviewed core filesystem, path, crypto, URL, and child-process imports; network and dynamic imports are forbidden",
  );
}
const forbiddenPortableNodeSyntax = [
  /\bprocess\s*\.\s*getBuiltinModule\s*\(/,
  /\b(?:global|globalThis)\s*\[/,
  /\[\s*["'](?:spawnSync|fetch|copyFile|link|rm|writeFile|rename|createWriteStream|request|connect)["']\s*\]/,
  /\b(?:import|require|eval|Function)\s*\(/,
  /\bReflect(?:\s*\.|\s*\[)/,
  /\b(?:const|let|var)\s+[A-Za-z_$][\w$]*\s*=\s*(?:spawnSync|copyFile|link|rm|writeFile|rename|fetch)\b(?!\s*\()/,
];
if (forbiddenPortableNodeSyntax.some((pattern) => pattern.test(portableNodeSource))) {
  fail(
    "portable Node syntax allowlist forbids computed/global/builtin-module access, capability aliases, and indirect execution, network, or write access",
  );
}

const expectedPortableCallCounts = new Map([
  ["copyFile", 3],
  ["link", 1],
  ["lstat", 6],
  ["mkdir", 1],
  ["randomUUID", 1],
  ["readFile", 3],
  ["readdir", 1],
  ["realpath", 2],
  ["rm", 2],
  ["spawnSync", 1],
  ["writeFile", 1],
]);
for (const [operation, expected] of expectedPortableCallCounts) {
  const count = [...portableNodeSource.matchAll(
    new RegExp(`\\b${operation}\\s*\\(`, "g"),
  )].length;
  if (count !== expected) {
    fail(
      `portable Node operation allowlist requires ${operation} exactly ${expected} time(s); found ${count}`,
    );
  }
}
if (
  !portableNodeSource.includes(
    'const targetDir = join(repoRoot, "target", "release");',
  ) ||
  !portableNodeSource.includes(
    'const bundleDir = join(targetDir, "bundle", "portable");',
  ) ||
  !portableNodeSource.includes(
    'await copyFile(executable, join(stageRoot, "ability-radar.exe"));',
  ) ||
  !portableNodeSource.includes(
    'await copyFile(readme, join(stageRoot, "README.txt"));',
  ) ||
  !portableNodeSource.includes("await copyFile(entry.path, destination);") ||
  !portableNodeSource.includes("await link(temporaryArchive, archivePath);") ||
  !portableNodeSource.includes("await rm(path, { recursive: true });") ||
  !portableNodeSource.includes("await rm(path);")
) {
  fail(
    "portable Node operation allowlist rejects copyFile, link, or removal destinations outside the reviewed target/release/bundle/portable flow",
  );
}
const exactPortableSpawn = `spawnSync(
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
    )`;
if (!portableNodeSource.includes(exactPortableSpawn)) {
  fail(
    "portable Node child process allowlist permits only powershell.exe with the reviewed compressor, stage source, and temporary archive destination",
  );
}

const reviewedPowerShell = portablePowerShellSource
  .replace(/^\s*#.*$/gm, "")
  .replace(/\r\n?/g, "\n");
const expectedPowerShellCounts = new Map([
  ["Compress-Archive", 1],
  ["Get-Item", 2],
  ["Split-Path", 1],
  ["Test-Path", 3],
]);
for (const [operation, expected] of expectedPowerShellCounts) {
  const count = [...reviewedPowerShell.matchAll(
    new RegExp(`\\b${operation}\\b`, "g"),
  )].length;
  if (count !== expected) {
    fail(
      `portable PowerShell operation allowlist requires ${operation} exactly ${expected} time(s); found ${count}`,
    );
  }
}
const allowedPowerShellStatements = new Set([
  "param(",
  "[Parameter(Mandatory = $true)]",
  "[string]$Source,",
  "[string]$Destination",
  ")",
  '$ErrorActionPreference = "Stop"',
  "$sourcePath = [System.IO.Path]::GetFullPath($Source)",
  "$destinationPath = [System.IO.Path]::GetFullPath($Destination)",
  "if (-not (Test-Path -LiteralPath $sourcePath -PathType Container)) {",
  'throw "Portable source directory does not exist."',
  "}",
  'if ([System.IO.Path]::GetExtension($destinationPath) -cne ".zip") {',
  'throw "Portable destination must be a .zip file."',
  "$destinationDirectory = Split-Path -Parent $destinationPath",
  "if (-not (Test-Path -LiteralPath $destinationDirectory -PathType Container)) {",
  'throw "Portable destination directory does not exist."',
  "if (Test-Path -LiteralPath $destinationPath) {",
  'throw "Portable destination already exists."',
  "$sourceItem = Get-Item -LiteralPath $sourcePath",
  "$destinationDirectoryItem = Get-Item -LiteralPath $destinationDirectory",
  "if (",
  "($sourceItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or",
  "($destinationDirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)",
  ") {",
  'throw "Portable compressor paths must not be reparse points."',
  "Compress-Archive `",
  "-LiteralPath $sourcePath `",
  "-DestinationPath $destinationPath `",
  "-CompressionLevel Optimal",
]);
for (const statement of reviewedPowerShell
  .split("\n")
  .map((line) => line.trim())
  .filter(Boolean)) {
  if (!allowedPowerShellStatements.has(statement)) {
    fail(
      `portable PowerShell operation allowlist rejects unsupported statement: ${statement}`,
    );
  }
}
if (
  /(?:^|[\s;|])(?:&|\.|Invoke-Expression|iex|Set-Alias|New-Alias|Get-Command)(?:\s|$)/im.test(
    reviewedPowerShell,
  ) ||
  !reviewedPowerShell.includes(
    "$sourcePath = [System.IO.Path]::GetFullPath($Source)",
  ) ||
  !reviewedPowerShell.includes(
    "$destinationPath = [System.IO.Path]::GetFullPath($Destination)",
  ) ||
  !reviewedPowerShell.includes(
    "$destinationDirectory = Split-Path -Parent $destinationPath",
  ) ||
  !reviewedPowerShell.includes(
    "$sourceItem = Get-Item -LiteralPath $sourcePath",
  ) ||
  !reviewedPowerShell.includes(
    "$destinationDirectoryItem = Get-Item -LiteralPath $destinationDirectory",
  ) ||
  !reviewedPowerShell.includes(
    "Compress-Archive `\n  -LiteralPath $sourcePath `\n  -DestinationPath $destinationPath `\n  -CompressionLevel Optimal",
  )
) {
  fail(
    "portable PowerShell operation allowlist permits only direct path validation and one direct Compress-Archive invocation",
  );
}
const portableSourceSeals = new Map([
  [
    "scripts/package-portable.mjs",
    "dd75003e1739cd130430801e41f107461db73d10278991f8d95fa13a45ba2fd9",
  ],
  [
    "scripts/compress-portable.ps1",
    "d42425e9544bd0d4e4c9d021d1ec8b8ce13b328d93da3f5e5d4a3417f81c550a",
  ],
]);
for (const [path, expected] of portableSourceSeals) {
  if (normalizedSourceSha256(portableSources.get(path)) !== expected) {
    fail(
      `${path} portable source seal mismatch; review provider invocations, network uploads, and writes outside target/release/bundle/portable`,
    );
  }
}
if (
  JSON.stringify(rootPackage.allowScripts) !==
  JSON.stringify({ "esbuild@0.28.1": true })
) {
  fail("package.json must approve only the locked esbuild lifecycle script");
}
const manifestVersions = [
  ["package.json", rootPackage.version],
  ["package-lock.json root", npmLock.packages?.[""]?.version],
  ["apps/desktop/package.json", desktopPackage.version],
  ["apps/desktop/src-tauri/tauri.conf.json", tauriConfig.version],
  ["package-lock.json workspace", npmLock.packages?.["apps/desktop"]?.version],
];
const firstPartyLicenses = [
  ["package.json", rootPackage.license],
  ["package-lock.json root", npmLock.packages?.[""]?.license],
  ["apps/desktop/package.json", desktopPackage.license],
  ["package-lock.json workspace", npmLock.packages?.["apps/desktop"]?.license],
];
for (const path of [
  "apps/desktop/src-tauri/Cargo.toml",
  "crates/ability-core/Cargo.toml",
  "crates/ability-adapters/Cargo.toml",
]) {
  const manifest = read(path);
  const version = manifest.match(/^version = "([^"]+)"$/m)?.[1];
  manifestVersions.push([path, version]);
  const license = manifest.match(/^license = "([^"]+)"$/m)?.[1];
  firstPartyLicenses.push([path, license]);
}
for (const [path, version] of manifestVersions) {
  if (version !== expectedVersion) {
    fail(`${path} version must be ${expectedVersion}; found ${version ?? "missing"}`);
  }
}
for (const [path, license] of firstPartyLicenses) {
  if (license !== "Apache-2.0") {
    fail(`${path} first-party license must be Apache-2.0; found ${license ?? "missing"}`);
  }
}
const ownedCargo = new Set(["ability-radar", "ability-core", "ability-adapters"]);
for (const pkg of cargoPackages(read("Cargo.lock")).filter(({ name }) => ownedCargo.has(name))) {
  if (pkg.version !== expectedVersion) {
    fail(`Cargo.lock ${pkg.name} must be ${expectedVersion}; found ${pkg.version}`);
  }
}

const workspaceManifest = read("Cargo.toml");
const workspaceMembers = tomlStringArray(
  workspaceManifest,
  "workspace",
  "members",
);
if (
  workspaceMembers?.filter((member) => member === "tools/fake-cli").length !==
  1
) {
  fail("Cargo workspace must include tools/fake-cli exactly once in the members array");
}
const fakeManifest = read("tools/fake-cli/Cargo.toml");
const fakeManifestSourceSeal = "7c767f6e1420f6a12547abd526b311c39861b8355a8973b1cc1553a1b800d57d";
if (normalizedSourceSha256(fakeManifest) !== fakeManifestSourceSeal) {
  fail("tools/fake-cli/Cargo.toml normalized source seal mismatch");
}
const fakeManifestContracts = [
  ["package name", /^name = "ability-radar-fake-cli"$/m],
  ["fixture version 0.1.0", /^version = "0\.1\.0"$/m],
  ["first-party Apache-2.0 license", /^license = "Apache-2\.0"$/m],
  ["publish = false", /^publish = false$/m],
  ["serde_json dependency", /^serde_json = "1"$/m],
];
for (const [label, pattern] of fakeManifestContracts) {
  if (!pattern.test(fakeManifest)) fail(`fake CLI manifest is missing ${label}`);
}
const parsedFakeManifestTables = tomlTables(fakeManifest);
for (const header of parsedFakeManifestTables.invalidHeaders) {
  fail(
    `fake CLI manifest has invalid TOML table header at line ${header.line}: ${header.raw}`,
  );
}
const dependencyTableSegments = new Set([
  "dependencies",
  "build-dependencies",
  "dev-dependencies",
]);
function isCargoDependencyTable({ segments }) {
  if (dependencyTableSegments.has(segments[0])) return true;
  return (
    segments.length >= 3 &&
    segments[0] === "target" &&
    dependencyTableSegments.has(segments[2])
  );
}
const fakeDependencyTables = parsedFakeManifestTables.tables.filter(
  isCargoDependencyTable,
);
const directFakeDependencyTables = fakeDependencyTables.filter(
  ({ segments }) => (
    segments.length === 1 && segments[0] === "dependencies"
  ),
);
const fakeDependencies = directFakeDependencyTables.flatMap(
  ({ entries }) => entries,
);
if (
  directFakeDependencyTables.length !== 1 ||
  fakeDependencies.length !== 1 ||
  fakeDependencies[0] !== 'serde_json = "1"'
) {
  fail('fake CLI dependency set must be exactly serde_json = "1"');
}
for (const table of fakeDependencyTables.filter(
  ({ segments, entries }) =>
    !(segments.length === 1 && segments[0] === "dependencies") &&
    entries.length > 0,
)) {
  fail(
    `fake CLI dependency surface must not declare ${table.name}; only direct serde_json is allowed`,
  );
}
const lockedFake = cargoPackages(read("Cargo.lock")).filter(
  ({ name }) => name === "ability-radar-fake-cli",
);
if (
  lockedFake.length !== 1 ||
  lockedFake[0].version !== "0.1.0" ||
  lockedFake[0].source ||
  lockedFake[0].dependenciesValid !== true ||
  JSON.stringify(lockedFake[0].dependencies) !== JSON.stringify(["serde_json"])
) {
  fail(
    "Cargo.lock must contain exactly one first-party fake CLI at 0.1.0 with dependencies exactly serde_json",
  );
}
if (/tools[\\/]fake-cli|ability-radar-fake-cli/i.test(JSON.stringify(tauriConfig.bundle?.resources ?? {}))) {
  fail("fake CLI must never be a bundled Tauri resource");
}

const actions = new Map([
  ["actions/checkout", ["df4cb1c069e1874edd31b4311f1884172cec0e10", "v6"]],
  ["actions/setup-node", ["249970729cb0ef3589644e2896645e5dc5ba9c38", "v6"]],
  ["actions/upload-artifact", ["043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", "v7"]],
  ["dtolnay/rust-toolchain", ["2c7215f132e9ebf062739d9130488b56d53c060c", "reviewed master"]],
  ["tauri-apps/tauri-action", ["944946e3e4cac6603d1fe8f514171e9ecd3c78aa", "v1"]],
  ["actions/configure-pages", ["983d7736d9b0ae728b81ab479565c72886d7745b", "v5"]],
  ["actions/upload-pages-artifact", ["fc324d3547104276b827a68afc52ff2a11cc49c9", "v5"]],
  ["actions/deploy-pages", ["cd2ce8fcbc39b97be8ca5fce6e763baed58fa128", "v5"]],
]);
const workflowPaths = [
  ".github/workflows/ci.yml",
  ".github/workflows/release.yml",
  ".github/workflows/pages.yml",
];
// Publication workflows are sealed as a fail-closed backstop for YAML syntax
// the lightweight structural parser intentionally does not model. Intentional
// changes must update both this reviewed normalized-source seal and the exact
// structural contracts below.
const publicationWorkflowSeals = new Map([
  [
    ".github/workflows/release.yml",
    "ce165a64435a551bd1fb43da7df0eeb99e3210cb0e329c6fae8516bcb9fcc2fc",
  ],
  [
    ".github/workflows/pages.yml",
    "64d04a6b6c57188551b246392c29b286b9c9e2deaea7000d5f0a6963265a0f29",
  ],
]);
const workflows = new Map();
for (const path of workflowPaths) {
  const source = read(path);
  const reviewedSeal = publicationWorkflowSeals.get(path);
  if (reviewedSeal && normalizedSourceSha256(source) !== reviewedSeal) {
    fail(`${path} normalized source seal mismatch`);
  }
  const workflow = parseWorkflow(source);
  workflows.set(path, { source, workflow });

  const steps = [...workflow.jobs.values()].flatMap((job) => job.steps);
  for (const step of steps.filter(({ uses }) => uses)) {
    const match = step.uses.match(/^([^@\s]+)@([^\s]+)$/);
    if (!match) {
      fail(`${path} has an invalid action reference: ${step.uses}`);
      continue;
    }
    const [, action, sha] = match;
    const expected = actions.get(action);
    if (!expected) {
      fail(`${path} uses unreviewed third-party action ${action}`);
      continue;
    }
    if (!/^[0-9a-f]{40}$/.test(sha)) {
      fail(`${path} action ${action} is not pinned to a full commit SHA`);
    }
    if (sha !== expected[0] || step.usesComment !== expected[1]) {
      fail(`${path} must pin ${action}@${expected[0]} # ${expected[1]}`);
    }
  }
  if ([...workflow.jobs.values()].some((job) => !/^\d+$/.test(job.timeoutMinutes ?? ""))) {
    fail(`${path} needs an explicit timeout on every job`);
  }
  if (!workflow.hasConcurrency) fail(`${path} needs concurrency control`);
  if (workflow.topPermissions !== "{}") {
    fail(`${path} needs deny-by-default top-level permissions`);
  }
  const checkouts = actionSteps(workflow, "actions/checkout");
  if (checkouts.some((step) => step.with["persist-credentials"] !== "false")) {
    fail(`${path} checkout must set persist-credentials: false`);
  }
  if (/(?:OPENAI|ANTHROPIC|CLAUDE|CODEX|PROVIDER)[A-Z0-9_]*(?:KEY|TOKEN|SECRET)/i.test(source)) {
    fail(`${path} defines a provider credential name`);
  }
  for (const step of runSteps(workflow)) {
    if (/(^|[\r\n;&|]\s*)(?:codex|claude)(?:\.exe)?(?:\s|$)/im.test(step.run)) {
      fail(`${path} must not invoke a real AI CLI`);
    }
  }
}

const requiredActions = new Map([
  [".github/workflows/ci.yml", [
    "actions/checkout",
    "actions/setup-node",
    "dtolnay/rust-toolchain",
    "actions/upload-artifact",
  ]],
  [".github/workflows/release.yml", [
    "actions/checkout",
    "actions/setup-node",
    "dtolnay/rust-toolchain",
    "tauri-apps/tauri-action",
  ]],
  [".github/workflows/pages.yml", [
    "actions/checkout",
    "actions/configure-pages",
    "actions/upload-pages-artifact",
    "actions/deploy-pages",
  ]],
]);
for (const [path, required] of requiredActions) {
  const workflow = workflows.get(path)?.workflow;
  for (const action of required) {
    const count = actionSteps(workflow, action).length;
    if (count !== 1) fail(`${path} must use ${action} exactly once; found ${count}`);
  }
  const actualSequence = [...workflow.jobs.values()]
    .flatMap((job) => job.steps)
    .filter((step) => step.uses)
    .map((step) => step.uses.split("@", 1)[0]);
  if (JSON.stringify(actualSequence) !== JSON.stringify(required)) {
    fail(
      `${path} approved action sequence must be exact: ${required.join(", ")}`,
    );
  }
}

function requireCommand(path, job, label, pattern) {
  if (!hasRunCommand(job, pattern)) {
    fail(`${path} is missing ${label}`);
  }
}

function exactObject(actual, expected) {
  const actualEntries = Object.entries(actual ?? {}).sort(([a], [b]) =>
    a.localeCompare(b, "en"),
  );
  const expectedEntries = Object.entries(expected).sort(([a], [b]) =>
    a.localeCompare(b, "en"),
  );
  return JSON.stringify(actualEntries) === JSON.stringify(expectedEntries);
}

const baseStepFields = [
  "env",
  "envDeclaration",
  "envValid",
  "name",
  "run",
  "uses",
  "usesComment",
  "with",
  "withDeclaration",
  "withValid",
];

function exactFields(actual, expected) {
  return (
    JSON.stringify(Object.keys(actual ?? {}).sort()) ===
    JSON.stringify([...expected].sort())
  );
}

function requireExactStepFields(path, step, extraFields, label) {
  if (!exactFields(step, [...baseStepFields, ...extraFields])) {
    fail(`${path} ${label} has unallowlisted step fields or controls`);
  }
}

function requireExactStepContract(path, step, expected, label) {
  const extra = expected.extra ?? {};
  requireExactStepFields(path, step, Object.keys(extra), label);
  const expectedWith = expected.with ?? {};
  const expectedEnv = expected.env ?? {};
  const expectedWithDeclaration =
    Object.keys(expectedWith).length > 0 ? "block" : "absent";
  const expectedEnvDeclaration =
    Object.keys(expectedEnv).length > 0 ? "block" : "absent";
  const scalarContract =
    step?.name === expected.name &&
    step?.uses === (expected.uses ?? "") &&
    step?.usesComment === (expected.usesComment ?? "") &&
    step?.run === (expected.run ?? "") &&
    step?.withDeclaration === expectedWithDeclaration &&
    step?.withValid === true &&
    step?.envDeclaration === expectedEnvDeclaration &&
    step?.envValid === true;
  const extraContract = Object.entries(extra).every(
    ([key, value]) => step?.[key] === value,
  );
  if (
    !scalarContract ||
    !extraContract ||
    !exactObject(step?.with, expectedWith) ||
    !exactObject(step?.env, expectedEnv)
  ) {
    fail(`${path} ${label} contract must be exact`);
  }
}

const baseJobFields = [
  "env",
  "envDeclaration",
  "envValid",
  "id",
  "permissions",
  "permissionsDeclaration",
  "permissionsValid",
  "runs-on",
  "steps",
  "timeoutMinutes",
];

function requireExactJobContract(path, job, expected, label) {
  const extra = expected.extra ?? {};
  const expectedEnv = expected.env ?? {};
  const expectedEnvDeclaration =
    Object.keys(expectedEnv).length > 0 ? "block" : "absent";
  const fieldsExact = exactFields(job, [
    ...baseJobFields,
    ...Object.keys(extra),
  ]);
  const extraContract = Object.entries(extra).every(
    ([key, value]) => job?.[key] === value,
  );
  if (
    !fieldsExact ||
    job?.id !== expected.id ||
    job?.["runs-on"] !== expected.runsOn ||
    job?.timeoutMinutes !== expected.timeoutMinutes ||
    job?.permissionsDeclaration !== "block" ||
    job?.permissionsValid !== true ||
    !exactObject(job?.permissions, expected.permissions) ||
    job?.envDeclaration !== expectedEnvDeclaration ||
    job?.envValid !== true ||
    !exactObject(job?.env, expectedEnv) ||
    !extraContract
  ) {
    fail(`${path} ${label} fields or contract must be exact`);
  }
}

function requireNoTopLevelEnv(path, workflow, label) {
  if (
    workflow?.topEnvDeclaration !== "absent" ||
    workflow?.topEnvValid !== true ||
    !exactObject(workflow?.topEnv, {})
  ) {
    fail(`${path} ${label} must have no top-level env declaration`);
  }
}

function namedStep(path, job, name) {
  const matches = job?.steps.filter((step) => step.name === name) ?? [];
  if (matches.length !== 1) {
    fail(`${path} must have exactly one step named ${name}; found ${matches.length}`);
  }
  return matches[0];
}

const ciPath = ".github/workflows/ci.yml";
const ciWorkflow = workflows.get(ciPath)?.workflow;
const ciJob = ciWorkflow?.jobs.get("test");
if (
  ciWorkflow?.topEnvDeclaration !== "absent" ||
  ciWorkflow?.topEnvValid !== true ||
  !exactObject(ciWorkflow?.topEnv, {})
) {
  fail(`${ciPath} CI workflow must have no env declaration or environment`);
}
if (
  ciJob?.envDeclaration !== "absent" ||
  ciJob?.envValid !== true ||
  !exactObject(ciJob?.env, {})
) {
  fail(`${ciPath} CI job must have no env declaration or environment`);
}
requireExactJobContract(
  ciPath,
  ciJob,
  {
    id: "test",
    runsOn: "windows-latest",
    timeoutMinutes: "60",
    permissions: { contents: "read" },
  },
  "CI job",
);
if (!exactPermissions(ciJob, { contents: "read" })) {
  fail(`${ciPath} test job permissions must be exactly contents: read`);
}
const ciNode = actionSteps(ciWorkflow, "actions/setup-node")[0];
if (ciNode?.with["node-version"] !== "22") fail(`${ciPath} must use Node.js 22`);
const ciRust = actionSteps(ciWorkflow, "dtolnay/rust-toolchain")[0];
if (ciRust?.with.toolchain !== "stable") {
  fail(`${ciPath} Rust toolchain action must explicitly select stable`);
}
if (ciRust?.with.components !== "clippy,rustfmt") {
  fail(`${ciPath} Rust toolchain action must install clippy,rustfmt`);
}
const ciCommands = [
  ["npm ci", /(?:^|\n)\s*npm ci\s*(?:$|\n)/],
  ["repository validation", /(?:^|\n)\s*npm run validate:repository\s*(?:$|\n)/],
  ["cargo-audit 0.22.2", /(?:^|\n)\s*cargo install cargo-audit --version 0\.22\.2 --locked\s*(?:$|\n)/],
  ["cargo audit", /(?:^|\n)\s*cargo audit\s*(?:$|\n)/],
  ["npm high-severity audit", /(?:^|\n)\s*npm audit --audit-level=high\s*(?:$|\n)/],
  ["Rust formatting check", /(?:^|\n)\s*cargo fmt --all --check\s*(?:$|\n)/],
  ["locked all-target clippy", /(?:^|\n)\s*cargo clippy --workspace --all-targets --locked -- -D warnings\s*(?:$|\n)/],
  ["locked all-target tests", /(?:^|\n)\s*cargo test --workspace --all-targets --locked\s*(?:$|\n)/],
  ["locked fake CLI build", /(?:^|\n)\s*cargo build -p ability-radar-fake-cli --locked\s*(?:$|\n)/],
  ["temporary fake CLI directory", /Join-Path \$env:RUNNER_TEMP "ability-radar-fake-bin"/],
  ["fake Codex executable copy", /Copy-Item target\/debug\/ability-radar-fake-cli\.exe \(Join-Path \$fakeBin "codex\.exe"\)/],
  ["fake Claude executable copy", /Copy-Item target\/debug\/ability-radar-fake-cli\.exe \(Join-Path \$fakeBin "claude\.exe"\)/],
  ["temporary fake CLI PATH install", /"\$fakeBin" \| Out-File -FilePath \$env:GITHUB_PATH -Encoding utf8 -Append/],
  ["locked opted-in fake CLI E2E", /(?:^|\n)\s*cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored\s*(?:$|\n)/],
  ["frontend tests", /(?:^|\n)\s*npm test\s*(?:$|\n)/],
  ["frontend build", /(?:^|\n)\s*npm run build\s*(?:$|\n)/],
  ["debug NSIS build", /(?:^|\n)\s*npm run tauri -- build --debug --bundles nsis\s*(?:$|\n)/],
];
for (const [label, pattern] of ciCommands) requireCommand(ciPath, ciJob, label, pattern);
const fakeInstallName = "Install deterministic fake CLIs";
const fakeE2eName = "Test real coordinator with deterministic fake CLIs";
const fakeInstall = namedStep(ciPath, ciJob, fakeInstallName);
const fakeE2e = namedStep(ciPath, ciJob, fakeE2eName);
requireExactStepFields(ciPath, fakeInstall, [], fakeInstallName);
requireExactStepFields(ciPath, fakeE2e, [], fakeE2eName);
const expectedFakeInstallRun = `cargo build -p ability-radar-fake-cli --locked
$fakeBin = Join-Path $env:RUNNER_TEMP "ability-radar-fake-bin"
New-Item -ItemType Directory -Force -Path $fakeBin | Out-Null
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "codex.exe")
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "claude.exe")
"$fakeBin" | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append`;
if (
  fakeInstall?.run !== expectedFakeInstallRun ||
  fakeInstall?.uses ||
  !exactObject(fakeInstall?.env, {}) ||
  !exactObject(fakeInstall?.with, {})
) {
  fail(`${ciPath} ${fakeInstallName} step must have the exact fake-only run contract`);
}
const expectedFakeE2eRun =
  "cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored";
requireExactStepContract(
  ciPath,
  fakeInstall,
  { name: fakeInstallName, run: expectedFakeInstallRun },
  fakeInstallName,
);
requireExactStepContract(
  ciPath,
  fakeE2e,
  {
    name: fakeE2eName,
    run: expectedFakeE2eRun,
    env: { ABILITY_RADAR_FAKE_CLI_E2E: "1" },
  },
  fakeE2eName,
);
if (
  fakeE2e?.run !== expectedFakeE2eRun ||
  fakeE2e?.uses ||
  !exactObject(fakeE2e?.with, {})
) {
  fail(`${ciPath} ${fakeE2eName} step must have the exact E2E run contract`);
}
if (
  fakeE2e?.envDeclaration !== "block" ||
  fakeE2e?.envValid !== true ||
  !exactObject(fakeE2e?.env, { ABILITY_RADAR_FAKE_CLI_E2E: "1" })
) {
  fail(`${ciPath} fake CLI E2E environment must exactly opt in on ${fakeE2eName}`);
}
const fakeInstallIndex = ciJob?.steps.indexOf(fakeInstall) ?? -1;
const fakeE2eIndex = ciJob?.steps.indexOf(fakeE2e) ?? -1;
if (fakeInstallIndex < 0 || fakeE2eIndex !== fakeInstallIndex + 1) {
  fail(`${ciPath} fake CLI install must be immediately before its E2E step`);
}
for (const step of ciJob?.steps ?? []) {
  if (step === fakeInstall || step === fakeE2e) continue;
  if (
    /ability-radar-fake-cli|ability-radar-fake-bin|fake_cli_e2e/.test(step.run) ||
    Object.hasOwn(step.env, "ABILITY_RADAR_FAKE_CLI_E2E")
  ) {
    fail(`${ciPath} fake CLI commands and opt-in may exist only in the named fake steps`);
  }
}
const ciArtifact = actionSteps(ciWorkflow, "actions/upload-artifact")[0];
requireExactStepFields(ciPath, ciArtifact, [], "CI artifact owner");
const expectedCiArtifactInputs = {
  name: "ability-radar-windows-debug-nsis",
  path: "target/debug/bundle/nsis/ability-radar_0.2.1_x64-setup.exe",
  "if-no-files-found": "error",
  "retention-days": "7",
};
requireExactStepContract(
  ciPath,
  ciArtifact,
  {
    name: "Upload exact debug installer",
    uses: "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
    usesComment: "v7",
    with: expectedCiArtifactInputs,
  },
  "CI artifact owner",
);
if (
  ciArtifact?.name !== "Upload exact debug installer" ||
  !exactObject(ciArtifact?.with, expectedCiArtifactInputs)
) {
  fail(`${ciPath} CI artifact input allowlist must contain only the exact debug NSIS installer`);
}
for (const path of workflowPaths) {
  const workflow = workflows.get(path)?.workflow;
  for (const step of actionSteps(workflow, "actions/upload-artifact")) {
    if (/fake|tools[\\/]fake-cli|ability-radar-fake-cli/i.test(step.with.path ?? "")) {
      fail(`${path} must never upload the fake CLI`);
    }
  }
}

const releasePath = ".github/workflows/release.yml";
const releaseWorkflow = workflows.get(releasePath)?.workflow;
const releaseJob = releaseWorkflow?.jobs.get("release");
requireNoTopLevelEnv(releasePath, releaseWorkflow, "release workflow");
requireExactJobContract(
  releasePath,
  releaseJob,
  {
    id: "release",
    runsOn: "windows-latest",
    timeoutMinutes: "60",
    permissions: { contents: "write" },
    env: { RELEASE_TAG: "${{ github.ref_name }}" },
  },
  "release job",
);
const exactVerifyTagRun = `$tag = $env:RELEASE_TAG
if ($tag -cnotmatch '^v(0|[1-9]\\d*)\\.(0|[1-9]\\d*)\\.(0|[1-9]\\d*)$') {
  throw "Release tag must be a strict vMAJOR.MINOR.PATCH semantic version."
}
$config = Get-Content apps/desktop/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
if ("v$($config.version)" -cne $tag) {
  throw "Release tag does not exactly match the Tauri application version."
}`;
const exactReleaseBody = `Windows 10/11 x64 v0.2 预览版。

**警告：安装程序未签名。** Windows SmartScreen 可能显示风险提示。
核心数据默认只保存在本机；真实 CLI 测试消耗运行者自己的订阅用量。
下载后请使用随发布提供的 SHA256SUMS.txt 校验安装程序。`;
const exactChecksumRun = `$files = Get-ChildItem target/release/bundle -Recurse -File |
  Where-Object { $_.Extension -in ".exe", ".msi" } |
  Sort-Object FullName
if (-not $files) {
  throw "No Windows installer was produced; refusing to publish an empty checksum file."
}
$lines = foreach ($file in $files) {
  $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant()
  "$hash  $($file.Name)"
}
Set-Content -LiteralPath SHA256SUMS.txt -Value $lines -Encoding utf8NoBOM`;
const exactReleaseSteps = [
  {
    name: "Check out tagged revision",
    label: "release checkout input",
    uses: "actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10",
    usesComment: "v6",
    with: { "fetch-depth": "0", "persist-credentials": "false" },
  },
  {
    name: "Verify release tag",
    run: exactVerifyTagRun,
    extra: { shell: "pwsh" },
  },
  {
    name: "Set up Node.js",
    uses: "actions/setup-node@249970729cb0ef3589644e2896645e5dc5ba9c38",
    usesComment: "v6",
    with: { "node-version": "22", cache: "npm" },
  },
  {
    name: "Set up Rust",
    uses: "dtolnay/rust-toolchain@2c7215f132e9ebf062739d9130488b56d53c060c",
    usesComment: "reviewed master",
    with: { toolchain: "stable", components: "clippy,rustfmt" },
  },
  { name: "Install frontend dependencies", run: "npm ci" },
  {
    name: "Validate repository contracts",
    run: "npm run validate:repository",
  },
  { name: "Check Rust formatting", run: "cargo fmt --all --check" },
  {
    name: "Lint Rust",
    run: "cargo clippy --workspace --all-targets --locked -- -D warnings",
  },
  {
    name: "Test Rust",
    run: "cargo test --workspace --all-targets --locked",
  },
  { name: "Test frontend", run: "npm test" },
  {
    name: "Build unsigned draft prerelease",
    label: "Tauri release input allowlist owner",
    uses: "tauri-apps/tauri-action@944946e3e4cac6603d1fe8f514171e9ecd3c78aa",
    usesComment: "v1",
    env: { GITHUB_TOKEN: "${{ github.token }}" },
    with: {
      projectPath: "apps/desktop",
      tauriScript: "npm run tauri --",
      tagName: "${{ env.RELEASE_TAG }}",
      releaseName: "AI 能力雷达 ${{ env.RELEASE_TAG }}",
      releaseBody: exactReleaseBody,
      releaseDraft: "true",
      prerelease: "true",
      uploadUpdaterJson: "false",
      uploadUpdaterSignatures: "false",
    },
    extra: { id: "tauri" },
  },
  {
    name: "Generate SHA-256 checksums",
    run: exactChecksumRun,
    extra: { shell: "pwsh" },
  },
  {
    name: "Upload checksums to the draft prerelease",
    label: "checksum upload",
    run: "gh release upload $env:RELEASE_TAG SHA256SUMS.txt --clobber",
    env: { GH_TOKEN: "${{ github.token }}" },
    extra: { shell: "pwsh" },
  },
];
if (
  JSON.stringify(releaseJob?.steps.map((step) => step.name)) !==
  JSON.stringify(exactReleaseSteps.map(({ name }) => name))
) {
  fail(`${releasePath} release step sequence must be exact`);
}
for (const [index, expected] of exactReleaseSteps.entries()) {
  requireExactStepContract(
    releasePath,
    releaseJob?.steps[index],
    expected,
    expected.label ?? expected.name,
  );
}

const pagesPath = ".github/workflows/pages.yml";
const pagesWorkflow = workflows.get(pagesPath)?.workflow;
const pagesBuild = pagesWorkflow?.jobs.get("build");
const pagesDeploy = pagesWorkflow?.jobs.get("deploy");
requireNoTopLevelEnv(pagesPath, pagesWorkflow, "Pages workflow");
requireExactJobContract(
  pagesPath,
  pagesBuild,
  {
    id: "build",
    runsOn: "ubuntu-latest",
    timeoutMinutes: "10",
    permissions: { contents: "read", pages: "read" },
  },
  "Pages build job",
);
requireExactJobContract(
  pagesPath,
  pagesDeploy,
  {
    id: "deploy",
    runsOn: "ubuntu-latest",
    timeoutMinutes: "10",
    permissions: { pages: "write", "id-token": "write" },
    extra: { needs: "build", environment: "" },
  },
  "Pages deploy job",
);
const expectedPagesBuildSteps = [
  "Check out repository",
  "Configure Pages",
  "Validate repository contracts",
  "Assemble static site",
  "Upload Pages artifact",
];
const expectedPagesDeploySteps = ["Deploy"];
if (
  JSON.stringify(pagesBuild?.steps.map((step) => step.name)) !==
    JSON.stringify(expectedPagesBuildSteps) ||
  JSON.stringify(pagesDeploy?.steps.map((step) => step.name)) !==
    JSON.stringify(expectedPagesDeploySteps)
) {
  fail(`${pagesPath} Pages step sequence must be exact`);
}
if (!exactPermissions(pagesBuild, { contents: "read", pages: "read" })) {
  fail(`${pagesPath} build permissions must be exactly contents: read and pages: read`);
}
if (!exactPermissions(pagesDeploy, { pages: "write", "id-token": "write" })) {
  fail(`${pagesPath} deploy permissions must be exactly pages: write and id-token: write`);
}
requireCommand(
  pagesPath,
  pagesBuild,
  "site assembly",
  /(?:^|\n)\s*cp docs\/privacy\.md _site\/docs\/privacy\.md\s*(?:$|\n)/,
);
const assembleSite = namedStep(pagesPath, pagesBuild, "Assemble static site");
const pagesCheckout = namedStep(pagesPath, pagesBuild, "Check out repository");
const configurePages = namedStep(pagesPath, pagesBuild, "Configure Pages");
const validatePages = namedStep(
  pagesPath,
  pagesBuild,
  "Validate repository contracts",
);
const deployPages = namedStep(pagesPath, pagesDeploy, "Deploy");
requireExactStepFields(pagesPath, pagesCheckout, [], "Pages checkout");
requireExactStepFields(pagesPath, configurePages, [], "Configure Pages");
requireExactStepFields(
  pagesPath,
  validatePages,
  [],
  "Validate repository contracts",
);
requireExactStepFields(pagesPath, assembleSite, [], "Assemble static site");
requireExactStepFields(pagesPath, deployPages, ["id"], "Deploy Pages owner");
if (validatePages?.run !== "node scripts/validate-repository.mjs") {
  fail(`${pagesPath} Pages step sequence and commands must be exact`);
}
const expectedSiteAssembly = `cp -R site _site
mkdir -p _site/docs
cp docs/privacy.md _site/docs/privacy.md
cp docs/security.md _site/docs/security.md
cp docs/methodology.md _site/docs/methodology.md
cp docs/troubleshooting.md _site/docs/troubleshooting.md`;
const exactPagesBuildContracts = [
  {
    name: "Check out repository",
    label: "Pages checkout input",
    uses: "actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10",
    usesComment: "v6",
    with: { "persist-credentials": "false" },
  },
  {
    name: "Configure Pages",
    label: "Configure Pages input",
    uses: "actions/configure-pages@983d7736d9b0ae728b81ab479565c72886d7745b",
    usesComment: "v5",
  },
  {
    name: "Validate repository contracts",
    label: "Pages validator",
    run: "node scripts/validate-repository.mjs",
  },
  {
    name: "Assemble static site",
    label: "Pages assembly",
    run: expectedSiteAssembly,
  },
  {
    name: "Upload Pages artifact",
    label: "Upload Pages artifact",
    uses:
      "actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9",
    usesComment: "v5",
    with: { path: "_site" },
  },
];
for (const [index, expected] of exactPagesBuildContracts.entries()) {
  requireExactStepContract(
    pagesPath,
    pagesBuild?.steps[index],
    expected,
    expected.label,
  );
}
requireExactStepContract(
  pagesPath,
  pagesDeploy?.steps[0],
  {
    name: "Deploy",
    uses: "actions/deploy-pages@cd2ce8fcbc39b97be8ca5fce6e763baed58fa128",
    usesComment: "v5",
    extra: { id: "deployment" },
  },
  "Deploy Pages input",
);
if (
  assembleSite?.run !== expectedSiteAssembly ||
  assembleSite?.uses ||
  !exactObject(assembleSite?.env, {}) ||
  !exactObject(assembleSite?.with, {})
) {
  fail(`${pagesPath} Assemble static site step must own the exact site assembly commands`);
}
for (const step of runSteps(pagesWorkflow)) {
  if (
    step !== assembleSite &&
    /_site(?:\/|\b)/.test(step.run)
  ) {
    fail(`${pagesPath} non-assembly steps must not write into _site`);
  }
}
const pagesArtifact = actionSteps(
  pagesWorkflow,
  "actions/upload-pages-artifact",
)[0];
requireExactStepFields(pagesPath, pagesArtifact, [], "Upload Pages artifact");
if (
  pagesArtifact?.name !== "Upload Pages artifact" ||
  !exactObject(pagesArtifact?.with, { path: "_site" })
) {
  fail(`${pagesPath} Pages artifact path must be exactly _site`);
}

const expectedTauriResources = {
  "../../../benchmark-packs/": "benchmark-packs/",
};
if (!exactObject(tauriConfig.bundle?.resources, expectedTauriResources)) {
  fail("Tauri resource allowlist must contain only the sealed benchmark packs");
}

const updaterInputs = [
  "package.json",
  "apps/desktop/package.json",
  "apps/desktop/src-tauri/Cargo.toml",
  "apps/desktop/src-tauri/tauri.conf.json",
  "Cargo.lock",
  "package-lock.json",
].map((path) => read(path)).join("\n");
if (/tauri-plugin-updater|@tauri-apps\/plugin-updater|\"updater\"\s*:|createUpdaterArtifacts/i.test(updaterInputs)) {
  fail("Tauri updater plugin/configuration must remain absent");
}

const site = requireText("site/index.html", [
  ["restrictive CSP", /default-src 'self';[^"]*img-src 'none';[^"]*font-src 'none';[^"]*connect-src 'none'/],
  ["Windows scope", /Windows 10\/11 x64/],
  ["Node LTS scope", /Node\.js 22\/24 LTS/],
  ["no degradation verdict", /不生成.*(?:降智|退化).*(?:裁决|结论)/],
  ["unsigned installer warning", /未签名/],
  ["subscription payer", /运行者.*(?:订阅|用量|费用)/],
  ["methodology link", /href="docs\/methodology\.md"/],
  ["privacy link", /href="docs\/privacy\.md"/],
  ["security link", /href="docs\/security\.md"/],
  ["v0.2.1 prerelease link", /\/releases\/tag\/v0\.2\.1/],
]);
if (/\/releases\/latest/.test(site)) {
  fail("site/index.html must not link a prerelease download through /releases/latest");
}
const forbiddenSitePatterns = [
  ["external resource URL", /(?:src|action)=["']https?:/i],
  ["external CSS URL", /url\(\s*["']?https?:/i],
  ["network API", /\b(?:fetch|XMLHttpRequest|sendBeacon|WebSocket)\s*\(/],
  ["analytics or tracking", /\b(?:analytics|gtag|googletagmanager|pixel|tracking)\b/i],
  ["cookie access", /document\.cookie/i],
  ["image element", /<img\b/i],
  ["font face", /@font-face/i],
];
for (const [label, pattern] of forbiddenSitePatterns) {
  if (pattern.test(site)) fail(`site/index.html contains forbidden ${label}`);
}

requireText("README.md", [
  ["v0.2 Windows preview status", /v0\.2.*Windows.*预览/],
  ["exact client task count", /8\s*道/],
  ["exact CLI task count", /2\s*(?:个|项)/],
  ["fake CI cost boundary", /GitHub CI.*(?:假|fake).*CLI/si],
  ["runner billing boundary", /GitHub.*runner.*仓库所有者.*GitHub.*计划/si],
  ["volunteer real-CLI cost boundary", /自愿.*测试.*自己的订阅/si],
  ["checksum verification", /SHA-?256/],
  ["design link", /docs\/superpowers\/specs\/2026-07-17-ai-ability-radar-design\.md/],
  ["plan link", /docs\/superpowers\/plans\/2026-07-17-ai-ability-radar-desktop-mvp\.md/],
]);
requireText("docs/methodology.md", [
  ["category-equal weighting", /类别等权/],
  ["original first-party tasks", /原创.*第一方/],
  ["Codex Radar exclusion", /Codex Radar/],
  ["DeepSWE exclusion", /DeepSWE/],
  ["contamination limitation", /污染/],
  ["default model semantics", /空白.*default/si],
  ["effort values", /low.*medium.*high/si],
  ["duration separation", /时长.*不.*跨.*比较/si],
  ["complete history key", /target kind.*trimmed reported model.*reasoning effort.*run mode.*suite ID\/version\/hash.*scoring-rule version.*OS family\/version.*app version.*CLI version.*Node verifier version.*clean-versus-resumed state/si],
  ["no v0.2 verdict", /v0\.2.*不生成.*(?:退化|降智).*裁决/si],
  ["planned v0.5 boundary", /v0\.5.*计划/si],
  ["infrastructure and budget distinction", /基础设施无效.*agent-budget/si],
  ["scoring rule version", /ability-v1/],
  ["pack schema version", /pack schema.*1/i],
  ["public report schema version", /public report schema.*1/i],
  ["backup schema version", /backup schema.*1/i],
]);
for (const path of ["docs/privacy.md", "docs/security.md"]) {
  requireText(path, [
    ["no app telemetry endpoint", /应用.*(?:没有|无).*遥测.*(?:上传端点|endpoint)/si],
    ["provider traffic disclosure", /提示词.*临时.*代码.*AI.*提供商/si],
    ["provider policy disclosure", /CLI.*提供商.*日志.*保留.*遥测/si],
    ["normal deletion", /正常.*删除/],
    ["SQLite secure_delete", /secure_delete/],
    ["WAL truncation", /WAL.*截断/si],
    ["retention limitations", /SSD.*文件系统快照.*杀毒.*外部备份/si],
    ["not a forensic wipe", /不是.*取证.*擦除/],
    ["real isolation controls", /workspace-write.*Read\/Edit\/Write.*dontAsk/si],
    ["not a strong sandbox", /不是.*(?:容器|VM|虚拟机).*(?:sandbox|沙箱)/si],
  ]);
}
requireText("docs/troubleshooting.md", [
  ["missing CLI", /CLI.*未找到/],
  ["login", /登录/],
  ["Node.js support", /Node\.js 22\/24 LTS/],
  ["quota", /配额/],
  ["network", /网络/],
  ["SmartScreen", /SmartScreen/],
  ["interrupted recovery", /中断.*恢复/si],
  ["local app data placeholder", /%APPDATA%/],
]);
requireText("SECURITY.md", [
  ["private GitHub advisory reporting", /Security.*Advisory.*Report a vulnerability/si],
  ["no raw public vulnerability details", /不要.*公开/],
]);
requireText(".github/ISSUE_TEMPLATE/bug.yml", [
  ["app version", /应用版本/],
  ["Windows version", /Windows 版本/],
  ["target type", /目标类型/],
  ["task pack version", /题包版本/],
  ["redacted category", /脱敏.*错误类别/],
  ["raw log warning", /不要.*原始日志.*(?:令牌|token)/si],
]);
requireText(".github/pull_request_template.md", [
  ["tests checklist", /测试.*(?:新增|更新)/],
  ["no real CI subscription CLI", /CI.*真实.*订阅.*CLI/],
  ["privacy field review", /隐私字段/],
  ["capability diff review", /capability.*diff/i],
  ["task license review", /题包.*许可/],
  ["Windows process check", /Windows.*进程.*取消/],
]);

const markdownPaths = [
  "README.md",
  "CONTRIBUTING.md",
  "SECURITY.md",
  "THIRD_PARTY_NOTICES.md",
  "docs/privacy.md",
  "docs/security.md",
  "docs/troubleshooting.md",
  "docs/methodology.md",
];
for (const path of markdownPaths) {
  const source = read(path);
  for (const match of source.matchAll(/!?\[[^\]]*]\(([^)]+)\)/g)) {
    let target = match[1].trim().replace(/^<|>$/g, "").split("#", 1)[0];
    if (!target || /^(?:https?:|mailto:|#)/i.test(target)) continue;
    target = decodeURIComponent(target);
    const absolute = normalize(resolve(root, dirname(path), target));
    if (relative(root, absolute).startsWith("..") || !existsSync(absolute)) {
      fail(`${path} has a broken internal link: ${match[1]}`);
    }
  }
}
for (const href of site.matchAll(/href="([^"]+)"/g)) {
  const target = href[1].split("#", 1)[0];
  if (!target || /^(?:https?:|#)/.test(target) || target === "./") continue;
  const sourcePath = join(root, "site", target);
  const repositoryPath = target.startsWith("docs/")
    ? join(root, target)
    : sourcePath;
  if (!existsSync(sourcePath) && !existsSync(repositoryPath)) {
    fail(`site/index.html has a broken internal link: ${href[1]}`);
  }
}

const rustReport = json("docs/licenses/rust-dependencies.json");
const npmReport = json("docs/licenses/npm-dependencies.json");
if (rustReport.generatedFrom !== "Cargo.lock") {
  fail("Rust license report must declare Cargo.lock as its source");
}
if (npmReport.generatedFrom !== "package-lock.json") {
  fail("npm license report must declare package-lock.json as its source");
}
if (rustReport.hashNormalization !== "UTF-8 text with CRLF and CR normalized to LF") {
  fail("Rust license report must declare cross-platform line-ending normalization");
}
if (npmReport.hashNormalization !== "UTF-8 text with CRLF and CR normalized to LF") {
  fail("npm license report must declare cross-platform line-ending normalization");
}
if (rustReport.lockfileSha256 !== sha256("Cargo.lock")) {
  fail("Rust license report is stale relative to Cargo.lock");
}
if (npmReport.lockfileSha256 !== sha256("package-lock.json")) {
  fail("npm license report is stale relative to package-lock.json");
}
const rustCoverage = new Map((rustReport.packages ?? []).map((pkg) => [packageKey(pkg), pkg]));
const npmCoverage = new Map((npmReport.packages ?? []).map((pkg) => [packageKey(pkg), pkg]));
const lockedRustPackages = cargoPackages(read("Cargo.lock")).filter(({ source }) => source);
const lockedNpmPackages = npmLockPackages(npmLock);
const expectedRustKeys = [...new Set(lockedRustPackages.map(packageKey))].sort();
const expectedNpmKeys = [...new Set(lockedNpmPackages.map(packageKey))].sort();
const reportedRustKeys = (rustReport.packages ?? []).map(packageKey);
const reportedNpmKeys = (npmReport.packages ?? []).map(packageKey);
if (new Set(reportedRustKeys).size !== reportedRustKeys.length) {
  fail("Rust license report contains duplicate package versions");
}
if (reportedRustKeys.some((key) => key.startsWith("ability-radar-fake-cli@"))) {
  fail("Rust third-party license report must exclude the first-party fake CLI workspace package");
}
if (new Set(reportedNpmKeys).size !== reportedNpmKeys.length) {
  fail("npm license report contains duplicate package versions");
}
if (JSON.stringify([...reportedRustKeys].sort()) !== JSON.stringify(expectedRustKeys)) {
  fail("Rust license report package set does not exactly match Cargo.lock");
}
if (JSON.stringify([...reportedNpmKeys].sort()) !== JSON.stringify(expectedNpmKeys)) {
  fail("npm license report package set does not exactly match package-lock.json");
}
for (const pkg of lockedRustPackages) {
  const metadata = rustCoverage.get(packageKey(pkg));
  if (!metadata) fail(`Rust license report does not cover ${packageKey(pkg)}`);
  else if (!metadata.license) fail(`Rust license report lacks a license for ${packageKey(pkg)}`);
}
for (const pkg of lockedNpmPackages) {
  const metadata = npmCoverage.get(packageKey(pkg));
  if (!metadata) fail(`npm license report does not cover ${packageKey(pkg)}`);
  else {
    if (!metadata.license) {
      fail(`npm license report lacks a license for ${packageKey(pkg)}`);
    }
    if (metadata.resolved !== pkg.resolved) {
      fail(`npm license report resolved URL differs from package-lock.json for ${packageKey(pkg)}`);
    }
    if (metadata.integrity !== pkg.integrity) {
      fail(`npm license report integrity differs from package-lock.json for ${packageKey(pkg)}`);
    }
  }
}
requireText("THIRD_PARTY_NOTICES.md", [
  ["Rust generated metadata", /docs\/licenses\/rust-dependencies\.json/],
  ["npm generated metadata", /docs\/licenses\/npm-dependencies\.json/],
  ["first-party client pack Apache-2.0", /client-quick.*Apache-2\.0/si],
  ["first-party CLI pack Apache-2.0", /cli-quick.*Apache-2\.0/si],
  ["DeepSWE excluded", /DeepSWE.*(?:未包含|不包含|excluded)/si],
  ["metadata-only limitation", /元数据.*不.*完整.*许可文本/si],
]);

assert.equal(extname(join(root, "site", ".nojekyll")), "");

if (errors.length > 0) {
  console.error(`Repository validation failed with ${errors.length} issue(s):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

console.log("Repository validation passed: workflows, versions, site, docs, links, licenses, and exclusions are consistent.");
