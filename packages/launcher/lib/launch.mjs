import { spawn } from "node:child_process";
import { lstat } from "node:fs/promises";
import { isAbsolute, relative } from "node:path";

import { LauncherError, isLauncherError } from "./errors.mjs";

function launchError(detail, cause) {
  return new LauncherError(
    "LAUNCH_FAILED",
    `桌面程序启动失败：${detail}。`,
    cause === undefined ? undefined : { cause },
  );
}

function identity(info) {
  return {
    dev: info.dev,
    ino: info.ino,
    birthtimeNs: info.birthtimeNs,
    size: info.size,
    mtimeNs: info.mtimeNs,
    ctimeNs: info.ctimeNs,
  };
}

function sameIdentity(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.birthtimeNs === right.birthtimeNs &&
    left.size === right.size &&
    left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs
  );
}

async function inspectLaunchPaths(executable, cwd) {
  if (
    typeof executable !== "string" ||
    typeof cwd !== "string" ||
    !isAbsolute(executable) ||
    !isAbsolute(cwd) ||
    relative(cwd, executable) !== "ability-radar.exe"
  ) {
    throw launchError("可执行文件路径无效");
  }
  try {
    const [directoryInfo, executableInfo] = await Promise.all([
      lstat(cwd, { bigint: true }),
      lstat(executable, { bigint: true }),
    ]);
    if (
      !directoryInfo.isDirectory() ||
      directoryInfo.isSymbolicLink() ||
      !executableInfo.isFile() ||
      executableInfo.isSymbolicLink() ||
      executableInfo.nlink !== 1n ||
      executableInfo.size === 0n
    ) {
      throw launchError("可执行文件或工作目录不可信");
    }
    return {
      directory: identity(directoryInfo),
      executable: identity(executableInfo),
    };
  } catch (error) {
    if (isLauncherError(error)) throw error;
    throw launchError("无法验证可执行文件", error);
  }
}

async function launchCore({ executable, cwd } = {}, spawnProcess) {
  const before = await inspectLaunchPaths(executable, cwd);
  let child;
  try {
    child = spawnProcess(executable, [], {
      cwd,
      detached: true,
      shell: false,
      stdio: "ignore",
      windowsHide: false,
    });
  } catch (error) {
    throw launchError("无法创建进程", error);
  }
  if (!child || typeof child.once !== "function" || typeof child.unref !== "function") {
    throw launchError("进程接口无效");
  }

  await new Promise((resolve, reject) => {
    let settled = false;
    const fail = (error) => {
      if (settled) return;
      settled = true;
      reject(launchError("无法创建进程", error));
    };
    const spawned = async () => {
      if (settled) return;
      try {
        const after = await inspectLaunchPaths(executable, cwd);
        if (
          !sameIdentity(before.directory, after.directory) ||
          !sameIdentity(before.executable, after.executable)
        ) {
          throw launchError("启动前文件身份发生变化");
        }
        child.removeListener?.("error", fail);
        child.on?.("error", () => {});
        child.unref();
        settled = true;
        resolve();
      } catch (error) {
        child.kill?.();
        fail(error);
      }
    };
    child.once("error", fail);
    child.once("spawn", spawned);
  });
}

export function launchVerifiedExecutable(options) {
  return launchCore(options, spawn);
}

export function launchVerifiedExecutableForTest(options, { spawnProcess } = {}) {
  if (!process.env.NODE_TEST_CONTEXT || typeof spawnProcess !== "function") {
    throw launchError("测试启动入口不可用于生产运行");
  }
  return launchCore(options, spawnProcess);
}
