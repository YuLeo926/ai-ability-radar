import { LauncherError } from "./errors.mjs";

export const MANIFEST_SCHEMA_VERSION = "launcher-release-manifest-v1";
export const RELEASE_REPOSITORY = "YuLeo926/ai-ability-radar";
export const PORTABLE_ROOT_DIRECTORY = "ability-radar-portable";
export const MAX_PORTABLE_BYTES = 256 * 1024 * 1024;
export const MAX_EXTRACTED_FILE_BYTES = 512 * 1024 * 1024;
export const MAX_EXTRACTED_TOTAL_BYTES = 1024 * 1024 * 1024;
export const MAX_MANIFEST_BYTES = 4 * 1024 * 1024;

const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;
const SHA256 = /^[a-f0-9]{64}$/u;
const SAFE_SEGMENT = /^[A-Za-z0-9 _.-]+$/u;
const RESERVED_WINDOWS_NAME = /^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?$/iu;

function invalidManifest(detail, cause) {
  return new LauncherError(
    "INVALID_MANIFEST",
    `发布清单无效：${detail}。`,
    cause === undefined ? undefined : { cause },
  );
}

function versionMismatch(detail) {
  return new LauncherError(
    "VERSION_MISMATCH",
    `启动器版本与发布资产不一致：${detail}。`,
  );
}

function plainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireExactKeys(value, expected, label) {
  if (!plainObject(value)) {
    throw invalidManifest(`${label}必须是对象`);
  }
  const actual = Object.keys(value).sort();
  const required = [...expected].sort();
  if (
    actual.length !== required.length ||
    actual.some((key, index) => key !== required[index])
  ) {
    throw invalidManifest(`${label}字段集合不正确`);
  }
}

function requireSha256(value, label) {
  if (typeof value !== "string" || !SHA256.test(value)) {
    throw invalidManifest(`${label}不是小写 SHA-256`);
  }
  return value;
}

function requireSafeInteger(value, minimum, maximum, label) {
  if (
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw invalidManifest(`${label}超出允许范围`);
  }
  return value;
}

function requireSafePortablePath(value, rootDirectory, label) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.startsWith("/") ||
    value.includes("\\") ||
    value.includes(":") ||
    !value.startsWith(`${rootDirectory}/`)
  ) {
    throw invalidManifest(`${label}不是受支持的相对路径`);
  }
  const segments = value.split("/");
  if (
    segments.some(
      (segment) =>
        segment.length === 0 ||
        segment === "." ||
        segment === ".." ||
        !SAFE_SEGMENT.test(segment) ||
        segment.trim() !== segment ||
        segment.endsWith(".") ||
        RESERVED_WINDOWS_NAME.test(segment),
    )
  ) {
    throw invalidManifest(`${label}包含不安全的 Windows 路径段`);
  }
  return value;
}

function compareUtf8Ascii(left, right) {
  if (left === right) return 0;
  return left < right ? -1 : 1;
}

function deepFreezeManifest(value) {
  const files = Object.freeze(
    value.assets.portable.files.map((entry) => Object.freeze({ ...entry })),
  );
  const portable = Object.freeze({ ...value.assets.portable, files });
  const checksums = Object.freeze({ ...value.assets.checksums });
  const assets = Object.freeze({ portable, checksums });
  return Object.freeze({ ...value, assets });
}

export function deriveReleaseIdentity(version) {
  if (typeof version !== "string" || !STABLE_VERSION.test(version)) {
    throw new LauncherError(
      "INVALID_VERSION",
      "npm 启动器版本必须是稳定的 MAJOR.MINOR.PATCH。",
    );
  }
  const components = version.split(".").map(Number);
  if (components.some((component) => !Number.isSafeInteger(component))) {
    throw new LauncherError(
      "INVALID_VERSION",
      "npm 启动器版本必须是稳定的 MAJOR.MINOR.PATCH。",
    );
  }
  return {
    version,
    repository: RELEASE_REPOSITORY,
    tag: `v${version}`,
    portableFileName: `ability-radar_${version}_windows-x64-portable.zip`,
    checksumsFileName: "SHA256SUMS.txt",
  };
}

export function validateReleaseManifest(value, { packageVersion } = {}) {
  const identity = deriveReleaseIdentity(packageVersion);
  requireExactKeys(
    value,
    [
      "schema_version",
      "repository",
      "launcher_version",
      "desktop_version",
      "tag",
      "assets",
    ],
    "顶层",
  );
  requireExactKeys(value.assets, ["portable", "checksums"], "assets");
  requireExactKeys(
    value.assets.portable,
    [
      "file_name",
      "size",
      "sha256",
      "root_directory",
      "executable",
      "files",
    ],
    "portable",
  );
  requireExactKeys(value.assets.checksums, ["file_name"], "checksums");

  if (value.schema_version !== MANIFEST_SCHEMA_VERSION) {
    throw invalidManifest("清单格式版本不受支持");
  }
  if (
    value.repository !== identity.repository ||
    value.launcher_version !== identity.version ||
    value.desktop_version !== identity.version ||
    value.tag !== identity.tag ||
    value.assets.portable.file_name !== identity.portableFileName ||
    value.assets.checksums.file_name !== identity.checksumsFileName
  ) {
    throw versionMismatch("版本、标签、仓库或文件名不匹配");
  }

  const portable = value.assets.portable;
  if (portable.root_directory !== PORTABLE_ROOT_DIRECTORY) {
    throw invalidManifest("便携根目录不正确");
  }
  const expectedExecutable = `${PORTABLE_ROOT_DIRECTORY}/ability-radar.exe`;
  if (portable.executable !== expectedExecutable) {
    throw invalidManifest("桌面可执行文件路径不正确");
  }
  requireSafeInteger(portable.size, 1, MAX_PORTABLE_BYTES, "ZIP 字节数");
  requireSha256(portable.sha256, "ZIP 哈希");
  if (
    !Array.isArray(portable.files) ||
    portable.files.length < 3 ||
    portable.files.length > 10_000
  ) {
    throw invalidManifest("解压文件清单数量不正确");
  }

  const normalizedFiles = [];
  const foldedPaths = new Set();
  let previousPath;
  let extractedTotal = 0;
  for (const [index, entry] of portable.files.entries()) {
    requireExactKeys(entry, ["path", "size", "sha256"], `files[${index}]`);
    const path = requireSafePortablePath(
      entry.path,
      portable.root_directory,
      `files[${index}].path`,
    );
    if (previousPath !== undefined && compareUtf8Ascii(previousPath, path) >= 0) {
      throw invalidManifest("解压文件清单必须按 UTF-8 路径严格排序");
    }
    previousPath = path;
    const folded = path.toUpperCase();
    if (foldedPaths.has(folded)) {
      throw invalidManifest("解压文件清单包含大小写冲突或重复路径");
    }
    foldedPaths.add(folded);
    const size = requireSafeInteger(
      entry.size,
      0,
      MAX_EXTRACTED_FILE_BYTES,
      `files[${index}].size`,
    );
    extractedTotal += size;
    if (!Number.isSafeInteger(extractedTotal) || extractedTotal > MAX_EXTRACTED_TOTAL_BYTES) {
      throw invalidManifest("解压文件总大小超出允许范围");
    }
    normalizedFiles.push({
      path,
      size,
      sha256: requireSha256(entry.sha256, `files[${index}].sha256`),
    });
  }
  const paths = new Set(normalizedFiles.map(({ path }) => path));
  if (!paths.has(expectedExecutable)) {
    throw invalidManifest("解压文件清单缺少桌面可执行文件");
  }
  if (!paths.has(`${PORTABLE_ROOT_DIRECTORY}/SHA256SUMS.txt`)) {
    throw invalidManifest("解压文件清单缺少内部 SHA256SUMS.txt");
  }

  return deepFreezeManifest({
    schema_version: value.schema_version,
    repository: value.repository,
    launcher_version: value.launcher_version,
    desktop_version: value.desktop_version,
    tag: value.tag,
    assets: {
      portable: {
        file_name: portable.file_name,
        size: portable.size,
        sha256: portable.sha256,
        root_directory: portable.root_directory,
        executable: portable.executable,
        files: normalizedFiles,
      },
      checksums: { file_name: value.assets.checksums.file_name },
    },
  });
}

function scanUniqueJsonObjectKeys(text) {
  let cursor = 0;
  const fail = () => {
    throw invalidManifest("JSON 格式错误或包含重复对象键");
  };
  const whitespace = () => {
    while (/\s/u.test(text[cursor] ?? "")) cursor += 1;
  };
  const string = () => {
    if (text[cursor] !== '"') fail();
    const start = cursor;
    cursor += 1;
    while (cursor < text.length) {
      const character = text[cursor];
      if (character === '"') {
        cursor += 1;
        try {
          return JSON.parse(text.slice(start, cursor));
        } catch {
          fail();
        }
      }
      if (character === "\\") {
        cursor += 1;
        const escape = text[cursor];
        if (escape === "u") {
          if (!/^[a-fA-F0-9]{4}$/u.test(text.slice(cursor + 1, cursor + 5))) fail();
          cursor += 5;
          continue;
        }
        if (!'"\\/bfnrt'.includes(escape ?? "")) fail();
        cursor += 1;
        continue;
      }
      if (character.charCodeAt(0) < 0x20) fail();
      cursor += 1;
    }
    fail();
  };
  const value = () => {
    whitespace();
    const character = text[cursor];
    if (character === "{") {
      cursor += 1;
      whitespace();
      const keys = new Set();
      if (text[cursor] === "}") {
        cursor += 1;
        return;
      }
      while (cursor < text.length) {
        whitespace();
        const key = string();
        if (keys.has(key)) fail();
        keys.add(key);
        whitespace();
        if (text[cursor] !== ":") fail();
        cursor += 1;
        value();
        whitespace();
        if (text[cursor] === "}") {
          cursor += 1;
          return;
        }
        if (text[cursor] !== ",") fail();
        cursor += 1;
      }
      fail();
    }
    if (character === "[") {
      cursor += 1;
      whitespace();
      if (text[cursor] === "]") {
        cursor += 1;
        return;
      }
      while (cursor < text.length) {
        value();
        whitespace();
        if (text[cursor] === "]") {
          cursor += 1;
          return;
        }
        if (text[cursor] !== ",") fail();
        cursor += 1;
      }
      fail();
    }
    if (character === '"') {
      string();
      return;
    }
    for (const literal of ["true", "false", "null"]) {
      if (text.startsWith(literal, cursor)) {
        cursor += literal.length;
        return;
      }
    }
    const number = text.slice(cursor).match(/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/u)?.[0];
    if (!number) fail();
    cursor += number.length;
  };

  value();
  whitespace();
  if (cursor !== text.length) fail();
}

export function parseReleaseManifest(text, options) {
  if (
    typeof text !== "string" ||
    text.startsWith("\uFEFF") ||
    Buffer.byteLength(text, "utf8") > MAX_MANIFEST_BYTES
  ) {
    throw invalidManifest("文本编码或大小不受支持");
  }
  scanUniqueJsonObjectKeys(text);
  let value;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw invalidManifest("JSON 无法解析", error);
  }
  return validateReleaseManifest(value, options);
}
