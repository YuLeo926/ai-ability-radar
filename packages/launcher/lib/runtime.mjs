import { LauncherError } from "./errors.mjs";

const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;

function unsupportedNode() {
  return new LauncherError(
    "UNSUPPORTED_NODE",
    "需要 Node.js 22.22+ 或 Node.js 24 LTS。",
  );
}

export function parseNodeVersion(value) {
  if (typeof value !== "string") {
    throw unsupportedNode();
  }
  const match = value.match(STABLE_VERSION);
  if (!match) {
    throw unsupportedNode();
  }
  const components = match.slice(1).map(Number);
  if (components.some((component) => !Number.isSafeInteger(component))) {
    throw unsupportedNode();
  }
  return {
    major: components[0],
    minor: components[1],
    patch: components[2],
    raw: value,
  };
}

export function assertSupportedRuntime({ platform, arch, nodeVersion } = {}) {
  if (platform !== "win32" || arch !== "x64") {
    throw new LauncherError(
      "UNSUPPORTED_PLATFORM",
      "当前版本只支持 Windows 10/11 x64。",
    );
  }
  const parsed = parseNodeVersion(nodeVersion);
  const supportedNode22 =
    parsed.major === 22 &&
    (parsed.minor > 22 || (parsed.minor === 22 && parsed.patch >= 0));
  const supportedNode24 = parsed.major === 24;
  if (!supportedNode22 && !supportedNode24) {
    throw unsupportedNode();
  }
  return { platform, arch, nodeVersion };
}
