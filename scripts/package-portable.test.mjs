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
const validatorLeaf = process.platform === "win32"
  ? "ability-pack-validator.exe"
  : "ability-pack-validator";
const builtValidator = join(
  scriptsDir,
  "..",
  "target",
  "debug",
  validatorLeaf,
);

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
    "ability-radar_0.2.2_windows-x64-portable.zip",
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
  await copyFile(builtValidator, join(targetDir, validatorLeaf));
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
      '{"name":"portable-fixture","version":"0.2.2","private":true}\n',
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

function zipMutationCompressorScript({
  addEntry,
  deleteEntry,
  directory = false,
}) {
  const mutation = addEntry
    ? [
        `$entry = $archive.CreateEntry('${addEntry.replaceAll("'", "''")}')`,
        ...(directory
          ? []
          : [
              "$stream = $entry.Open()",
              "try {",
              "  $bytes = [System.Text.Encoding]::UTF8.GetBytes('unexpected')",
              "  $stream.Write($bytes, 0, $bytes.Length)",
              "} finally {",
              "  $stream.Dispose()",
              "}",
            ]),
      ]
    : [
        `$deleteName = '${deleteEntry.replaceAll("'", "''")}'`,
        "$entry = $archive.Entries | Where-Object {",
        "  $_.FullName.Replace('\\', '/') -ceq $deleteName",
        "} | Select-Object -First 1",
        "if ($null -eq $entry) { throw 'entry to delete was not found' }",
        "$entry.Delete()",
      ];
  return [
    "param([string]$Source, [string]$Destination)",
    "$ErrorActionPreference = 'Stop'",
    "Compress-Archive -LiteralPath $Source -DestinationPath $Destination -CompressionLevel Optimal",
    "Add-Type -AssemblyName System.IO.Compression.FileSystem",
    "$archive = [System.IO.Compression.ZipFile]::Open(",
    "  $Destination,",
    "  [System.IO.Compression.ZipArchiveMode]::Update",
    ")",
    "try {",
    ...mutation.map((line) => `  ${line}`),
    "} finally {",
    "  $archive.Dispose()",
    "}",
    "",
  ].join("\n");
}

function archiveCommentCompressorScript({ ambiguous = false } = {}) {
  const appendComment = ambiguous
    ? [
        "$comment = [byte[]]@(",
        "  0x41, 0x41, 0x50, 0x4b, 0x05, 0x06,",
        "  0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41,",
        "  0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41,",
        "  0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41, 0x41,",
        "  0x41, 0x41",
        ")",
        "$commentLength = [uint16]$comment.Length",
        "[Buffer]::BlockCopy([BitConverter]::GetBytes($commentLength), 0, $bytes, $endOffset + 20, 2)",
        "$output = New-Object byte[] ($bytes.Length + $comment.Length)",
        "[Buffer]::BlockCopy($bytes, 0, $output, 0, $bytes.Length)",
        "[Buffer]::BlockCopy($comment, 0, $output, $bytes.Length, $comment.Length)",
      ]
    : [
        "$comment = [Text.Encoding]::UTF8.GetBytes('legacy archive comment')",
        "$commentLength = [uint16]$comment.Length",
        "[Buffer]::BlockCopy([BitConverter]::GetBytes($commentLength), 0, $bytes, $endOffset + 20, 2)",
        "$output = New-Object byte[] ($bytes.Length + $comment.Length)",
        "[Buffer]::BlockCopy($bytes, 0, $output, 0, $bytes.Length)",
        "[Buffer]::BlockCopy($comment, 0, $output, $bytes.Length, $comment.Length)",
      ];
  return [
    "param([string]$Source, [string]$Destination)",
    "$ErrorActionPreference = 'Stop'",
    "Compress-Archive -LiteralPath $Source -DestinationPath $Destination -CompressionLevel Optimal",
    "$bytes = [IO.File]::ReadAllBytes($Destination)",
    "$endOffset = $bytes.Length - 22",
    "if ([BitConverter]::ToUInt32($bytes, $endOffset) -ne 0x06054b50) {",
    "  throw 'standard compressor did not emit a classic EOCD at EOF'",
    "}",
    ...appendComment,
    "[IO.File]::WriteAllBytes($Destination, $output)",
    "",
  ].join("\n");
}

function minimalZip({
  centralCompressedSize = 0,
  centralUncompressedSize = 0,
  diskNumber = 0,
  flags = 0,
  method = 0,
  nameBytes = Buffer.from("ability-radar-portable/README.txt", "ascii"),
  versionNeeded = 20,
}) {
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(versionNeeded, 4);
  local.writeUInt16LE(flags, 6);
  local.writeUInt16LE(method, 8);
  local.writeUInt32LE(centralCompressedSize, 18);
  local.writeUInt32LE(centralUncompressedSize, 22);
  local.writeUInt16LE(nameBytes.length, 26);

  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50, 0);
  central.writeUInt16LE(20, 4);
  central.writeUInt16LE(versionNeeded, 6);
  central.writeUInt16LE(flags, 8);
  central.writeUInt16LE(method, 10);
  central.writeUInt32LE(centralCompressedSize, 20);
  central.writeUInt32LE(centralUncompressedSize, 24);
  central.writeUInt16LE(nameBytes.length, 28);

  const centralOffset = local.length + nameBytes.length;
  const centralSize = central.length + nameBytes.length;
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(diskNumber, 4);
  end.writeUInt16LE(diskNumber, 6);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(centralSize, 12);
  end.writeUInt32LE(centralOffset, 16);

  return Buffer.concat([local, nameBytes, central, nameBytes, end]);
}

function rawClientManifest({
  grader = '{"type":"exact_text","expected":"ok"}',
  maxTurns = "1",
  schema = "1",
  timeBudget = "30",
} = {}) {
  return `{"schema_version":${schema},"id":"client-quick","version":"1.0.0","title":"Client fixture","target_kinds":["chat_gpt_client","claude_client"],"tasks":[{"id":"client-task","category":"logic","prompt_file":"prompts/client.txt","starter_dir":null,"time_budget_secs":${timeBudget},"max_turns":${maxTurns},"grader":${grader}}]}\n`;
}

async function installResealedClientManifest(fixture, contents) {
  const manifestPath = join(
    fixture.targetDir,
    "benchmark-packs",
    "client-quick-v1",
    "manifest.json",
  );
  await writeFile(manifestPath, contents);
  const resealedFiles = new Map(clientFiles);
  resealedFiles.set("manifest.json", contents);
  const contentSeal = packContentHash(resealedFiles);
  for (const registryPath of [
    join(fixture.repoRoot, "benchmark-packs", "registry.json"),
    join(fixture.targetDir, "benchmark-packs", "registry.json"),
  ]) {
    const value = JSON.parse(await readFile(registryPath, "utf8"));
    value.packs.find(({ id }) => id === "client-quick").content_sha256 =
      contentSeal;
    await writeFile(registryPath, `${JSON.stringify(value, null, 2)}\n`);
  }
}

test("stages the exact rooted tree and complete deterministic checksums", async () => {
  const fixture = await createFixture();
  try {
    const first = await stagePortable({ ...fixture, version: "0.2.2" });
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
    assert.equal(
      (await entriesUnder(first.stageRoot))
        .some((entry) => entry.includes("ability-pack-validator")),
      false,
    );
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

    const second = await stagePortable({ ...fixture, version: "0.2.2" });
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
      stagePortable({ ...fixture, version: "0.2.2" }),
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
      stagePortable({ ...fixture, version: "0.2.2" }),
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
      stagePortable({ ...fixture, version: "0.2.2" }),
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
        stagePortable({ ...fixture, version: "0.2.2" }),
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
        stagePortable({ ...fixture, version: "0.2.2" }),
        /unsafe.*path|traversal|registry.*path/i,
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  }
});

for (const [name, manifest] of [
  ["UTF-8 BOM", `\uFEFF${rawClientManifest()}`],
  [
    "duplicate object keys",
    rawClientManifest({ schema: "2,\"schema_version\":1" }),
  ],
  ["fractional integer lexemes", rawClientManifest({ maxTurns: "1.0" })],
  ["exponent integer lexemes", rawClientManifest({ timeBudget: "3e1" })],
  [
    "overflowing integers",
    rawClientManifest({ timeBudget: "18446744073709551616" }),
  ],
  [
    "non-finite exact JSON numbers",
    rawClientManifest({
      grader: '{"type":"exact_json","expected":1e400}',
    }),
  ],
  ["time budgets below range", rawClientManifest({ timeBudget: "0" })],
  ["time budgets above range", rawClientManifest({ timeBudget: "7201" })],
  ["turn counts below range", rawClientManifest({ maxTurns: "0" })],
  ["turn counts above range", rawClientManifest({ maxTurns: "101" })],
]) {
  test(`rejects ${name} with the runtime pack parser before staging`, async () => {
    const fixture = await createFixture();
    try {
      await installResealedClientManifest(fixture, manifest);

      await assert.rejects(
        stagePortable({ ...fixture, version: "0.2.2" }),
        /runtime pack parser|portable pack|manifest|JSON/i,
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  });
}

test("accepts the inclusive runtime task range endpoints", async () => {
  const fixture = await createFixture();
  try {
    await installResealedClientManifest(
      fixture,
      rawClientManifest({ timeBudget: "7200", maxTurns: "100" }),
    );

    const staged = await stagePortable({ ...fixture, version: "0.2.2" });
    assert.equal((await lstat(staged.stageRoot)).isDirectory(), true);
  } finally {
    await rm(fixture.root, { recursive: true, force: true });
  }
});

test("fails closed when the runtime pack validator is missing or cannot execute", async () => {
  for (const mutate of [
    (path) => rm(path),
    (path) => writeFile(path, "not an executable"),
  ]) {
    const fixture = await createFixture();
    try {
      const validatorPath = join(fixture.targetDir, validatorLeaf);
      await mutate(validatorPath);
      await assert.rejects(
        stagePortable({ ...fixture, version: "0.2.2" }),
        /runtime pack validator|runtime pack parser/i,
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  }
});

test("portable scripts build only the first-party runtime validator from the existing app build", async () => {
  const manifest = JSON.parse(
    await readFile(join(scriptsDir, "..", "package.json"), "utf8"),
  );
  assert.match(
    manifest.scripts["package:portable:from-build"],
    /^cargo build -p ability-core --bin ability-pack-validator --release --locked --offline && node scripts\/package-portable\.mjs$/,
  );
  assert.doesNotMatch(
    manifest.scripts["package:portable:from-build"],
    /tauri|ability-radar\.exe/i,
  );
});

test("refuses an output directory outside the selected target tree", async () => {
  await assert.rejects(
    stagePortable({
      repoRoot: "C:\\repo",
      targetDir: "C:\\repo\\target\\release",
      bundleDir: "C:\\outside",
      version: "0.2.2",
    }),
    /inside target directory/,
  );
});

test("rejects invalid or path-shaped versions inside stagePortable", async () => {
  const fixture = await createFixture();
  try {
    for (const version of [
      "../0.2.2",
      "0.2.2/escape",
      "0.2.2\\escape",
      "v0.2.2",
      "01.2.3",
      "0.2",
      "0.2.2-beta",
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
      stagePortable({ ...fixture, version: "0.2.2" }),
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
      stagePortable({ ...fixture, version: "0.2.2" }),
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
      stagePortable({ ...fixture, version: "0.2.2" }),
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
      stagePortable({ ...fixture, version: "0.2.2" }),
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

    const staged = await stagePortable({ ...fixture, version: "0.2.2" });

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

    const staged = await stagePortable({ ...fixture, version: "0.2.2" });

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
      stagePortable({ ...fixture, version: "0.2.2" }),
      stagePortable({ ...fixture, version: "0.2.2" }),
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
  "standard Windows compressor produces a comment-free archive accepted by raw validation",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const result = runCli(fixture.repoRoot);
      assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.2_windows-x64-portable.zip",
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
      const archive = await readFile(archivePath);
      const endOffset = archive.length - 22;
      assert.equal(archive.readUInt32LE(endOffset), 0x06054b50);
      assert.equal(archive.readUInt16LE(endOffset + 20), 0);
      await assert.rejects(lstat(join(fixture.bundleDir, ".stage")), {
        code: "ENOENT",
      });
      assert.deepEqual(
        (await readdir(fixture.bundleDir)).filter((name) => name !==
          "ability-radar_0.2.2_windows-x64-portable.zip"),
        [],
      );
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

for (const [name, compressor] of [
  [
    "a nonzero classic archive comment",
    archiveCommentCompressorScript(),
  ],
  [
    "an archive comment containing an EOCD-like byte sequence",
    archiveCommentCompressorScript({ ambiguous: true }),
  ],
]) {
  test(
    `raw ZIP validation rejects ${name} before extractor spawn`,
    { skip: process.platform !== "win32" },
    async () => {
      const fixture = await createFixture({ cli: true });
      try {
        const preservedStage = join(fixture.bundleDir, ".stage");
        await mkdir(preservedStage, { recursive: true });
        await writeFile(join(preservedStage, "owner.txt"), "preserve\n");
        await writeFile(
          join(fixture.repoRoot, "scripts", "compress-portable.ps1"),
          compressor,
        );
        const extractorMarker = join(fixture.root, "extractor-started.txt");
        await writeFile(
          join(fixture.repoRoot, "scripts", "extract-portable.ps1"),
          [
            "param([string]$Source, [string]$Destination)",
            `Set-Content -LiteralPath '${extractorMarker.replaceAll("'", "''")}' -Value started`,
            "throw 'extractor must not start for an invalid raw ZIP'",
            "",
          ].join("\n"),
        );

        const portable = await import("./package-portable.mjs");
        await assert.rejects(
          portable.packagePortableFromBuildForTest(fixture.repoRoot),
          /portable packaging processing failed|raw ZIP/i,
        );

        await assertNoFinalArchive(fixture);
        assert.equal(
          await readFile(join(preservedStage, "owner.txt"), "utf8"),
          "preserve\n",
        );
        assert.deepEqual(await readdir(fixture.bundleDir), [".stage"]);
        await assert.rejects(lstat(extractorMarker), { code: "ENOENT" });
      } finally {
        await rm(fixture.root, { recursive: true, force: true });
      }
    },
  );
}

test(
  "a successful package invokes the real runtime parser at all four checkpoints",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    const portable = await import("./package-portable.mjs");
    const labels = [];
    portable.observeRuntimePackValidatorForTest((label) => labels.push(label));
    try {
      await portable.packagePortableFromBuildForTest(fixture.repoRoot);
      assert.deepEqual(labels, [
        "source portable benchmark packs",
        "staged portable benchmark packs",
        "pre-compression portable benchmark packs",
        "extracted portable benchmark packs",
      ]);
    } finally {
      portable.observeRuntimePackValidatorForTest(undefined);
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
  "afterChecksums failure cleans initialized resources without a false cleanup warning",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      const portable = await import("./package-portable.mjs");
      let caught;
      try {
        await portable.packagePortableFromBuildForTest(
          fixture.repoRoot,
          async ({ phase }) => {
            if (phase === "afterChecksums") {
              throw new Error("simulated early packaging failure");
            }
          },
        );
      } catch (error) {
        caught = error;
      }

      assert.ok(caught, "early packaging failure must reject");
      assert.match(caught.message, /portable packaging processing failed/i);
      assert.doesNotMatch(caught.message, /cleanup incomplete/i);
      await assertNoFinalArchive(fixture);
      assert.deepEqual(await readdir(fixture.bundleDir), []);
    } finally {
      await rm(fixture.root, { recursive: true, force: true });
    }
  },
);

test("raw ZIP parser fails closed on unsupported encodings and structures", async () => {
  const portable = await import("./package-portable.mjs");
  assert.equal(typeof portable.validateRawZipForTest, "function");
  const expectedEntries = [{ name: "README.txt", directory: false }];
  const invalidArchives = [
    minimalZip({ nameBytes: Buffer.from([0xff]) }),
    minimalZip({
      centralCompressedSize: 0xffffffff,
      centralUncompressedSize: 0xffffffff,
      versionNeeded: 45,
    }),
    minimalZip({ method: 99 }),
    minimalZip({ diskNumber: 1 }),
  ];

  for (const archive of invalidArchives) {
    assert.throws(
      () => portable.validateRawZipForTest(archive, expectedEntries),
      /raw ZIP/i,
    );
  }
});

for (const [name, mutation] of [
  [
    "an NTFS alternate-data-stream component",
    { addEntry: "ability-radar-portable/README.txt:secret" },
  ],
  [
    "a dot-segment component",
    { addEntry: "ability-radar-portable/benchmark-packs/../escape.txt" },
  ],
  [
    "a trailing-dot component",
    { addEntry: "ability-radar-portable/trailing./payload.txt" },
  ],
  [
    "a trailing-space component",
    { addEntry: "ability-radar-portable/trailing /payload.txt" },
  ],
  [
    "a DOS reserved-name alias",
    { addEntry: "ability-radar-portable/CON.txt" },
  ],
  [
    "a case-normalized collision",
    { addEntry: "ability-radar-portable/README.TXT" },
  ],
  [
    "a file-directory alias",
    { addEntry: "ability-radar-portable/README.txt/", directory: true },
  ],
  [
    "an exact duplicate destination",
    { addEntry: "ability-radar-portable/README.txt" },
  ],
  [
    "an unexpected otherwise-safe file",
    { addEntry: "ability-radar-portable/unexpected.txt" },
  ],
  [
    "a missing expected file",
    { deleteEntry: "ability-radar-portable/README.txt" },
  ],
]) {
  test(
    `raw ZIP validation rejects ${name} before extraction`,
    { skip: process.platform !== "win32" },
    async () => {
      const fixture = await createFixture({ cli: true });
      try {
        const preservedStage = join(fixture.bundleDir, ".stage");
        await mkdir(preservedStage, { recursive: true });
        await writeFile(join(preservedStage, "owner.txt"), "preserve\n");
        await writeFile(
          join(fixture.repoRoot, "scripts", "compress-portable.ps1"),
          zipMutationCompressorScript(mutation),
        );
        const extractorMarker = join(fixture.root, "extractor-started.txt");
        await writeFile(
          join(fixture.repoRoot, "scripts", "extract-portable.ps1"),
          [
            "param([string]$Source, [string]$Destination)",
            `Set-Content -LiteralPath '${extractorMarker.replaceAll("'", "''")}' -Value started`,
            "throw 'extractor must not start for an invalid raw ZIP'",
            "",
          ].join("\n"),
        );

        const portable = await import("./package-portable.mjs");
        await assert.rejects(
          portable.packagePortableFromBuildForTest(fixture.repoRoot),
          /portable packaging processing failed|raw ZIP/i,
        );

        await assertNoFinalArchive(fixture);
        assert.equal(
          await readFile(join(preservedStage, "owner.txt"), "utf8"),
          "preserve\n",
        );
        assert.deepEqual(await readdir(fixture.bundleDir), [".stage"]);
        await assert.rejects(lstat(extractorMarker), { code: "ENOENT" });
      } finally {
        await rm(fixture.root, { recursive: true, force: true });
      }
    },
  );
}

test(
  "existing final archive is preserved and never overwritten",
  { skip: process.platform !== "win32" },
  async () => {
    const fixture = await createFixture({ cli: true });
    try {
      await mkdir(fixture.bundleDir, { recursive: true });
      const archivePath = join(
        fixture.bundleDir,
        "ability-radar_0.2.2_windows-x64-portable.zip",
      );
      await writeFile(archivePath, "existing-final");
      const result = runCli(fixture.repoRoot);
      assert.notEqual(result.status, 0);
      assert.equal(await readFile(archivePath, "utf8"), "existing-final");
      assert.deepEqual(await readdir(fixture.bundleDir), [
        "ability-radar_0.2.2_windows-x64-portable.zip",
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
        "ability-radar_0.2.2_windows-x64-portable.zip",
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
        "ability-radar_0.2.2_windows-x64-portable.zip",
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
        "ability-radar_0.2.2_windows-x64-portable.zip",
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
        "ability-radar_0.2.2_windows-x64-portable.zip",
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
        "ability-radar_0.2.2_windows-x64-portable.zip",
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
