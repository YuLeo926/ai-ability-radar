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

function cargoPackages(source) {
  return [...source.matchAll(/\[\[package\]\]\r?\n([\s\S]*?)(?=\r?\n\[\[package\]\]|\s*$)/g)]
    .map((match) => ({
      name: match[1].match(/^name = "([^"]+)"$/m)?.[1],
      version: match[1].match(/^version = "([^"]+)"$/m)?.[1],
      source: match[1].match(/^source = "([^"]+)"$/m)?.[1],
    }))
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

function sha256(path) {
  const canonical = readFileSync(join(root, path), "utf8")
    .replace(/\r\n?/g, "\n");
  return createHash("sha256").update(canonical, "utf8").digest("hex");
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
  "site/index.html",
  "site/.nojekyll",
];
for (const path of requiredFiles) read(path);

const expectedVersion = "0.2.0";
const rootPackage = json("package.json");
const desktopPackage = json("apps/desktop/package.json");
const tauriConfig = json("apps/desktop/src-tauri/tauri.conf.json");
const npmLock = json("package-lock.json");
if (rootPackage.scripts?.tauri !== "npm run tauri --workspace apps/desktop --") {
  fail("package.json tauri script must preserve the argument separator for workspace forwarding");
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
const workflows = new Map();
for (const path of workflowPaths) {
  const source = read(path);
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
}

function requireCommand(path, job, label, pattern) {
  if (!hasRunCommand(job, pattern)) {
    fail(`${path} is missing ${label}`);
  }
}

const ciPath = ".github/workflows/ci.yml";
const ciWorkflow = workflows.get(ciPath)?.workflow;
const ciJob = ciWorkflow?.jobs.get("test");
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
  ["frontend tests", /(?:^|\n)\s*npm test\s*(?:$|\n)/],
  ["frontend build", /(?:^|\n)\s*npm run build\s*(?:$|\n)/],
  ["debug NSIS build", /(?:^|\n)\s*npm run tauri -- build --debug --bundles nsis\s*(?:$|\n)/],
];
for (const [label, pattern] of ciCommands) requireCommand(ciPath, ciJob, label, pattern);
const ciArtifact = actionSteps(ciWorkflow, "actions/upload-artifact")[0];
if (
  ciArtifact?.with.path !==
  "target/debug/bundle/nsis/ability-radar_0.2.0_x64-setup.exe"
) {
  fail(`${ciPath} must upload the exact debug NSIS installer`);
}
if (ciArtifact?.with["if-no-files-found"] !== "error") {
  fail(`${ciPath} artifact upload must fail when the installer is missing`);
}

const releasePath = ".github/workflows/release.yml";
const releaseSource = workflows.get(releasePath)?.source ?? "";
const releaseWorkflow = workflows.get(releasePath)?.workflow;
const releaseJob = releaseWorkflow?.jobs.get("release");
if (!exactPermissions(releaseJob, { contents: "write" })) {
  fail(`${releasePath} release job permissions must be exactly contents: write`);
}
if (releaseJob?.env.RELEASE_TAG !== "${{ github.ref_name }}") {
  fail(`${releasePath} must import github.ref_name through RELEASE_TAG`);
}
const releaseRust = actionSteps(releaseWorkflow, "dtolnay/rust-toolchain")[0];
if (releaseRust?.with.toolchain !== "stable") {
  fail(`${releasePath} Rust toolchain action must explicitly select stable`);
}
const verifyTag = releaseJob?.steps.find(({ name }) => name === "Verify release tag");
if (!verifyTag?.run.includes("$env:RELEASE_TAG")) {
  fail(`${releasePath} release tag gate must use the RELEASE_TAG environment variable`);
}
if (!/\^v\(0\|\[1-9\]\\d\*\)\\\.\(0\|\[1-9\]\\d\*\)\\\.\(0\|\[1-9\]\\d\*\)\$/.test(verifyTag?.run ?? "")) {
  fail(`${releasePath} must enforce a strict semantic-version release tag`);
}
if (!/"v\$\(\$config\.version\)"\s*-cne\s*\$tag/.test(verifyTag?.run ?? "")) {
  fail(`${releasePath} must compare the release tag to the exact app version`);
}
const tauriRelease = actionSteps(releaseWorkflow, "tauri-apps/tauri-action")[0];
const expectedReleaseInputs = {
  tagName: "${{ env.RELEASE_TAG }}",
  releaseDraft: "true",
  prerelease: "true",
  uploadUpdaterJson: "false",
  uploadUpdaterSignatures: "false",
};
for (const [key, value] of Object.entries(expectedReleaseInputs)) {
  if (tauriRelease?.with[key] !== value) {
    fail(`${releasePath} tauri release input ${key} must be ${value}`);
  }
}
if (!/未签名/.test(tauriRelease?.with.releaseBody ?? "")) {
  fail(`${releasePath} release body must warn that the installer is unsigned`);
}
requireCommand(
  releasePath,
  releaseJob,
  "SHA-256 checksum generation",
  /Set-Content -LiteralPath SHA256SUMS\.txt/,
);
requireCommand(
  releasePath,
  releaseJob,
  "checksum upload",
  /gh release upload \$env:RELEASE_TAG SHA256SUMS\.txt --clobber/,
);
if ([...releaseSource.matchAll(/\$\{\{\s*github\.ref_name\s*\}\}/g)].length !== 1) {
  fail(".github/workflows/release.yml must import github.ref_name exactly once through RELEASE_TAG");
}

const pagesPath = ".github/workflows/pages.yml";
const pagesWorkflow = workflows.get(pagesPath)?.workflow;
const pagesBuild = pagesWorkflow?.jobs.get("build");
const pagesDeploy = pagesWorkflow?.jobs.get("deploy");
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
  ["v0.2.0 prerelease link", /\/releases\/tag\/v0\.2\.0/],
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
