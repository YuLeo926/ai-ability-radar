import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { stagePortable } from "./package-portable.mjs";

test("stages one rooted no-install package with deterministic checksums", async () => {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-portable-"));
  try {
    const repoRoot = join(root, "repo");
    const targetDir = join(root, "target", "release");
    const bundleDir = join(targetDir, "bundle", "portable");
    await mkdir(join(repoRoot, "packaging", "windows-portable"), {
      recursive: true,
    });
    await mkdir(join(targetDir, "benchmark-packs", "client-quick-v1"), {
      recursive: true,
    });
    await mkdir(join(targetDir, "benchmark-packs", "cli-quick-v1"), {
      recursive: true,
    });
    await writeFile(join(targetDir, "ability-radar.exe"), "fake-exe");
    await writeFile(
      join(targetDir, "benchmark-packs", "registry.json"),
      '{"schema_version":1,"packs":[]}\n',
    );
    await writeFile(
      join(targetDir, "benchmark-packs", "client-quick-v1", "manifest.json"),
      "{}\n",
    );
    await writeFile(
      join(targetDir, "benchmark-packs", "cli-quick-v1", "manifest.json"),
      "{}\n",
    );
    await writeFile(
      join(repoRoot, "packaging", "windows-portable", "README.txt"),
      "no install\n",
    );

    const result = await stagePortable({
      repoRoot,
      targetDir,
      bundleDir,
      version: "0.2.1",
    });

    assert.equal(
      result.archivePath,
      join(bundleDir, "ability-radar_0.2.1_windows-x64-portable.zip"),
    );
    const checksums = await readFile(
      join(result.stageRoot, "SHA256SUMS.txt"),
      "utf8",
    );
    assert.match(checksums, /  ability-radar\.exe$/m);
    assert.match(checksums, /  benchmark-packs\/registry\.json$/m);
    assert.doesNotMatch(checksums, /SHA256SUMS\.txt/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("refuses an output directory outside the selected target tree", async () => {
  await assert.rejects(
    stagePortable({
      repoRoot: "C:\\repo",
      targetDir: "C:\\repo\\target\\release",
      bundleDir: "C:\\outside",
      version: "0.2.1",
    }),
    /inside target directory/,
  );
});
