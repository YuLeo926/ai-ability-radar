import assert from "node:assert/strict";
import {
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { ensureCacheRoot } from "../lib/cache.mjs";
import { acquireVersionLock } from "../lib/lock.mjs";
import { resolveCachePaths } from "../lib/paths.mjs";

const TOKEN_ONE = "11111111-1111-4111-8111-111111111111";
const TOKEN_TWO = "22222222-2222-4222-8222-222222222222";

async function withFixture(run) {
  const localAppData = await mkdtemp(join(tmpdir(), "ability-radar-lock-"));
  const paths = resolveCachePaths({ localAppData, version: "0.2.2" });
  try {
    await ensureCacheRoot(paths, { token: TOKEN_ONE });
    return await run(paths);
  } finally {
    await rm(localAppData, { recursive: true, force: true });
  }
}

test("acquires, proves, and releases one version lock", async () => {
  await withFixture(async (paths) => {
    const lock = await acquireVersionLock(paths, { token: TOKEN_ONE });
    await lock.assertOwned();
    await lock.release();
    await assert.rejects(lock.assertOwned(), (error) => error?.code === "LOCK_LOST");
  });
});

test("a waiting caller acquires only after the current owner releases", async () => {
  await withFixture(async (paths) => {
    const first = await acquireVersionLock(paths, { token: TOKEN_ONE });
    const pending = acquireVersionLock(paths, {
      token: TOKEN_TWO,
      timeoutMs: 1_000,
      staleMs: 60_000,
      pollMs: 5,
    });
    setTimeout(() => first.release(), 25);
    const second = await pending;
    await second.assertOwned();
    await second.release();
  });
});

test("reports a busy lock after a bounded wait", async () => {
  await withFixture(async (paths) => {
    const first = await acquireVersionLock(paths, { token: TOKEN_ONE });
    await assert.rejects(
      acquireVersionLock(paths, {
        token: TOKEN_TWO,
        timeoutMs: 20,
        staleMs: 60_000,
        pollMs: 5,
      }),
      (error) => error?.code === "LOCK_BUSY",
    );
    await first.release();
  });
});

test("stale takeover invalidates the old token before it can publish", async () => {
  await withFixture(async (paths) => {
    const first = await acquireVersionLock(paths, {
      token: TOKEN_ONE,
      now: () => 1_000,
    });
    const second = await acquireVersionLock(paths, {
      token: TOKEN_TWO,
      now: () => 10_000,
      timeoutMs: 200,
      staleMs: 1_000,
      pollMs: 5,
    });
    await assert.rejects(first.assertOwned(), (error) => error?.code === "LOCK_LOST");
    await second.assertOwned();
    await second.release();
  });
});

test("never deletes a lock after its owner record is replaced", async () => {
  await withFixture(async (paths) => {
    const lock = await acquireVersionLock(paths, { token: TOKEN_ONE });
    const ownerPath = join(paths.lockDirectory, "owner.json");
    const owner = JSON.parse(await readFile(ownerPath, "utf8"));
    owner.token = TOKEN_TWO;
    await writeFile(ownerPath, `${JSON.stringify(owner)}\n`);
    await assert.rejects(lock.release(), (error) => error?.code === "LOCK_LOST");
    assert.equal(JSON.parse(await readFile(ownerPath, "utf8")).token, TOKEN_TWO);
  });
});
