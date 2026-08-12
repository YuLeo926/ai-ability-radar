import assert from "node:assert/strict";
import test from "node:test";

import {
  parseCliArguments,
  renderCliCommand,
} from "../lib/cli.mjs";

test("parses only the four reviewed commands", () => {
  assert.deepEqual(parseCliArguments([]), { kind: "launch" });
  assert.deepEqual(parseCliArguments(["--help"]), { kind: "help" });
  assert.deepEqual(parseCliArguments(["--version"]), { kind: "version" });
  assert.deepEqual(parseCliArguments(["--clear-cache"]), {
    kind: "clear-cache",
  });
});

test("rejects unknown, combined, and positional arguments", () => {
  for (const args of [
    ["--latest"],
    ["--help", "--version"],
    ["start"],
    [""],
  ]) {
    assert.throws(
      () => parseCliArguments(args),
      /不支持的参数.*--help/u,
      JSON.stringify(args),
    );
  }
});

test("renders stable help and version output", () => {
  const help = renderCliCommand({ kind: "help" }, "0.2.2");
  assert.equal(help.exitCode, 0);
  assert.equal(help.stderr, "");
  assert.match(help.stdout, /^AI 能力雷达 npm 启动器/u);
  assert.match(help.stdout, /npx ai-ability-radar/u);
  assert.match(help.stdout, /--clear-cache/u);
  assert.match(help.stdout, /Windows 10\/11 x64/u);
  assert.equal(help.stdout.endsWith("\n"), true);

  assert.deepEqual(renderCliCommand({ kind: "version" }, "0.2.2"), {
    exitCode: 0,
    stdout: "0.2.2\n",
    stderr: "",
  });
});

test("leaves operational commands to the asynchronous runner", () => {
  for (const command of [{ kind: "launch" }, { kind: "clear-cache" }]) {
    assert.throws(
      () => renderCliCommand(command, "0.2.2"),
      /异步执行/u,
    );
  }
});
