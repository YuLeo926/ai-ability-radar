# AI 能力雷达

> 当前状态：v0.2.1 Windows 预览版。支持 Windows 10/11 x64；安装程序尚未签名，也没有自动更新。

本项目把“感觉表现变差了”变成条件明确、可以复核的本地测试记录。它对固定题包做客观评分，
不测量 IQ，不推断服务端发生了什么，v0.2 也不生成“降智”或退化裁决。

界面仍处于预览期，因此暂不放截图或 GIF；等 UI 稳定后再添加，避免文档与实际界面漂移。

## 支持的目标与题数

| 轨道 | 目标 | v0.2 快速题包 |
| --- | --- | --- |
| 客户端辅助 | ChatGPT、Claude | 8 道：3 道指令遵循、3 道逻辑、2 道代码审查 |
| 自动 CLI | Codex CLI、Claude Code | 2 个原创 JavaScript 微型项目 |

客户端轨道由用户在新对话中复制题目并粘贴原始回答。CLI 轨道在独立临时目录中运行所选工具，
再由本地允许列表验证器评分；只有 CLI 轨道需要 Node.js 22/24 LTS。

## 费用边界

**谁运行真实 CLI，谁承担自己的订阅用量。** 应用不提供共享账号、凭据或代付。

- 普通 GitHub CI 只运行假 CLI 和本地测试，不调用真实 AI 服务，也不消耗 AI 订阅。
- GitHub-hosted runner 的计费遵循仓库所有者的 GitHub 计划，与 AI 订阅费用分开。
- 任何可选的真实 CLI 发布前检查只能由自愿测试者在自己的电脑上手动进行，并消耗该测试者自己的订阅。
- ChatGPT / Claude 客户端题目同样由测试者在自己的会话中运行，受其账号方案约束。

## 安装与校验

1. 从仓库的 **Releases** 页面下载 v0.2.1 Windows x64 预览安装程序或
   `ability-radar_0.2.1_windows-x64-portable.zip`，并下载 `SHA256SUMS.txt`。
2. 在 PowerShell 中计算下载文件的 SHA-256：

   ```powershell
   Get-FileHash -Algorithm SHA256 -LiteralPath .\ability-radar_0.2.1_x64-setup.exe
   ```

3. 将输出与 `SHA256SUMS.txt` 中同名文件的值逐字符比较。下载多个文件时逐一校验。

预览安装程序和免安装 ZIP 均未签名，Windows SmartScreen 可能提示“未知发布者”。校验值证明下载
内容与草稿发布中上传的构建产物一致，但不替代商业代码签名。免安装 ZIP 不创建安装/卸载项，
历史和设置仍写入 `%APPDATA%\com.aiability.radar`。项目当前没有 Tauri updater 插件或更新清单。

## 本地开发

前置条件：

- Windows 10/11 x64；
- Node.js 22 或 24 LTS 与 npm；
- `rust-toolchain.toml` 指定的 Rust 工具链；
- Tauri Windows 构建所需的 Microsoft C++ Build Tools 与 WebView2。

安装依赖并打开完整的 Tauri 桌面开发窗口：

```powershell
npm ci
npm start
```

`npm start` 会打开 Tauri 桌面开发窗口。单独在普通浏览器中打开 `http://localhost:1420`
不是完整产品：浏览器页面没有经过审查的 Tauri IPC 和本地桌面能力。

从当前源码构建 release 可执行文件并生成 Windows x64 免安装 ZIP：

```powershell
npm run package:portable
```

其他常用检查命令：

```powershell
npm run validate:repository
npm test
npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm run tauri -- build --debug --bundles nsis
```

更新锁文件后，运行 `npm run licenses:generate` 重建确定性的依赖许可元数据，再运行仓库验证器。
自动测试必须使用已有的 fake process runner；不要在测试或 CI 中启动真实 Codex CLI 或 Claude Code。

## 架构边界

- `apps/desktop` 是 React/Tauri 桌面壳；WebView 只能调用经过审查的 Tauri command 允许列表。
- `crates/ability-core` 负责题包校验、客观评分、历史、SQLite 持久化、导出与本地数据生命周期。
- `crates/ability-adapters` 只负责受限进程调用、CLI 协议解析、取消与本地验证器协调。
- `benchmark-packs` 包含两个固定的第一方快速题包；它们由内容哈希和 registry 绑定。
- 应用没有遥测或上传端点。真实 CLI 的提示词和临时题目代码仍会发给所选提供商。
- v0.2 的工作目录与工具允许列表降低误操作范围，但不是容器、VM 或恶意代码沙箱。

更完整的行为与安全边界见[方法说明](docs/methodology.md)、[隐私说明](docs/privacy.md)、
[安全说明](docs/security.md)和[故障排查](docs/troubleshooting.md)。

## 设计依据

实现遵循已批准的[产品设计](docs/superpowers/specs/2026-07-17-ai-ability-radar-design.md)和
[实施计划](docs/superpowers/plans/2026-07-17-ai-ability-radar-desktop-mvp.md)。

## 贡献与许可

请先阅读[贡献指南](CONTRIBUTING.md)和[安全政策](SECURITY.md)。项目第一方代码与内置快速题包
按 [Apache License 2.0](LICENSE) 提供；依赖许可元数据见[第三方声明](THIRD_PARTY_NOTICES.md)。
