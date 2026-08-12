# v0.2 Windows Preview Release Checklist

Unchecked items are release gates, not implementation claims. Clean Windows 10
and Windows 11 VM evidence is still Pending.

## Scope and truthfulness

- [ ] Exactly 8 client tasks and 2 CLI tasks are bundled.
- [ ] ChatGPT, Claude, Codex CLI, and Claude Code never share a score.
- [ ] No screen says IQ or claims certainty from insufficient evidence.
- [ ] Time and subscription-cost estimates appear before starting.
- [ ] Infrastructure invalidity and agent-budget failure remain distinct.

## Reproducibility

- [ ] All pack hashes match the release resources.
- [ ] Same fixture history produces byte-equivalent analysis JSON.
- [ ] Clean and resumed runs never share one history series.
- [ ] v0.2 never emits a degradation verdict from historical scores.

## Privacy and security

- [ ] No API key, login token, CLI auth file, or environment dump is collected.
- [ ] Tauri capability file has no shell, HTTP, filesystem, or SQL permission.
- [ ] Public report contains no raw answer, log, username, hostname, or path.
- [ ] Full backup is explicitly labeled unencrypted/private.
- [ ] Cancellation kills the Windows child process tree.
- [ ] Only signed bundled verifier IDs can execute.

## Quality

- [ ] cargo fmt, clippy, tests, npm tests, build, and axe pass.
- [ ] Windows 10 and 11 acceptance matrix is complete.
- [ ] Claude Desktop 客户端可见选择器识别／诚实回退已在受支持的 Windows 真机上人工验证；synthetic tests 不能满足此 gate。
- [ ] NSIS and MSI install, launch, and uninstall.
- [ ] 免安装 ZIP 在干净的 Windows 10 和 Windows 11 x64 VM 中解压并启动。
- [ ] 免安装 ZIP 不创建安装项，数据仍写入 `%APPDATA%\com.aiability.radar`。
- [ ] 免安装 ZIP 内部 `SHA256SUMS.txt` 与解压后的文件逐一匹配。
- [ ] 100–200% scaling and keyboard-only operation pass.
- [ ] Offline client-only use works without Node.js.

## GitHub release

- [ ] Version matches tag and documentation.
- [ ] THIRD_PARTY_NOTICES is current.
- [ ] 外层 `SHA256SUMS.txt` 覆盖 NSIS、MSI 和带精确版本号的 portable ZIP。
- [ ] Tauri action 是唯一的安装程序上传者；精确的 `gh release upload` 只上传 portable ZIP 和校验文件。
- [ ] 免安装归档只在 Tauri action 创建草稿预发布之后构建。
- [ ] 发布说明明确安装程序和免安装 ZIP 均未签名，并要求校验所有下载文件。
- [ ] Updater remains disabled.
- [ ] Pages links point to the correct repository and release.

## npm launcher publication

- [ ] Windows Node.js 22.22.0 与 24 launcher CI 矩阵全部通过，且没有 secrets、真实 AI CLI、真实
  Release 下载或发布步骤。
- [ ] `npm run test:launcher`、`npm run test:launcher:package`、仓库验证和 `npm audit --audit-level=high`
  全部通过。
- [ ] `npm pack --workspace packages/launcher --json` 的 tarball 内容审计只包含批准的 18 个文件，
  不含测试、证书、桌面二进制、便携 ZIP、日志、缓存或用户数据。
- [ ] 在全新目录使用 `--ignore-scripts` 安装最终 `.tgz`，复验帮助、版本、未知参数、首次下载、离线命中、
  损坏修复和 `--clear-cache`。
- [ ] clean Windows 10/11 验收完成后才公开 GitHub Release；公开后从全新目录重新下载 ZIP 与
  `SHA256SUMS.txt`，重新生成清单并确认与候选清单逐字节相同，完成公开 Release 清单二次复核。
- [ ] 使用 `npm view ai-ability-radar` 核对包名所有权与当前状态；发布者本人完成 `npm login`，并用
  `npm whoami` 确认身份。令牌不得发送给维护工具或写入仓库。
- [ ] 执行不可逆的 `npm publish --workspace packages/launcher --access public` 前，再次人工确认包名
  `ai-ability-radar`、版本 `0.2.2` 和公开可见性。
- [ ] 发布后在不含仓库源码的全新目录完成真实 `npx` 验收，记录 npm integrity、`.tgz` SHA-256、
  GitHub Release URL 与结果。真实 `npx` 只启动桌面程序，不自动开始能力测试。
