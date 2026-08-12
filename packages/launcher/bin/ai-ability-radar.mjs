#!/usr/bin/env node

import { readFile } from "node:fs/promises";

import { parseCliArguments, renderCliCommand } from "../lib/cli.mjs";
import { isLauncherError } from "../lib/errors.mjs";
import { parseReleaseManifest } from "../lib/manifest.mjs";
import { runLauncherCommand } from "../lib/run.mjs";

async function main() {
  const packageManifest = JSON.parse(
    await readFile(new URL("../package.json", import.meta.url), "utf8"),
  );
  const command = parseCliArguments(process.argv.slice(2));
  let result;
  if (command.kind === "help" || command.kind === "version") {
    result = renderCliCommand(command, packageManifest.version);
  } else {
    const manifest = command.kind === "launch"
      ? parseReleaseManifest(
        await readFile(new URL("../release-manifest.json", import.meta.url), "utf8"),
        { packageVersion: packageManifest.version },
      )
      : undefined;
    result = await runLauncherCommand({
      command,
      version: packageManifest.version,
      manifest,
    });
  }
  if (result.stdout) {
    process.stdout.write(result.stdout);
  }
  if (result.stderr) {
    process.stderr.write(result.stderr);
  }
  process.exitCode = result.exitCode;
}

main().catch((error) => {
  const message = isLauncherError(error)
    ? error.message
    : "启动器发生未知错误。";
  process.stderr.write(`${message}\n`);
  process.exitCode = isLauncherError(error) ? error.exitCode : 2;
});
