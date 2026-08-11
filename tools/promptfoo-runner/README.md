# Promptfoo Agent Runner

此目录承载 AI 能力雷达的本地 Agent 执行桥。当前执行契约版本为
`promptfoo-agent-v2`，固定支持以下 Promptfoo provider：

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

## 进程协议

桌面端只通过标准输入发送一条不超过 256 KiB 的 JSON 请求，命令参数不携带 prompt、登录数据或工作区文件内容。
请求只允许 `provider`、`workspace`、`prompt`、`requested_model`、`reasoning_effort`、
`time_budget_seconds`、`max_turns` 和 `run_id`。未知字段、相对或不存在的工作区、超限文本和不支持的档位会在
provider 启动前失败。

标准输出只写一行 `promptfoo-agent-v2` 结果 JSON，包含状态、脱敏最终文本、session 是否存在、Token、
工具计数、命令状态、退出码分布、工具错误摘要、文件修改事件数、模型证据和稳定的 provider 错误码。协议不输出命令、
命令输出、补丁、文件路径或完整 session ID。诊断只写入标准错误，且不回显异常正文。Promptfoo 缓存默认关闭，缓存命中
不能作为有效能力结果。
