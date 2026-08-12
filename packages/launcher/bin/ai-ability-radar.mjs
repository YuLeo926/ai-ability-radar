#!/usr/bin/env node

import { readFile } from "node:fs/promises";

import { parseCliArguments, renderCliCommand } from "../lib/cli.mjs";

async function main() {
  const packageManifest = JSON.parse(
    await readFile(new URL("../package.json", import.meta.url), "utf8"),
  );
  const command = parseCliArguments(process.argv.slice(2));
  const result = renderCliCommand(command, packageManifest.version);
  if (result.stdout) {
    process.stdout.write(result.stdout);
  }
  if (result.stderr) {
    process.stderr.write(result.stderr);
  }
  process.exitCode = result.exitCode;
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : "启动器发生未知错误。";
  process.stderr.write(`${message}\n`);
  process.exitCode = 2;
});
