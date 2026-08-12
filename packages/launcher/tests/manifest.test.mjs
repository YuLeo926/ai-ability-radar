import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  deriveReleaseIdentity,
  parseReleaseManifest,
  validateReleaseManifest,
} from "../lib/manifest.mjs";

const fixtureUrl = new URL("./fixtures/release-manifest.valid.json", import.meta.url);
const validText = await readFile(fixtureUrl, "utf8");
const validValue = JSON.parse(validText);

function clone(value = validValue) {
  return structuredClone(value);
}

test("derives one exact GitHub release identity from a stable package version", () => {
  assert.deepEqual(deriveReleaseIdentity("0.2.2"), {
    version: "0.2.2",
    repository: "YuLeo926/ai-ability-radar",
    tag: "v0.2.2",
    portableFileName: "ability-radar_0.2.2_windows-x64-portable.zip",
    checksumsFileName: "SHA256SUMS.txt",
  });

  for (const version of ["v0.2.2", "0.2.2-beta.1", "0.2.2+build", "00.2.2", "0.2", "../0.2.2", ""] ) {
    assert.throws(
      () => deriveReleaseIdentity(version),
      (error) => error?.code === "INVALID_VERSION",
      version,
    );
  }
});

test("parses and freezes the exact release manifest", () => {
  const manifest = parseReleaseManifest(validText, { packageVersion: "0.2.2" });
  assert.equal(manifest.tag, "v0.2.2");
  assert.equal(manifest.assets.portable.files.length, 3);
  assert.equal(Object.isFrozen(manifest), true);
  assert.equal(Object.isFrozen(manifest.assets.portable.files), true);
  assert.throws(() => manifest.assets.portable.files.push({}), TypeError);
});

test("rejects invalid JSON, byte-order marks, and duplicate object keys", () => {
  for (const text of [
    "{",
    `\uFEFF${validText}`,
    validText.replace(
      '"launcher_version": "0.2.2",',
      '"launcher_version": "0.2.2",\n  "launcher_version": "0.2.2",',
    ),
    '{"outer":{"same":1,"same":2}}',
  ]) {
    assert.throws(
      () => parseReleaseManifest(text, { packageVersion: "0.2.2" }),
      (error) => error?.code === "INVALID_MANIFEST",
    );
  }
});

test("rejects unknown or missing fields at every schema level", () => {
  const mutations = [
    (value) => { value.extra = true; },
    (value) => { delete value.repository; },
    (value) => { value.assets.extra = true; },
    (value) => { value.assets.portable.extra = true; },
    (value) => { delete value.assets.portable.sha256; },
    (value) => { value.assets.portable.files[0].extra = true; },
    (value) => { value.assets.checksums.extra = true; },
  ];
  for (const mutate of mutations) {
    const value = clone();
    mutate(value);
    assert.throws(
      () => validateReleaseManifest(value, { packageVersion: "0.2.2" }),
      (error) => error?.code === "INVALID_MANIFEST",
    );
  }
});

test("rejects version, repository, tag, and asset identity drift", () => {
  const mutations = [
    (value) => { value.repository = "someone/example"; },
    (value) => { value.launcher_version = "0.2.1"; },
    (value) => { value.desktop_version = "0.2.1"; },
    (value) => { value.tag = "v0.2.1"; },
    (value) => { value.assets.portable.file_name = "latest.zip"; },
    (value) => { value.assets.checksums.file_name = "checksums.txt"; },
    (value) => { value.schema_version = "launcher-release-manifest-v2"; },
  ];
  for (const mutate of mutations) {
    const value = clone();
    mutate(value);
    assert.throws(
      () => validateReleaseManifest(value, { packageVersion: "0.2.2" }),
      (error) => error?.code === "VERSION_MISMATCH" || error?.code === "INVALID_MANIFEST",
    );
  }
});

test("rejects unsafe hashes, sizes, paths, duplicates, and missing required files", () => {
  const mutations = [
    (value) => { value.assets.portable.sha256 = "A".repeat(64); },
    (value) => { value.assets.portable.size = 0; },
    (value) => { value.assets.portable.size = 256 * 1024 * 1024 + 1; },
    (value) => { value.assets.portable.files[0].size = -1; },
    (value) => { value.assets.portable.files[0].sha256 = "0".repeat(63); },
    (value) => { value.assets.portable.files[0].path = "../escape.txt"; },
    (value) => { value.assets.portable.files[0].path = "ability-radar-portable\\escape.txt"; },
    (value) => { value.assets.portable.files[0].path = "ability-radar-portable/CON"; },
    (value) => { value.assets.portable.files[0].path = "ability-radar-portable/trailing. "; },
    (value) => { value.assets.portable.files[1].path = value.assets.portable.files[0].path.toLowerCase(); },
    (value) => { value.assets.portable.files.reverse(); },
    (value) => { value.assets.portable.files = value.assets.portable.files.filter(({ path }) => !path.endsWith("ability-radar.exe")); },
    (value) => { value.assets.portable.files = value.assets.portable.files.filter(({ path }) => !path.endsWith("SHA256SUMS.txt")); },
  ];
  for (const mutate of mutations) {
    const value = clone();
    mutate(value);
    assert.throws(
      () => validateReleaseManifest(value, { packageVersion: "0.2.2" }),
      (error) => error?.code === "INVALID_MANIFEST",
    );
  }
});
