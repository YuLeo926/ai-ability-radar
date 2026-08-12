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

import {
  extractPortableArchive,
  inspectPortableArchive,
} from "../lib/archive.mjs";
import { verifyExtractedTree } from "../lib/tree.mjs";
import { createPortableFixture } from "./helpers/zip-fixture.mjs";

test("extracts only into an existing empty directory with the fixed script", async () => {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-extract-"));
  try {
    const fixture = createPortableFixture();
    const archivePath = join(root, "portable.zip");
    const destination = join(root, "extracted");
    await writeFile(archivePath, fixture.archive);
    await mkdir(destination);
    await inspectPortableArchive(archivePath, fixture.manifest);
    await extractPortableArchive({ archivePath, destination });
    await verifyExtractedTree(destination, fixture.manifest);
    assert.deepEqual(
      await readFile(join(destination, "ability-radar-portable", "README.txt")),
      fixture.payloads.get("ability-radar-portable/README.txt"),
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("refuses a non-empty extraction destination", async () => {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-extract-"));
  try {
    const fixture = createPortableFixture();
    const archivePath = join(root, "portable.zip");
    const destination = join(root, "extracted");
    await writeFile(archivePath, fixture.archive);
    await mkdir(destination);
    await writeFile(join(destination, "owner.txt"), "preserve");
    await assert.rejects(
      extractPortableArchive({ archivePath, destination }),
      (error) => error?.code === "EXTRACTION_FAILED",
    );
    assert.equal(await readFile(join(destination, "owner.txt"), "utf8"), "preserve");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
