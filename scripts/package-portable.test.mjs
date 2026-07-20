import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFile,
  lstat,
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { stagePortable } from "./package-portable.mjs";

const scriptsDir = dirname(fileURLToPath(import.meta.url));
const packageScript = join(scriptsDir, "package-portable.mjs");
const compressorScript = join(scriptsDir, "compress-portable.ps1");
const fileContents = new Map([
  ["README.txt", "no install\n"],
  ["ability-radar.exe", "fake-exe"],
  ["benchmark-packs/client-quick-v1/manifest.json", "{}\n"],
  ["benchmark-packs/client-quick-v1/payload.txt", "alpha\n"],
  ["benchmark-packs/cli-quick-v1/manifest.json", "{}\n"],
  ["benchmark-packs/registry.json", '{"schema_version":1,"packs":[]}\n'],
]);

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

async function createFixture({ cli = false } = {}) {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-portable-"));
  const repoRoot = join(root, "repo");
  const targetDir = cli
    ? join(repoRoot, "target", "release")
    : join(root, "target", "release");
  const bundleDir = join(targetDir, "bundle", "portable");
  await mkdir(join(repoRoot, "packaging", "windows-portable"), {
    recursive: true,
  });
  await mkdir(join(targetDir, "benchmark-packs", "client-quick-v1"), {
    recursive: true,
  });
  await mkdir(join(targetDir, "benchmark-packs", "cli-quick-v1"), {
    recursive: true,
  });
  await writeFile(join(targetDir, "ability-radar.exe"), "fake-exe");
  await writeFile(
    join(targetDir, "benchmark-packs", "registry.json"),
    '{"schema_version":1,"packs":[]}\n',
  );
  await writeFile(
    join(targetDir, "benchmark-packs", "client-quick-v1", "manifest.json"),
    "{}\n",
  );
  await writeFile(
    join(targetDir, "benchmark-packs", "client-quick-v1", "payload.txt"),
    "alpha\n",
  );
  await writeFile(
    join(targetDir, "benchmark-packs", "cli-quick-v1", "manifest.json"),
    "{}\n",
  );
  await writeFile(
    join(repoRoot, "packaging", "windows-portable", "README.txt"),
    "no install\n",
  );
  if (cli) {
    await mkdir(join(repoRoot, "scripts"), { recursive: true });
    await copyFile(packageScript, join(repoRoot, "scripts", "package-portable.mjs"));
    await copyFile(
      compressorScript,
      join(repoRoot, "scripts", "compress-portable.ps1"),
    );
    await writeFile(
      join(repoRoot, "package.json"),
      '{"name":"portable-fixture","version":"0.2.1","private":true}\n',
    );
  }
  return { root, repoRoot, targetDir, bundleDir };
}

async function entriesUnder(root, current = root) {
  const result = [];
  for (const entry of await readdir(current, { withFileTypes: true })) {
    const path = join(current, entry.name);
    const name = relative(root, path).replaceAll("\\", "/");
    if (entry.isDirectory()) {
      result.push(`${name}/`, ...await entriesUnder(root, path));
    } else {
      result.push(name);
    }
  }
  return result.sort();
}

function runCli(repoRoot) {
  return spawnSync(process.execPath, [join(repoRoot, "scripts", "package-portable.mjs")], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

test("stages the exact rooted tree and complete deterministic checksums", async () => {
  const fixture = await createFixture();
  try {
    const first = await stagePortable({ ...fixture, version: "0.2.1" });
    assert.deepEqual(await entriesUnder(first.stageRoot), [
      "README.txt",
      "SHA256SUMS.txt",
      "ability-radar.exe",
      "benchmark-packs/",
      "benchmark-packs/cli-quick-v1/",
      "benchmark-packs/cli-quick-v1/manifest.json",
      "benchmark-packs/client-quick-v1/",
      "benchmark-packs/client-quick-v1/manifest.json",
      "benchmark-packs/client-quick-v1/payload.txt",
      "benchmark-packs/registry.json",
    ]);
    const expectedChecksums = [...fileContents]
      .sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0)
      .map(([name, contents]) => `${sha256(contents)}  ${name}`)
      .join("\n") + "\n";
    const firstChecksums = await readFile(
      join(first.stageRoot, "SHA256SUMS.txt"),
      "utf8",
    );
    assert.equal(firstChecksums, expectedChecksums);
    assert.doesNotMatch(firstChecksums, /SHA256SUMS\.txt/);

    const second = await stagePortable({ ...fixture, version: "0.2.1" });
    assert.equal(
      await readFile(join(second.stageRoot, "SHA256SUMS.txt"), "utf8"),
      firstChecksums,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("refuses an output directory outside the selected target tree", async () => {
  await assert.rejects(
    stagePortable({
      repoRoot: "C:\\repo",
      targetDir: "C:\\repo\\target\\release",
      bundleDir: "C:\\outside",
      version: "0.2.1",
    }),
    /inside target directory/,
  );
});

test("rejects invalid or path-shaped versions inside stagePortable", async () => {
  const fixture = await createFixture();
  try {
    for (const version of [
      "../0.2.1",
      "0.2.1/escape",
      "0.2.1\\escape",
      "v0.2.1",
      "01.2.3",
      "0.2",
      "0.2.1-beta",
      "",
    ]) {
      await assert.rejects(
        stagePortable({ ...fixture, version }),
        /strict semantic version/,
        version,
      );
    }
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a junction in the selected target path before reading inputs", async () => {
  const fixture = await createFixture();
  try {
    const realTarget = join(fixture.root, "real-target");
    await rename(fixture.targetDir, realTarget);
    await symlink(realTarget, fixture.targetDir, "junction");
    await assert.rejects(
      stagePortable({ ...fixture, version: "0.2.1" }),
      /indirection|reparse|symbolic link/i,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a junction in the README input path", async () => {
  const fixture = await createFixture();
  try {
    const readmeParent = join(
      fixture.repoRoot,
      "packaging",
      "windows-portable",
    );
    const realReadmeParent = join(fixture.root, "real-readme");
    await rename(readmeParent, realReadmeParent);
    await symlink(realReadmeParent, readmeParent, "junction");
    await assert.rejects(
      stagePortable({ ...fixture, version: "0.2.1" }),
      /indirection|reparse|symbolic link/i,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a recursive benchmark-pack junction", async () => {
  const fixture = await createFixture();
  try {
    const outside = join(fixture.root, "outside-pack");
    await mkdir(outside);
    await writeFile(join(outside, "payload.txt"), "outside\n");
    await symlink(
      outside,
      join(fixture.targetDir, "benchmark-packs", "linked"),
      "junction",
    );
    await assert.rejects(
      stagePortable({ ...fixture, version: "0.2.1" }),
      /indirection|reparse|symbolic link/i,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects junctioned bundle and stage roots before recursive removal", async () => {
  for (const kind of ["bundle", "stage"]) {
    const fixture = await createFixture();
    try {
      const outside = join(fixture.root, `outside-${kind}`);
      await mkdir(outside);
      if (kind === "bundle") {
        await mkdir(dirname(fixture.bundleDir), { recursive: true });
        await symlink(outside, fixture.bundleDir, "junction");
      } else {
        await mkdir(fixture.bundleDir, { recursive: true });
        await symlink(outside, join(fixture.bundleDir, ".stage"), "junction");
      }
      await assert.rejects(
        stagePortable({ ...fixture, version: "0.2.1" }),
        /indirection|reparse|symbolic link/i,
        kind,
      );
      assert.deepEqual(await readdir(outside), []);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  }
});

test(
  "Windows compressor produces one archive root and removes staging",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const result = runCli(fixture.repoRoot);
      assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.1_windows-x64-portable.zip",
      );
      const listing = spawnSync("tar.exe", ["-tf", archivePath], {
        encoding: "utf8",
      });
      assert.equal(listing.status, 0, listing.stderr);
      const entries = listing.stdout.trim().split(/\r?\n/);
      assert.ok(entries.length >= fileContents.size + 1);
      assert.ok(
        entries.every((entry) => entry.startsWith("ability-radar-portable/")),
      );
      for (const name of [...fileContents.keys(), "SHA256SUMS.txt"]) {
        assert.ok(
          entries.includes(`ability-radar-portable/${name}`),
          name,
        );
      }
      await assert.rejects(lstat(join(fixture.bundleDir, ".stage")), {
        code: "ENOENT",
      });
      assert.deepEqual(
        (await readdir(fixture.bundleDir)).filter((name) => name !==
          "ability-radar_0.2.1_windows-x64-portable.zip"),
        [],
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "compression failure removes temporary ZIP and stage without a partial final",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      await writeFile(
        join(fixture.repoRoot, "scripts", "compress-portable.ps1"),
        [
          "param([string]$Source, [string]$Destination)",
          '[System.IO.File]::WriteAllText($Destination, "partial")',
          "exit 9",
          "",
        ].join("\n"),
      );
      const result = runCli(fixture.repoRoot);
      assert.notEqual(result.status, 0);
      const entries = await readdir(fixture.bundleDir);
      assert.deepEqual(entries, []);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "existing final archive is preserved and never overwritten",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      await mkdir(fixture.bundleDir, { recursive: true });
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.1_windows-x64-portable.zip",
      );
      await writeFile(archivePath, "existing-final");
      const result = runCli(fixture.repoRoot);
      assert.notEqual(result.status, 0);
      assert.equal(await readFile(archivePath, "utf8"), "existing-final");
      assert.deepEqual(await readdir(fixture.bundleDir), [
        "ability-radar_0.2.1_windows-x64-portable.zip",
      ]);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "hard-link publication atomically preserves a racing final",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      assert.equal(
        typeof portable.packagePortableFromBuildForTest,
        "function",
        "test-only publication seam must exist",
      );
      let temporaryArchive;
      await assert.rejects(
        portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase, archivePath, temporaryArchive: temporary }) => {
            if (phase !== "beforeLink") return;
            temporaryArchive = temporary;
            await writeFile(archivePath, "racing-final");
          },
        ),
        (error) => error?.code === "EEXIST" || error?.cause?.code === "EEXIST",
      );
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.1_windows-x64-portable.zip",
      );
      assert.equal(await readFile(archivePath, "utf8"), "racing-final");
      await assert.rejects(lstat(temporaryArchive), { code: "ENOENT" });
      await assert.rejects(lstat(join(fixture.bundleDir, ".stage")), {
        code: "ENOENT",
      });
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "successful hard-link publication preserves identity and removes temp",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      assert.equal(typeof portable.packagePortableFromBuildForTest, "function");
      let temporaryArchive;
      let temporaryIdentity;
      const archivePath = await portable.packagePortableFromBuildForTest(
        fixture.repoRoot,
        async ({ phase, temporaryArchive: temporary }) => {
          if (phase !== "beforeLink") return;
          temporaryArchive = temporary;
          temporaryIdentity = await stat(temporary);
        },
      );
      const finalIdentity = await stat(archivePath);
      assert.equal(finalIdentity.dev, temporaryIdentity.dev);
      assert.equal(finalIdentity.ino, temporaryIdentity.ino);
      await assert.rejects(lstat(temporaryArchive), { code: "ENOENT" });
      await assert.rejects(lstat(join(fixture.bundleDir, ".stage")), {
        code: "ENOENT",
      });
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "post-link failure reports publication and never rolls back final",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      assert.equal(typeof portable.packagePortableFromBuildForTest, "function");
      let temporaryArchive;
      await assert.rejects(
        portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase, temporaryArchive: temporary }) => {
            temporaryArchive = temporary;
            if (phase === "afterLink") throw new Error("simulated cleanup failure");
          },
        ),
        /published.*cleanup/i,
      );
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.1_windows-x64-portable.zip",
      );
      const signature = (await readFile(archivePath)).subarray(0, 2);
      assert.deepEqual([...signature], [0x50, 0x4b]);
      await assert.rejects(lstat(temporaryArchive), { code: "ENOENT" });
      await assert.rejects(lstat(join(fixture.bundleDir, ".stage")), {
        code: "ENOENT",
      });
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);
