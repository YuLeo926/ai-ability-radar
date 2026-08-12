import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";

import { verifyExtractedTree } from "../lib/tree.mjs";
import { createPortableFixture } from "./helpers/zip-fixture.mjs";

async function withExtractedFixture(options, run) {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-tree-"));
  const fixture = createPortableFixture(options);
  try {
    for (const [relativePath, bytes] of fixture.payloads) {
      const path = join(root, ...relativePath.split("/"));
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, bytes);
    }
    return await run({ root, ...fixture });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("verifies the exact extracted file tree and internal checksums", async () => {
  await withExtractedFixture({}, async ({ root, manifest }) => {
    assert.deepEqual(await verifyExtractedTree(root, manifest), {
      fileCount: 3,
      totalBytes: manifest.assets.portable.files.reduce((sum, file) => sum + file.size, 0),
    });
  });
});

test("rejects changed, missing, extra, and linked extracted entries", async () => {
  await withExtractedFixture({}, async ({ root, manifest }) => {
    await writeFile(join(root, "ability-radar-portable", "README.txt"), "changed");
    await assert.rejects(
      verifyExtractedTree(root, manifest),
      (error) => error?.code === "INVALID_EXTRACTED_TREE",
    );
  });
  await withExtractedFixture({}, async ({ root, manifest }) => {
    await unlink(join(root, "ability-radar-portable", "README.txt"));
    await assert.rejects(
      verifyExtractedTree(root, manifest),
      (error) => error?.code === "INVALID_EXTRACTED_TREE",
    );
  });
  await withExtractedFixture({}, async ({ root, manifest }) => {
    await writeFile(join(root, "ability-radar-portable", "extra.txt"), "extra");
    await assert.rejects(
      verifyExtractedTree(root, manifest),
      (error) => error?.code === "INVALID_EXTRACTED_TREE",
    );
  });
  await withExtractedFixture({}, async ({ root, manifest }) => {
    const outside = join(root, "outside");
    await mkdir(outside);
    await symlink(outside, join(root, "ability-radar-portable", "linked"), "junction");
    await assert.rejects(
      verifyExtractedTree(root, manifest),
      (error) => error?.code === "INVALID_EXTRACTED_TREE",
    );
  });
});

test("rejects an internally inconsistent checksum file even when its outer hash matches", async () => {
  await withExtractedFixture(
    { checksumText: `${"0".repeat(64)}  README.txt\n${"1".repeat(64)}  ability-radar.exe\n` },
    async ({ root, manifest }) => {
      await assert.rejects(
        verifyExtractedTree(root, manifest),
        (error) => error?.code === "INVALID_EXTRACTED_TREE",
      );
    },
  );
});
