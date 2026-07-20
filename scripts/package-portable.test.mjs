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
const extractorScript = join(scriptsDir, "extract-portable.ps1");

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function packContentHash(files) {
  const digest = createHash("sha256");
  for (const [name, contents] of [...files].sort(([left], [right]) =>
    left < right ? -1 : left > right ? 1 : 0)) {
    const nameBytes = Buffer.from(name, "utf8");
    const contentBytes = Buffer.from(contents, "utf8");
    const nameLength = Buffer.alloc(8);
    const contentLength = Buffer.alloc(8);
    nameLength.writeBigUInt64LE(BigInt(nameBytes.length));
    contentLength.writeBigUInt64LE(BigInt(contentBytes.length));
    digest.update(nameLength);
    digest.update(nameBytes);
    digest.update(contentLength);
    digest.update(contentBytes);
  }
  return digest.digest("hex");
}

const clientFiles = new Map([
  ["manifest.json", `${JSON.stringify({
    schema_version: 1,
    id: "client-quick",
    version: "1.0.0",
    title: "Client fixture",
    target_kinds: ["chat_gpt_client", "claude_client"],
    tasks: [{
      id: "client-task",
      category: "logic",
      prompt_file: "prompts/client.txt",
      starter_dir: null,
      time_budget_secs: 30,
      max_turns: 1,
      grader: { type: "exact_text", expected: "ok" },
    }],
  }, null, 2)}\n`],
  ["prompts/client.txt", "Return ok.\n"],
]);
const cliFiles = new Map([
  ["manifest.json", `${JSON.stringify({
    schema_version: 1,
    id: "cli-quick",
    version: "1.0.0",
    title: "CLI fixture",
    target_kinds: ["codex_cli", "claude_code"],
    tasks: [{
      id: "cli-task",
      category: "cli_coding",
      prompt_file: "tasks/cli-task/prompt.md",
      starter_dir: "tasks/cli-task/starter",
      time_budget_secs: 60,
      max_turns: 2,
      grader: { type: "external_verifier", verifier_id: "cli-task-v1" },
    }],
  }, null, 2)}\n`],
  ["tasks/cli-task/prompt.md", "Fix the fixture.\n"],
  ["tasks/cli-task/starter/index.mjs", "export const ready = false;\n"],
]);
const registry = `${JSON.stringify({
  schema_version: 1,
  packs: [
    {
      bundled: true,
      content_sha256: packContentHash(clientFiles),
      id: "client-quick",
      license: "Apache-2.0",
      path: "client-quick-v1",
      version: "1.0.0",
    },
    {
      bundled: true,
      content_sha256: packContentHash(cliFiles),
      id: "cli-quick",
      license: "Apache-2.0",
      path: "cli-quick-v1",
      version: "1.0.0",
    },
  ],
}, null, 2)}\n`;
const packContents = new Map([
  ["registry.json", registry],
  ...[...clientFiles].map(([name, contents]) => [
    `client-quick-v1/${name}`,
    contents,
  ]),
  ...[...cliFiles].map(([name, contents]) => [
    `cli-quick-v1/${name}`,
    contents,
  ]),
]);
const fileContents = new Map([
  ["README.txt", "no install\n"],
  ["ability-radar.exe", "fake-exe"],
  ...[...packContents].map(([name, contents]) => [
    `benchmark-packs/${name}`,
    contents,
  ]),
]);

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function portableArchivePath(fixture) {
  return join(
    fixture.bundleDir,
    "ability-radar_0.2.1_windows-x64-portable.zip",
  );
}

async function assertNoFinalArchive(fixture) {
  await assert.rejects(lstat(portableArchivePath(fixture)), { code: "ENOENT" });
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
  await mkdir(targetDir, { recursive: true });
  await writeFile(join(targetDir, "ability-radar.exe"), "fake-exe");
  for (const packsRoot of [
    join(repoRoot, "benchmark-packs"),
    join(targetDir, "benchmark-packs"),
  ]) {
    for (const [name, contents] of packContents) {
      const path = join(packsRoot, ...name.split("/"));
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, contents);
    }
  }
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
    await copyFile(
      extractorScript,
      join(repoRoot, "scripts", "extract-portable.ps1"),
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
      "benchmark-packs/cli-quick-v1/tasks/",
      "benchmark-packs/cli-quick-v1/tasks/cli-task/",
      "benchmark-packs/cli-quick-v1/tasks/cli-task/prompt.md",
      "benchmark-packs/cli-quick-v1/tasks/cli-task/starter/",
      "benchmark-packs/cli-quick-v1/tasks/cli-task/starter/index.mjs",
      "benchmark-packs/client-quick-v1/",
      "benchmark-packs/client-quick-v1/manifest.json",
      "benchmark-packs/client-quick-v1/prompts/",
      "benchmark-packs/client-quick-v1/prompts/client.txt",
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

test("rejects a corrupt portable registry schema before staging", async () => {
  const fixture = await createFixture();
  try {
    const registryPath = join(
      fixture.targetDir,
      "benchmark-packs",
      "registry.json",
    );
    const changed = JSON.parse(await readFile(registryPath, "utf8"));
    changed.schema_version = 2;
    await writeFile(registryPath, `${JSON.stringify(changed)}\n`);

    await assert.rejects(
      stagePortable({ ...fixture, version: "0.2.1" }),
      /registry schema|portable pack registry/i,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects manifest identity mismatches before staging", async () => {
  const fixture = await createFixture();
  try {
    const manifestPath = join(
      fixture.targetDir,
      "benchmark-packs",
      "client-quick-v1",
      "manifest.json",
    );
    const changed = JSON.parse(await readFile(manifestPath, "utf8"));
    changed.id = "stale-client";
    await writeFile(manifestPath, `${JSON.stringify(changed)}\n`);

    await assert.rejects(
      stagePortable({ ...fixture, version: "0.2.1" }),
      /manifest.*identity|registry.*mismatch/i,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects portable pack content that mismatches the registry seal", async () => {
  const fixture = await createFixture();
  try {
    await writeFile(
      join(
        fixture.targetDir,
        "benchmark-packs",
        "client-quick-v1",
        "prompts",
        "client.txt",
      ),
      "stale prompt\n",
    );

    await assert.rejects(
      stagePortable({ ...fixture, version: "0.2.1" }),
      /content.*hash|registry.*seal/i,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects missing and extra portable pack entries", async () => {
  for (const mutate of [
    async (fixture) => {
      await rm(join(
        fixture.targetDir,
        "benchmark-packs",
        "cli-quick-v1",
        "tasks",
        "cli-task",
        "prompt.md",
      ));
    },
    async (fixture) => {
      await writeFile(
        join(fixture.targetDir, "benchmark-packs", "unexpected.json"),
        "{}\n",
      );
    },
  ]) {
    const fixture = await createFixture();
    try {
      await mutate(fixture);
      await assert.rejects(
        stagePortable({ ...fixture, version: "0.2.1" }),
        /missing|exact.*pack|unexpected.*pack|content.*hash/i,
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  }
});

test("rejects traversal-shaped registry and manifest entries", async () => {
  for (const mutate of [
    async (fixture) => {
      const registryPath = join(
        fixture.targetDir,
        "benchmark-packs",
        "registry.json",
      );
      const changed = JSON.parse(await readFile(registryPath, "utf8"));
      changed.packs[0].path = "../client-quick-v1";
      await writeFile(registryPath, `${JSON.stringify(changed)}\n`);
    },
    async (fixture) => {
      const manifestPath = join(
        fixture.targetDir,
        "benchmark-packs",
        "client-quick-v1",
        "manifest.json",
      );
      const changed = JSON.parse(await readFile(manifestPath, "utf8"));
      changed.tasks[0].prompt_file = "../registry.json";
      await writeFile(manifestPath, `${JSON.stringify(changed)}\n`);
    },
  ]) {
    const fixture = await createFixture();
    try {
      await mutate(fixture);
      await assert.rejects(
        stagePortable({ ...fixture, version: "0.2.1" }),
        /unsafe.*path|traversal|registry.*path/i,
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
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
      /indirection|reparse|symbolic link|exact two portable pack/i,
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
      /indirection|reparse|symbolic link|exact two portable pack/i,
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
      /indirection|reparse|symbolic link|exact two portable pack/i,
    );
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("rejects a junctioned bundle root before staging", async () => {
  const fixture = await createFixture();
  try {
    const outside = join(fixture.root, "outside-bundle");
    await mkdir(outside);
    await mkdir(dirname(fixture.bundleDir), { recursive: true });
    await symlink(outside, fixture.bundleDir, "junction");
    await assert.rejects(
      stagePortable({ ...fixture, version: "0.2.1" }),
      /indirection|reparse|symbolic link|ownership/i,
    );
    assert.deepEqual(await readdir(outside), []);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("leaves an unknown fixed .stage junction untouched", async () => {
  const fixture = await createFixture();
  try {
    const outside = join(fixture.root, "outside-stage");
    await mkdir(outside);
    await writeFile(join(outside, "owner.txt"), "preserve\n");
    await mkdir(fixture.bundleDir, { recursive: true });
    const fixedStage = join(fixture.bundleDir, ".stage");
    await symlink(outside, fixedStage, "junction");

    const staged = await stagePortable({ ...fixture, version: "0.2.1" });

    assert.notEqual(staged.stageParent, fixedStage);
    assert.equal((await lstat(fixedStage)).isSymbolicLink(), true);
    assert.equal(await readFile(join(outside, "owner.txt"), "utf8"), "preserve\n");
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("preserves an unknown pre-existing fixed .stage directory", async () => {
  const fixture = await createFixture();
  try {
    const fixedStage = join(fixture.bundleDir, ".stage");
    await mkdir(fixedStage, { recursive: true });
    await writeFile(join(fixedStage, "owner.txt"), "preserve\n");

    const staged = await stagePortable({ ...fixture, version: "0.2.1" });

    assert.notEqual(staged.stageParent, fixedStage);
    assert.equal(await readFile(join(fixedStage, "owner.txt"), "utf8"), "preserve\n");
    assert.equal((await lstat(staged.stageRoot)).isDirectory(), true);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("concurrent staging invocations use isolated owned directories", async () => {
  const fixture = await createFixture();
  try {
    const [first, second] = await Promise.all([
      stagePortable({ ...fixture, version: "0.2.1" }),
      stagePortable({ ...fixture, version: "0.2.1" }),
    ]);

    assert.notEqual(first.stageParent, second.stageParent);
    assert.notEqual(first.stageRoot, second.stageRoot);
    assert.equal((await lstat(first.stageRoot)).isDirectory(), true);
    assert.equal((await lstat(second.stageRoot)).isDirectory(), true);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
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
  "packaging failure cleans only its invocation-owned staging",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const fixedStage = join(fixture.bundleDir, ".stage");
      await mkdir(fixedStage, { recursive: true });
      await writeFile(join(fixedStage, "owner.txt"), "preserve\n");
      await writeFile(
        join(fixture.repoRoot, "scripts", "compress-portable.ps1"),
        "param([string]$Source, [string]$Destination)\nexit 9\n",
      );

      const result = runCli(fixture.repoRoot);

      assert.notEqual(result.status, 0);
      assert.equal(await readFile(join(fixedStage, "owner.txt"), "utf8"), "preserve\n");
      assert.deepEqual(await readdir(fixture.bundleDir), [".stage"]);
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

test(
  "real temporary cleanup failure after publication still cleans stage",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      let caught;
      try {
        await portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase, temporaryArchive }) => {
            if (phase !== "afterLink") return;
            await rm(temporaryArchive);
            await mkdir(temporaryArchive);
          },
        );
      } catch (error) {
        caught = error;
      }
      assert.ok(caught, "cleanup failure must reject packaging");
      assert.match(caught.message, /published.*cleanup incomplete/i);
      assert.doesNotMatch(caught.message, new RegExp(escapeRegExp(fixture.root)));
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.1_windows-x64-portable.zip",
      );
      assert.deepEqual(
        [...(await readFile(archivePath)).subarray(0, 2)],
        [0x50, 0x4b],
      );
      await assert.rejects(lstat(join(fixture.bundleDir, ".stage")), {
        code: "ENOENT",
      });
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "published archive aggregates simultaneous cleanup failures without paths",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      let caught;
      try {
        await portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase, temporaryArchive, stageParent }) => {
            if (phase !== "afterLink") return;
            await rm(temporaryArchive);
            await mkdir(temporaryArchive);
            await rm(stageParent, { recursive: true });
            await writeFile(stageParent, "blocks directory cleanup");
          },
        );
      } catch (error) {
        caught = error;
      }
      assert.ok(caught, "cleanup failures must reject packaging");
      assert.match(caught.message, /published.*cleanup incomplete.*2 operations/i);
      assert.doesNotMatch(caught.message, new RegExp(escapeRegExp(fixture.root)));
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.1_windows-x64-portable.zip",
      );
      assert.deepEqual(
        [...(await readFile(archivePath)).subarray(0, 2)],
        [0x50, 0x4b],
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "pre-publication primary failure survives simultaneous cleanup failures",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      let caught;
      try {
        await portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase, temporaryArchive, stageParent }) => {
            if (phase !== "beforeLink") return;
            await rm(temporaryArchive);
            await mkdir(temporaryArchive);
            await rm(stageParent, { recursive: true });
            await writeFile(stageParent, "blocks directory cleanup");
            throw new Error("primary publication-boundary failure");
          },
        );
      } catch (error) {
        caught = error;
      }
      assert.ok(caught, "primary failure must reject packaging");
      assert.match(caught.message, /processing failed/i);
      assert.match(caught.message, /cleanup incomplete.*2 operations/i);
      assert.doesNotMatch(caught.message, new RegExp(escapeRegExp(fixture.root)));
      assert.match(caught.cause?.message ?? "", /primary publication-boundary failure/i);
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.1_windows-x64-portable.zip",
      );
      await assert.rejects(lstat(archivePath), { code: "ENOENT" });
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

for (const [name, mutateStage] of [
  [
    "modified staged payload",
    async ({ stageRoot }) => {
      await writeFile(join(stageRoot, "README.txt"), "tampered payload\n");
    },
  ],
  [
    "extra staged file",
    async ({ stageRoot }) => {
      await writeFile(join(stageRoot, "unexpected.txt"), "unexpected\n");
    },
  ],
  [
    "modified checksum manifest",
    async ({ stageRoot }) => {
      await writeFile(join(stageRoot, "SHA256SUMS.txt"), "forged checksums\n");
    },
  ],
]) {
  test(
    `archive verification rejects ${name} before publication`,
    { skip: process.platform !== "win32" },
    async () => {
      const fixture = await createFixture({ cli: true });
      try {
        const portable = await import("./package-portable.mjs");
        await assert.rejects(
          portable.packagePortableFromBuildForTest(
            fixture.repoRoot,
            async (context) => {
              if (context.phase === "beforeCompression") {
                await mutateStage(context);
              }
            },
          ),
          /archive verification|processing failed/i,
        );
        await assertNoFinalArchive(fixture);
        assert.deepEqual(await readdir(fixture.bundleDir), []);
      } finally {
        await rm(fixture.root, { recursive: true, force: true });
      }
    },
  );
}

test(
  "staged pack tampering is rejected before the compressor starts",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const compressorMarker = join(fixture.root, "compressor-started.txt");
      const escapedMarker = compressorMarker.replaceAll("'", "''");
      await writeFile(
        join(fixture.repoRoot, "scripts", "compress-portable.ps1"),
        `param([string]$Source, [string]$Destination)\nSet-Content -LiteralPath '${escapedMarker}' -Value started\nthrow 'compressor must not start'\n`,
      );
      const portable = await import("./package-portable.mjs");
      await assert.rejects(
        portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase, stageRoot }) => {
            if (phase !== "beforeCompression") return;
            await writeFile(
              join(
                stageRoot,
                "benchmark-packs",
                "client-quick-v1",
                "prompts",
                "client.txt",
              ),
              "tampered staged prompt\n",
            );
          },
        ),
        /processing failed/i,
      );
      await assert.rejects(lstat(compressorMarker), { code: "ENOENT" });
      await assertNoFinalArchive(fixture);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "archive verification rejects a raced staged subtree junction",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    const external = join(fixture.root, "external-client-pack");
    try {
      await mkdir(external);
      await writeFile(join(external, "manifest.json"), '{"external":true}\n');
      await writeFile(join(external, "marker.txt"), "attacker-owned\n");
      const portable = await import("./package-portable.mjs");
      await assert.rejects(
        portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase, stageRoot }) => {
            if (phase !== "beforeCompression") return;
            const clientPack = join(
              stageRoot,
              "benchmark-packs",
              "client-quick-v1",
            );
            await rm(clientPack, { recursive: true });
            await symlink(external, clientPack, "junction");
          },
        ),
        /archive verification|processing failed|cleanup incomplete/i,
      );
      await assertNoFinalArchive(fixture);
      assert.equal(await readFile(join(external, "marker.txt"), "utf8"), "attacker-owned\n");
      const entries = await readdir(fixture.bundleDir);
      assert.equal(entries.some((entry) => entry.endsWith(".tmp.zip")), false);
      assert.equal(entries.some((entry) => entry.startsWith(".verify.")), false);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "identity-replaced stage is left untouched while other cleanup completes",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    const external = join(fixture.root, "attacker-stage");
    try {
      await mkdir(external);
      await writeFile(join(external, "marker.txt"), "do-not-delete\n");
      const portable = await import("./package-portable.mjs");
      let verificationDirectory;
      let temporaryArchive;
      let stageParent;
      let caught;
      try {
        await portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async (context) => {
            if (context.phase !== "beforeCompression") return;
            ({ verificationDirectory, temporaryArchive } = context);
            stageParent = context.stageParent;
            await rm(context.stageParent, { recursive: true });
            await symlink(external, context.stageParent, "junction");
            throw new Error("simulated post-staging validation failure");
          },
        );
      } catch (error) {
        caught = error;
      }
      assert.ok(caught);
      assert.match(caught.message, /processing failed.*cleanup incomplete/i);
      assert.doesNotMatch(caught.message, new RegExp(escapeRegExp(fixture.root)));
      assert.equal(await readFile(join(external, "marker.txt"), "utf8"), "do-not-delete\n");
      assert.equal((await lstat(stageParent)).isSymbolicLink(), true);
      await assert.rejects(lstat(verificationDirectory), { code: "ENOENT" });
      await assert.rejects(lstat(temporaryArchive), { code: "ENOENT" });
      await assertNoFinalArchive(fixture);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test(
  "published processing and three cleanup failures are all reported",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      let caught;
      try {
        await portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({
            phase,
            temporaryArchive,
            verificationDirectory,
            stageParent,
          }) => {
            if (phase !== "afterLink") return;
            await rm(temporaryArchive);
            await mkdir(temporaryArchive);
            await rm(verificationDirectory, { recursive: true });
            await writeFile(verificationDirectory, "blocks verification cleanup");
            await rm(stageParent, { recursive: true });
            await writeFile(stageParent, "blocks stage cleanup");
            throw new Error("simulated post-publication processing failure");
          },
        );
      } catch (error) {
        caught = error;
      }
      assert.ok(caught);
      assert.match(caught.message, /published.*processing failed.*cleanup incomplete.*3/i);
      assert.doesNotMatch(caught.message, new RegExp(escapeRegExp(fixture.root)));
      assert.deepEqual(
        [...(await readFile(portableArchivePath(fixture))).subarray(0, 2)],
        [0x50, 0x4b],
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);
