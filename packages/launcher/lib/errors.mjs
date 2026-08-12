export class LauncherError extends Error {
  constructor(code, message, { exitCode = 1, cause } = {}) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "LauncherError";
    this.code = code;
    this.exitCode = exitCode;
  }
}

export function isLauncherError(error) {
  return error instanceof LauncherError;
}
