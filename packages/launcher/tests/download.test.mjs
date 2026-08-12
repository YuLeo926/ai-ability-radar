import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  buildReleaseAssetUrl,
  downloadReleaseAssetForTest,
} from "../lib/download.mjs";
import { deriveReleaseIdentity } from "../lib/manifest.mjs";

const identity = deriveReleaseIdentity("0.2.2");

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function response({ statusCode = 200, headers = {}, chunks = [], failure } = {}) {
  let destroyed = false;
  return {
    statusCode,
    headers,
    get destroyed() {
      return destroyed;
    },
    destroy() {
      destroyed = true;
    },
    async *[Symbol.asyncIterator]() {
      for (const chunk of chunks) yield chunk;
      if (failure) throw failure;
    },
  };
}

function scriptedTransport(steps) {
  const calls = [];
  return {
    calls,
    async request(url, options) {
      calls.push({ url: url.toString(), options });
      const step = steps[calls.length - 1];
      if (typeof step === "function") return step(url, options);
      if (!step) throw new Error("unexpected request");
      return step;
    },
  };
}

async function withTempDirectory(run) {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-download-"));
  try {
    return await run(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("builds exact versioned GitHub Release asset URLs", () => {
  assert.equal(
    buildReleaseAssetUrl(identity, identity.portableFileName).toString(),
    "https://github.com/YuLeo926/ai-ability-radar/releases/download/v0.2.2/ability-radar_0.2.2_windows-x64-portable.zip",
  );
  assert.equal(
    buildReleaseAssetUrl(identity, identity.checksumsFileName).toString(),
    "https://github.com/YuLeo926/ai-ability-radar/releases/download/v0.2.2/SHA256SUMS.txt",
  );
  assert.throws(
    () => buildReleaseAssetUrl(identity, "latest.zip"),
    (error) => error?.code === "INVALID_DOWNLOAD_URL",
  );
});

test("follows only reviewed HTTPS redirects and streams a verified portable file", async () => {
  await withTempDirectory(async (root) => {
    const body = Buffer.from("verified portable bytes");
    const redirect = response({
      statusCode: 302,
      headers: {
        location: "https://release-assets.githubusercontent.com/github-production-release-asset/example?sig=secret",
      },
    });
    const final = response({
      headers: {
        "content-length": String(body.length),
        "content-encoding": "identity",
      },
      chunks: [body.subarray(0, 7), body.subarray(7)],
    });
    const transport = scriptedTransport([redirect, final]);
    const destination = join(root, "portable.zip.part");

    const result = await downloadReleaseAssetForTest({
      identity,
      kind: "portable",
      expectedSize: body.length,
      expectedSha256: sha256(body),
      destination,
      transport: transport.request.bind(transport),
    });

    assert.deepEqual(result, {
      bytes: body.length,
      sha256: sha256(body),
      source: "github-release",
    });
    assert.deepEqual(await readFile(destination), body);
    assert.equal(transport.calls.length, 2);
    assert.equal(transport.calls[0].options.headers["Accept-Encoding"], "identity");
    assert.match(transport.calls[0].options.headers["User-Agent"], /^ai-ability-radar-launcher\/0\.2\.2$/u);
    assert.equal(redirect.destroyed, true);
  });
});

test("rejects unreviewed redirect targets without writing a file", async () => {
  for (const location of [
    "http://release-assets.githubusercontent.com/file",
    "https://example.com/file",
    "https://user:password@github.com/file",
    "https://github.com:444/file",
    "https://github.com/file#fragment",
  ]) {
    await withTempDirectory(async (root) => {
      const transport = scriptedTransport([
        response({ statusCode: 302, headers: { location } }),
      ]);
      const destination = join(root, "asset.part");
      await assert.rejects(
        downloadReleaseAssetForTest({
          identity,
          kind: "checksums",
          destination,
          transport: transport.request.bind(transport),
        }),
        (error) => error?.code === "INVALID_DOWNLOAD_URL" && !error.message.includes(location),
        location,
      );
      await assert.rejects(readFile(destination), { code: "ENOENT" });
    });
  }
});

test("limits redirect count and rejects redirect loops", async () => {
  await withTempDirectory(async (root) => {
    const redirects = Array.from({ length: 6 }, (_, index) =>
      response({
        statusCode: 302,
        headers: {
          location: `https://github.com/YuLeo926/ai-ability-radar/releases/download/v0.2.2/SHA256SUMS.txt?step=${index}`,
        },
      }),
    );
    const transport = scriptedTransport(redirects);
    await assert.rejects(
      downloadReleaseAssetForTest({
        identity,
        kind: "checksums",
        destination: join(root, "checksums.part"),
        transport: transport.request.bind(transport),
      }),
      (error) => error?.code === "TOO_MANY_REDIRECTS",
    );
    assert.equal(transport.calls.length, 6);
  });
});

test("rejects status, content encoding, and declared length errors", async () => {
  const cases = [
    [response({ statusCode: 404 }), "DOWNLOAD_FAILED"],
    [response({ headers: { "content-encoding": "gzip" } }), "DOWNLOAD_FAILED"],
    [response({ headers: { "content-length": "not-a-number" } }), "DOWNLOAD_FAILED"],
    [response({ headers: { "content-length": "2" }, chunks: [Buffer.from("abc")] }), "DOWNLOAD_INTEGRITY"],
  ];
  for (const [fakeResponse, code] of cases) {
    await withTempDirectory(async (root) => {
      const transport = scriptedTransport([fakeResponse]);
      const destination = join(root, "asset.part");
      await assert.rejects(
        downloadReleaseAssetForTest({
          identity,
          kind: "portable",
          expectedSize: 3,
          expectedSha256: sha256("abc"),
          destination,
          transport: transport.request.bind(transport),
        }),
        (error) => error?.code === code,
      );
      await assert.rejects(readFile(destination), { code: "ENOENT" });
    });
  }
});

test("rejects short, long, corrupt, and interrupted bodies and removes owned partial files", async () => {
  const expected = Buffer.from("expected");
  const cases = [
    response({ chunks: [Buffer.from("short")] }),
    response({ chunks: [Buffer.concat([expected, Buffer.from("extra")])] }),
    response({ chunks: [Buffer.from("corrupt!")] }),
    response({ chunks: [expected.subarray(0, 2)], failure: new Error("signed-url?secret") }),
  ];
  for (const fakeResponse of cases) {
    await withTempDirectory(async (root) => {
      const transport = scriptedTransport([fakeResponse]);
      const destination = join(root, "asset.part");
      await assert.rejects(
        downloadReleaseAssetForTest({
          identity,
          kind: "portable",
          expectedSize: expected.length,
          expectedSha256: sha256(expected),
          destination,
          transport: transport.request.bind(transport),
        }),
        (error) =>
          ["DOWNLOAD_FAILED", "DOWNLOAD_INTEGRITY", "ASSET_TOO_LARGE"].includes(error?.code) &&
          !error.message.includes("secret"),
      );
      await assert.rejects(readFile(destination), { code: "ENOENT" });
    });
  }
});

test("never overwrites a pre-existing destination", async () => {
  await withTempDirectory(async (root) => {
    const destination = join(root, "asset.part");
    await writeFile(destination, "owner data", { flag: "wx" });
    const transport = scriptedTransport([
      response({ chunks: [Buffer.from("new data")] }),
    ]);
    await assert.rejects(
      downloadReleaseAssetForTest({
        identity,
        kind: "checksums",
        destination,
        transport: transport.request.bind(transport),
      }),
      (error) => error?.code === "DOWNLOAD_FAILED",
    );
    assert.equal(await readFile(destination, "utf8"), "owner data");
    assert.equal(transport.calls.length, 0);
  });
});

test("enforces the checksum size cap and total timeout", async () => {
  await withTempDirectory(async (root) => {
    const oversized = Buffer.alloc(64 * 1024 + 1, 1);
    const transport = scriptedTransport([response({ chunks: [oversized] })]);
    await assert.rejects(
      downloadReleaseAssetForTest({
        identity,
        kind: "checksums",
        destination: join(root, "large.part"),
        transport: transport.request.bind(transport),
      }),
      (error) => error?.code === "ASSET_TOO_LARGE",
    );
  });

  await withTempDirectory(async (root) => {
    const transport = scriptedTransport([
      (_url, { signal }) => new Promise((resolve, reject) => {
        signal.addEventListener("abort", () => reject(signal.reason), { once: true });
      }),
    ]);
    await assert.rejects(
      downloadReleaseAssetForTest({
        identity,
        kind: "checksums",
        destination: join(root, "timeout.part"),
        transport: transport.request.bind(transport),
        totalTimeoutMs: 20,
      }),
      (error) => error?.code === "DOWNLOAD_TIMEOUT",
    );
  });
});
