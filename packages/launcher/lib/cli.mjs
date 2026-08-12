const HELP_TEXT = `AI 能力雷达 npm 启动器

用法：
  npx ai-ability-radar              验证并启动严格对应版本
  npx ai-ability-radar --help       显示帮助
  npx ai-ability-radar --version    显示启动器版本
  npx ai-ability-radar --clear-cache  清理启动器缓存

支持：Windows 10/11 x64
首次启动需要联网；验证后的缓存可供后续离线启动。
`;

const COMMANDS = new Map([
  ["--help", "help"],
  ["--version", "version"],
  ["--clear-cache", "clear-cache"],
]);

export function parseCliArguments(args) {
  if (!Array.isArray(args) || args.some((value) => typeof value !== "string")) {
    throw new TypeError("命令行参数格式无效。");
  }
  if (args.length === 0) {
    return { kind: "launch" };
  }
  if (args.length === 1 && COMMANDS.has(args[0])) {
    return { kind: COMMANDS.get(args[0]) };
  }
  throw new Error("不支持的参数。请运行 npx ai-ability-radar --help 查看用法。");
}

export function renderCliCommand(command, version) {
  if (command?.kind === "help") {
    return { exitCode: 0, stdout: HELP_TEXT, stderr: "" };
  }
  if (command?.kind === "version") {
    return { exitCode: 0, stdout: `${version}\n`, stderr: "" };
  }
  if (command?.kind === "launch" || command?.kind === "clear-cache") {
    return {
      exitCode: 1,
      stdout: "",
      stderr: "启动功能尚未接线；请稍后再试。\n",
    };
  }
  throw new TypeError("启动器命令无效。");
}
