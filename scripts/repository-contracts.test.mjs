import assert from "node:assert/strict";
import {
  cpSync,
  mkdtempSync,
  readFileSync,
  rmSync,
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

function runNegativeFixture(mutate) {
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
    return spawnSync(process.execPath, [validator], {
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

test("preview CTA must target the exact v0.2.0 release tag", () => {
  const result = runNegativeFixture((fixture) => {
    replace(join(fixture, "site", "index.html"), (source) =>
      source.replace(
        /\/releases\/tag\/v0\.2\.0|\/releases\/latest/g,
        "/releases/latest",
      ));
  });
  assertRejected(result, /releases\/tag\/v0\.2\.0|releases\/latest/);
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
