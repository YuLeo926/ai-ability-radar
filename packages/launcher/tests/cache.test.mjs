import assert from "node:assert/strict";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  CACHE_ROOT_MARKER_NAME,
  assertCacheRootOwned,
  clearCacheRoot,
  createVersionStaging,
  ensureCacheRoot,
  publishVersionStaging,
  publishVersionStagingForTest,
  recoverVersionPublication,
} from "../lib/cache.mjs";
import { resolveCachePaths } from "../lib/paths.mjs";

const TOKEN_ONE = "11111111-1111-4111-8111-111111111111";
const TOKEN_TWO = "22222222-2222-4222-8222-222222222222";

async function withFixture(run) {
  const localAppData = await mkdtemp(join(tmpdir(), "ability-radar-cache-"));
  const paths = resolveCachePaths({ localAppData, version: "0.2.2" });
  try {
    return await run({ localAppData, paths });
  } finally {
    await rm(localAppData, { recursive: true, force: true });
  }
}

const fakeLock = Object.freeze({
  async assertOwned() {},
});

async function payload(directory) {
  return readFile(join(directory, "payload.txt"), "utf8");
}

async function validPayload(directory) {
  const value = await payload(directory);
  if (value !== "one" && value !== "two") {
    throw new Error("invalid candidate");
  }
}

test("creates and reopens only a marked cache root", async () => {
  await withFixture(async ({ paths }) => {
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    await assertCacheRootOwned(paths);
    const marker = await readFile(join(paths.cacheRoot, CACHE_ROOT_MARKER_NAME), "utf8");
    assert.match(marker, /ai-ability-radar-launcher/u);
    await ensureCacheRoot(paths, { token: TOKEN_TWO });
  });
});

test("refuses unmarked, tampered, and junctioned cache roots", async () => {
  await withFixture(async ({ paths }) => {
    await mkdir(paths.appRoot);
    await mkdir(paths.cacheRoot);
    await assert.rejects(
      ensureCacheRoot(paths, { token: TOKEN_ONE }),
      (error) => error?.code === "CACHE_OWNERSHIP",
    );
  });

  await withFixture(async ({ paths }) => {
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    await writeFile(join(paths.cacheRoot, CACHE_ROOT_MARKER_NAME), "{}\n");
    await assert.rejects(
      assertCacheRootOwned(paths),
      (error) => error?.code === "CACHE_OWNERSHIP",
    );
  });

  await withFixture(async ({ localAppData, paths }) => {
    const real = join(localAppData, "real-cache");
    await mkdir(paths.appRoot);
    await mkdir(real);
    await symlink(real, paths.cacheRoot, "junction");
    await assert.rejects(
      ensureCacheRoot(paths, { token: TOKEN_ONE }),
      (error) => error?.code === "CACHE_OWNERSHIP",
    );
  });
});

test("clears only the marked launcher root and preserves siblings", async () => {
  await withFixture(async ({ localAppData, paths }) => {
    const sibling = join(localAppData, "keep.txt");
    await writeFile(sibling, "keep");
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    await writeFile(join(paths.cacheRoot, "owned.txt"), "remove");
    assert.deepEqual(await clearCacheRoot(paths, { token: TOKEN_TWO }), {
      removed: true,
    });
    await assert.rejects(lstat(paths.cacheRoot), { code: "ENOENT" });
    assert.equal(await readFile(sibling, "utf8"), "keep");
    assert.deepEqual(await clearCacheRoot(paths, { token: TOKEN_ONE }), {
      removed: false,
    });
  });
});

test("publishes isolated staging directories and atomically replaces an owned version", async () => {
  await withFixture(async ({ paths }) => {
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    const first = await createVersionStaging(paths, { token: TOKEN_ONE });
    await writeFile(join(first, "payload.txt"), "one");
    await publishVersionStaging({
      paths,
      stagingDirectory: first,
      token: TOKEN_ONE,
      lock: fakeLock,
      validateCandidate: validPayload,
    });
    assert.equal(await payload(paths.versionDirectory), "one");

    const second = await createVersionStaging(paths, { token: TOKEN_TWO });
    await writeFile(join(second, "payload.txt"), "two");
    await publishVersionStaging({
      paths,
      stagingDirectory: second,
      token: TOKEN_TWO,
      lock: fakeLock,
      validateCandidate: validPayload,
    });
    assert.equal(await payload(paths.versionDirectory), "two");
  });
});

test("failed validation and a pre-publish failure preserve the previous version", async () => {
  await withFixture(async ({ paths }) => {
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    const first = await createVersionStaging(paths, { token: TOKEN_ONE });
    await writeFile(join(first, "payload.txt"), "one");
    await publishVersionStaging({
      paths,
      stagingDirectory: first,
      token: TOKEN_ONE,
      lock: fakeLock,
      validateCandidate: validPayload,
    });

    const invalid = await createVersionStaging(paths, { token: TOKEN_TWO });
    await writeFile(join(invalid, "payload.txt"), "invalid");
    await assert.rejects(
      publishVersionStaging({
        paths,
        stagingDirectory: invalid,
        token: TOKEN_TWO,
        lock: fakeLock,
        validateCandidate: validPayload,
      }),
      /invalid candidate/u,
    );
    assert.equal(await payload(paths.versionDirectory), "one");
    await rm(invalid, { recursive: true });

    const second = await createVersionStaging(paths, { token: TOKEN_TWO });
    await writeFile(join(second, "payload.txt"), "two");
    await assert.rejects(
      publishVersionStagingForTest(
        {
          paths,
          stagingDirectory: second,
          token: TOKEN_TWO,
          lock: fakeLock,
          validateCandidate: validPayload,
        },
        ({ phase }) => {
          if (phase === "afterQuarantine") throw new Error("simulated failure");
        },
      ),
      /simulated failure/u,
    );
    assert.equal(await payload(paths.versionDirectory), "one");
  });
});

test("refuses a link inside a staging tree without touching the current version", async () => {
  await withFixture(async ({ localAppData, paths }) => {
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    const first = await createVersionStaging(paths, { token: TOKEN_ONE });
    await writeFile(join(first, "payload.txt"), "one");
    await publishVersionStaging({
      paths,
      stagingDirectory: first,
      token: TOKEN_ONE,
      lock: fakeLock,
      validateCandidate: validPayload,
    });

    const outside = join(localAppData, "outside");
    await mkdir(outside);
    const second = await createVersionStaging(paths, { token: TOKEN_TWO });
    await writeFile(join(second, "payload.txt"), "two");
    await symlink(outside, join(second, "linked"), "junction");
    await assert.rejects(
      publishVersionStaging({
        paths,
        stagingDirectory: second,
        token: TOKEN_TWO,
        lock: fakeLock,
        validateCandidate: validPayload,
      }),
      (error) => error?.code === "CACHE_OWNERSHIP",
    );
    assert.equal(await payload(paths.versionDirectory), "one");
  });
});

test("recovers a valid owned staging directory after an interrupted replacement", async () => {
  await withFixture(async ({ paths }) => {
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    const first = await createVersionStaging(paths, { token: TOKEN_ONE });
    await writeFile(join(first, "payload.txt"), "one");
    await publishVersionStaging({
      paths,
      stagingDirectory: first,
      token: TOKEN_ONE,
      lock: fakeLock,
      validateCandidate: validPayload,
    });
    const second = await createVersionStaging(paths, { token: TOKEN_TWO });
    await writeFile(join(second, "payload.txt"), "two");

    const old = join(paths.cacheRoot, `.old-v0.2.2-${TOKEN_ONE}`);
    await rename(paths.versionDirectory, old);
    await writeFile(join(old, "payload.txt"), "corrupt");
    const recovered = await recoverVersionPublication({
      paths,
      lock: fakeLock,
      validateCandidate: validPayload,
    });
    assert.equal(recovered, paths.versionDirectory);
    assert.equal(await payload(paths.versionDirectory), "two");
  });
});
