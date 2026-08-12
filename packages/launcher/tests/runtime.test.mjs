import assert from "node:assert/strict";
import test from "node:test";

import {
  assertSupportedRuntime,
  parseNodeVersion,
} from "../lib/runtime.mjs";

test("accepts the reviewed Node 22 and 24 Windows x64 runtimes", () => {
  for (const nodeVersion of ["22.22.0", "22.22.2", "22.99.0", "24.0.0", "24.8.1"]) {
    assert.deepEqual(
      assertSupportedRuntime({ platform: "win32", arch: "x64", nodeVersion }),
      {
        platform: "win32",
        arch: "x64",
        nodeVersion,
      },
    );
  }
});

test("rejects unsupported operating systems and architectures", () => {
  for (const runtime of [
    { platform: "linux", arch: "x64", nodeVersion: "22.22.0" },
    { platform: "darwin", arch: "arm64", nodeVersion: "24.0.0" },
    { platform: "win32", arch: "arm64", nodeVersion: "24.0.0" },
    { platform: "win32", arch: "ia32", nodeVersion: "22.22.0" },
  ]) {
    assert.throws(
      () => assertSupportedRuntime(runtime),
      (error) => error?.code === "UNSUPPORTED_PLATFORM" && /Windows 10\/11 x64/u.test(error.message),
    );
  }
});

test("rejects unsupported and malformed Node versions", () => {
  for (const nodeVersion of [
    "22.21.99",
    "21.99.0",
    "23.0.0",
    "25.0.0",
    "22.22",
    "v22.22.0",
    "022.22.0",
    "24.0.0-rc.1",
    "",
  ]) {
    assert.throws(
      () => assertSupportedRuntime({ platform: "win32", arch: "x64", nodeVersion }),
      (error) => error?.code === "UNSUPPORTED_NODE" && /Node\.js 22\.22\+.*24 LTS/u.test(error.message),
      nodeVersion,
    );
  }
});

test("parses strict stable Node versions", () => {
  assert.deepEqual(parseNodeVersion("22.22.2"), {
    major: 22,
    minor: 22,
    patch: 2,
    raw: "22.22.2",
  });
  assert.throws(() => parseNodeVersion("22.22.2.1"), /Node\.js/u);
});
