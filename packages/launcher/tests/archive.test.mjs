import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  enumeratePortableArchive,
  inspectPortableArchive,
} from "../lib/archive.mjs";
import {
  createPortableFixture,
  createStoredZip,
} from "./helpers/zip-fixture.mjs";

async function withArchive(bytes, run) {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-archive-"));
  const archivePath = join(root, "portable.zip");
  try {
    await writeFile(archivePath, bytes);
    return await run(archivePath);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("accepts the exact classic portable ZIP structure", async () => {
  const fixture = createPortableFixture();
  await withArchive(fixture.archive, async (archivePath) => {
    const inspected = await inspectPortableArchive(archivePath, fixture.manifest);
    assert.deepEqual(
      inspected.files.map(({ path }) => path),
      fixture.manifest.assets.portable.files.map(({ path }) => path),
    );
    assert.equal(inspected.totalUncompressedBytes > 0, true);
    const enumerated = await enumeratePortableArchive(archivePath);
    assert.deepEqual(enumerated.files, inspected.files);
  });
});

test("rejects unsafe names, duplicates, links, and unexpected members before extraction", async () => {
  const unsafeArchives = [
    createStoredZip([{ name: "ability-radar-portable/../escape.txt", data: "x" }]),
    createStoredZip([{ name: "ability-radar-portable/file.txt:secret", data: "x" }]),
    createStoredZip([{ name: "ability-radar-portable/CON.txt", data: "x" }]),
    createStoredZip([{ name: "ability-radar-portable/trailing./file.txt", data: "x" }]),
    createStoredZip([
      { name: "ability-radar-portable/file.txt", data: "x" },
      { name: "ability-radar-portable/FILE.TXT", data: "y" },
    ]),
    createStoredZip([{
      name: "ability-radar-portable/link",
      data: "target",
      versionMadeBy: (3 << 8) | 20,
      externalAttributes: 0xa000 << 16,
    }]),
  ];
  for (const archive of unsafeArchives) {
    await withArchive(archive, async (archivePath) => {
      await assert.rejects(
        enumeratePortableArchive(archivePath),
        (error) => error?.code === "INVALID_ARCHIVE",
      );
    });
  }

  const fixture = createPortableFixture();
  const extra = createStoredZip([
    { name: "ability-radar-portable/" },
    ...[...fixture.payloads].map(([name, data]) => ({ name, data })),
    { name: "ability-radar-portable/unexpected.txt", data: "x" },
  ]);
  await withArchive(extra, async (archivePath) => {
    await assert.rejects(
      inspectPortableArchive(archivePath, fixture.manifest),
      (error) => error?.code === "INVALID_ARCHIVE",
    );
  });
});

test("rejects unsupported ZIP encodings and container structures", async () => {
  const invalid = [
    createStoredZip([{ name: "ability-radar-portable/file", flags: 1 }]),
    createStoredZip([{ name: "ability-radar-portable/file", flags: 8 }]),
    createStoredZip([{ name: "ability-radar-portable/file", method: 99 }]),
    createStoredZip([{ nameBytes: Buffer.from([0xff]), name: "ignored" }]),
    createStoredZip([{ name: "ability-radar-portable/file" }], { diskNumber: 1 }),
    createStoredZip([{ name: "ability-radar-portable/file" }], { comment: Buffer.from("comment") }),
  ];
  for (const archive of invalid) {
    await withArchive(archive, async (archivePath) => {
      await assert.rejects(
        enumeratePortableArchive(archivePath),
        (error) => error?.code === "INVALID_ARCHIVE",
      );
    });
  }
});

test("rejects a manifest size or file membership mismatch", async () => {
  const fixture = createPortableFixture();
  await withArchive(fixture.archive, async (archivePath) => {
    const wrongSize = structuredClone(fixture.manifest);
    wrongSize.assets.portable.files[0].size += 1;
    await assert.rejects(
      inspectPortableArchive(archivePath, wrongSize),
      (error) => error?.code === "INVALID_ARCHIVE",
    );

    const missing = structuredClone(fixture.manifest);
    missing.assets.portable.files.pop();
    await assert.rejects(
      inspectPortableArchive(archivePath, missing),
      (error) => error?.code === "INVALID_ARCHIVE",
    );
  });
});
