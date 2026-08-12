import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { LauncherError } from "../lib/errors.mjs";
import { resolveCachePaths } from "../lib/paths.mjs";
import { runLauncherCommandForTest } from "../lib/run.mjs";
import { createPortableFixture } from "./helpers/zip-fixture.mjs";

const RUNTIME = Object.freeze({
  platform: "win32",
  arch: "x64",
  nodeVersion: "22.22.2",
});

async function withRunner(run) {
  const localAppData = await mkdtemp(join(tmpdir(), "ability-radar-run-"));
  const fixture = createPortableFixture();
  const counters = { checksums: 0, portable: 0, launches: 0 };
  const dependencies = {
    async downloadChecksums({ identity, destination }) {
      counters.checksums += 1;
      await writeFile(
        destination,
        `${fixture.manifest.assets.portable.sha256}  ${identity.portableFileName}\n`,
        { flag: "wx" },
      );
    },
    async downloadPortable({ destination }) {
      counters.portable += 1;
      await writeFile(destination, fixture.archive, { flag: "wx" });
    },
    async launchApplication({ executable, cwd }) {
      counters.launches += 1;
      assert.equal(executable, join(cwd, "ability-radar.exe"));
      assert.equal((await readFile(executable)).subarray(0, 2).toString(), "MZ");
    },
  };
  const options = {
    command: { kind: "launch" },
    version: "0.2.2",
    manifest: fixture.manifest,
    localAppData,
    runtime: RUNTIME,
  };
  try {
    return await run({ localAppData, fixture, counters, dependencies, options });
  } finally {
    await rm(localAppData, { recursive: true, force: true });
  }
}

test("downloads once, launches offline, repairs locally, then redownloads a corrupt ZIP", async () => {
  await withRunner(async ({ localAppData, counters, dependencies, options }) => {
    const first = await runLauncherCommandForTest(options, dependencies);
    assert.equal(first.source, "downloaded");
    assert.deepEqual(counters, { checksums: 1, portable: 1, launches: 1 });

    const offline = {
      ...dependencies,
      async downloadChecksums() { throw new Error("network must not be used"); },
      async downloadPortable() { throw new Error("network must not be used"); },
    };
    const second = await runLauncherCommandForTest(options, offline);
    assert.equal(second.source, "cache");
    assert.equal(counters.launches, 2);

    const paths = resolveCachePaths({ localAppData, version: "0.2.2" });
    const readme = join(
      paths.versionDirectory,
      "app",
      "ability-radar-portable",
      "README.txt",
    );
    await writeFile(readme, "tampered");
    const repaired = await runLauncherCommandForTest(options, offline);
    assert.equal(repaired.source, "repaired");
    assert.deepEqual(counters, { checksums: 1, portable: 1, launches: 3 });

    await writeFile(readme, "tampered again");
    await writeFile(
      join(paths.versionDirectory, options.manifest.assets.portable.file_name),
      "broken zip",
    );
    const redownloaded = await runLauncherCommandForTest(options, dependencies);
    assert.equal(redownloaded.source, "downloaded");
    assert.deepEqual(counters, { checksums: 2, portable: 2, launches: 4 });
  });
});

test("serializes concurrent first launches into one download and one cache", async () => {
  await withRunner(async ({ counters, dependencies, options }) => {
    let releaseDownload;
    const gate = new Promise((resolve) => { releaseDownload = resolve; });
    let entered = false;
    const delayed = {
      ...dependencies,
      async downloadChecksums(args) {
        entered = true;
        await gate;
        return dependencies.downloadChecksums(args);
      },
    };
    const first = runLauncherCommandForTest(options, delayed);
    while (!entered) await new Promise((resolve) => setTimeout(resolve, 5));
    const second = runLauncherCommandForTest(options, delayed);
    releaseDownload();
    const results = await Promise.all([first, second]);
    assert.deepEqual(results.map(({ source }) => source).sort(), ["cache", "downloaded"]);
    assert.deepEqual(counters, { checksums: 1, portable: 1, launches: 2 });
  });
});

test("reports first-run network failure and removes only its staging directory", async () => {
  await withRunner(async ({ localAppData, dependencies, options }) => {
    const failing = {
      ...dependencies,
      async downloadChecksums() {
        throw new LauncherError("DOWNLOAD_FAILED", "secret network detail");
      },
      async downloadPortable() {
        throw new Error("must not reach portable download");
      },
    };
    await assert.rejects(
      runLauncherCommandForTest(options, failing),
      (error) =>
        error?.code === "NETWORK_REQUIRED" &&
        !error.message.includes("secret"),
    );
    const paths = resolveCachePaths({ localAppData, version: "0.2.2" });
    const names = await readdir(paths.cacheRoot);
    assert.equal(names.some((name) => name.startsWith(".stage-")), false);
    assert.equal(names.includes(paths.versionTag), false);
  });
});

test("rejects a release checksum that differs from the npm-pinned ZIP hash", async () => {
  await withRunner(async ({ localAppData, dependencies, options, counters }) => {
    const mismatched = {
      ...dependencies,
      async downloadChecksums({ identity, destination }) {
        counters.checksums += 1;
        await writeFile(
          destination,
          `${"0".repeat(64)}  ${identity.portableFileName}\r\n`,
          { flag: "wx" },
        );
      },
    };
    await assert.rejects(
      runLauncherCommandForTest(options, mismatched),
      (error) => error?.code === "DOWNLOAD_INTEGRITY",
    );
    assert.deepEqual(counters, { checksums: 1, portable: 0, launches: 0 });
    const paths = resolveCachePaths({ localAppData, version: "0.2.2" });
    const names = await readdir(paths.cacheRoot);
    assert.equal(names.some((name) => name.startsWith(".stage-")), false);
  });
});

test("refuses a tampered cache ownership marker without launching or replacing it", async () => {
  await withRunner(async ({ localAppData, counters, dependencies, options }) => {
    await runLauncherCommandForTest(options, dependencies);
    const paths = resolveCachePaths({ localAppData, version: "0.2.2" });
    const markerPath = join(paths.versionDirectory, ".cache-entry.json");
    await writeFile(markerPath, "{}\n");
    await assert.rejects(
      runLauncherCommandForTest(options, dependencies),
      (error) => error?.code === "CACHE_OWNERSHIP",
    );
    assert.deepEqual(counters, { checksums: 1, portable: 1, launches: 1 });
    assert.equal(await readFile(markerPath, "utf8"), "{}\n");
  });
});

test("clear-cache never launches and preserves data beside the owned cache", async () => {
  await withRunner(async ({ localAppData, counters, dependencies, options }) => {
    await runLauncherCommandForTest(options, dependencies);
    const sibling = join(localAppData, "keep.txt");
    await writeFile(sibling, "keep");
    const result = await runLauncherCommandForTest(
      { ...options, command: { kind: "clear-cache" }, manifest: undefined },
      dependencies,
    );
    assert.equal(result.kind, "cache-cleared");
    assert.equal(counters.launches, 1);
    assert.equal(await readFile(sibling, "utf8"), "keep");
  });
});
