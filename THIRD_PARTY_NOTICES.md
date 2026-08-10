# 第三方声明与题包来源

AI 能力雷达第一方代码按仓库根目录的 [Apache License 2.0](LICENSE) 提供。

## 第一方基准题包

下列内置内容由本项目原创并按 Apache-2.0 提供，不是第三方依赖：

- `benchmark-packs/client-quick-v1`（题包 ID `client-quick` v1.0.0）：Apache-2.0。
- `benchmark-packs/cli-quick-v1`（题包 ID `cli-quick` v1.0.0）：Apache-2.0。

任何未来导入的题目、starter、verifier 或数据集都必须在这里单独列出来源、版本、修改与再分发许可，
不能借用上述第一方声明。DeepSWE 内容当前未包含；其再分发审查完成前不会进入仓库或安装包。

## 单独审查的运行时依赖

### Claude Agent SDK 0.3.226

`@anthropic-ai/claude-agent-sdk` 0.3.226 在 npm 元数据中声明的许可为
`SEE LICENSE IN README.md`。它不是按 Apache-2.0 或 MIT 许可发布的项目代码；使用和再分发时必须同时
遵守该包 README 中的 Anthropic 商业许可条款。上游包与许可说明：
<https://www.npmjs.com/package/@anthropic-ai/claude-agent-sdk/v/0.3.226>。

本项目仅把它作为 Claude Code 本地执行的可选运行时，不改变其许可，也不把它并入本项目的
Apache-2.0 授权范围。机器可读清单通过 `noticeId: claude-agent-sdk` 指向本节。

## Rust 与 npm 依赖

锁定依赖的机器可读许可报告：

- [Rust 依赖元数据](docs/licenses/rust-dependencies.json)，由 `Cargo.lock` 和 Cargo package metadata 生成。
- [npm 依赖元数据](docs/licenses/npm-dependencies.json)，由 `package-lock.json` 的锁定条目生成。

每份报告按 `name@version` 排序，并记录对应锁文件规范化文本的 SHA-256：UTF-8 文本中的 CRLF
或 CR 会先统一为 LF，避免 Windows/Linux checkout 的行尾差异导致假失败。
`npm run validate:repository` 会检查锁文件哈希与逐包覆盖。可用以下命令机械重建：

```powershell
npm ci
npm run licenses:generate
npm run validate:repository
```

这些文件是许可标识与来源的**元数据清单**，不是所有依赖完整许可文本的合集，也不声称安装包捆绑了
每个依赖的完整许可文本。需要审计具体条款时，Rust 依赖应使用报告中的精确 crate 名、版本及
source/repository，npm 依赖应使用精确包名、版本及 resolved/integrity，与锁文件和上游发布内容
核对；不同 license expression 可能允许多种选择。
