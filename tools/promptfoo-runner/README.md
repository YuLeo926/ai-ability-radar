# Promptfoo Agent Runner

此目录承载 AI 能力雷达的本地 Agent 执行桥。首个执行契约版本为
`promptfoo-agent-v1`，固定支持以下 Promptfoo provider：

- Codex CLI：`openai:codex-sdk`；
- Claude Code：`anthropic:claude-agent-sdk`。

运行依赖由仓库根目录的 `package.json` 和 `package-lock.json` 精确锁定。生产与测试脚本不使用
`npx`、`@latest` 或运行时联网安装。依赖缺失时应由免费预检报告，不能自动下载。

正常自动化测试只能使用假 provider，不得调用真实 Codex、Claude 或消耗用户订阅额度。真实 provider
验收必须经过应用内的额度说明和用户单独确认。

## 间接依赖安全锁定

根目录还精确覆盖了 `ai@6.0.237`、`adm-zip@0.6.0` 和 `sharp@0.35.3`。前者位于
Promptfoo 声明的兼容版本范围内；后两者只修复本项目不使用的 Hugging Face 可选执行路径。保留这些锁定是为了让
完整依赖树通过高危漏洞审计，不能在没有重新运行导入测试和 `npm audit --omit=dev` 的情况下升级或删除。
