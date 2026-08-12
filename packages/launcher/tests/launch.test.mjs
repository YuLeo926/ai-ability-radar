import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { launchVerifiedExecutableForTest } from "../lib/launch.mjs";

async function withExecutable(run) {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-launch-"));
  const cwd = join(root, "ability-radar-portable");
  const executable = join(cwd, "ability-radar.exe");
  try {
    await mkdir(cwd);
    await writeFile(executable, "MZ test executable");
    return await run({ root, cwd, executable });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

function successfulSpawner(calls, beforeSpawn) {
  return (executable, args, options) => {
    const child = new EventEmitter();
    child.unref = () => { child.unrefCalled = true; };
    child.kill = () => { child.killCalled = true; };
    calls.push({ executable, args, options, child });
    queueMicrotask(async () => {
      await beforeSpawn?.();
      child.emit("spawn");
    });
    return child;
  };
}

test("spawns only the fixed verified executable without a shell", async () => {
  await withExecutable(async ({ cwd, executable }) => {
    const calls = [];
    await launchVerifiedExecutableForTest(
      { executable, cwd },
      { spawnProcess: successfulSpawner(calls) },
    );
    assert.equal(calls.length, 1);
    assert.equal(calls[0].executable, executable);
    assert.deepEqual(calls[0].args, []);
    assert.deepEqual(calls[0].options, {
      cwd,
      detached: true,
      shell: false,
      stdio: "ignore",
      windowsHide: false,
    });
    assert.equal(calls[0].child.unrefCalled, true);
  });
});

test("rejects missing, linked, and replaced executables", async () => {
  await withExecutable(async ({ root, cwd, executable }) => {
    await rm(executable);
    await assert.rejects(
      launchVerifiedExecutableForTest(
        { executable, cwd },
        { spawnProcess: successfulSpawner([]) },
      ),
      (error) => error?.code === "LAUNCH_FAILED",
    );

    const outside = join(root, "outside-directory");
    await mkdir(outside);
    await writeFile(join(outside, "ability-radar.exe"), "outside");
    await rm(cwd, { recursive: true });
    await symlink(outside, cwd, "junction");
    await assert.rejects(
      launchVerifiedExecutableForTest(
        { executable, cwd },
        { spawnProcess: successfulSpawner([]) },
      ),
      (error) => error?.code === "LAUNCH_FAILED",
    );
  });

  await withExecutable(async ({ cwd, executable }) => {
    const calls = [];
    await assert.rejects(
      launchVerifiedExecutableForTest(
        { executable, cwd },
        {
          spawnProcess: successfulSpawner(calls, async () => {
            await rm(executable);
            await writeFile(executable, "replacement");
          }),
        },
      ),
      (error) => error?.code === "LAUNCH_FAILED",
    );
    assert.equal(calls[0].child.killCalled, true);
    assert.notEqual(calls[0].child.unrefCalled, true);
  });
});

test("maps process creation errors to a stable path-free failure", async () => {
  await withExecutable(async ({ cwd, executable }) => {
    const spawnProcess = () => {
      const child = new EventEmitter();
      child.unref = () => {};
      queueMicrotask(() => child.emit("error", new Error(`cannot run ${executable}`)));
      return child;
    };
    await assert.rejects(
      launchVerifiedExecutableForTest({ executable, cwd }, { spawnProcess }),
      (error) =>
        error?.code === "LAUNCH_FAILED" &&
        !error.message.includes(executable),
    );
  });
});
