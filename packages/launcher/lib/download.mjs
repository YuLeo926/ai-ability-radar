import { createHash } from "node:crypto";
import { open, lstat, rm } from "node:fs/promises";
import { request as httpsRequest } from "node:https";
import { isAbsolute } from "node:path";

import { LauncherError, isLauncherError } from "./errors.mjs";
import { MAX_PORTABLE_BYTES } from "./manifest.mjs";

const CHECKSUM_MAX_BYTES = 64 * 1024;
const DEFAULT_TOTAL_TIMEOUT_MS = 120_000;
const CONNECT_TIMEOUT_MS = 15_000;
const IDLE_TIMEOUT_MS = 30_000;
const MAX_REDIRECTS = 5;
const MAX_URL_CHARACTERS = 8_192;
const SHA256 = /^[a-f0-9]{64}$/u;
const ALLOWED_HOSTS = new Set([
  "github.com",
  "objects.githubusercontent.com",
  "release-assets.githubusercontent.com",
]);

function downloadError(code, message) {
  return new LauncherError(code, message);
}

function validateHttpsUrl(value) {
  let url;
  try {
    url = value instanceof URL ? new URL(value) : new URL(value);
  } catch {
    throw downloadError("INVALID_DOWNLOAD_URL", "下载地址不受支持。");
  }
  if (
    url.toString().length > MAX_URL_CHARACTERS ||
    url.protocol !== "https:" ||
    !ALLOWED_HOSTS.has(url.hostname.toLowerCase()) ||
    url.username !== "" ||
    url.password !== "" ||
    url.port !== "" ||
    url.hash !== ""
  ) {
    throw downloadError("INVALID_DOWNLOAD_URL", "下载地址不受支持。");
  }
  return url;
}

function responseHeader(headers, name) {
  const value = headers?.[name];
  if (Array.isArray(value)) {
    if (value.length !== 1) {
      throw downloadError("DOWNLOAD_FAILED", "下载响应头无效。");
    }
    return value[0];
  }
  return value;
}

function parseContentLength(headers) {
  const value = responseHeader(headers, "content-length");
  if (value === undefined) return undefined;
  if (typeof value !== "string" || !/^(?:0|[1-9]\d*)$/u.test(value)) {
    throw downloadError("DOWNLOAD_FAILED", "下载响应长度无效。");
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number)) {
    throw downloadError("DOWNLOAD_FAILED", "下载响应长度无效。");
  }
  return number;
}

function fileIdentity(info) {
  return {
    dev: info.dev,
    ino: info.ino,
    birthtimeNs: info.birthtimeNs,
  };
}

function sameFileIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.birthtimeNs === right.birthtimeNs
  );
}

async function removeOwnedPartial(destination, identity) {
  if (!identity) return;
  try {
    const current = await lstat(destination, { bigint: true });
    if (current.isFile() && sameFileIdentity(identity, fileIdentity(current))) {
      await rm(destination);
    }
  } catch (error) {
    if (error?.code !== "ENOENT") {
      throw downloadError("DOWNLOAD_FAILED", "下载临时文件清理失败。");
    }
  }
}

async function writeFully(file, chunk) {
  let offset = 0;
  while (offset < chunk.length) {
    const { bytesWritten } = await file.write(
      chunk,
      offset,
      chunk.length - offset,
      null,
    );
    if (!Number.isSafeInteger(bytesWritten) || bytesWritten <= 0) {
      throw downloadError("DOWNLOAD_FAILED", "下载临时文件写入失败。");
    }
    offset += bytesWritten;
  }
}

function openHttpsResponse(url, { headers, signal }) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const request = httpsRequest(
      url,
      {
        method: "GET",
        headers,
        signal,
      },
      (response) => {
        settled = true;
        clearTimeout(connectTimer);
        response.setTimeout(IDLE_TIMEOUT_MS, () => {
          response.destroy(new Error("download idle timeout"));
        });
        resolve(response);
      },
    );
    const connectTimer = setTimeout(() => {
      request.destroy(new Error("download connection timeout"));
    }, CONNECT_TIMEOUT_MS);
    connectTimer.unref?.();
    request.once("error", (error) => {
      clearTimeout(connectTimer);
      if (!settled) reject(error);
    });
    request.end();
  });
}

export function buildReleaseAssetUrl(identity, fileName) {
  if (
    identity?.repository !== "YuLeo926/ai-ability-radar" ||
    typeof identity.tag !== "string" ||
    typeof identity.version !== "string" ||
    (fileName !== identity.portableFileName && fileName !== identity.checksumsFileName)
  ) {
    throw downloadError("INVALID_DOWNLOAD_URL", "下载地址不受支持。");
  }
  return validateHttpsUrl(
    `https://github.com/${identity.repository}/releases/download/${identity.tag}/${fileName}`,
  );
}

async function downloadReleaseAsset({
  identity,
  kind,
  expectedSize,
  expectedSha256,
  destination,
  transport = openHttpsResponse,
  totalTimeoutMs = DEFAULT_TOTAL_TIMEOUT_MS,
}) {
  if (!isAbsolute(destination)) {
    throw downloadError("DOWNLOAD_FAILED", "下载临时文件路径无效。");
  }
  if (!Number.isSafeInteger(totalTimeoutMs) || totalTimeoutMs < 1) {
    throw downloadError("DOWNLOAD_FAILED", "下载超时配置无效。");
  }
  let fileName;
  let maximumBytes;
  if (kind === "checksums") {
    fileName = identity?.checksumsFileName;
    maximumBytes = CHECKSUM_MAX_BYTES;
  } else if (
    kind === "portable" &&
    Number.isSafeInteger(expectedSize) &&
    expectedSize > 0 &&
    expectedSize <= MAX_PORTABLE_BYTES &&
    typeof expectedSha256 === "string" &&
    SHA256.test(expectedSha256)
  ) {
    fileName = identity?.portableFileName;
    maximumBytes = expectedSize;
  } else {
    throw downloadError("DOWNLOAD_FAILED", "下载资产参数无效。");
  }
  const initialUrl = buildReleaseAssetUrl(identity, fileName);
  let file;
  let ownedIdentity;
  let succeeded = false;
  let timedOut = false;
  let currentResponse;
  const controller = new AbortController();
  const timeout = setTimeout(() => {
    timedOut = true;
    const reason = new Error("download total timeout");
    controller.abort(reason);
    currentResponse?.destroy?.(reason);
  }, totalTimeoutMs);

  try {
    try {
      file = await open(destination, "wx", 0o600);
      ownedIdentity = fileIdentity(await file.stat({ bigint: true }));
    } catch {
      throw downloadError("DOWNLOAD_FAILED", "无法创建下载临时文件。");
    }

    const headers = {
      "User-Agent": `ai-ability-radar-launcher/${identity.version}`,
      Accept: "application/octet-stream",
      "Accept-Encoding": "identity",
    };
    const visited = new Set();
    let url = initialUrl;
    let redirects = 0;
    while (true) {
      const serialized = url.toString();
      if (visited.has(serialized)) {
        throw downloadError("TOO_MANY_REDIRECTS", "下载跳转形成循环。");
      }
      visited.add(serialized);
      currentResponse = await transport(url, {
        headers,
        signal: controller.signal,
      });
      const statusCode = currentResponse?.statusCode;
      if ([301, 302, 303, 307, 308].includes(statusCode)) {
        const location = responseHeader(currentResponse.headers, "location");
        currentResponse.destroy?.();
        currentResponse = undefined;
        if (typeof location !== "string" || redirects >= MAX_REDIRECTS) {
          throw downloadError("TOO_MANY_REDIRECTS", "下载跳转次数过多。");
        }
        let next;
        try {
          next = new URL(location, url);
        } catch {
          throw downloadError("INVALID_DOWNLOAD_URL", "下载地址不受支持。");
        }
        url = validateHttpsUrl(next);
        redirects += 1;
        continue;
      }
      if (statusCode !== 200) {
        currentResponse?.destroy?.();
        currentResponse = undefined;
        throw downloadError("DOWNLOAD_FAILED", "GitHub Release 下载失败。");
      }
      break;
    }

    const contentEncoding = responseHeader(currentResponse.headers, "content-encoding");
    if (
      contentEncoding !== undefined &&
      (typeof contentEncoding !== "string" || contentEncoding.toLowerCase() !== "identity")
    ) {
      currentResponse.destroy?.();
      throw downloadError("DOWNLOAD_FAILED", "下载响应使用了不受支持的内容编码。");
    }
    const declaredLength = parseContentLength(currentResponse.headers);
    if (declaredLength !== undefined) {
      if (declaredLength > maximumBytes) {
        throw downloadError("ASSET_TOO_LARGE", "下载资产超过大小上限。");
      }
      if (kind === "portable" && declaredLength !== expectedSize) {
        throw downloadError("DOWNLOAD_INTEGRITY", "下载资产长度与发布清单不一致。");
      }
    }

    const digest = createHash("sha256");
    let bytes = 0;
    for await (const value of currentResponse) {
      const chunk = Buffer.isBuffer(value) ? value : Buffer.from(value);
      bytes += chunk.length;
      if (!Number.isSafeInteger(bytes) || bytes > maximumBytes) {
        currentResponse.destroy?.();
        throw downloadError("ASSET_TOO_LARGE", "下载资产超过大小上限。");
      }
      digest.update(chunk);
      await writeFully(file, chunk);
    }
    currentResponse = undefined;
    const actualSha256 = digest.digest("hex");
    if (
      bytes === 0 ||
      (declaredLength !== undefined && declaredLength !== bytes) ||
      (kind === "portable" && (bytes !== expectedSize || actualSha256 !== expectedSha256))
    ) {
      throw downloadError("DOWNLOAD_INTEGRITY", "下载资产完整性校验失败。");
    }
    await file.sync();
    await file.close();
    file = undefined;
    succeeded = true;
    return {
      bytes,
      sha256: actualSha256,
      source: "github-release",
    };
  } catch (error) {
    if (timedOut) {
      throw downloadError("DOWNLOAD_TIMEOUT", "下载超时，请检查网络后重试。");
    }
    if (isLauncherError(error)) throw error;
    throw downloadError("DOWNLOAD_FAILED", "下载过程中发生网络或文件错误。");
  } finally {
    clearTimeout(timeout);
    currentResponse?.destroy?.();
    if (file) {
      try {
        await file.close();
      } catch {
        // Cleanup below still verifies the path identity before removal.
      }
    }
    if (!succeeded) {
      await removeOwnedPartial(destination, ownedIdentity);
    }
  }
}

export function downloadChecksums({ identity, destination }) {
  return downloadReleaseAsset({ identity, kind: "checksums", destination });
}

export function downloadPortable({ identity, portable, destination }) {
  return downloadReleaseAsset({
    identity,
    kind: "portable",
    expectedSize: portable?.size,
    expectedSha256: portable?.sha256,
    destination,
  });
}

export function downloadReleaseAssetForTest(options) {
  if (!process.env.NODE_TEST_CONTEXT) {
    throw downloadError("DOWNLOAD_FAILED", "测试下载入口不可用于生产运行。");
  }
  return downloadReleaseAsset(options);
}
