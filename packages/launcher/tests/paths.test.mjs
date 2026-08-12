import assert from "node:assert/strict";
import { join } from "node:path";
import test from "node:test";

import { resolveCachePaths } from "../lib/paths.mjs";

test("resolves one fixed per-user cache tree", () => {
  const localAppData = "C:\\Users\\tester\\AppData\\Local";
  const paths = resolveCachePaths({ localAppData, version: "0.2.2" });
  assert.equal(paths.localAppData, localAppData);
  assert.equal(paths.appRoot, join(localAppData, "AI Ability Radar"));
  assert.equal(paths.cacheRoot, join(localAppData, "AI Ability Radar", "launcher"));
  assert.equal(paths.versionDirectory, join(paths.cacheRoot, "v0.2.2"));
  assert.equal(paths.lockDirectory, join(paths.cacheRoot, ".lock-v0.2.2"));
  assert.equal(Object.isFrozen(paths), true);
});

test("rejects missing, relative, and path-shaped cache inputs", () => {
  for (const options of [
    { localAppData: "", version: "0.2.2" },
    { localAppData: ".\\relative", version: "0.2.2" },
    { localAppData: "C:\\Users\\tester\\AppData\\Local", version: "../0.2.2" },
    { localAppData: "C:\\Users\\tester\\AppData\\Local", version: "v0.2.2" },
  ]) {
    assert.throws(
      () => resolveCachePaths(options),
      (error) => ["INVALID_CACHE_PATH", "INVALID_VERSION"].includes(error?.code),
    );
  }
});
