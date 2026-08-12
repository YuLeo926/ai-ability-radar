import assert from "node:assert/strict";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  EXPECTED_LAUNCHER_FILES,
  auditPackedFileListForTest,
  testLauncherPackageForTest,
} from "./test-launcher-package.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

test("accepts only the exact public launcher tarball file set", () => {
  assert.deepEqual(
    auditPackedFileListForTest([...EXPECTED_LAUNCHER_FILES]),
    EXPECTED_LAUNCHER_FILES,
  );
  for (const unsafe of [
    "tests/run.test.mjs",
    "ability-radar.exe",
    "portable.zip",
    "private-key.pem",
    "debug.log",
  ]) {
    assert.throws(
      () => auditPackedFileListForTest([...EXPECTED_LAUNCHER_FILES, unsafe]),
      (error) => error?.code === "PACKAGE_AUDIT_FAILED",
      unsafe,
    );
  }
});

test("packs, installs offline, and exercises the installed launcher", async () => {
  const result = await testLauncherPackageForTest({ repositoryRoot: root });
  assert.equal(result.packageName, "ai-ability-radar");
  assert.equal(result.version, "0.2.2");
  assert.deepEqual(result.files, EXPECTED_LAUNCHER_FILES);
  assert.match(result.tarballSha256, /^[a-f0-9]{64}$/u);
  assert.equal(result.helpExitCode, 0);
  assert.equal(result.versionExitCode, 0);
  assert.equal(result.unknownExitCode, 2);
  assert.deepEqual(result.sources, ["downloaded", "cache", "repaired", "downloaded"]);
  assert.deepEqual(result.networkRequests, { checksums: 2, portable: 2 });
  assert.equal(result.launches, 4);
  assert.equal(result.cacheCleared, true);
});
