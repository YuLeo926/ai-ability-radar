import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { generateLauncherManifestForTest } from "./generate-launcher-manifest.mjs";
import {
  createPortableFixture,
  sha256,
} from "../packages/launcher/tests/helpers/zip-fixture.mjs";

async function withAssets(options, run) {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-manifest-"));
  const assetsDir = join(root, "assets");
  const outputPath = join(root, "release-manifest.json");
  const fixture = createPortableFixture(options);
  try {
    await mkdir(assetsDir);
    await writeFile(
      join(assetsDir, fixture.manifest.assets.portable.file_name),
      fixture.archive,
    );
    await writeFile(
      join(assetsDir, "SHA256SUMS.txt"),
      options?.outerChecksums ??
        `${fixture.manifest.assets.portable.sha256}  ${fixture.manifest.assets.portable.file_name}\r\n`,
    );
    return await run({ root, assetsDir, outputPath, fixture });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("generates a deterministic strict launcher manifest from local release assets", async () => {
  await withAssets({}, async ({ assetsDir, outputPath, fixture }) => {
    const first = await generateLauncherManifestForTest({ assetsDir, outputPath });
    const firstBytes = await readFile(outputPath);
    const second = await generateLauncherManifestForTest({ assetsDir, outputPath });
    assert.deepEqual(await readFile(outputPath), firstBytes);
    assert.deepEqual(second, first);
    assert.equal(first.launcher_version, "0.2.2");
    assert.equal(first.assets.portable.size, fixture.archive.length);
    assert.equal(first.assets.portable.sha256, sha256(fixture.archive));
    assert.deepEqual(
      first.assets.portable.files.map(({ path }) => path),
      [...fixture.payloads.keys()].sort((left, right) =>
        Buffer.compare(Buffer.from(left), Buffer.from(right))),
    );
    assert.equal(firstBytes.at(-1), 0x0a);
    assert.equal(firstBytes.includes(0x0d), false);
  });
});

test("requires exactly the reviewed local asset files", async () => {
  await withAssets({}, async ({ assetsDir, outputPath }) => {
    await writeFile(join(assetsDir, "extra.txt"), "extra");
    await assert.rejects(
      generateLauncherManifestForTest({ assetsDir, outputPath }),
      (error) => error?.code === "MANIFEST_GENERATION_FAILED",
    );
  });
  await withAssets({}, async ({ assetsDir, outputPath }) => {
    await rm(join(assetsDir, "SHA256SUMS.txt"));
    await assert.rejects(
      generateLauncherManifestForTest({ assetsDir, outputPath }),
      (error) => error?.code === "MANIFEST_GENERATION_FAILED",
    );
  });
});

test("rejects an outer checksum mismatch without replacing an existing output", async () => {
  await withAssets(
    { outerChecksums: `${"0".repeat(64)}  ability-radar_0.2.2_windows-x64-portable.zip\n` },
    async ({ assetsDir, outputPath }) => {
      await writeFile(outputPath, "preserve\n");
      await assert.rejects(
        generateLauncherManifestForTest({ assetsDir, outputPath }),
        (error) => error?.code === "MANIFEST_GENERATION_FAILED",
      );
      assert.equal(await readFile(outputPath, "utf8"), "preserve\n");
    },
  );
});

test("atomically replaces a valid older manifest when reviewed assets change", async () => {
  await withAssets({}, async ({ assetsDir, outputPath, fixture }) => {
    await generateLauncherManifestForTest({ assetsDir, outputPath });
    const first = await readFile(outputPath);
    const changed = createPortableFixture({ readmeText: "changed reviewed fixture\n" });
    await writeFile(
      join(assetsDir, fixture.manifest.assets.portable.file_name),
      changed.archive,
    );
    await writeFile(
      join(assetsDir, "SHA256SUMS.txt"),
      `${changed.manifest.assets.portable.sha256}  ${changed.manifest.assets.portable.file_name}\n`,
    );
    await generateLauncherManifestForTest({ assetsDir, outputPath });
    const second = await readFile(outputPath);
    assert.notDeepEqual(second, first);
    assert.equal(
      JSON.parse(second).assets.portable.sha256,
      changed.manifest.assets.portable.sha256,
    );
  });
});

test("rejects an internally forged checksum file after extraction", async () => {
  const checksumText = `${"0".repeat(64)}  README.txt\n${"1".repeat(64)}  ability-radar.exe\n`;
  await withAssets({ checksumText }, async ({ assetsDir, outputPath }) => {
    await assert.rejects(
      generateLauncherManifestForTest({ assetsDir, outputPath }),
      (error) => error?.code === "MANIFEST_GENERATION_FAILED",
    );
  });
});

test("rejects relative inputs and command-line version overrides", async () => {
  await assert.rejects(
    generateLauncherManifestForTest({
      assetsDir: "relative-assets",
      outputPath: "relative-output.json",
    }),
    (error) => error?.code === "MANIFEST_GENERATION_FAILED",
  );
});
