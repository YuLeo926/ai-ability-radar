import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, extname, join, normalize, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import {
  actionSteps,
  exactPermissions,
  hasRunCommand,
  parseWorkflow,
  runSteps,
} from "./workflow-contracts.mjs";

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = process.env.REPOSITORY_ROOT
  ? resolve(process.env.REPOSITORY_ROOT)
  : defaultRoot;
const errors = [];

function fail(message) {
  errors.push(message);
}

function read(path) {
  const absolute = join(root, path);
  if (!existsSync(absolute)) {
    fail(`missing required file: ${path}`);
    return "";
  }
  return readFileSync(absolute, "utf8").replace(/^\uFEFF/, "");
}

function json(path) {
  const source = read(path);
  if (!source) return {};
  try {
    return JSON.parse(source);
  } catch (error) {
    fail(`${path} is not valid JSON: ${error.message}`);
    return {};
  }
}

function requireText(path, patterns) {
  const source = read(path);
  for (const [label, pattern] of patterns) {
    if (!pattern.test(source)) fail(`${path} is missing ${label}`);
  }
  return source;
}

function stripTomlComment(line) {
  let quote;
  let escaped = false;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (quote === '"') {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        quote = undefined;
      }
      continue;
    }
    if (quote === "'") {
      if (character === "'") quote = undefined;
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
    } else if (character === "#") {
      return line.slice(0, index);
    }
  }
  return line;
}

function tomlSection(source, sectionName) {
  const lines = [];
  let active = false;
  for (const rawLine of source.replace(/\r\n?/g, "\n").split("\n")) {
    const line = stripTomlComment(rawLine);
    const header = line.trim().match(/^\[([^\]]+)\]$/);
    if (header) {
      active = header[1] === sectionName;
      continue;
    }
    if (active) lines.push(line);
  }
  return lines.join("\n");
}

function tomlBasicKey(source, start) {
  let value = "";
  let cursor = start + 1;
  const escapes = new Map([
    ['"', '"'],
    ["\\", "\\"],
    ["b", "\b"],
    ["t", "\t"],
    ["n", "\n"],
    ["f", "\f"],
    ["r", "\r"],
  ]);
  while (cursor < source.length) {
    const character = source[cursor];
    if (character === '"') {
      return { valid: true, value, cursor: cursor + 1 };
    }
    if (character === "\\") {
      const escape = source[cursor + 1];
      if (escapes.has(escape)) {
        value += escapes.get(escape);
        cursor += 2;
        continue;
      }
      if (escape === "u" || escape === "U") {
        const digits = escape === "u" ? 4 : 8;
        const hex = source.slice(cursor + 2, cursor + 2 + digits);
        if (
          hex.length !== digits ||
          !/^[0-9A-Fa-f]+$/.test(hex)
        ) {
          return { valid: false };
        }
        const codePoint = Number.parseInt(hex, 16);
        if (
          codePoint > 0x10ffff ||
          (codePoint >= 0xd800 && codePoint <= 0xdfff)
        ) {
          return { valid: false };
        }
        value += String.fromCodePoint(codePoint);
        cursor += digits + 2;
        continue;
      }
      return { valid: false };
    }
    const codePoint = source.codePointAt(cursor);
    if ((codePoint < 0x20 && codePoint !== 0x09) || codePoint === 0x7f) {
      return { valid: false };
    }
    value += String.fromCodePoint(codePoint);
    cursor += codePoint > 0xffff ? 2 : 1;
  }
  return { valid: false };
}

function tomlLiteralKey(source, start) {
  let value = "";
  let cursor = start + 1;
  while (cursor < source.length) {
    const character = source[cursor];
    if (character === "'") {
      return { valid: true, value, cursor: cursor + 1 };
    }
    const codePoint = source.codePointAt(cursor);
    if ((codePoint < 0x20 && codePoint !== 0x09) || codePoint === 0x7f) {
      return { valid: false };
    }
    value += String.fromCodePoint(codePoint);
    cursor += codePoint > 0xffff ? 2 : 1;
  }
  return { valid: false };
}

function tomlDottedKey(source) {
  const segments = [];
  let cursor = 0;
  const skipWhitespace = () => {
    while (source[cursor] === " " || source[cursor] === "\t") cursor += 1;
  };
  skipWhitespace();
  while (cursor < source.length) {
    let segment;
    if (source[cursor] === '"') {
      segment = tomlBasicKey(source, cursor);
    } else if (source[cursor] === "'") {
      segment = tomlLiteralKey(source, cursor);
    } else {
      const match = source.slice(cursor).match(/^[A-Za-z0-9_-]+/);
      if (!match) return { valid: false };
      segment = {
        valid: true,
        value: match[0],
        cursor: cursor + match[0].length,
      };
    }
    if (!segment.valid) return { valid: false };
    segments.push(segment.value);
    cursor = segment.cursor;
    skipWhitespace();
    if (cursor === source.length) {
      return { valid: segments.length > 0, segments };
    }
    if (source[cursor] !== ".") return { valid: false };
    cursor += 1;
    skipWhitespace();
    if (cursor === source.length) return { valid: false };
  }
  return { valid: false };
}

function tomlTableHeader(line) {
  const trimmed = stripTomlComment(line).trim();
  if (!trimmed.startsWith("[")) return { present: false };
  let array = false;
  let keySource;
  if (trimmed.startsWith("[[")) {
    if (
      trimmed.startsWith("[[[") ||
      !trimmed.endsWith("]]") ||
      trimmed.endsWith("]]]")
    ) {
      return { present: true, valid: false, raw: trimmed };
    }
    array = true;
    keySource = trimmed.slice(2, -2);
  } else {
    if (!trimmed.endsWith("]") || trimmed.endsWith("]]")) {
      return { present: true, valid: false, raw: trimmed };
    }
    keySource = trimmed.slice(1, -1);
  }
  const parsed = tomlDottedKey(keySource);
  if (!parsed.valid) {
    return { present: true, valid: false, raw: trimmed };
  }
  return {
    present: true,
    valid: true,
    raw: trimmed,
    array,
    segments: parsed.segments,
  };
}

function tomlTables(source) {
  const tables = [];
  const invalidHeaders = [];
  let current;
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    const rawLine = lines[index];
    const parsedHeader = tomlTableHeader(rawLine);
    if (parsedHeader.present) {
      if (!parsedHeader.valid) {
        invalidHeaders.push({ line: index + 1, raw: parsedHeader.raw });
        current = undefined;
        continue;
      }
      current = {
        name: parsedHeader.segments.join("."),
        array: parsedHeader.array,
        segments: parsedHeader.segments,
        entries: [],
      };
      tables.push(current);
      continue;
    }
    const line = stripTomlComment(rawLine);
    if (current && line.trim()) current.entries.push(line.trim());
  }
  return { tables, invalidHeaders };
}

function tomlStringArray(source, sectionName, key) {
  const section = tomlSection(source, sectionName);
  const match = section.match(
    new RegExp(`(?:^|\\n)\\s*${key}\\s*=\\s*\\[([\\s\\S]*?)\\]`),
  );
  if (!match) return undefined;
  const values = match[1]
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
  if (values.some((value) => !/^"[^"]+"$/.test(value))) return undefined;
  return values.map((value) => value.slice(1, -1));
}

function tomlStringArrayValue(source, key) {
  const assignment = source.match(new RegExp(`^${key}\\s*=\\s*\\[`, "m"));
  if (!assignment) return { present: false, valid: true, values: undefined };
  const start = assignment.index + assignment[0].length;
  let quoted = false;
  let end = -1;
  for (let index = start; index < source.length; index += 1) {
    const character = source[index];
    if (character === '"' && source[index - 1] !== "\\") quoted = !quoted;
    if (character === "]" && !quoted) {
      end = index;
      break;
    }
  }
  if (end < 0) return { present: true, valid: false, values: [] };

  const body = source.slice(start, end);
  const values = [];
  let cursor = 0;
  while (cursor < body.length) {
    while (cursor < body.length && /\s/.test(body[cursor])) cursor += 1;
    if (cursor >= body.length) break;
    const value = body.slice(cursor).match(/^"([^"\\]*(?:\\.[^"\\]*)*)"/);
    if (!value) return { present: true, valid: false, values };
    values.push(value[1]);
    cursor += value[0].length;
    while (cursor < body.length && /\s/.test(body[cursor])) cursor += 1;
    if (cursor >= body.length) break;
    if (body[cursor] !== ",") {
      return { present: true, valid: false, values };
    }
    cursor += 1;
  }
  return { present: true, valid: true, values };
}

function cargoPackages(source) {
  return [...source.matchAll(/\[\[package\]\]\r?\n([\s\S]*?)(?=\r?\n\[\[package\]\]|\s*$)/g)]
    .map((match) => {
      const dependencyArray = tomlStringArrayValue(
        match[1],
        "dependencies",
      );
      return {
        name: match[1].match(/^name = "([^"]+)"$/m)?.[1],
        version: match[1].match(/^version = "([^"]+)"$/m)?.[1],
        source: match[1].match(/^source = "([^"]+)"$/m)?.[1],
        dependencies: dependencyArray.values,
        dependenciesValid: dependencyArray.valid,
      };
    })
    .filter(({ name, version }) => name && version);
}

function npmLockPackages(lock) {
  const packages = [];
  for (const [path, metadata] of Object.entries(lock.packages ?? {})) {
    const marker = "node_modules/";
    const index = path.lastIndexOf(marker);
    if (index < 0 || !metadata.version) continue;
    const name = path.slice(index + marker.length);
    packages.push({
      name,
      version: metadata.version,
      resolved: metadata.resolved,
      integrity: metadata.integrity,
    });
  }
  return packages;
}

function packageKey({ name, version }) {
  return `${name}@${version}`;
}

function normalizedSourceSha256(source) {
  const canonical = source.replace(/\r\n?/g, "\n");
  return createHash("sha256").update(canonical, "utf8").digest("hex");
}

function sha256(path) {
  return normalizedSourceSha256(readFileSync(join(root, path), "utf8"));
}

const requiredFiles = [
  ".github/workflows/ci.yml",
  ".github/workflows/release.yml",
  ".github/workflows/pages.yml",
  ".github/ISSUE_TEMPLATE/bug.yml",
  ".github/pull_request_template.md",
  "README.md",
  "CONTRIBUTING.md",
  "SECURITY.md",
  "THIRD_PARTY_NOTICES.md",
  "docs/privacy.md",
  "docs/security.md",
  "docs/troubleshooting.md",
  "docs/methodology.md",
  "docs/licenses/npm-dependencies.json",
  "docs/licenses/rust-dependencies.json",
  "docs/release-checklist.md",
  "docs/test-matrix.md",
  "tools/fake-cli/Cargo.toml",
  "tools/fake-cli/src/main.rs",
  "tools/fake-cli/tests/fake_cli.rs",
  "apps/desktop/src-tauri/tests/fake_cli_e2e.rs",
  "scripts/package-portable.mjs",
  "scripts/package-portable.test.mjs",
  "scripts/compress-portable.ps1",
  "scripts/extract-portable.ps1",
  "packaging/windows-portable/README.txt",
  "site/index.html",
  "site/.nojekyll",
];
for (const path of requiredFiles) read(path);

const expectedVersion = "0.2.1";
const rootPackage = json("package.json");
const desktopPackage = json("apps/desktop/package.json");
const tauriConfig = json("apps/desktop/src-tauri/tauri.conf.json");
const npmLock = json("package-lock.json");
const expectedTypeScriptParserVersion = "5.8.3";
if (
  rootPackage.devDependencies?.typescript !== expectedTypeScriptParserVersion ||
  npmLock.packages?.[""]?.devDependencies?.typescript !==
    expectedTypeScriptParserVersion ||
  npmLock.packages?.["node_modules/typescript"]?.version !==
    expectedTypeScriptParserVersion ||
  ts.version !== expectedTypeScriptParserVersion
) {
  fail(
    `TypeScript parser must be declared, locked, and loaded at exactly ${expectedTypeScriptParserVersion}`,
  );
}
if (rootPackage.scripts?.tauri !== "npm run tauri --workspace apps/desktop --") {
  fail("package.json tauri script must preserve the argument separator for workspace forwarding");
}
const expectedPortableScripts = {
  start: "npm run tauri -- dev",
  "package:portable":
    "npm run tauri -- build --no-bundle && npm run package:portable:from-build",
  "package:portable:from-build": "node scripts/package-portable.mjs",
};
for (const [name, expected] of Object.entries(expectedPortableScripts)) {
  if (rootPackage.scripts?.[name] !== expected) {
    fail(`package.json ${name} script must be exactly: ${expected}`);
  }
}
if (
  !rootPackage.scripts?.test
    ?.split(" && ")
    .includes("node --test scripts/package-portable.test.mjs")
) {
  fail("package.json test script must run scripts/package-portable.test.mjs");
}

const portableSources = new Map([
  ["scripts/package-portable.mjs", read("scripts/package-portable.mjs")],
  ["scripts/compress-portable.ps1", read("scripts/compress-portable.ps1")],
  ["scripts/extract-portable.ps1", read("scripts/extract-portable.ps1")],
]);
const portableNodeSource = portableSources.get("scripts/package-portable.mjs");
const portablePowerShellSource = portableSources.get(
  "scripts/compress-portable.ps1",
);
const portableExtractorSource = portableSources.get(
  "scripts/extract-portable.ps1",
);

function portableAstFailure(message) {
  fail(`portable Node AST allowlist ${message}`);
}

function expressionShape(node) {
  if (ts.isIdentifier(node)) return ["identifier", node.text];
  if (ts.isStringLiteral(node)) return ["string", node.text];
  if (ts.isNumericLiteral(node)) return ["number", node.text];
  if (node.kind === ts.SyntaxKind.TrueKeyword) return ["boolean", true];
  if (node.kind === ts.SyntaxKind.FalseKeyword) return ["boolean", false];
  if (ts.isPropertyAccessExpression(node)) {
    return ["property", expressionShape(node.expression), node.name.text];
  }
  if (ts.isElementAccessExpression(node)) {
    return [
      "element",
      expressionShape(node.expression),
      expressionShape(node.argumentExpression),
    ];
  }
  if (ts.isCallExpression(node)) {
    return [
      "call",
      expressionShape(node.expression),
      node.arguments.map(expressionShape),
    ];
  }
  if (ts.isArrayLiteralExpression(node)) {
    return ["array", node.elements.map(expressionShape)];
  }
  if (ts.isObjectLiteralExpression(node)) {
    return [
      "object",
      node.properties.map((property) => {
        if (
          !ts.isPropertyAssignment(property) ||
          (!ts.isIdentifier(property.name) && !ts.isStringLiteral(property.name))
        ) {
          return ["unsupported"];
        }
        return [
          property.name.text,
          expressionShape(property.initializer),
        ];
      }),
    ];
  }
  if (ts.isTemplateExpression(node)) return ["template"];
  return ["unsupported", ts.SyntaxKind[node.kind]];
}

function shapeKey(value) {
  return JSON.stringify(value);
}

function reviewPortableNodeAst(source) {
  const sourceFile = ts.createSourceFile(
    "scripts/package-portable.mjs",
    source,
    ts.ScriptTarget.ESNext,
    true,
    ts.ScriptKind.JS,
  );
  if (sourceFile.parseDiagnostics.length > 0) {
    portableAstFailure("requires parseable JavaScript with no diagnostics");
    return;
  }

  const expectedImports = new Map([
    ["node:crypto", ["createHash", "randomUUID"]],
    ["node:child_process", ["spawnSync"]],
    [
      "node:fs/promises",
      [
        "copyFile",
        "link",
        "lstat",
        "mkdir",
        "readFile",
        "readdir",
        "realpath",
        "rename",
        "rm",
        "writeFile",
      ],
    ],
    ["node:url", ["fileURLToPath"]],
    ["node:util", ["TextDecoder", "isDeepStrictEqual"]],
    [
      "node:path",
      [
        "basename",
        "dirname",
        "isAbsolute",
        "join",
        "parse",
        "relative",
        "resolve",
        "sep",
      ],
    ],
  ]);
  const actualImports = new Map();
  for (const statement of sourceFile.statements) {
    if (!ts.isImportDeclaration(statement)) continue;
    const moduleName = ts.isStringLiteral(statement.moduleSpecifier)
      ? statement.moduleSpecifier.text
      : "";
    const bindings = statement.importClause?.namedBindings;
    if (
      statement.importClause?.name ||
      !bindings ||
      !ts.isNamedImports(bindings) ||
      bindings.elements.some((element) => element.propertyName)
    ) {
      portableAstFailure("rejects default, namespace, and aliased imports");
      continue;
    }
    actualImports.set(
      moduleName,
      bindings.elements.map((element) => element.name.text).sort(),
    );
  }
  const sortedImports = (imports) =>
    [...imports].sort(([left], [right]) =>
      left < right ? -1 : left > right ? 1 : 0,
    );
  if (
    shapeKey(sortedImports(actualImports)) !==
    shapeKey(sortedImports(expectedImports))
  ) {
    portableAstFailure("requires the exact reviewed core import specifiers");
  }

  const reviewedImportNames = new Set(
    [...expectedImports.values()].flat(),
  );
  const reviewedLocalFunctions = new Set([
    "archiveLeaf",
    "assertArchiveCandidate",
    "assertInside",
    "canonicalExisting",
    "captureOwnedDirectory",
    "comparable",
    "copyValidatedTree",
    "ensureDirectory",
    "entriesUnder",
    "fileIdentity",
    "loadTrustedRegistry",
    "main",
    "packDirectoryHash",
    "packagePortableFromBuild",
    "packagePortableFromBuildForTest",
    "pathInfo",
    "plainObject",
    "readBoundedJson",
    "requireDirectory",
    "requireExactKeys",
    "requireFile",
    "requireOwnedDirectoryIdentity",
    "requireOwnedFileIdentity",
    "requirePackChild",
    "safePackRelativePath",
    "safeRemoveOwnedFile",
    "safeRemoveOwnedTree",
    "sameIdentity",
    "samePath",
    "settlePortableCleanup",
    "sha256",
    "stagePortable",
    "validateExtractedArchive",
    "validateGrader",
    "validateManifest",
    "validatePortablePacks",
    "validateRegistry",
  ]);
  const reviewedBindingNames = new Set([
    ...reviewedImportNames,
    ...reviewedLocalFunctions,
    "publicationHook",
  ]);
  const callableImportNames = new Set(
    [...reviewedImportNames].filter(
      (name) => name !== "sep" && name !== "TextDecoder",
    ),
  );
  const callableBindingNames = new Set([
    ...callableImportNames,
    ...reviewedLocalFunctions,
    "publicationHook",
  ]);

  function boundIdentifiers(name) {
    if (!name) return [];
    if (ts.isIdentifier(name)) return [name];
    if (ts.isObjectBindingPattern(name) || ts.isArrayBindingPattern(name)) {
      return name.elements.flatMap((element) =>
        ts.isBindingElement(element) ? boundIdentifiers(element.name) : [],
      );
    }
    return [];
  }

  function assignmentIdentifiers(target) {
    if (ts.isIdentifier(target)) return [target];
    if (ts.isParenthesizedExpression(target)) {
      return assignmentIdentifiers(target.expression);
    }
    if (ts.isArrayLiteralExpression(target)) {
      return target.elements.flatMap((element) =>
        ts.isSpreadElement(element)
          ? assignmentIdentifiers(element.expression)
          : assignmentIdentifiers(element),
      );
    }
    if (ts.isObjectLiteralExpression(target)) {
      return target.properties.flatMap((property) => {
        if (ts.isShorthandPropertyAssignment(property)) return [property.name];
        if (ts.isPropertyAssignment(property)) {
          return assignmentIdentifiers(property.initializer);
        }
        if (ts.isSpreadAssignment(property)) {
          return assignmentIdentifiers(property.expression);
        }
        return [];
      });
    }
    return [];
  }

  function rejectReviewedBinding(nodes, context, allowed = () => false) {
    for (const identifier of nodes) {
      if (
        reviewedBindingNames.has(identifier.text) &&
        !allowed(identifier)
      ) {
        portableAstFailure(
          `rejects ${context} binding or shadow of ${identifier.text}`,
        );
      }
    }
  }

  const expectedDirectCalls = new Map([
    ["archiveLeaf", 1],
    ["assertArchiveCandidate", 4],
    ["assertInside", 22],
    ["basename", 6],
    ["BigInt", 2],
    ["canonicalExisting", 9],
    ["captureOwnedDirectory", 2],
    ["comparable", 2],
    ["copyFile", 3],
    ["copyValidatedTree", 1],
    ["createHash", 2],
    ["dirname", 6],
    ["ensureDirectory", 5],
    ["entriesUnder", 6],
    ["fileIdentity", 2],
    ["fileURLToPath", 2],
    ["isAbsolute", 1],
    ["isDeepStrictEqual", 3],
    ["join", 37],
    ["link", 1],
    ["loadTrustedRegistry", 1],
    ["lstat", 12],
    ["main", 1],
    ["mkdir", 3],
    ["packagePortableFromBuild", 2],
    ["packDirectoryHash", 1],
    ["parse", 1],
    ["pathInfo", 9],
    ["plainObject", 2],
    ["publicationHook", 4],
    ["randomUUID", 4],
    ["readBoundedJson", 3],
    ["readFile", 7],
    ["readdir", 2],
    ["realpath", 2],
    ["relative", 7],
    ["rename", 1],
    ["requireDirectory", 15],
    ["requireExactKeys", 8],
    ["requireFile", 9],
    ["requireOwnedDirectoryIdentity", 3],
    ["requireOwnedFileIdentity", 3],
    ["requirePackChild", 2],
    ["resolve", 10],
    ["rm", 2],
    ["safePackRelativePath", 3],
    ["safeRemoveOwnedFile", 1],
    ["safeRemoveOwnedTree", 3],
    ["sameIdentity", 5],
    ["samePath", 3],
    ["settlePortableCleanup", 1],
    ["sha256", 2],
    ["spawnSync", 2],
    ["stagePortable", 1],
    ["validateExtractedArchive", 1],
    ["validateGrader", 1],
    ["validateManifest", 1],
    ["validatePortablePacks", 4],
    ["validateRegistry", 2],
    ["writeFile", 1],
  ]);
  const expectedMemberCalls = new Map([
    ["add", 1],
    ["alloc", 2],
    ["allSettled", 2],
    ["catch", 1],
    ["compare", 1],
    ["decode", 2],
    ["digest", 2],
    ["entries", 1],
    ["filter", 5],
    ["from", 2],
    ["has", 3],
    ["includes", 4],
    ["isArray", 5],
    ["isDirectory", 7],
    ["isFile", 6],
    ["isSafeInteger", 3],
    ["isSymbolicLink", 9],
    ["join", 5],
    ["keys", 1],
    ["map", 5],
    ["parse", 2],
    ["push", 8],
    ["reverse", 1],
    ["some", 2],
    ["sort", 7],
    ["split", 8],
    ["startsWith", 3],
    ["stringify", 2],
    ["subarray", 1],
    ["test", 5],
    ["toLowerCase", 1],
    ["trim", 1],
    ["update", 5],
    ["write", 2],
    ["writeBigUInt64LE", 2],
  ]);
  const expectedProperties = new Map([
    ["add", 1],
    ["alloc", 2],
    ["NODE_TEST_CONTEXT", 1],
    ["allSettled", 2],
    ["argv", 2],
    ["bundled", 1],
    ["catch", 1],
    ["category", 1],
    ["checksumManifest", 1],
    ["code", 2],
    ["compare", 1],
    ["content_sha256", 2],
    ["decode", 2],
    ["dev", 3],
    ["digest", 2],
    ["directory", 1],
    ["entries", 2],
    ["env", 1],
    ["error", 4],
    ["exitCode", 1],
    ["expected", 5],
    ["filter", 5],
    ["from", 2],
    ["grader", 1],
    ["has", 3],
    ["id", 10],
    ["includes", 4],
    ["ino", 3],
    ["isArray", 5],
    ["isDirectory", 7],
    ["isFile", 6],
    ["isSafeInteger", 3],
    ["isSymbolicLink", 9],
    ["join", 5],
    ["keys", 1],
    ["length", 11],
    ["license", 1],
    ["map", 5],
    ["max_turns", 3],
    ["message", 1],
    ["name", 20],
    ["packs", 4],
    ["parse", 2],
    ["path", 10],
    ["payloads", 1],
    ["platform", 2],
    ["prompt_file", 1],
    ["push", 8],
    ["reverse", 1],
    ["root", 1],
    ["schema_version", 2],
    ["sha256", 1],
    ["size", 8],
    ["some", 2],
    ["sort", 7],
    ["split", 8],
    ["starter_dir", 3],
    ["startsWith", 3],
    ["status", 2],
    ["stderr", 1],
    ["stdout", 1],
    ["stringify", 2],
    ["subarray", 1],
    ["target_kinds", 3],
    ["tasks", 3],
    ["test", 5],
    ["time_budget_secs", 3],
    ["title", 2],
    ["toLowerCase", 1],
    ["trim", 1],
    ["trustedRegistry", 2],
    ["type", 2],
    ["update", 5],
    ["url", 2],
    ["verifier_id", 2],
    ["version", 4],
    ["write", 2],
    ["writeBigUInt64LE", 2],
  ]);
  const sensitiveCapabilities = new Set([
    "copyFile",
    "link",
    "lstat",
    "mkdir",
    "readFile",
    "readdir",
    "realpath",
    "rename",
    "rm",
    "spawnSync",
    "writeFile",
  ]);
  const forbiddenIdentifiers = new Set([
    "EventSource",
    "Function",
    "Reflect",
    "WebSocket",
    "XMLHttpRequest",
    "eval",
    "fetch",
    "global",
    "globalThis",
    "require",
    "sendBeacon",
  ]);
  const directCalls = new Map();
  const memberCalls = new Map();
  const properties = new Map();
  const directCallNodes = new Map();

  function increment(map, name) {
    map.set(name, (map.get(name) ?? 0) + 1);
  }

  function visit(node) {
    if (ts.isVariableDeclaration(node)) {
      rejectReviewedBinding(
        boundIdentifiers(node.name),
        "variable declaration",
      );
    }
    if (ts.isParameter(node)) {
      rejectReviewedBinding(
        boundIdentifiers(node.name),
        "parameter",
        (identifier) =>
          identifier.text === "publicationHook" &&
          ts.isFunctionDeclaration(node.parent) &&
          [
            "packagePortableFromBuild",
            "packagePortableFromBuildForTest",
          ].includes(node.parent.name?.text) &&
          node.parent.parameters[1] === node,
      );
    }
    if (ts.isCatchClause(node) && node.variableDeclaration) {
      rejectReviewedBinding(
        boundIdentifiers(node.variableDeclaration.name),
        "catch",
      );
    }
    if (ts.isFunctionDeclaration(node)) {
      rejectReviewedBinding(
        node.name ? [node.name] : [],
        "function declaration",
        (identifier) =>
          reviewedLocalFunctions.has(identifier.text) &&
          node.parent === sourceFile,
      );
    }
    if (ts.isFunctionExpression(node)) {
      rejectReviewedBinding(
        node.name ? [node.name] : [],
        "function expression",
      );
    }
    if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) {
      rejectReviewedBinding(
        node.name ? [node.name] : [],
        "class declaration",
      );
    }
    if (
      ts.isPropertyDeclaration(node) ||
      ts.isMethodDeclaration(node) ||
      ts.isGetAccessorDeclaration(node) ||
      ts.isSetAccessorDeclaration(node)
    ) {
      rejectReviewedBinding(
        ts.isIdentifier(node.name) ? [node.name] : [],
        "class member",
      );
    }
    if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
      node.operatorToken.kind <= ts.SyntaxKind.LastAssignment
    ) {
      rejectReviewedBinding(
        assignmentIdentifiers(node.left),
        "assignment",
      );
    }
    if (
      (ts.isPrefixUnaryExpression(node) ||
        ts.isPostfixUnaryExpression(node)) &&
      (node.operator === ts.SyntaxKind.PlusPlusToken ||
        node.operator === ts.SyntaxKind.MinusMinusToken)
    ) {
      rejectReviewedBinding(
        assignmentIdentifiers(node.operand),
        "update assignment",
      );
    }
    if (ts.isIdentifier(node)) {
      if (forbiddenIdentifiers.has(node.text)) {
        portableAstFailure(`rejects forbidden capability identifier ${node.text}`);
      }
      if (sensitiveCapabilities.has(node.text)) {
        const imported =
          ts.isImportSpecifier(node.parent) && node.parent.name === node;
        const directCallee =
          ts.isCallExpression(node.parent) && node.parent.expression === node;
        if (!imported && !directCallee) {
          portableAstFailure(`rejects aliases of capability ${node.text}`);
        }
      }
      if (callableBindingNames.has(node.text)) {
        const imported =
          ts.isImportSpecifier(node.parent) && node.parent.name === node;
        const localDeclaration =
          ts.isFunctionDeclaration(node.parent) &&
          node.parent.name === node &&
          reviewedLocalFunctions.has(node.text) &&
          node.parent.parent === sourceFile;
        const publicationParameter =
          ts.isParameter(node.parent) &&
          node.parent.name === node &&
          node.text === "publicationHook" &&
          ts.isFunctionDeclaration(node.parent.parent) &&
          [
            "packagePortableFromBuild",
            "packagePortableFromBuildForTest",
          ].includes(node.parent.parent.name?.text);
        const directCallee =
          ts.isCallExpression(node.parent) && node.parent.expression === node;
        const propertyName =
          (ts.isPropertyAccessExpression(node.parent) &&
            node.parent.name === node) ||
          ((ts.isPropertyAssignment(node.parent) ||
            ts.isMethodDeclaration(node.parent) ||
            ts.isPropertyDeclaration(node.parent)) &&
            node.parent.name === node);
        const reviewedForwarding =
          node.text === "publicationHook" &&
          ts.isCallExpression(node.parent) &&
          node.parent.arguments[1] === node &&
          ts.isIdentifier(node.parent.expression) &&
          node.parent.expression.text === "packagePortableFromBuild";
        if (
          !imported &&
          !localDeclaration &&
          !publicationParameter &&
          !directCallee &&
          !propertyName &&
          !reviewedForwarding
        ) {
          portableAstFailure(
            `rejects alias reference to reviewed binding ${node.text}`,
          );
        }
      }
    }
    if (ts.isPropertyAccessExpression(node)) {
      increment(properties, node.name.text);
      if (
        node.name.text === "getBuiltinModule" ||
        node.name.text === "binding"
      ) {
        portableAstFailure("rejects process builtin-module access");
      }
    }
    if (ts.isCallExpression(node)) {
      if (node.expression.kind === ts.SyntaxKind.ImportKeyword) {
        portableAstFailure("rejects dynamic import calls");
      } else if (ts.isIdentifier(node.expression)) {
        const name = node.expression.text;
        increment(directCalls, name);
        const nodes = directCallNodes.get(name) ?? [];
        nodes.push(node);
        directCallNodes.set(name, nodes);
        if (!expectedDirectCalls.has(name)) {
          portableAstFailure(`rejects unknown direct callee ${name}`);
        }
      } else if (ts.isPropertyAccessExpression(node.expression)) {
        const name = node.expression.name.text;
        increment(memberCalls, name);
        if (!expectedMemberCalls.has(name)) {
          portableAstFailure(`rejects unknown member callee ${name}`);
        }
      } else {
        portableAstFailure("rejects computed, element-access, and indirect callees");
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(sourceFile);

  const compareCounts = (label, actual, expected) => {
    if (
      shapeKey([...actual].sort()) !==
      shapeKey([...expected].sort())
    ) {
      portableAstFailure(`requires exact reviewed ${label} counts`);
    }
  };
  compareCounts("direct call", directCalls, expectedDirectCalls);
  compareCounts("member call", memberCalls, expectedMemberCalls);
  compareCounts("member access", properties, expectedProperties);

  const actualElementShapes = [];
  function collectElementShape(node) {
    if (ts.isElementAccessExpression(node)) {
      actualElementShapes.push(shapeKey(expressionShape(node)));
    }
    ts.forEachChild(node, collectElementShape);
  }
  collectElementShape(sourceFile);
  const reviewedElementShapes = [
    [
      "element",
      ["identifier", "expectedPackIdentities"],
      ["identifier", "index"],
    ],
    [
      "element",
      ["property", ["identifier", "process"], "argv"],
      ["number", "1"],
    ],
    [
      "element",
      ["property", ["identifier", "process"], "argv"],
      ["number", "1"],
    ],
    ["element", ["identifier", "signature"], ["number", "0"]],
    ["element", ["identifier", "signature"], ["number", "1"]],
  ];
  if (
    shapeKey(actualElementShapes.sort()) !==
    shapeKey(reviewedElementShapes.map(shapeKey).sort())
  ) {
    portableAstFailure("rejects unreviewed computed or element access");
  }

  const expectedCopyArguments = [
    [
      ["property", ["identifier", "entry"], "path"],
      ["identifier", "destination"],
    ],
    [
      ["identifier", "executable"],
      [
        "call",
        ["identifier", "join"],
        [
          ["identifier", "stageRoot"],
          ["string", "ability-radar.exe"],
        ],
      ],
    ],
    [
      ["identifier", "readme"],
      [
        "call",
        ["identifier", "join"],
        [
          ["identifier", "stageRoot"],
          ["string", "README.txt"],
        ],
      ],
    ],
  ];
  const actualCopyArguments = (directCallNodes.get("copyFile") ?? [])
    .map((call) => call.arguments.map(expressionShape))
    .map(shapeKey)
    .sort();
  if (
    shapeKey(actualCopyArguments) !==
    shapeKey(expectedCopyArguments.map(shapeKey).sort())
  ) {
    portableAstFailure("rejects unreviewed copyFile sources or destinations");
  }

  const expectedSimpleMutationArguments = new Map([
    [
      "link",
      [[["identifier", "temporaryArchive"], ["identifier", "archivePath"]]],
    ],
    [
      "mkdir",
      [
        [["identifier", "current"]],
        [["identifier", "stageParent"]],
        [["identifier", "stageRoot"]],
      ],
    ],
    [
      "rename",
      [[["identifier", "path"], ["identifier", "quarantine"]]],
    ],
    [
      "rm",
      [
        [["identifier", "path"]],
        [
          ["identifier", "quarantine"],
          ["object", [["recursive", ["boolean", true]]]],
        ],
      ],
    ],
  ]);
  for (const [name, expected] of expectedSimpleMutationArguments) {
    const actual = (directCallNodes.get(name) ?? [])
      .map((call) => call.arguments.map(expressionShape))
      .map(shapeKey)
      .sort();
    if (shapeKey(actual) !== shapeKey(expected.map(shapeKey).sort())) {
      portableAstFailure(`rejects unreviewed ${name} destinations`);
    }
  }

  const writeCall = directCallNodes.get("writeFile")?.[0];
  const expectedWriteDestination = [
    "call",
    ["identifier", "join"],
    [
      ["identifier", "stageRoot"],
      ["string", "SHA256SUMS.txt"],
    ],
  ];
  if (
    !writeCall ||
    writeCall.arguments.length !== 3 ||
    shapeKey(expressionShape(writeCall.arguments[0])) !==
      shapeKey(expectedWriteDestination) ||
    shapeKey(expressionShape(writeCall.arguments[2])) !==
      shapeKey(["object", [["flag", ["string", "wx"]]]])
  ) {
    portableAstFailure("rejects unreviewed writeFile destinations or options");
  }

  const powerShellOptions = [
    "object",
    [
      ["cwd", ["identifier", "repoRoot"]],
      ["stdio", ["string", "inherit"]],
    ],
  ];
  const powerShellPrefix = [
    ["string", "-NoLogo"],
    ["string", "-NoProfile"],
    ["string", "-NonInteractive"],
    ["string", "-ExecutionPolicy"],
    ["string", "Bypass"],
    ["string", "-File"],
  ];
  const expectedSpawnArguments = [
    [
      ["string", "powershell.exe"],
      [
        "array",
        [
          ...powerShellPrefix,
          [
            "call",
            ["identifier", "join"],
            [
              ["identifier", "repoRoot"],
              ["string", "scripts"],
              ["string", "compress-portable.ps1"],
            ],
          ],
          ["string", "-Source"],
          ["identifier", "stageRoot"],
          ["string", "-Destination"],
          ["identifier", "temporaryArchive"],
        ],
      ],
      powerShellOptions,
    ],
    [
      ["string", "powershell.exe"],
      [
        "array",
        [
          ...powerShellPrefix,
          [
            "call",
            ["identifier", "join"],
            [
              ["identifier", "repoRoot"],
              ["string", "scripts"],
              ["string", "extract-portable.ps1"],
            ],
          ],
          ["string", "-Source"],
          ["identifier", "temporaryArchive"],
          ["string", "-Destination"],
          ["identifier", "verificationDirectory"],
        ],
      ],
      powerShellOptions,
    ],
  ];
  const actualSpawnArguments = (directCallNodes.get("spawnSync") ?? [])
    .map((call) => call.arguments.map(expressionShape))
    .map(shapeKey)
    .sort();
  if (
    shapeKey(actualSpawnArguments) !==
    shapeKey(expectedSpawnArguments.map(shapeKey).sort())
  ) {
    portableAstFailure(
      "permits only the exact reviewed powershell.exe compressor and extractor invocations",
    );
  }
}

reviewPortableNodeAst(portableNodeSource);

const reviewedPowerShell = portablePowerShellSource
  .replace(/^\s*#.*$/gm, "")
  .replace(/\r\n?/g, "\n");
const expectedPowerShellCounts = new Map([
  ["Compress-Archive", 1],
  ["Get-Item", 2],
  ["Split-Path", 1],
  ["Test-Path", 3],
]);
for (const [operation, expected] of expectedPowerShellCounts) {
  const count = [...reviewedPowerShell.matchAll(
    new RegExp(`\\b${operation}\\b`, "g"),
  )].length;
  if (count !== expected) {
    fail(
      `portable PowerShell operation allowlist requires ${operation} exactly ${expected} time(s); found ${count}`,
    );
  }
}
const allowedPowerShellStatements = new Set([
  "param(",
  "[Parameter(Mandatory = $true)]",
  "[string]$Source,",
  "[string]$Destination",
  ")",
  '$ErrorActionPreference = "Stop"',
  "$sourcePath = [System.IO.Path]::GetFullPath($Source)",
  "$destinationPath = [System.IO.Path]::GetFullPath($Destination)",
  "if (-not (Test-Path -LiteralPath $sourcePath -PathType Container)) {",
  'throw "Portable source directory does not exist."',
  "}",
  'if ([System.IO.Path]::GetExtension($destinationPath) -cne ".zip") {',
  'throw "Portable destination must be a .zip file."',
  "$destinationDirectory = Split-Path -Parent $destinationPath",
  "if (-not (Test-Path -LiteralPath $destinationDirectory -PathType Container)) {",
  'throw "Portable destination directory does not exist."',
  "if (Test-Path -LiteralPath $destinationPath) {",
  'throw "Portable destination already exists."',
  "$sourceItem = Get-Item -LiteralPath $sourcePath",
  "$destinationDirectoryItem = Get-Item -LiteralPath $destinationDirectory",
  "if (",
  "($sourceItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or",
  "($destinationDirectoryItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)",
  ") {",
  'throw "Portable compressor paths must not be reparse points."',
  "Compress-Archive `",
  "-LiteralPath $sourcePath `",
  "-DestinationPath $destinationPath `",
  "-CompressionLevel Optimal",
]);
for (const statement of reviewedPowerShell
  .split("\n")
  .map((line) => line.trim())
  .filter(Boolean)) {
  if (!allowedPowerShellStatements.has(statement)) {
    fail(
      `portable PowerShell operation allowlist rejects unsupported statement: ${statement}`,
    );
  }
}
if (
  /(?:^|[\s;|])(?:&|\.|Invoke-Expression|iex|Set-Alias|New-Alias|Get-Command)(?:\s|$)/im.test(
    reviewedPowerShell,
  ) ||
  !reviewedPowerShell.includes(
    "$sourcePath = [System.IO.Path]::GetFullPath($Source)",
  ) ||
  !reviewedPowerShell.includes(
    "$destinationPath = [System.IO.Path]::GetFullPath($Destination)",
  ) ||
  !reviewedPowerShell.includes(
    "$destinationDirectory = Split-Path -Parent $destinationPath",
  ) ||
  !reviewedPowerShell.includes(
    "$sourceItem = Get-Item -LiteralPath $sourcePath",
  ) ||
  !reviewedPowerShell.includes(
    "$destinationDirectoryItem = Get-Item -LiteralPath $destinationDirectory",
  ) ||
  !reviewedPowerShell.includes(
    "Compress-Archive `\n  -LiteralPath $sourcePath `\n  -DestinationPath $destinationPath `\n  -CompressionLevel Optimal",
  )
) {
  fail(
    "portable PowerShell operation allowlist permits only direct path validation and one direct Compress-Archive invocation",
  );
}
const reviewedExtractor = portableExtractorSource
  .replace(/^\s*#.*$/gm, "")
  .replace(/\r\n?/g, "\n");
const expectedExtractorCounts = new Map([
  ["Add-Type", 1],
  ["Expand-Archive", 1],
  ["Get-ChildItem", 1],
  ["Get-Item", 2],
  ["OpenRead", 1],
  ["Test-Path", 2],
]);
for (const [operation, expected] of expectedExtractorCounts) {
  const count = [...reviewedExtractor.matchAll(
    new RegExp(`\\b${operation}\\b`, "g"),
  )].length;
  if (count !== expected) {
    fail(
      `portable extractor allowlist requires ${operation} exactly ${expected} time(s); found ${count}`,
    );
  }
}
const allowedExtractorStatements = new Set([
  "param(",
  "[Parameter(Mandatory = $true)]",
  "[string]$Source,",
  "[string]$Destination",
  ")",
  '$ErrorActionPreference = "Stop"',
  "Add-Type -AssemblyName System.IO.Compression.FileSystem",
  "$sourcePath = [System.IO.Path]::GetFullPath($Source)",
  "$destinationPath = [System.IO.Path]::GetFullPath($Destination)",
  "if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {",
  'throw "Portable archive source does not exist."',
  "}",
  'if ([System.IO.Path]::GetExtension($sourcePath) -cne ".zip") {',
  'throw "Portable archive source must be a .zip file."',
  "if (-not (Test-Path -LiteralPath $destinationPath -PathType Container)) {",
  'throw "Portable verification directory does not exist."',
  "$sourceItem = Get-Item -LiteralPath $sourcePath",
  "$destinationItem = Get-Item -LiteralPath $destinationPath",
  "if (",
  "($sourceItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -or",
  "($destinationItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)",
  ") {",
  'throw "Portable extractor paths must not be reparse points."',
  "if ((Get-ChildItem -LiteralPath $destinationPath).Count -ne 0) {",
  'throw "Portable verification directory must be empty."',
  "$separator = [System.IO.Path]::DirectorySeparatorChar",
  "$destinationPrefix = $destinationPath.TrimEnd($separator) + $separator",
  "$seen = [System.Collections.Generic.HashSet[string]]::new(",
  "[System.StringComparer]::OrdinalIgnoreCase",
  ")",
  "$archive = [System.IO.Compression.ZipFile]::OpenRead($sourcePath)",
  "try {",
  "foreach ($entry in $archive.Entries) {",
  '$entryName = $entry.FullName.Replace("\\", "/").TrimEnd("/")',
  "if ([string]::IsNullOrWhiteSpace($entryName)) {",
  'throw "Portable archive contains an empty entry name."',
  "if (-not $seen.Add($entryName)) {",
  'throw "Portable archive contains a duplicate entry."',
  "$entryPath = [System.IO.Path]::GetFullPath(",
  "[System.IO.Path]::Combine(",
  "$destinationPath,",
  '$entryName.Replace("/", $separator)',
  ")",
  "if (-not $entryPath.StartsWith(",
  "$destinationPrefix,",
  "[System.StringComparison]::OrdinalIgnoreCase",
  ")) {",
  'throw "Portable archive entry escapes the verification directory."',
  "finally {",
  "$archive.Dispose()",
  "Expand-Archive `",
  "-LiteralPath $sourcePath `",
  "-DestinationPath $destinationPath",
]);
for (const statement of reviewedExtractor
  .split("\n")
  .map((line) => line.trim())
  .filter(Boolean)) {
  if (!allowedExtractorStatements.has(statement)) {
    fail(`portable extractor allowlist rejects unsupported statement: ${statement}`);
  }
}
if (
  /(?:^|[\s;|])(?:&|\.|Invoke-Expression|iex|Set-Alias|New-Alias|Get-Command|Start-Process|curl|wget|Invoke-WebRequest)(?:\s|$)/im.test(
    reviewedExtractor,
  ) ||
  !reviewedExtractor.includes(
    "$archive = [System.IO.Compression.ZipFile]::OpenRead($sourcePath)",
  ) ||
  !reviewedExtractor.includes(
    "Expand-Archive `\n  -LiteralPath $sourcePath `\n  -DestinationPath $destinationPath",
  )
) {
  fail(
    "portable extractor allowlist permits only the reviewed duplicate/path validation and direct extraction destination",
  );
}
const portableSourceSeals = new Map([
  [
    "scripts/package-portable.mjs",
    "949dd69f2ef8c298148de27744f61c492a8eacdf536c2968778781b396a0b67e",
  ],
  [
    "scripts/compress-portable.ps1",
    "d42425e9544bd0d4e4c9d021d1ec8b8ce13b328d93da3f5e5d4a3417f81c550a",
  ],
  [
    "scripts/extract-portable.ps1",
    "5d8076986a54331e0c3f2c2603630772cd025a87212f9a41f10f6891e21ff49e",
  ],
]);
for (const [path, expected] of portableSourceSeals) {
  if (normalizedSourceSha256(portableSources.get(path)) !== expected) {
    fail(
      `${path} portable source seal mismatch; review provider invocations, network uploads, and writes outside target/release/bundle/portable`,
    );
  }
}
if (
  JSON.stringify(rootPackage.allowScripts) !==
  JSON.stringify({ "esbuild@0.28.1": true })
) {
  fail("package.json must approve only the locked esbuild lifecycle script");
}
const manifestVersions = [
  ["package.json", rootPackage.version],
  ["package-lock.json root", npmLock.packages?.[""]?.version],
  ["apps/desktop/package.json", desktopPackage.version],
  ["apps/desktop/src-tauri/tauri.conf.json", tauriConfig.version],
  ["package-lock.json workspace", npmLock.packages?.["apps/desktop"]?.version],
];
const firstPartyLicenses = [
  ["package.json", rootPackage.license],
  ["package-lock.json root", npmLock.packages?.[""]?.license],
  ["apps/desktop/package.json", desktopPackage.license],
  ["package-lock.json workspace", npmLock.packages?.["apps/desktop"]?.license],
];
for (const path of [
  "apps/desktop/src-tauri/Cargo.toml",
  "crates/ability-core/Cargo.toml",
  "crates/ability-adapters/Cargo.toml",
]) {
  const manifest = read(path);
  const version = manifest.match(/^version = "([^"]+)"$/m)?.[1];
  manifestVersions.push([path, version]);
  const license = manifest.match(/^license = "([^"]+)"$/m)?.[1];
  firstPartyLicenses.push([path, license]);
}
for (const [path, version] of manifestVersions) {
  if (version !== expectedVersion) {
    fail(`${path} version must be ${expectedVersion}; found ${version ?? "missing"}`);
  }
}
for (const [path, license] of firstPartyLicenses) {
  if (license !== "Apache-2.0") {
    fail(`${path} first-party license must be Apache-2.0; found ${license ?? "missing"}`);
  }
}
const ownedCargo = new Set(["ability-radar", "ability-core", "ability-adapters"]);
for (const pkg of cargoPackages(read("Cargo.lock")).filter(({ name }) => ownedCargo.has(name))) {
  if (pkg.version !== expectedVersion) {
    fail(`Cargo.lock ${pkg.name} must be ${expectedVersion}; found ${pkg.version}`);
  }
}

const workspaceManifest = read("Cargo.toml");
const workspaceMembers = tomlStringArray(
  workspaceManifest,
  "workspace",
  "members",
);
if (
  workspaceMembers?.filter((member) => member === "tools/fake-cli").length !==
  1
) {
  fail("Cargo workspace must include tools/fake-cli exactly once in the members array");
}
const fakeManifest = read("tools/fake-cli/Cargo.toml");
const fakeManifestSourceSeal = "7c767f6e1420f6a12547abd526b311c39861b8355a8973b1cc1553a1b800d57d";
if (normalizedSourceSha256(fakeManifest) !== fakeManifestSourceSeal) {
  fail("tools/fake-cli/Cargo.toml normalized source seal mismatch");
}
const fakeManifestContracts = [
  ["package name", /^name = "ability-radar-fake-cli"$/m],
  ["fixture version 0.1.0", /^version = "0\.1\.0"$/m],
  ["first-party Apache-2.0 license", /^license = "Apache-2\.0"$/m],
  ["publish = false", /^publish = false$/m],
  ["serde_json dependency", /^serde_json = "1"$/m],
];
for (const [label, pattern] of fakeManifestContracts) {
  if (!pattern.test(fakeManifest)) fail(`fake CLI manifest is missing ${label}`);
}
const parsedFakeManifestTables = tomlTables(fakeManifest);
for (const header of parsedFakeManifestTables.invalidHeaders) {
  fail(
    `fake CLI manifest has invalid TOML table header at line ${header.line}: ${header.raw}`,
  );
}
const dependencyTableSegments = new Set([
  "dependencies",
  "build-dependencies",
  "dev-dependencies",
]);
function isCargoDependencyTable({ segments }) {
  if (dependencyTableSegments.has(segments[0])) return true;
  return (
    segments.length >= 3 &&
    segments[0] === "target" &&
    dependencyTableSegments.has(segments[2])
  );
}
const fakeDependencyTables = parsedFakeManifestTables.tables.filter(
  isCargoDependencyTable,
);
const directFakeDependencyTables = fakeDependencyTables.filter(
  ({ segments }) => (
    segments.length === 1 && segments[0] === "dependencies"
  ),
);
const fakeDependencies = directFakeDependencyTables.flatMap(
  ({ entries }) => entries,
);
if (
  directFakeDependencyTables.length !== 1 ||
  fakeDependencies.length !== 1 ||
  fakeDependencies[0] !== 'serde_json = "1"'
) {
  fail('fake CLI dependency set must be exactly serde_json = "1"');
}
for (const table of fakeDependencyTables.filter(
  ({ segments, entries }) =>
    !(segments.length === 1 && segments[0] === "dependencies") &&
    entries.length > 0,
)) {
  fail(
    `fake CLI dependency surface must not declare ${table.name}; only direct serde_json is allowed`,
  );
}
const lockedFake = cargoPackages(read("Cargo.lock")).filter(
  ({ name }) => name === "ability-radar-fake-cli",
);
if (
  lockedFake.length !== 1 ||
  lockedFake[0].version !== "0.1.0" ||
  lockedFake[0].source ||
  lockedFake[0].dependenciesValid !== true ||
  JSON.stringify(lockedFake[0].dependencies) !== JSON.stringify(["serde_json"])
) {
  fail(
    "Cargo.lock must contain exactly one first-party fake CLI at 0.1.0 with dependencies exactly serde_json",
  );
}
if (/tools[\\/]fake-cli|ability-radar-fake-cli/i.test(JSON.stringify(tauriConfig.bundle?.resources ?? {}))) {
  fail("fake CLI must never be a bundled Tauri resource");
}

const actions = new Map([
  ["actions/checkout", ["df4cb1c069e1874edd31b4311f1884172cec0e10", "v6"]],
  ["actions/setup-node", ["249970729cb0ef3589644e2896645e5dc5ba9c38", "v6"]],
  ["actions/upload-artifact", ["043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", "v7"]],
  ["dtolnay/rust-toolchain", ["2c7215f132e9ebf062739d9130488b56d53c060c", "reviewed master"]],
  ["tauri-apps/tauri-action", ["944946e3e4cac6603d1fe8f514171e9ecd3c78aa", "v1"]],
  ["actions/configure-pages", ["983d7736d9b0ae728b81ab479565c72886d7745b", "v5"]],
  ["actions/upload-pages-artifact", ["fc324d3547104276b827a68afc52ff2a11cc49c9", "v5"]],
  ["actions/deploy-pages", ["cd2ce8fcbc39b97be8ca5fce6e763baed58fa128", "v5"]],
]);
const workflowPaths = [
  ".github/workflows/ci.yml",
  ".github/workflows/release.yml",
  ".github/workflows/pages.yml",
];
// Publication workflows are sealed as a fail-closed backstop for YAML syntax
// the lightweight structural parser intentionally does not model. Intentional
// changes must update both this reviewed normalized-source seal and the exact
// structural contracts below.
const publicationWorkflowSeals = new Map([
  [
    ".github/workflows/release.yml",
    "a22609db904a231afb1522369d3141b1551c9595189d0a93e277062f4b3d89fb",
  ],
  [
    ".github/workflows/pages.yml",
    "d53beed428390726a50565b7b603526e2ab01a6d9a2a26e4b89c9f5d6d464f75",
  ],
]);
const workflows = new Map();
for (const path of workflowPaths) {
  const source = read(path);
  const reviewedSeal = publicationWorkflowSeals.get(path);
  if (reviewedSeal && normalizedSourceSha256(source) !== reviewedSeal) {
    fail(`${path} normalized source seal mismatch`);
  }
  const workflow = parseWorkflow(source);
  workflows.set(path, { source, workflow });

  const steps = [...workflow.jobs.values()].flatMap((job) => job.steps);
  for (const step of steps.filter(({ uses }) => uses)) {
    const match = step.uses.match(/^([^@\s]+)@([^\s]+)$/);
    if (!match) {
      fail(`${path} has an invalid action reference: ${step.uses}`);
      continue;
    }
    const [, action, sha] = match;
    const expected = actions.get(action);
    if (!expected) {
      fail(`${path} uses unreviewed third-party action ${action}`);
      continue;
    }
    if (!/^[0-9a-f]{40}$/.test(sha)) {
      fail(`${path} action ${action} is not pinned to a full commit SHA`);
    }
    if (sha !== expected[0] || step.usesComment !== expected[1]) {
      fail(`${path} must pin ${action}@${expected[0]} # ${expected[1]}`);
    }
  }
  if ([...workflow.jobs.values()].some((job) => !/^\d+$/.test(job.timeoutMinutes ?? ""))) {
    fail(`${path} needs an explicit timeout on every job`);
  }
  if (!workflow.hasConcurrency) fail(`${path} needs concurrency control`);
  if (workflow.topPermissions !== "{}") {
    fail(`${path} needs deny-by-default top-level permissions`);
  }
  const checkouts = actionSteps(workflow, "actions/checkout");
  if (checkouts.some((step) => step.with["persist-credentials"] !== "false")) {
    fail(`${path} checkout must set persist-credentials: false`);
  }
  if (/(?:OPENAI|ANTHROPIC|CLAUDE|CODEX|PROVIDER)[A-Z0-9_]*(?:KEY|TOKEN|SECRET)/i.test(source)) {
    fail(`${path} defines a provider credential name`);
  }
  for (const step of runSteps(workflow)) {
    if (/(^|[\r\n;&|]\s*)(?:codex|claude)(?:\.exe)?(?:\s|$)/im.test(step.run)) {
      fail(`${path} must not invoke a real AI CLI`);
    }
  }
}

const requiredActions = new Map([
  [".github/workflows/ci.yml", [
    "actions/checkout",
    "actions/setup-node",
    "dtolnay/rust-toolchain",
    "actions/upload-artifact",
  ]],
  [".github/workflows/release.yml", [
    "actions/checkout",
    "actions/setup-node",
    "dtolnay/rust-toolchain",
    "tauri-apps/tauri-action",
  ]],
  [".github/workflows/pages.yml", [
    "actions/checkout",
    "actions/setup-node",
    "actions/configure-pages",
    "actions/upload-pages-artifact",
    "actions/deploy-pages",
  ]],
]);
for (const [path, required] of requiredActions) {
  const workflow = workflows.get(path)?.workflow;
  for (const action of required) {
    const count = actionSteps(workflow, action).length;
    if (count !== 1) fail(`${path} must use ${action} exactly once; found ${count}`);
  }
  const actualSequence = [...workflow.jobs.values()]
    .flatMap((job) => job.steps)
    .filter((step) => step.uses)
    .map((step) => step.uses.split("@", 1)[0]);
  if (JSON.stringify(actualSequence) !== JSON.stringify(required)) {
    fail(
      `${path} approved action sequence must be exact: ${required.join(", ")}`,
    );
  }
}

function requireCommand(path, job, label, pattern) {
  if (!hasRunCommand(job, pattern)) {
    fail(`${path} is missing ${label}`);
  }
}

function exactObject(actual, expected) {
  const actualEntries = Object.entries(actual ?? {}).sort(([a], [b]) =>
    a.localeCompare(b, "en"),
  );
  const expectedEntries = Object.entries(expected).sort(([a], [b]) =>
    a.localeCompare(b, "en"),
  );
  return JSON.stringify(actualEntries) === JSON.stringify(expectedEntries);
}

const baseStepFields = [
  "env",
  "envDeclaration",
  "envValid",
  "name",
  "run",
  "uses",
  "usesComment",
  "with",
  "withDeclaration",
  "withValid",
];

function exactFields(actual, expected) {
  return (
    JSON.stringify(Object.keys(actual ?? {}).sort()) ===
    JSON.stringify([...expected].sort())
  );
}

function requireExactStepFields(path, step, extraFields, label) {
  if (!exactFields(step, [...baseStepFields, ...extraFields])) {
    fail(`${path} ${label} has unallowlisted step fields or controls`);
  }
}

function requireExactStepContract(path, step, expected, label) {
  const extra = expected.extra ?? {};
  requireExactStepFields(path, step, Object.keys(extra), label);
  const expectedWith = expected.with ?? {};
  const expectedEnv = expected.env ?? {};
  const expectedWithDeclaration =
    Object.keys(expectedWith).length > 0 ? "block" : "absent";
  const expectedEnvDeclaration =
    Object.keys(expectedEnv).length > 0 ? "block" : "absent";
  const scalarContract =
    step?.name === expected.name &&
    step?.uses === (expected.uses ?? "") &&
    step?.usesComment === (expected.usesComment ?? "") &&
    step?.run === (expected.run ?? "") &&
    step?.withDeclaration === expectedWithDeclaration &&
    step?.withValid === true &&
    step?.envDeclaration === expectedEnvDeclaration &&
    step?.envValid === true;
  const extraContract = Object.entries(extra).every(
    ([key, value]) => step?.[key] === value,
  );
  if (
    !scalarContract ||
    !extraContract ||
    !exactObject(step?.with, expectedWith) ||
    !exactObject(step?.env, expectedEnv)
  ) {
    fail(`${path} ${label} contract must be exact`);
  }
}

const baseJobFields = [
  "env",
  "envDeclaration",
  "envValid",
  "id",
  "permissions",
  "permissionsDeclaration",
  "permissionsValid",
  "runs-on",
  "steps",
  "timeoutMinutes",
];

function requireExactJobContract(path, job, expected, label) {
  const extra = expected.extra ?? {};
  const expectedEnv = expected.env ?? {};
  const expectedEnvDeclaration =
    Object.keys(expectedEnv).length > 0 ? "block" : "absent";
  const fieldsExact = exactFields(job, [
    ...baseJobFields,
    ...Object.keys(extra),
  ]);
  const extraContract = Object.entries(extra).every(
    ([key, value]) => job?.[key] === value,
  );
  if (
    !fieldsExact ||
    job?.id !== expected.id ||
    job?.["runs-on"] !== expected.runsOn ||
    job?.timeoutMinutes !== expected.timeoutMinutes ||
    job?.permissionsDeclaration !== "block" ||
    job?.permissionsValid !== true ||
    !exactObject(job?.permissions, expected.permissions) ||
    job?.envDeclaration !== expectedEnvDeclaration ||
    job?.envValid !== true ||
    !exactObject(job?.env, expectedEnv) ||
    !extraContract
  ) {
    fail(`${path} ${label} fields or contract must be exact`);
  }
}

function requireNoTopLevelEnv(path, workflow, label) {
  if (
    workflow?.topEnvDeclaration !== "absent" ||
    workflow?.topEnvValid !== true ||
    !exactObject(workflow?.topEnv, {})
  ) {
    fail(`${path} ${label} must have no top-level env declaration`);
  }
}

function namedStep(path, job, name) {
  const matches = job?.steps.filter((step) => step.name === name) ?? [];
  if (matches.length !== 1) {
    fail(`${path} must have exactly one step named ${name}; found ${matches.length}`);
  }
  return matches[0];
}

const ciPath = ".github/workflows/ci.yml";
const ciWorkflow = workflows.get(ciPath)?.workflow;
const ciJob = ciWorkflow?.jobs.get("test");
if (
  ciWorkflow?.topEnvDeclaration !== "absent" ||
  ciWorkflow?.topEnvValid !== true ||
  !exactObject(ciWorkflow?.topEnv, {})
) {
  fail(`${ciPath} CI workflow must have no env declaration or environment`);
}
if (
  ciJob?.envDeclaration !== "absent" ||
  ciJob?.envValid !== true ||
  !exactObject(ciJob?.env, {})
) {
  fail(`${ciPath} CI job must have no env declaration or environment`);
}
requireExactJobContract(
  ciPath,
  ciJob,
  {
    id: "test",
    runsOn: "windows-latest",
    timeoutMinutes: "60",
    permissions: { contents: "read" },
  },
  "CI job",
);
if (!exactPermissions(ciJob, { contents: "read" })) {
  fail(`${ciPath} test job permissions must be exactly contents: read`);
}
const ciNode = actionSteps(ciWorkflow, "actions/setup-node")[0];
if (ciNode?.with["node-version"] !== "22") fail(`${ciPath} must use Node.js 22`);
const ciRust = actionSteps(ciWorkflow, "dtolnay/rust-toolchain")[0];
if (ciRust?.with.toolchain !== "stable") {
  fail(`${ciPath} Rust toolchain action must explicitly select stable`);
}
if (ciRust?.with.components !== "clippy,rustfmt") {
  fail(`${ciPath} Rust toolchain action must install clippy,rustfmt`);
}
const ciCommands = [
  ["npm ci", /(?:^|\n)\s*npm ci\s*(?:$|\n)/],
  ["repository validation", /(?:^|\n)\s*npm run validate:repository\s*(?:$|\n)/],
  ["cargo-audit 0.22.2", /(?:^|\n)\s*cargo install cargo-audit --version 0\.22\.2 --locked\s*(?:$|\n)/],
  ["cargo audit", /(?:^|\n)\s*cargo audit\s*(?:$|\n)/],
  ["npm high-severity audit", /(?:^|\n)\s*npm audit --audit-level=high\s*(?:$|\n)/],
  ["Rust formatting check", /(?:^|\n)\s*cargo fmt --all --check\s*(?:$|\n)/],
  ["locked all-target clippy", /(?:^|\n)\s*cargo clippy --workspace --all-targets --locked -- -D warnings\s*(?:$|\n)/],
  ["locked all-target tests", /(?:^|\n)\s*cargo test --workspace --all-targets --locked\s*(?:$|\n)/],
  ["locked fake CLI build", /(?:^|\n)\s*cargo build -p ability-radar-fake-cli --locked\s*(?:$|\n)/],
  ["temporary fake CLI directory", /Join-Path \$env:RUNNER_TEMP "ability-radar-fake-bin"/],
  ["fake Codex executable copy", /Copy-Item target\/debug\/ability-radar-fake-cli\.exe \(Join-Path \$fakeBin "codex\.exe"\)/],
  ["fake Claude executable copy", /Copy-Item target\/debug\/ability-radar-fake-cli\.exe \(Join-Path \$fakeBin "claude\.exe"\)/],
  ["temporary fake CLI PATH install", /"\$fakeBin" \| Out-File -FilePath \$env:GITHUB_PATH -Encoding utf8 -Append/],
  ["locked opted-in fake CLI E2E", /(?:^|\n)\s*cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored\s*(?:$|\n)/],
  ["frontend tests", /(?:^|\n)\s*npm test\s*(?:$|\n)/],
  ["frontend build", /(?:^|\n)\s*npm run build\s*(?:$|\n)/],
  ["debug NSIS build", /(?:^|\n)\s*npm run tauri -- build --debug --bundles nsis\s*(?:$|\n)/],
];
for (const [label, pattern] of ciCommands) requireCommand(ciPath, ciJob, label, pattern);
const fakeInstallName = "Install deterministic fake CLIs";
const fakeE2eName = "Test real coordinator with deterministic fake CLIs";
const fakeInstall = namedStep(ciPath, ciJob, fakeInstallName);
const fakeE2e = namedStep(ciPath, ciJob, fakeE2eName);
requireExactStepFields(ciPath, fakeInstall, [], fakeInstallName);
requireExactStepFields(ciPath, fakeE2e, [], fakeE2eName);
const expectedFakeInstallRun = `cargo build -p ability-radar-fake-cli --locked
$fakeBin = Join-Path $env:RUNNER_TEMP "ability-radar-fake-bin"
New-Item -ItemType Directory -Force -Path $fakeBin | Out-Null
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "codex.exe")
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "claude.exe")
"$fakeBin" | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append`;
if (
  fakeInstall?.run !== expectedFakeInstallRun ||
  fakeInstall?.uses ||
  !exactObject(fakeInstall?.env, {}) ||
  !exactObject(fakeInstall?.with, {})
) {
  fail(`${ciPath} ${fakeInstallName} step must have the exact fake-only run contract`);
}
const expectedFakeE2eRun =
  "cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored";
requireExactStepContract(
  ciPath,
  fakeInstall,
  { name: fakeInstallName, run: expectedFakeInstallRun },
  fakeInstallName,
);
requireExactStepContract(
  ciPath,
  fakeE2e,
  {
    name: fakeE2eName,
    run: expectedFakeE2eRun,
    env: { ABILITY_RADAR_FAKE_CLI_E2E: "1" },
  },
  fakeE2eName,
);
if (
  fakeE2e?.run !== expectedFakeE2eRun ||
  fakeE2e?.uses ||
  !exactObject(fakeE2e?.with, {})
) {
  fail(`${ciPath} ${fakeE2eName} step must have the exact E2E run contract`);
}
if (
  fakeE2e?.envDeclaration !== "block" ||
  fakeE2e?.envValid !== true ||
  !exactObject(fakeE2e?.env, { ABILITY_RADAR_FAKE_CLI_E2E: "1" })
) {
  fail(`${ciPath} fake CLI E2E environment must exactly opt in on ${fakeE2eName}`);
}
const fakeInstallIndex = ciJob?.steps.indexOf(fakeInstall) ?? -1;
const fakeE2eIndex = ciJob?.steps.indexOf(fakeE2e) ?? -1;
if (fakeInstallIndex < 0 || fakeE2eIndex !== fakeInstallIndex + 1) {
  fail(`${ciPath} fake CLI install must be immediately before its E2E step`);
}
for (const step of ciJob?.steps ?? []) {
  if (step === fakeInstall || step === fakeE2e) continue;
  if (
    /ability-radar-fake-cli|ability-radar-fake-bin|fake_cli_e2e/.test(step.run) ||
    Object.hasOwn(step.env, "ABILITY_RADAR_FAKE_CLI_E2E")
  ) {
    fail(`${ciPath} fake CLI commands and opt-in may exist only in the named fake steps`);
  }
}
const ciArtifact = actionSteps(ciWorkflow, "actions/upload-artifact")[0];
requireExactStepFields(ciPath, ciArtifact, [], "CI artifact owner");
const expectedCiArtifactInputs = {
  name: "ability-radar-windows-debug-nsis",
  path: "target/debug/bundle/nsis/ability-radar_0.2.1_x64-setup.exe",
  "if-no-files-found": "error",
  "retention-days": "7",
};
requireExactStepContract(
  ciPath,
  ciArtifact,
  {
    name: "Upload exact debug installer",
    uses: "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
    usesComment: "v7",
    with: expectedCiArtifactInputs,
  },
  "CI artifact owner",
);
if (
  ciArtifact?.name !== "Upload exact debug installer" ||
  !exactObject(ciArtifact?.with, expectedCiArtifactInputs)
) {
  fail(`${ciPath} CI artifact input allowlist must contain only the exact debug NSIS installer`);
}
for (const path of workflowPaths) {
  const workflow = workflows.get(path)?.workflow;
  for (const step of actionSteps(workflow, "actions/upload-artifact")) {
    if (/fake|tools[\\/]fake-cli|ability-radar-fake-cli/i.test(step.with.path ?? "")) {
      fail(`${path} must never upload the fake CLI`);
    }
  }
}

const releasePath = ".github/workflows/release.yml";
const releaseWorkflow = workflows.get(releasePath)?.workflow;
const releaseJob = releaseWorkflow?.jobs.get("release");
requireNoTopLevelEnv(releasePath, releaseWorkflow, "release workflow");
requireExactJobContract(
  releasePath,
  releaseJob,
  {
    id: "release",
    runsOn: "windows-latest",
    timeoutMinutes: "60",
    permissions: { contents: "write" },
    env: { RELEASE_TAG: "${{ github.ref_name }}" },
  },
  "release job",
);
const exactVerifyTagRun = `$tag = $env:RELEASE_TAG
if ($tag -cnotmatch '^v(0|[1-9]\\d*)\\.(0|[1-9]\\d*)\\.(0|[1-9]\\d*)$') {
  throw "Release tag must be a strict vMAJOR.MINOR.PATCH semantic version."
}
$config = Get-Content apps/desktop/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
if ("v$($config.version)" -cne $tag) {
  throw "Release tag does not exactly match the Tauri application version."
}`;
const exactReleaseBody = `Windows 10/11 x64 v0.2.1 预览版。

**警告：安装程序和免安装 ZIP 均未签名。** Windows SmartScreen 可能显示风险提示。
核心数据默认只保存在本机；真实 CLI 测试消耗运行者自己的订阅用量。
下载后请使用随发布提供的 SHA256SUMS.txt 校验所有下载文件。`;
const exactChecksumRun = `$version = $env:RELEASE_TAG.Substring(1)
$bundleRoot = "target/release/bundle"
$expected = @(
  "target/release/bundle/nsis/ability-radar_\${version}_x64-setup.exe"
  "target/release/bundle/msi/ability-radar_\${version}_x64_en-US.msi"
  "target/release/bundle/portable/ability-radar_\${version}_windows-x64-portable.zip"
)
$leafNames = @($expected | ForEach-Object { Split-Path -Leaf $_ })
if ($expected.Count -ne 3 -or ($leafNames | Select-Object -Unique).Count -ne 3) {
  throw "The reviewed release checksum set must contain three distinct leaf names."
}
foreach ($path in $expected) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Missing expected release asset: $path"
  }
}
$expectedFullNames = @($expected | ForEach-Object { [IO.Path]::GetFullPath($_) })
$observed = @(Get-ChildItem -LiteralPath $bundleRoot -Recurse -File |
  Where-Object { $_.Extension -in ".exe", ".msi", ".zip" } |
  ForEach-Object { $_.FullName })
$unexpected = @($observed | Where-Object { $_ -notin $expectedFullNames })
if ($observed.Count -ne 3 -or $unexpected.Count -ne 0) {
  throw "Unexpected release asset set: $($unexpected -join ', ')"
}
$lines = foreach ($path in $expected) {
  $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
  "$hash  $(Split-Path -Leaf $path)"
}
Set-Content -LiteralPath SHA256SUMS.txt -Value $lines -Encoding utf8NoBOM`;
const exactPortableUploadRun = `$version = $env:RELEASE_TAG.Substring(1)
$portable = "target/release/bundle/portable/ability-radar_\${version}_windows-x64-portable.zip"
if (-not (Test-Path -LiteralPath $portable -PathType Leaf)) {
  throw "The exact portable archive is missing."
}
gh release upload $env:RELEASE_TAG $portable SHA256SUMS.txt --clobber`;
const exactReleaseSteps = [
  {
    name: "Check out tagged revision",
    label: "release checkout input",
    uses: "actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10",
    usesComment: "v6",
    with: { "fetch-depth": "0", "persist-credentials": "false" },
  },
  {
    name: "Verify release tag",
    run: exactVerifyTagRun,
    extra: { shell: "pwsh" },
  },
  {
    name: "Set up Node.js",
    uses: "actions/setup-node@249970729cb0ef3589644e2896645e5dc5ba9c38",
    usesComment: "v6",
    with: { "node-version": "22", cache: "npm" },
  },
  {
    name: "Set up Rust",
    uses: "dtolnay/rust-toolchain@2c7215f132e9ebf062739d9130488b56d53c060c",
    usesComment: "reviewed master",
    with: { toolchain: "stable", components: "clippy,rustfmt" },
  },
  { name: "Install frontend dependencies", run: "npm ci" },
  {
    name: "Validate repository contracts",
    run: "npm run validate:repository",
  },
  { name: "Check Rust formatting", run: "cargo fmt --all --check" },
  {
    name: "Lint Rust",
    run: "cargo clippy --workspace --all-targets --locked -- -D warnings",
  },
  {
    name: "Test Rust",
    run: "cargo test --workspace --all-targets --locked",
  },
  { name: "Test frontend", run: "npm test" },
  {
    name: "Build unsigned draft prerelease",
    label: "Tauri release input allowlist owner",
    uses: "tauri-apps/tauri-action@944946e3e4cac6603d1fe8f514171e9ecd3c78aa",
    usesComment: "v1",
    env: { GITHUB_TOKEN: "${{ github.token }}" },
    with: {
      projectPath: "apps/desktop",
      tauriScript: "npm run tauri --",
      tagName: "${{ env.RELEASE_TAG }}",
      releaseName: "AI 能力雷达 ${{ env.RELEASE_TAG }}",
      releaseBody: exactReleaseBody,
      releaseDraft: "true",
      prerelease: "true",
      uploadUpdaterJson: "false",
      uploadUpdaterSignatures: "false",
    },
    extra: { id: "tauri" },
  },
  {
    name: "Build portable archive from reviewed release output",
    run: "npm run package:portable:from-build",
  },
  {
    name: "Generate SHA-256 checksums",
    run: exactChecksumRun,
    extra: { shell: "pwsh" },
  },
  {
    name: "Upload portable archive and checksums to the draft prerelease",
    label: "portable archive and checksum upload",
    run: exactPortableUploadRun,
    env: { GH_TOKEN: "${{ github.token }}" },
    extra: { shell: "pwsh" },
  },
];
if (
  JSON.stringify(releaseJob?.steps.map((step) => step.name)) !==
  JSON.stringify(exactReleaseSteps.map(({ name }) => name))
) {
  fail(`${releasePath} release step sequence must be exact`);
}
for (const [index, expected] of exactReleaseSteps.entries()) {
  requireExactStepContract(
    releasePath,
    releaseJob?.steps[index],
    expected,
    expected.label ?? expected.name,
  );
}

const pagesPath = ".github/workflows/pages.yml";
const pagesWorkflow = workflows.get(pagesPath)?.workflow;
const pagesBuild = pagesWorkflow?.jobs.get("build");
const pagesDeploy = pagesWorkflow?.jobs.get("deploy");
requireNoTopLevelEnv(pagesPath, pagesWorkflow, "Pages workflow");
requireExactJobContract(
  pagesPath,
  pagesBuild,
  {
    id: "build",
    runsOn: "ubuntu-latest",
    timeoutMinutes: "10",
    permissions: { contents: "read", pages: "read" },
  },
  "Pages build job",
);
requireExactJobContract(
  pagesPath,
  pagesDeploy,
  {
    id: "deploy",
    runsOn: "ubuntu-latest",
    timeoutMinutes: "10",
    permissions: { pages: "write", "id-token": "write" },
    extra: { needs: "build", environment: "" },
  },
  "Pages deploy job",
);
const expectedPagesBuildSteps = [
  "Check out repository",
  "Set up Node.js",
  "Install repository dependencies",
  "Configure Pages",
  "Validate repository contracts",
  "Assemble static site",
  "Upload Pages artifact",
];
const expectedPagesDeploySteps = ["Deploy"];
if (
  JSON.stringify(pagesBuild?.steps.map((step) => step.name)) !==
    JSON.stringify(expectedPagesBuildSteps) ||
  JSON.stringify(pagesDeploy?.steps.map((step) => step.name)) !==
    JSON.stringify(expectedPagesDeploySteps)
) {
  fail(`${pagesPath} Pages step sequence must be exact`);
}
if (!exactPermissions(pagesBuild, { contents: "read", pages: "read" })) {
  fail(`${pagesPath} build permissions must be exactly contents: read and pages: read`);
}
if (!exactPermissions(pagesDeploy, { pages: "write", "id-token": "write" })) {
  fail(`${pagesPath} deploy permissions must be exactly pages: write and id-token: write`);
}
requireCommand(
  pagesPath,
  pagesBuild,
  "site assembly",
  /(?:^|\n)\s*cp docs\/privacy\.md _site\/docs\/privacy\.md\s*(?:$|\n)/,
);
const assembleSite = namedStep(pagesPath, pagesBuild, "Assemble static site");
const pagesCheckout = namedStep(pagesPath, pagesBuild, "Check out repository");
const pagesNodeSetup = namedStep(pagesPath, pagesBuild, "Set up Node.js");
const pagesInstall = namedStep(
  pagesPath,
  pagesBuild,
  "Install repository dependencies",
);
const configurePages = namedStep(pagesPath, pagesBuild, "Configure Pages");
const validatePages = namedStep(
  pagesPath,
  pagesBuild,
  "Validate repository contracts",
);
const deployPages = namedStep(pagesPath, pagesDeploy, "Deploy");
requireExactStepFields(pagesPath, pagesCheckout, [], "Pages checkout");
requireExactStepFields(pagesPath, pagesNodeSetup, [], "Pages Node setup");
requireExactStepFields(pagesPath, pagesInstall, [], "Pages dependency installation");
requireExactStepFields(pagesPath, configurePages, [], "Configure Pages");
requireExactStepFields(
  pagesPath,
  validatePages,
  [],
  "Validate repository contracts",
);
requireExactStepFields(pagesPath, assembleSite, [], "Assemble static site");
requireExactStepFields(pagesPath, deployPages, ["id"], "Deploy Pages owner");
if (validatePages?.run !== "node scripts/validate-repository.mjs") {
  fail(`${pagesPath} Pages step sequence and commands must be exact`);
}
const expectedSiteAssembly = `cp -R site _site
mkdir -p _site/docs
cp docs/privacy.md _site/docs/privacy.md
cp docs/security.md _site/docs/security.md
cp docs/methodology.md _site/docs/methodology.md
cp docs/troubleshooting.md _site/docs/troubleshooting.md`;
const exactPagesBuildContracts = [
  {
    name: "Check out repository",
    label: "Pages checkout input",
    uses: "actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10",
    usesComment: "v6",
    with: { "persist-credentials": "false" },
  },
  {
    name: "Set up Node.js",
    label: "Pages Node setup",
    uses: "actions/setup-node@249970729cb0ef3589644e2896645e5dc5ba9c38",
    usesComment: "v6",
    with: { "node-version": "22", cache: "npm" },
  },
  {
    name: "Install repository dependencies",
    label: "Pages dependency installation",
    run: "npm ci",
  },
  {
    name: "Configure Pages",
    label: "Configure Pages input",
    uses: "actions/configure-pages@983d7736d9b0ae728b81ab479565c72886d7745b",
    usesComment: "v5",
  },
  {
    name: "Validate repository contracts",
    label: "Pages validator",
    run: "node scripts/validate-repository.mjs",
  },
  {
    name: "Assemble static site",
    label: "Pages assembly",
    run: expectedSiteAssembly,
  },
  {
    name: "Upload Pages artifact",
    label: "Upload Pages artifact",
    uses:
      "actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9",
    usesComment: "v5",
    with: { path: "_site" },
  },
];
for (const [index, expected] of exactPagesBuildContracts.entries()) {
  requireExactStepContract(
    pagesPath,
    pagesBuild?.steps[index],
    expected,
    expected.label,
  );
}
requireExactStepContract(
  pagesPath,
  pagesDeploy?.steps[0],
  {
    name: "Deploy",
    uses: "actions/deploy-pages@cd2ce8fcbc39b97be8ca5fce6e763baed58fa128",
    usesComment: "v5",
    extra: { id: "deployment" },
  },
  "Deploy Pages input",
);
if (
  assembleSite?.run !== expectedSiteAssembly ||
  assembleSite?.uses ||
  !exactObject(assembleSite?.env, {}) ||
  !exactObject(assembleSite?.with, {})
) {
  fail(`${pagesPath} Assemble static site step must own the exact site assembly commands`);
}
for (const step of runSteps(pagesWorkflow)) {
  if (
    step !== assembleSite &&
    /_site(?:\/|\b)/.test(step.run)
  ) {
    fail(`${pagesPath} non-assembly steps must not write into _site`);
  }
}
const pagesArtifact = actionSteps(
  pagesWorkflow,
  "actions/upload-pages-artifact",
)[0];
requireExactStepFields(pagesPath, pagesArtifact, [], "Upload Pages artifact");
if (
  pagesArtifact?.name !== "Upload Pages artifact" ||
  !exactObject(pagesArtifact?.with, { path: "_site" })
) {
  fail(`${pagesPath} Pages artifact path must be exactly _site`);
}

const expectedTauriResources = {
  "../../../benchmark-packs/": "benchmark-packs/",
};
if (
  JSON.stringify(tauriConfig.bundle?.targets) !==
  JSON.stringify(["nsis", "msi"])
) {
  fail("Tauri bundle targets must be exactly NSIS and MSI");
}
if (!exactObject(tauriConfig.bundle?.resources, expectedTauriResources)) {
  fail("Tauri resource allowlist must contain only the sealed benchmark packs");
}

const updaterInputs = [
  "package.json",
  "apps/desktop/package.json",
  "apps/desktop/src-tauri/Cargo.toml",
  "apps/desktop/src-tauri/tauri.conf.json",
  "Cargo.lock",
  "package-lock.json",
].map((path) => read(path)).join("\n");
if (/tauri-plugin-updater|@tauri-apps\/plugin-updater|\"updater\"\s*:|createUpdaterArtifacts/i.test(updaterInputs)) {
  fail("Tauri updater plugin/configuration must remain absent");
}

const site = requireText("site/index.html", [
  ["restrictive CSP", /default-src 'self';[^"]*img-src 'none';[^"]*font-src 'none';[^"]*connect-src 'none'/],
  ["Windows scope", /Windows 10\/11 x64/],
  ["Node LTS scope", /Node\.js 22\/24 LTS/],
  ["no degradation verdict", /不生成.*(?:降智|退化).*(?:裁决|结论)/],
  ["unsigned installer warning", /未签名/],
  ["subscription payer", /运行者.*(?:订阅|用量|费用)/],
  ["methodology link", /href="docs\/methodology\.md"/],
  ["privacy link", /href="docs\/privacy\.md"/],
  ["security link", /href="docs\/security\.md"/],
  ["v0.2.1 candidate/pending status", /v0\.2\.1.*候选\/待发布/si],
  ["inactive v0.2.1 CTA", /<span[^>]*id="release-link"[^>]*aria-disabled="true"[^>]*>v0\.2\.1 下载待开放<\/span>/],
  ["clean Windows public-release download gate", /clean Windows 10\/11 x64.*公开发布.*开放下载/si],
]);
if (/\/releases\/(?:latest|tag\/v0\.2\.1)/.test(site)) {
  fail("site/index.html must not expose a release URL while v0.2.1 is pending");
}
if (/<a\b[^>]*id="(?:release-link|footer-release)"|releaseUrl\s*=|下载 v0\.2\.1 安装程序/si.test(site)) {
  fail("site/index.html must keep every v0.2.1 public download CTA inactive while pending");
}
if (/v0\.2\.1\s*(?:公开预览|当前发布|正式发布).*?(?:提供|开放).*?下载/si.test(site)) {
  fail("site/index.html must not claim that pending v0.2.1 is a current public release");
}
const forbiddenSitePatterns = [
  ["external resource URL", /(?:src|action)=["']https?:/i],
  ["external CSS URL", /url\(\s*["']?https?:/i],
  ["network API", /\b(?:fetch|XMLHttpRequest|sendBeacon|WebSocket)\s*\(/],
  ["analytics or tracking", /\b(?:analytics|gtag|googletagmanager|pixel|tracking)\b/i],
  ["cookie access", /document\.cookie/i],
  ["image element", /<img\b/i],
  ["font face", /@font-face/i],
];
for (const [label, pattern] of forbiddenSitePatterns) {
  if (pattern.test(site)) fail(`site/index.html contains forbidden ${label}`);
}

const readme = requireText("README.md", [
  ["exact v0.2.1 pending-status banner", /^> 当前状态：v0\.2\.1 Windows 候选\/待发布构建。支持 Windows 10\/11 x64；公开下载尚未开放，安装程序尚未签名，也没有自动更新。$/m],
  ["exact client task count", /8\s*道/],
  ["exact CLI task count", /2\s*(?:个|项)/],
  ["fake CI cost boundary", /GitHub CI.*(?:假|fake).*CLI/si],
  ["runner billing boundary", /GitHub.*runner.*仓库所有者.*GitHub.*计划/si],
  ["volunteer real-CLI cost boundary", /自愿.*测试.*自己的订阅/si],
  ["checksum verification", /SHA-?256/],
  ["clean Windows public-release download gate", /clean Windows 10\/11 x64.*公开发布.*开放下载/si],
  ["npm ci and npm start commands", /```powershell\r?\nnpm ci\r?\nnpm start\r?\n```/],
  ["package:portable command", /```powershell\r?\nnpm run package:portable\r?\n```/],
  ["Tauri desktop development window", /npm start.*Tauri 桌面开发窗口/si],
  ["normal browser is incomplete", /普通浏览器.*http:\/\/localhost:1420.*不是完整产品/si],
  ["portable APPDATA location", /免安装 ZIP.*%APPDATA%\\com\.aiability\.radar/si],
  ["design link", /docs\/superpowers\/specs\/2026-07-17-ai-ability-radar-design\.md/],
  ["plan link", /docs\/superpowers\/plans\/2026-07-17-ai-ability-radar-desktop-mvp\.md/],
]);
if (/normal browser is the complete product/i.test(readme)) {
  fail("README.md must not describe localhost:1420 in a normal browser as the complete product");
}
if (/从仓库的\s*\*\*Releases\*\*\s*页面下载 v0\.2\.1|v0\.2\.1\s*(?:公开预览|当前发布|正式发布).*?(?:提供|开放).*?下载/si.test(readme)) {
  fail("README.md must not claim that pending v0.2.1 is currently downloadable");
}
requireText("docs/methodology.md", [
  ["category-equal weighting", /类别等权/],
  ["original first-party tasks", /原创.*第一方/],
  ["Codex Radar exclusion", /Codex Radar/],
  ["DeepSWE exclusion", /DeepSWE/],
  ["contamination limitation", /污染/],
  ["default model semantics", /空白.*default/si],
  ["effort values", /low.*medium.*high/si],
  ["duration separation", /时长.*不.*跨.*比较/si],
  ["complete history key", /target kind.*trimmed reported model.*reasoning effort.*run mode.*suite ID\/version\/hash.*scoring-rule version.*OS family\/version.*app version.*CLI version.*Node verifier version.*clean-versus-resumed state/si],
  ["no v0.2 verdict", /v0\.2.*不生成.*(?:退化|降智).*裁决/si],
  ["planned v0.5 boundary", /v0\.5.*计划/si],
  ["infrastructure and budget distinction", /基础设施无效.*agent-budget/si],
  ["scoring rule version", /ability-v1/],
  ["pack schema version", /pack schema.*1/i],
  ["public report schema version", /public report schema.*1/i],
  ["backup schema version", /backup schema.*1/i],
  ["provider effort matrix ChatGPT", /\| ChatGPT 客户端 \|[^|]*轻度[^|]*中[^|]*高[^|]*极高[^|]*最高[^|]*Ultra[^|]*原样填写[^|]*\|/],
  ["provider effort matrix Claude", /\| Claude 客户端 \|[^|]*低[^|]*中[^|]*高[^|]*极高[^|]*最高[^|]*原样填写[^|]*\|/],
  ["provider effort matrix Codex", /\| Codex CLI \|[^|]*`minimal`[^|]*`low`[^|]*`medium`[^|]*`high`[^|]*`xhigh`[^|]*`max`[^|]*`ultra`[^|]*自定义 \|/],
  ["provider effort matrix Claude Code", /\| Claude Code \|[^|]*`low`[^|]*`medium`[^|]*`high`[^|]*`xhigh`[^|]*`max`[^|]*自定义 \|/],
  ["known reasoning lowercase canonical strings", /已知推理值.*保持小写规范字符串/],
  ["manual custom trimmed display text", /手动客户端自定义标签.*保留去除首尾空白后的\s*显示文本/],
  ["CLI custom lowercase safe tokens", /CLI 自定义值.*规范化为小写安全 token/],
  ["history recovery comparison normalization without migration", /写入历史.*恢复运行核对.*同条件比较.*不需要数据库迁移/si],
  ["existing low medium high remains readable", /已有 `low`、`medium`、`high` 历史.*仍可读取.*恢复.*比较/si],
]);
const methodology = read("docs/methodology.md");
if (/已知推理值保留输入大小写/.test(methodology)) {
  fail("docs/methodology.md contradicts the known reasoning lowercase canonical rule");
}
if (/手动自定义标签统一转为小写/.test(methodology)) {
  fail("docs/methodology.md contradicts the manual custom display-text rule");
}
if (/CLI 自定义值保留原始大小写/.test(methodology)) {
  fail("docs/methodology.md contradicts the CLI custom lowercase safe-token rule");
}
for (const path of ["docs/privacy.md", "docs/security.md"]) {
  requireText(path, [
    ["no app telemetry endpoint", /应用.*(?:没有|无).*遥测.*(?:上传端点|endpoint)/si],
    ["provider traffic disclosure", /提示词.*临时.*代码.*AI.*提供商/si],
    ["provider policy disclosure", /CLI.*提供商.*日志.*保留.*遥测/si],
    ["normal deletion", /正常.*删除/],
    ["SQLite secure_delete", /secure_delete/],
    ["WAL truncation", /WAL.*截断/si],
    ["retention limitations", /SSD.*文件系统快照.*杀毒.*外部备份/si],
    ["not a forensic wipe", /不是.*取证.*擦除/],
    ["real isolation controls", /workspace-write.*Read\/Edit\/Write.*dontAsk/si],
    ["not a strong sandbox", /不是.*(?:容器|VM|虚拟机).*(?:sandbox|沙箱)/si],
  ]);
}
requireText("apps/desktop/src/pages/HomePage.tsx", [
  ["inherited PATH immediate recheck", /已继承 PATH 目录内的变化可立即重新检测/],
  ["new PATH directory restart boundary", /新增 PATH\s*目录，请重启应用后再重新检测/],
]);
requireText("docs/troubleshooting.md", [
  ["missing CLI", /CLI.*未找到/],
  ["login", /登录/],
  ["Node.js support", /Node\.js 22\/24 LTS/],
  ["quota", /配额/],
  ["network", /网络/],
  ["SmartScreen", /SmartScreen/],
  ["interrupted recovery", /中断.*恢复/si],
  ["local app data placeholder", /%APPDATA%/],
  ["npm shim checks with codex.cmd --version", /```powershell\r?\nGet-Command codex -All\r?\nwhere\.exe codex\r?\ncodex\.cmd --version\r?\n```/],
  ["--version sends no model request", /`--version`.*不会发送模型请求/si],
  ["in-app 重新检测 CLI path", /重新检测 CLI/],
  ["inherited PATH immediate recheck", /已经继承的 PATH 目录内，可以立即重新检测/],
  ["new User or Machine PATH restart boundary", /新增了 User 或 Machine PATH 目录，请先重启应用/],
]);
const troubleshooting = read("docs/troubleshooting.md");
if (/重新检测 CLI[”"`'，,\s]*无需重启应用/.test(troubleshooting)) {
  fail("docs/troubleshooting.md must not make an unconditional no-restart CLI re-detection claim");
}
requireText("docs/release-checklist.md", [
  ["portable archive gates", /免安装 ZIP.*Windows 10.*Windows 11/si],
  ["portable APPDATA gate", /免安装 ZIP.*%APPDATA%\\com\.aiability\.radar/si],
  ["outer checksum release assets", /外层 `SHA256SUMS\.txt`.*NSIS.*MSI.*portable ZIP/si],
  ["release upload ownership", /Tauri action.*唯一.*安装程序上传者.*gh release upload/si],
]);
requireText("docs/test-matrix.md", [
  ["Portable ZIP launch Windows matrix row", /^\| Portable ZIP launch \| Yes \| Yes \| No \| Yes \|$/m],
  ["Portable APPDATA Windows matrix row", /^\| Portable data remains in `%APPDATA%\\com\.aiability\.radar` \| Yes \| Yes \| No \| Yes \|$/m],
  ["Portable checksum Windows matrix row", /^\| Portable inner checksum verification \| Yes \| Yes \| Yes \| Yes \|$/m],
]);
requireText("SECURITY.md", [
  ["private GitHub advisory reporting", /Security.*Advisory.*Report a vulnerability/si],
  ["no raw public vulnerability details", /不要.*公开/],
]);
requireText(".github/ISSUE_TEMPLATE/bug.yml", [
  ["app version", /应用版本/],
  ["Windows version", /Windows 版本/],
  ["target type", /目标类型/],
  ["task pack version", /题包版本/],
  ["redacted category", /脱敏.*错误类别/],
  ["raw log warning", /不要.*原始日志.*(?:令牌|token)/si],
  ["v0.2.1 app version example", /placeholder:\s*例如 0\.2\.1/],
]);
requireText(".github/pull_request_template.md", [
  ["tests checklist", /测试.*(?:新增|更新)/],
  ["no real CI subscription CLI", /CI.*真实.*订阅.*CLI/],
  ["privacy field review", /隐私字段/],
  ["capability diff review", /capability.*diff/i],
  ["task license review", /题包.*许可/],
  ["Windows process check", /Windows.*进程.*取消/],
]);

const markdownPaths = [
  "README.md",
  "CONTRIBUTING.md",
  "SECURITY.md",
  "THIRD_PARTY_NOTICES.md",
  "docs/privacy.md",
  "docs/security.md",
  "docs/troubleshooting.md",
  "docs/methodology.md",
];
for (const path of markdownPaths) {
  const source = read(path);
  for (const match of source.matchAll(/!?\[[^\]]*]\(([^)]+)\)/g)) {
    let target = match[1].trim().replace(/^<|>$/g, "").split("#", 1)[0];
    if (!target || /^(?:https?:|mailto:|#)/i.test(target)) continue;
    target = decodeURIComponent(target);
    const absolute = normalize(resolve(root, dirname(path), target));
    if (relative(root, absolute).startsWith("..") || !existsSync(absolute)) {
      fail(`${path} has a broken internal link: ${match[1]}`);
    }
  }
}
for (const href of site.matchAll(/href="([^"]+)"/g)) {
  const target = href[1].split("#", 1)[0];
  if (!target || /^(?:https?:|#)/.test(target) || target === "./") continue;
  const sourcePath = join(root, "site", target);
  const repositoryPath = target.startsWith("docs/")
    ? join(root, target)
    : sourcePath;
  if (!existsSync(sourcePath) && !existsSync(repositoryPath)) {
    fail(`site/index.html has a broken internal link: ${href[1]}`);
  }
}

const rustReport = json("docs/licenses/rust-dependencies.json");
const npmReport = json("docs/licenses/npm-dependencies.json");
if (rustReport.generatedFrom !== "Cargo.lock") {
  fail("Rust license report must declare Cargo.lock as its source");
}
if (npmReport.generatedFrom !== "package-lock.json") {
  fail("npm license report must declare package-lock.json as its source");
}
if (rustReport.hashNormalization !== "UTF-8 text with CRLF and CR normalized to LF") {
  fail("Rust license report must declare cross-platform line-ending normalization");
}
if (npmReport.hashNormalization !== "UTF-8 text with CRLF and CR normalized to LF") {
  fail("npm license report must declare cross-platform line-ending normalization");
}
if (rustReport.lockfileSha256 !== sha256("Cargo.lock")) {
  fail("Rust license report is stale relative to Cargo.lock");
}
if (npmReport.lockfileSha256 !== sha256("package-lock.json")) {
  fail("npm license report is stale relative to package-lock.json");
}
const rustCoverage = new Map((rustReport.packages ?? []).map((pkg) => [packageKey(pkg), pkg]));
const npmCoverage = new Map((npmReport.packages ?? []).map((pkg) => [packageKey(pkg), pkg]));
const lockedRustPackages = cargoPackages(read("Cargo.lock")).filter(({ source }) => source);
const lockedNpmPackages = npmLockPackages(npmLock);
const expectedRustKeys = [...new Set(lockedRustPackages.map(packageKey))].sort();
const expectedNpmKeys = [...new Set(lockedNpmPackages.map(packageKey))].sort();
const reportedRustKeys = (rustReport.packages ?? []).map(packageKey);
const reportedNpmKeys = (npmReport.packages ?? []).map(packageKey);
if (new Set(reportedRustKeys).size !== reportedRustKeys.length) {
  fail("Rust license report contains duplicate package versions");
}
if (reportedRustKeys.some((key) => key.startsWith("ability-radar-fake-cli@"))) {
  fail("Rust third-party license report must exclude the first-party fake CLI workspace package");
}
if (new Set(reportedNpmKeys).size !== reportedNpmKeys.length) {
  fail("npm license report contains duplicate package versions");
}
if (JSON.stringify([...reportedRustKeys].sort()) !== JSON.stringify(expectedRustKeys)) {
  fail("Rust license report package set does not exactly match Cargo.lock");
}
if (JSON.stringify([...reportedNpmKeys].sort()) !== JSON.stringify(expectedNpmKeys)) {
  fail("npm license report package set does not exactly match package-lock.json");
}
for (const pkg of lockedRustPackages) {
  const metadata = rustCoverage.get(packageKey(pkg));
  if (!metadata) fail(`Rust license report does not cover ${packageKey(pkg)}`);
  else if (!metadata.license) fail(`Rust license report lacks a license for ${packageKey(pkg)}`);
}
for (const pkg of lockedNpmPackages) {
  const metadata = npmCoverage.get(packageKey(pkg));
  if (!metadata) fail(`npm license report does not cover ${packageKey(pkg)}`);
  else {
    if (!metadata.license) {
      fail(`npm license report lacks a license for ${packageKey(pkg)}`);
    }
    if (metadata.resolved !== pkg.resolved) {
      fail(`npm license report resolved URL differs from package-lock.json for ${packageKey(pkg)}`);
    }
    if (metadata.integrity !== pkg.integrity) {
      fail(`npm license report integrity differs from package-lock.json for ${packageKey(pkg)}`);
    }
  }
}
requireText("THIRD_PARTY_NOTICES.md", [
  ["Rust generated metadata", /docs\/licenses\/rust-dependencies\.json/],
  ["npm generated metadata", /docs\/licenses\/npm-dependencies\.json/],
  ["first-party client pack Apache-2.0", /client-quick.*Apache-2\.0/si],
  ["first-party CLI pack Apache-2.0", /cli-quick.*Apache-2\.0/si],
  ["DeepSWE excluded", /DeepSWE.*(?:未包含|不包含|excluded)/si],
  ["metadata-only limitation", /元数据.*不.*完整.*许可文本/si],
]);

assert.equal(extname(join(root, "site", ".nojekyll")), "");

if (errors.length > 0) {
  console.error(`Repository validation failed with ${errors.length} issue(s):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

console.log("Repository validation passed: workflows, versions, site, docs, links, licenses, and exclusions are consistent.");
