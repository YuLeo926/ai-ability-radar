import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputDirectory = join(root, "docs", "licenses");
mkdirSync(outputDirectory, { recursive: true });

function bytes(path) {
  return readFileSync(join(root, path));
}

function sha256(path) {
  const canonical = bytes(path).toString("utf8").replace(/\r\n?/g, "\n");
  return createHash("sha256").update(canonical, "utf8").digest("hex");
}

function writeReport(name, report) {
  writeFileSync(
    join(outputDirectory, name),
    `${JSON.stringify(report, null, 2)}\n`,
    "utf8",
  );
}

function packageKey({ name, version }) {
  return `${name}@${version}`;
}

const npmLock = JSON.parse(bytes("package-lock.json"));
const npmPackages = new Map();
for (const [path, metadata] of Object.entries(npmLock.packages ?? {})) {
  const marker = "node_modules/";
  const index = path.lastIndexOf(marker);
  if (index < 0 || !metadata.version) continue;
  const entry = {
    name: path.slice(index + marker.length),
    version: metadata.version,
    license: metadata.license,
    resolved: metadata.resolved,
    integrity: metadata.integrity,
  };
  if (!entry.license) {
    throw new Error(`package-lock.json lacks license metadata for ${packageKey(entry)}`);
  }
  npmPackages.set(packageKey(entry), entry);
}
writeReport("npm-dependencies.json", {
  schemaVersion: 1,
  generatedFrom: "package-lock.json",
  lockfileSha256: sha256("package-lock.json"),
  hashNormalization: "UTF-8 text with CRLF and CR normalized to LF",
  note: "Metadata inventory only; dependency license texts are not bundled here.",
  packages: [...npmPackages.values()].sort((a, b) =>
    packageKey(a).localeCompare(packageKey(b), "en"),
  ),
});

const metadata = spawnSync(
  "cargo",
  ["metadata", "--locked", "--format-version", "1"],
  {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  },
);
if (metadata.status !== 0) {
  throw new Error(`cargo metadata failed:\n${metadata.stderr}`);
}
const cargoMetadata = JSON.parse(metadata.stdout);
const workspaceIds = new Set(cargoMetadata.workspace_members);
const rustPackages = new Map();
for (const pkg of cargoMetadata.packages) {
  if (workspaceIds.has(pkg.id)) continue;
  const entry = {
    name: pkg.name,
    version: pkg.version,
    license: pkg.license,
    source: pkg.source,
    repository: pkg.repository,
  };
  if (!entry.license) {
    throw new Error(`Cargo metadata lacks a license for ${packageKey(entry)}`);
  }
  rustPackages.set(packageKey(entry), entry);
}
writeReport("rust-dependencies.json", {
  schemaVersion: 1,
  generatedFrom: "Cargo.lock",
  lockfileSha256: sha256("Cargo.lock"),
  hashNormalization: "UTF-8 text with CRLF and CR normalized to LF",
  note: "Metadata inventory only; dependency license texts are not bundled here.",
  packages: [...rustPackages.values()].sort((a, b) =>
    packageKey(a).localeCompare(packageKey(b), "en"),
  ),
});

console.log(
  `Wrote deterministic metadata for ${npmPackages.size} npm and ${rustPackages.size} Rust dependency versions.`,
);
