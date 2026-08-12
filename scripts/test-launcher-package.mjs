import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { Readable } from "node:stream";

import { createPortableFixture } from "../packages/launcher/tests/helpers/zip-fixture.mjs";

export const EXPECTED_LAUNCHER_FILES = Object.freeze([
  "LICENSE",
  "README.md",
  "bin/ai-ability-radar.mjs",
  "extract.ps1",
  "lib/archive.mjs",
  "lib/cache.mjs",
  "lib/cli.mjs",
  "lib/download.mjs",
  "lib/errors.mjs",
  "lib/launch.mjs",
  "lib/lock.mjs",
  "lib/manifest.mjs",
  "lib/paths.mjs",
  "lib/run.mjs",
  "lib/runtime.mjs",
  "lib/tree.mjs",
  "package.json",
  "release-manifest.json",
]);

class PackageAuditError extends Error {
  constructor(detail, cause) {
    super(
      `npm 启动器包审计失败：${detail}。`,
      cause === undefined ? undefined : { cause },
    );
    this.name = "PackageAuditError";
    this.code = "PACKAGE_AUDIT_FAILED";
  }
}

function byteSort(left, right) {
  return Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));
}

function auditPackedFileList(paths) {
  if (!Array.isArray(paths) || paths.some((path) => typeof path !== "string")) {
    throw new PackageAuditError("npm pack 文件列表格式无效");
  }
  const actual = [...paths].sort(byteSort);
  const expected = [...EXPECTED_LAUNCHER_FILES].sort(byteSort);
  if (
    actual.length !== expected.length ||
    actual.some((path, index) => path !== expected[index])
  ) {
    throw new PackageAuditError("tarball 文件集合不符合精确白名单");
  }
  return Object.freeze(actual);
}

async function locateNpmCli() {
  const candidates = [
    process.env.npm_execpath,
    join(dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js"),
  ].filter((value) => typeof value === "string" && value.length > 0);
  for (const candidate of candidates) {
    try {
      const info = await lstat(candidate);
      if (info.isFile() && !info.isSymbolicLink()) return candidate;
    } catch (error) {
      if (error?.code !== "ENOENT") {
        throw new PackageAuditError("无法检查 npm CLI", error);
      }
    }
  }
  throw new PackageAuditError("找不到当前 Node.js 配套的 npm CLI");
}

function runProcess(executable, args, { cwd, env = process.env } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      cwd,
      env,
      shell: false,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    const maximumOutputBytes = 4 * 1024 * 1024;
    child.stdout.on("data", (value) => {
      const chunk = Buffer.from(value);
      stdoutBytes += chunk.length;
      if (stdoutBytes > maximumOutputBytes) child.kill();
      else stdout.push(chunk);
    });
    child.stderr.on("data", (value) => {
      const chunk = Buffer.from(value);
      stderrBytes += chunk.length;
      if (stderrBytes > maximumOutputBytes) child.kill();
      else stderr.push(chunk);
    });
    child.once("error", reject);
    child.once("exit", (status, signal) => {
      resolve({
        status,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      });
    });
  });
}

function requireSuccess(result, detail) {
  if (result.status !== 0 || result.signal !== null) {
    throw new PackageAuditError(detail);
  }
  return result;
}

async function hashFile(path) {
  const bytes = await readFile(path);
  return createHash("sha256").update(bytes).digest("hex");
}

async function collectPlainFiles(root, directory = root, prefix = "") {
  const names = await readdir(directory);
  names.sort(byteSort);
  const files = [];
  for (const name of names) {
    const path = join(directory, name);
    const relativePath = prefix ? `${prefix}/${name}` : name;
    const info = await lstat(path, { bigint: true });
    if (info.isSymbolicLink()) {
      throw new PackageAuditError("已安装包内部包含符号链接");
    }
    if (info.isDirectory()) {
      files.push(...await collectPlainFiles(root, path, relativePath));
    } else if (info.isFile() && info.nlink === 1n) {
      files.push(relativePath);
    } else {
      throw new PackageAuditError("已安装包内部包含不支持的条目类型");
    }
  }
  return files;
}

async function compareInstalledFiles(repositoryRoot, installedPackage, paths) {
  const sourcePackage = join(repositoryRoot, "packages", "launcher");
  for (const path of paths) {
    const source = await readFile(join(sourcePackage, ...path.split("/")));
    const installed = await readFile(join(installedPackage, ...path.split("/")));
    if (Buffer.compare(source, installed) !== 0) {
      throw new PackageAuditError(`已安装文件与工作区打包输入不一致：${path}`);
    }
  }
}

function validateInstalledManifest(value) {
  if (
    value?.name !== "ai-ability-radar" ||
    value?.version !== "0.2.2" ||
    value?.private === true ||
    value?.bin?.["ai-ability-radar"] !== "bin/ai-ability-radar.mjs"
  ) {
    throw new PackageAuditError("已安装 package.json 的身份不正确");
  }
  for (const forbidden of [
    "scripts",
    "dependencies",
    "optionalDependencies",
    "peerDependencies",
    "devDependencies",
    "bundledDependencies",
  ]) {
    if (Object.hasOwn(value, forbidden)) {
      throw new PackageAuditError(`已安装 package.json 不得包含 ${forbidden}`);
    }
  }
}

function fakeResponse(bytes) {
  const response = Readable.from([bytes]);
  response.statusCode = 200;
  response.headers = { "content-length": String(bytes.length) };
  response.setTimeout = () => response;
  return response;
}

async function exerciseInstalledLauncher({ installedPackage, localAppData }) {
  const fixture = createPortableFixture();
  const identity = {
    version: "0.2.2",
    repository: "YuLeo926/ai-ability-radar",
    tag: "v0.2.2",
    portableFileName: "ability-radar_0.2.2_windows-x64-portable.zip",
    checksumsFileName: "SHA256SUMS.txt",
  };
  const checksums = Buffer.from(
    `${fixture.manifest.assets.portable.sha256}  ${identity.portableFileName}\n`,
  );
  const requests = { checksums: 0, portable: 0 };
  const launches = { count: 0 };
  const downloadModule = await import(
    pathToFileURL(join(installedPackage, "lib", "download.mjs")).href
  );
  const runModule = await import(
    pathToFileURL(join(installedPackage, "lib", "run.mjs")).href
  );
  const pathsModule = await import(
    pathToFileURL(join(installedPackage, "lib", "paths.mjs")).href
  );
  const transport = async (url) => {
    if (url.pathname.endsWith(`/${identity.checksumsFileName}`)) {
      requests.checksums += 1;
      return fakeResponse(checksums);
    }
    if (url.pathname.endsWith(`/${identity.portableFileName}`)) {
      requests.portable += 1;
      return fakeResponse(fixture.archive);
    }
    throw new Error("unexpected fake HTTPS request");
  };
  const dependencies = {
    downloadChecksums({ identity: releaseIdentity, destination }) {
      return downloadModule.downloadReleaseAssetForTest({
        identity: releaseIdentity,
        kind: "checksums",
        destination,
        transport,
      });
    },
    downloadPortable({ identity: releaseIdentity, portable, destination }) {
      return downloadModule.downloadReleaseAssetForTest({
        identity: releaseIdentity,
        kind: "portable",
        expectedSize: portable.size,
        expectedSha256: portable.sha256,
        destination,
        transport,
      });
    },
    async launchApplication() {
      launches.count += 1;
    },
  };
  const options = {
    command: { kind: "launch" },
    version: "0.2.2",
    manifest: fixture.manifest,
    localAppData,
    runtime: { platform: "win32", arch: "x64", nodeVersion: "22.22.2" },
  };

  const sources = [];
  sources.push((await runModule.runLauncherCommandForTest(options, dependencies)).source);
  const offline = {
    ...dependencies,
    async downloadChecksums() { throw new Error("offline package test used the network"); },
    async downloadPortable() { throw new Error("offline package test used the network"); },
  };
  sources.push((await runModule.runLauncherCommandForTest(options, offline)).source);
  const cachePaths = pathsModule.resolveCachePaths({
    localAppData,
    version: "0.2.2",
  });
  const readme = join(
    cachePaths.versionDirectory,
    "app",
    "ability-radar-portable",
    "README.txt",
  );
  await writeFile(readme, "tampered package test");
  sources.push((await runModule.runLauncherCommandForTest(options, offline)).source);
  await writeFile(readme, "tampered package test again");
  await writeFile(
    join(cachePaths.versionDirectory, identity.portableFileName),
    "broken zip",
  );
  sources.push((await runModule.runLauncherCommandForTest(options, dependencies)).source);
  const cleared = await runModule.runLauncherCommandForTest(
    { ...options, command: { kind: "clear-cache" }, manifest: undefined },
    dependencies,
  );
  try {
    await lstat(cachePaths.cacheRoot);
    throw new PackageAuditError("--clear-cache 未删除已拥有的测试缓存");
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  return {
    sources,
    networkRequests: requests,
    launches: launches.count,
    cacheCleared: cleared.removed,
  };
}

async function testLauncherPackage({ repositoryRoot }) {
  if (typeof repositoryRoot !== "string" || !isAbsolute(repositoryRoot)) {
    throw new PackageAuditError("仓库根目录无效");
  }
  const root = resolve(repositoryRoot);
  const temporaryRoot = await mkdtemp(join(tmpdir(), "ability-radar-package-"));
  try {
    const packDirectory = join(temporaryRoot, "pack");
    const installDirectory = join(temporaryRoot, "install");
    const npmCache = join(temporaryRoot, "npm-cache");
    const localAppData = join(temporaryRoot, "local-app-data");
    await Promise.all([
      mkdir(packDirectory),
      mkdir(installDirectory),
      mkdir(npmCache),
      mkdir(localAppData),
    ]);
    await writeFile(
      join(installDirectory, "package.json"),
      `${JSON.stringify({ name: "launcher-package-audit", private: true })}\n`,
    );
    const npmCli = await locateNpmCli();
    const npmEnvironment = {
      ...process.env,
      npm_config_cache: npmCache,
      npm_config_offline: "true",
      npm_config_audit: "false",
      npm_config_fund: "false",
      npm_config_ignore_scripts: "true",
      npm_config_update_notifier: "false",
      npm_config_registry: "https://invalid.invalid/",
    };
    const packed = requireSuccess(
      await runProcess(
        process.execPath,
        [
          npmCli,
          "pack",
          "--workspace",
          "packages/launcher",
          "--json",
          "--pack-destination",
          packDirectory,
        ],
        { cwd: root, env: npmEnvironment },
      ),
      "npm pack 执行失败",
    );
    let packResults;
    try {
      packResults = JSON.parse(packed.stdout);
    } catch (error) {
      throw new PackageAuditError("npm pack JSON 无法解析", error);
    }
    if (!Array.isArray(packResults) || packResults.length !== 1) {
      throw new PackageAuditError("npm pack 必须只产生一个包");
    }
    const pack = packResults[0];
    if (
      pack.name !== "ai-ability-radar" ||
      pack.version !== "0.2.2" ||
      !Array.isArray(pack.files)
    ) {
      throw new PackageAuditError("npm pack 元数据身份无效");
    }
    const files = auditPackedFileList(pack.files.map(({ path }) => path));
    const tarballPath = join(packDirectory, basename(pack.filename));
    const tarballInfo = await lstat(tarballPath, { bigint: true });
    if (
      !tarballInfo.isFile() ||
      tarballInfo.isSymbolicLink() ||
      tarballInfo.nlink !== 1n ||
      Number(tarballInfo.size) !== pack.size
    ) {
      throw new PackageAuditError("实际 tarball 与 npm pack 元数据不一致");
    }
    const tarballSha256 = await hashFile(tarballPath);

    requireSuccess(
      await runProcess(
        process.execPath,
        [
          npmCli,
          "install",
          "--ignore-scripts",
          "--no-audit",
          "--no-fund",
          "--offline",
          "--package-lock=false",
          tarballPath,
        ],
        { cwd: installDirectory, env: npmEnvironment },
      ),
      "tarball 离线安装失败",
    );
    const installedPackage = join(
      installDirectory,
      "node_modules",
      "ai-ability-radar",
    );
    const installedFiles = auditPackedFileList(await collectPlainFiles(installedPackage));
    await compareInstalledFiles(root, installedPackage, installedFiles);
    validateInstalledManifest(
      JSON.parse(await readFile(join(installedPackage, "package.json"), "utf8")),
    );
    const installedBin = join(installedPackage, "bin", "ai-ability-radar.mjs");
    const help = await runProcess(process.execPath, [installedBin, "--help"], {
      cwd: installDirectory,
      env: npmEnvironment,
    });
    const version = await runProcess(process.execPath, [installedBin, "--version"], {
      cwd: installDirectory,
      env: npmEnvironment,
    });
    const unknown = await runProcess(process.execPath, [installedBin, "--unknown"], {
      cwd: installDirectory,
      env: npmEnvironment,
    });
    if (
      help.status !== 0 ||
      !help.stdout.includes("AI 能力雷达 npm 启动器") ||
      version.status !== 0 ||
      version.stdout !== "0.2.2\n" ||
      unknown.status !== 2 ||
      !unknown.stderr.includes("不支持的参数") ||
      unknown.stderr.includes(installedPackage)
    ) {
      throw new PackageAuditError("安装后 bin 行为不符合命令契约");
    }
    const exercised = await exerciseInstalledLauncher({
      installedPackage,
      localAppData,
    });
    return Object.freeze({
      packageName: pack.name,
      version: pack.version,
      files,
      tarballSha256,
      tarballBytes: pack.size,
      helpExitCode: help.status,
      versionExitCode: version.status,
      unknownExitCode: unknown.status,
      ...exercised,
    });
  } catch (error) {
    if (error instanceof PackageAuditError) throw error;
    throw new PackageAuditError("tarball 验证过程中发生错误", error);
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

export function auditPackedFileListForTest(paths) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw new PackageAuditError("测试审计入口不可用于生产运行");
  }
  return auditPackedFileList(paths);
}

export function testLauncherPackageForTest(options) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw new PackageAuditError("测试打包入口不可用于生产运行");
  }
  return testLauncherPackage(options);
}
