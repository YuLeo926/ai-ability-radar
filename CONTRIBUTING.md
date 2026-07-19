# 贡献指南

感谢你帮助改进 AI 能力雷达。v0.2 的首要约束是：结果可复核、隐私字段最小化、自动化完全不消耗
贡献者或维护者的 AI 订阅。

## 开始之前

在 Windows 10/11 x64 上安装 Node.js 22/24 LTS、npm、仓库指定的 Rust 工具链，以及 Tauri 所需的
Microsoft C++ Build Tools 和 WebView2。然后运行：

```powershell
npm ci
npm run validate:repository
npm test
cargo test --workspace --all-targets --locked
```

所有自动 adapter 测试都必须使用 fake process runner。不要在单元测试、集成测试或 GitHub CI 中
安装、登录或运行真实订阅 CLI。真实 CLI 手工检查必须由明确自愿的测试者在自己的电脑与账号上进行。

## 提交改动

1. 先添加能失败的测试或仓库约束，再实现最小修复。
2. 新增跨 Tauri 边界的字段时，审查 Rust DTO、TypeScript runtime validator、公开报告允许列表、
   完整备份提示和文档；默认不公开新字段。
3. 修改 capabilities 或 command 注册时，附上 capability diff 并说明最小权限理由。
4. 修改题包时，更新 registry 哈希，确认内容来源和再分发许可。第一方题包与外部导入内容必须分开记录。
5. 修改进程启动、超时、取消或恢复时，在 Windows 上手工检查子进程终止和中断恢复。
6. 修改 `Cargo.lock` 或 `package-lock.json` 后运行 `npm run licenses:generate`。

## 提交前验证

```powershell
npm run validate:repository
npm test
npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm run tauri -- build --debug --bundles nsis
git diff --check
```

不要把原始回答、CLI 日志、用户名、设备名、绝对路径、访问令牌、登录信息或订阅账单放入 issue、
测试夹具、快照或提交历史。报告错误时只提供脱敏后的错误类别和可公开环境元数据。

提交 Pull Request 时请完整填写仓库模板。安全漏洞不要公开在 issue；请遵循[安全政策](SECURITY.md)。
