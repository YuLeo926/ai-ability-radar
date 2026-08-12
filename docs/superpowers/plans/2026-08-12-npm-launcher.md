# npm 轻量启动器实施计划

> **执行要求：**逐任务实施，每个任务遵循 RED → GREEN → 回归验证 → 独立提交。自动测试只使用本地假资产和假进程，不调用 Codex、Claude 或其他模型。GitHub Release 和 npm 的公开发布必须等到本计划的人工发布门槛满足后再执行。

**目标：**新增公开 npm 包 `ai-ability-radar`，使 Windows x64 用户可以通过 `npx ai-ability-radar@0.2.2` 下载、验证、缓存并启动严格对应的 GitHub Release 便携版。

**设计依据：**`docs/superpowers/specs/2026-08-11-npm-launcher-design.md`

**技术栈：**Node.js 22.22+/24 LTS 内置模块、Windows PowerShell、Node test runner、GitHub Actions、npm workspaces。

## 已核实的执行前提

计划编写时（2026-08-12）已核实：

- 根项目版本和桌面版本均为 `0.2.2`；
- Git 标签 `v0.2.2` 已存在，Windows Release 工作流已经生成草稿预发布；
- 草稿中已有 EXE、MSI、portable ZIP 和 `SHA256SUMS.txt`，但尚未满足干净 Windows 10/11 x64 人工验收门槛；
- 主 CI 使用 Windows 和 Node.js 22，现有仓库验证器会封印工作流顺序、发布资产和便携打包脚本；
- 根项目尚无 `packages/` 目录；
- 便携 ZIP 已有内部 `SHA256SUMS.txt`、严格根目录、原始 ZIP 结构检查和解压后复验；
- npm 当前未登录，公开发布前仍需用户本人完成 npm 登录；
- 已存在的 `v0.2.2` 标签不得移动，草稿 Release 的同名资产不得静默替换。

## 全局限制

- 启动器只接受 Windows x64；`--help` 和 `--version` 可在其他平台显示，但默认启动必须返回明确的不支持提示。
- Node.js 只接受 `>=22.22.0 <23` 或 `>=24.0.0 <25`；npm `engines` 与运行时预检保持一致。
- 生产入口不接受下载地址、镜像、缓存目录、可执行文件或 PowerShell 脚本的环境变量覆盖。
- 只从固定的 `YuLeo926/ai-ability-radar`、固定标签和固定资产名下载。
- ZIP 的期望 SHA-256、字节数和解压文件清单必须内置于 npm 包；远程 `SHA256SUMS.txt` 只作为交叉核对。
- 不使用 shell，不执行安装脚本，不请求管理员权限，不读模型登录凭据，不上传遥测。
- 不把绝对用户目录、GitHub 重定向签名参数、环境变量或凭据写入错误输出。
- 自动测试不得访问 GitHub Release、npm registry 或真实模型；联网发布命令只出现在最终人工发布任务。
- 所有缓存和临时目录删除都必须验证固定祖先、所有权标记、路径类型和调用方 token。
- 每个任务只暂存列出的文件，禁止 `git add -A`、`git add .` 和重写既有标签。

## 里程碑

1. **M1 包壳：**公开 npm 包结构、命令帮助和版本契约可打包；
2. **M2 安全核心：**下载、清单、缓存、锁和解压复验全部由离线测试覆盖；
3. **M3 启动闭环：**假便携资产首次下载、离线命中、损坏修复和直接启动通过；
4. **M4 发布候选：**真实 `v0.2.2` portable ZIP 生成候选内置清单，`.tgz` 内容审计和双 Node 版本 CI 通过；
5. **M5 正式上线：**Windows 10/11 验收、GitHub Release 公开、清单复核、npm 发布和真实 `npx` 验收完成。

---

## 任务 1：建立独立公开 npm 工作区和只读命令

**文件：**

- 修改 `package.json`
- 修改 `package-lock.json`
- 新建 `packages/launcher/package.json`
- 新建 `packages/launcher/bin/ai-ability-radar.mjs`
- 新建 `packages/launcher/lib/cli.mjs`
- 新建 `packages/launcher/README.md`
- 新建 `packages/launcher/LICENSE`
- 新建 `packages/launcher/tests/cli.test.mjs`
- 修改 `scripts/repository-contracts.test.mjs`

**RED：**

- 根 workspaces 必须包含 `apps/desktop` 和 `packages/launcher`；
- 私有根包改名为 `ai-ability-radar-monorepo`，公开名称 `ai-ability-radar` 只由 launcher workspace 持有，避免根包与工作区同名；
- 启动器包名和版本严格为 `ai-ability-radar@0.2.2`，`private` 不得为 `true`；
- `bin` 只映射 `ai-ability-radar`，发布文件使用精确 `files` 白名单；
- 包中没有 dependencies、optionalDependencies、生命周期脚本或任意 install/postinstall；
- `engines.node` 严格为 `>=22.22.0 <23 || >=24.0.0 <25`；
- 不设置 npm `os`/`cpu` 拦截，以便非 Windows 用户仍能读取帮助和明确错误；
- `--help`、`--version`、无参数和未知参数的解析结果稳定；
- `LICENSE` 必须与仓库根许可证逐字节一致。

**GREEN：**

- CLI 解析实现为无副作用纯函数；
- 首个版本只实现 `--help` 和 `--version` 的输出，无参数暂返回“启动功能尚未接线”的受控内部错误；
- `bin` 使用 ESM、固定 shebang 和顶层错误归一，不输出堆栈或绝对路径；
- npm README 说明 Windows x64、首次联网、版本一一对应、缓存目录、隐私和未签名程序提示。

**验证：**

```powershell
npm install --package-lock-only --ignore-scripts
node --test packages/launcher/tests/cli.test.mjs
node --test scripts/repository-contracts.test.mjs
node packages/launcher/bin/ai-ability-radar.mjs --help
node packages/launcher/bin/ai-ability-radar.mjs --version
git diff --check
```

**提交：**`feat: scaffold npm launcher package`

---

## 任务 2：定义内置 Release 清单和严格版本契约

**文件：**

- 新建 `packages/launcher/lib/errors.mjs`
- 新建 `packages/launcher/lib/runtime.mjs`
- 新建 `packages/launcher/lib/manifest.mjs`
- 新建 `packages/launcher/tests/runtime.test.mjs`
- 新建 `packages/launcher/tests/manifest.test.mjs`
- 新建 `packages/launcher/tests/fixtures/release-manifest.valid.json`

**RED：**

- 严格解析包版本，只接受无前缀、无预发布、无 build metadata 的 `MAJOR.MINOR.PATCH`；
- 版本 `0.2.2` 必须唯一生成标签 `v0.2.2` 和资产名 `ability-radar_0.2.2_windows-x64-portable.zip`；
- 清单拒绝未知字段、重复路径、绝对路径、反斜杠、`.`/`..`、空段、控制字符和 Windows 保留名；
- ZIP SHA-256 必须是 64 位小写十六进制，字节数为有界安全整数；
- 解压清单必须恰好包含一个固定根目录、内部 `SHA256SUMS.txt` 和可执行文件；
- 包版本、清单版本、标签、资产名、桌面版本和下载路径任一不一致都失败；
- Node 22.21、23、25 和非 Windows x64 默认启动均被拒绝，Node 22.22+/24 x64 通过。

**GREEN：**

- 建立版本化 `launcher-release-manifest-v1` schema；
- 把用户可见失败归一为稳定码和简短中文消息；
- 仅让生产组合层读取 `process.platform`、`process.arch` 和 `process.versions.node`，核心验证函数接受显式值便于离线测试。

**验证：**

```powershell
node --test packages/launcher/tests/runtime.test.mjs packages/launcher/tests/manifest.test.mjs
git diff --check
```

**提交：**`feat: define launcher release contract`

---

## 任务 3：实现固定来源、受限大小的 HTTPS 下载器

**文件：**

- 新建 `packages/launcher/lib/download.mjs`
- 新建 `packages/launcher/tests/download.test.mjs`

**RED：**

- 初始请求只允许 `https://github.com/YuLeo926/ai-ability-radar/releases/download/v<version>/<asset>`；
- 手动跟随最多 5 次重定向，只接受 HTTPS，目标主机只允许 `github.com`、`objects.githubusercontent.com` 和 `release-assets.githubusercontent.com`；
- 拒绝用户名/密码、非默认端口、URL fragment、异常编码文件名和降级到 HTTP；
- 请求固定 `User-Agent`、`Accept-Encoding: identity`，拒绝压缩响应和非 200 最终状态；
- `SHA256SUMS.txt` 上限 64 KiB；ZIP 同时受清单精确字节数和 256 MiB 硬上限约束；
- 连接空闲超时、总超时、重定向循环、中断、短包、长包和流错误都关闭句柄并删除本次临时文件；
- 流式计算 SHA-256，不把完整 ZIP 读入内存；
- 日志和错误不能包含带查询参数的重定向 URL。

**GREEN：**

- 使用 `node:https` 直接请求，不使用 shell、curl、PowerShell 下载或自动跟随重定向的 `fetch`；
- 下载到调用方提供的已拥有临时文件，使用 `wx` 防止覆盖；
- 返回固定的字节数、SHA-256 和安全来源分类；
- 测试通过注入受控 HTTPS transport 运行，覆盖请求、重定向和流边界；仓库不保存测试私钥，生产 API 不暴露 URL 覆盖。

**验证：**

```powershell
node --test packages/launcher/tests/download.test.mjs
git diff --check
```

**提交：**`feat: add constrained release downloader`

---

## 任务 4：实现缓存所有权、版本锁和崩溃恢复

**文件：**

- 新建 `packages/launcher/lib/paths.mjs`
- 新建 `packages/launcher/lib/cache.mjs`
- 新建 `packages/launcher/lib/lock.mjs`
- 新建 `packages/launcher/tests/paths.test.mjs`
- 新建 `packages/launcher/tests/cache.test.mjs`
- 新建 `packages/launcher/tests/lock.test.mjs`

**RED：**

- 缓存根唯一为 `%LOCALAPPDATA%/AI Ability Radar/launcher`，缺失或相对 `LOCALAPPDATA` 被拒绝；
- 根目录创建后写入版本化所有权标记，既有目录缺失标记或标记不符时不接管、不删除；
- 检查根目录、版本目录、锁、临时目录和所有父路径的符号链接/重解析点；
- 每个版本目录隔离，禁止 `v0.2.2` 操作其他版本；
- 版本锁通过原子目录创建取得，包含随机 token、版本和有界时间戳；
- 等待者在锁释放后重新验证缓存；锁超时回收使用原子改名，旧持有者发布前必须再次证明 token 所有权；
- 每个调用只清理带自身 token 的临时目录；
- 发布缓存使用同根目录的原子重命名；中断发生在旧缓存隔离或新缓存发布任一步时，下一次运行均能恢复到一个可验证候选；
- `--clear-cache` 仅在绝对路径、固定祖先、所有权标记和非重解析点全部成立时删除根目录。

**GREEN：**

- 状态文件只作为快速提示，任何命中最终仍回到内置清单和文件哈希；
- 随机临时目录、隔离旧目录和锁目录都使用调用 token；
- 崩溃恢复枚举范围只限当前版本和已验证命名格式，不扫描用户其他目录；
- 所有等待和锁寿命使用固定上限，失败时给出“另一个启动正在进行”或“缓存需要人工清理”的稳定错误。

**验证：**

```powershell
node --test packages/launcher/tests/paths.test.mjs packages/launcher/tests/cache.test.mjs packages/launcher/tests/lock.test.mjs
git diff --check
```

**提交：**`feat: secure launcher cache lifecycle`

---

## 任务 5：实现 ZIP 结构检查、固定解压和文件树复验

**文件：**

- 新建 `packages/launcher/lib/archive.mjs`
- 新建 `packages/launcher/lib/tree.mjs`
- 新建 `packages/launcher/extract.ps1`
- 新建 `packages/launcher/tests/archive.test.mjs`
- 新建 `packages/launcher/tests/tree.test.mjs`
- 新建 `packages/launcher/tests/extract.test.mjs`
- 新建 `packages/launcher/tests/helpers/zip-fixture.mjs`

**RED：**

- 在调用 PowerShell 前解析 ZIP 中央目录并拒绝：ZIP64、多磁盘、加密、数据描述符歧义、重复项、大小不符和目录/文件类型冲突；
- 拒绝绝对路径、盘符、UNC、反斜杠、`.`/`..`、备用数据流、尾随点/空格、保留设备名和大小写折叠冲突；
- 拒绝 Unix symlink 位、未知外部属性和超出清单的任何 entry；
- 固定 `extract.ps1` 只接受源 ZIP 和已创建的空目标目录，不创建目标、不下载、不启动程序；
- 直接 `spawn powershell.exe`，参数固定包含 `-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File`；
- 解压后逐项 `lstat`，拒绝 reparse point、链接、额外/缺失路径、大小或 SHA-256 不符；
- 内部 `SHA256SUMS.txt` 必须与解压文件一致，并与 npm 清单的文件集合交叉核对；
- 任意失败不产生可启动的最终缓存。

**GREEN：**

- ZIP 验证器使用 Node 内置 Buffer 和文件读取接口，不增加运行时依赖；
- 结构检查返回规范化 entry 集合；运行时要求它与内置清单完全一致，清单生成器则只使用安全枚举模式形成待审查的候选集合；
- PowerShell 脚本保持最小能力面，只做重复/越界复核和 `Expand-Archive`；
- 解压后的信任根始终是 npm 内置清单，ZIP 内部校验文件仅作第二份一致性证据。

**验证：**

```powershell
node --test packages/launcher/tests/archive.test.mjs packages/launcher/tests/tree.test.mjs packages/launcher/tests/extract.test.mjs
git diff --check
```

**提交：**`feat: verify and extract portable archive`

---

## 任务 6：接通首次下载、离线修复和直接启动

**文件：**

- 新建 `packages/launcher/lib/launch.mjs`
- 新建 `packages/launcher/lib/run.mjs`
- 修改 `packages/launcher/lib/cli.mjs`
- 修改 `packages/launcher/bin/ai-ability-radar.mjs`
- 新建 `packages/launcher/tests/launch.test.mjs`
- 新建 `packages/launcher/tests/run.test.mjs`

**RED：**

- 有效缓存直接启动，且不创建网络请求；
- 无缓存时依次下载远程 `SHA256SUMS.txt` 和 ZIP，校验表中的 ZIP 哈希必须与内置哈希完全相同；
- ZIP 哈希、字节数、结构、解压树和内部校验全部通过后才发布缓存；
- 网络失败但缓存有效时离线启动；无有效缓存时明确提示首次运行需要联网；
- 解压目录损坏但固定 ZIP 有效时从本地 ZIP 修复，不访问网络；ZIP 也损坏时重新下载；
- 两个并发调用最多只有一个下载/发布，等待者复验后使用同一缓存；
- 使用 `spawn(executable, [], { shell: false })`，固定 cwd，成功收到 `spawn` 事件后 `unref`；
- 启动失败、进程创建错误或可执行文件被替换时返回非零且不输出绝对路径；
- `--clear-cache` 不启动程序，未知参数不转发。

**GREEN：**

- 组合层通过显式依赖对象连接下载、文件系统、时钟和进程创建，单元测试可完全离线；
- 生产入口只构造固定 GitHub URL、固定缓存路径和内置清单；
- 命令输出保持简短，首次下载显示版本和阶段，不显示签名 URL 或用户路径。

**验证：**

```powershell
node --test packages/launcher/tests/launch.test.mjs packages/launcher/tests/run.test.mjs
node --test packages/launcher/tests/*.test.mjs
git diff --check
```

**提交：**`feat: launch verified portable release`

---

## 任务 7：生成可复核的 Release 清单

**文件：**

- 新建 `scripts/generate-launcher-manifest.mjs`
- 新建 `scripts/generate-launcher-manifest.test.mjs`
- 修改 `package.json`
- 修改 `packages/launcher/package.json`
- 生成 `packages/launcher/release-manifest.json`
- 修改 `scripts/repository-contracts.test.mjs`

**RED：**

- 生成器只接受显式的本地 Release 资产目录和显式输出文件；
- 资产目录必须恰好包含目标 portable ZIP 和 `SHA256SUMS.txt`，不得通过网络补齐；
- 从启动器包版本推导严格标签/文件名，拒绝 CLI 传入版本覆盖；
- 远程校验表格式严格，目标 ZIP 恰好一条，哈希与本地 ZIP 相同；
- 生成前执行任务 5 的 ZIP 结构验证、固定解压和完整文件树扫描；
- 输出 JSON 使用稳定排序、LF、末尾换行和原子替换；
- 同一资产重复生成必须字节相同；任何源资产变化都造成可见 diff；
- 仓库测试要求生产清单不存在占位符、测试 URL、测试哈希或额外文件。

**GREEN：**

- 增加根命令 `npm run launcher:manifest -- --assets-dir <绝对目录>`；
- 首次实施可从当前 `v0.2.2` 草稿 Release 下载到本机临时目录生成“候选清单”，但不得因此公开 Release 或 npm；
- 正式 npm 发布前必须从公开后的同一 Release 重新下载并生成，要求结果与候选清单字节一致，否则停止发布。

**验证：**

```powershell
node --test scripts/generate-launcher-manifest.test.mjs
npm run launcher:manifest -- --assets-dir <本地已验证的-v0.2.2-资产目录>
node --test packages/launcher/tests/manifest.test.mjs scripts/repository-contracts.test.mjs
git diff --check
```

**提交：**`build: seal v0.2.2 launcher release`

---

## 任务 8：审计真实 npm tarball，而不是工作区源码

**文件：**

- 新建 `scripts/test-launcher-package.mjs`
- 新建 `scripts/test-launcher-package.test.mjs`
- 修改 `package.json`
- 修改 `packages/launcher/package.json`
- 修改 `scripts/repository-contracts.test.mjs`

**RED：**

- `npm pack --workspace packages/launcher --json` 的文件集合必须与白名单完全一致；
- `.tgz` 不含测试、证书、源码仓库文件、桌面二进制、portable ZIP、日志、缓存或用户文件；
- tarball 中清单、包版本、许可证和 README 与工作区预期一致；
- 在随机空目录用 `npm install --ignore-scripts --no-audit --no-fund <tgz>` 安装；
- 从安装后的真实 bin 执行 `--help`、`--version` 和未知参数测试；
- 从安装后的 `lib` 组合层运行本地假 HTTPS + 假 portable ZIP 场景，覆盖首次下载、二次命中、离线、篡改修复和安全清理；
- 打包/安装过程不得运行生命周期脚本，也不得联系真实 GitHub Release 或模型。

**GREEN：**

- 增加 `npm run test:launcher` 和 `npm run test:launcher:package`；
- tarball 测试创建、验证并清理自己的临时目录，清理失败明确报错；
- 根 `npm test` 包含 launcher 单元测试和 tarball 测试。

**验证：**

```powershell
npm run test:launcher
npm run test:launcher:package
npm pack --workspace packages/launcher --json
npm test
npm audit --audit-level=high
git diff --check
```

**提交：**`test: prove packed npm launcher`

---

## 任务 9：接入 Windows Node 22/24 CI 和仓库封印

**文件：**

- 修改 `.github/workflows/ci.yml`
- 修改 `scripts/workflow-contracts.mjs`
- 修改 `scripts/validate-repository.mjs`
- 修改 `scripts/repository-contracts.test.mjs`
- 修改 `docs/security.md`
- 修改 `docs/troubleshooting.md`
- 修改 `docs/release-checklist.md`
- 修改 `README.md`

**RED：**

- 新增轻量 launcher CI job，Windows 上分别使用 Node `22.22.0` 和 `24`；
- 每个矩阵项执行干净安装、launcher 单元测试、tarball 测试和仓库验证；
- 工作流无 secrets、无写权限、无模型调用、无 Release 下载、无 npm publish；
- action 必须固定到审核过的完整提交 SHA，checkout 禁用凭据持久化；
- 仓库验证器要求 launcher 包版本、桌面版本、标签规则、内置清单和 npm 文件白名单一致；
- 文档明确启动器是唯一需要 GitHub 下载的独立组件，桌面运行和结果仍只保存在本机；
- 故障文档覆盖首次联网、代理/证书、哈希不符、缓存损坏、并发启动、Node/平台不支持和 `--clear-cache`；
- Release checklist 增加 npm 登录、包名所有权、tarball 审计、公开 Release 后清单复核和真实 `npx` 门槛。

**GREEN：**

- 更新工作流结构解析和精确步骤封印，不能为了通过测试放宽现有发布权限或资产规则；
- README 在 npm 尚未发布前只说明“即将提供”，不放置会失败的默认安装命令；
- npm 发布成功后再用独立提交把 README/Pages CTA 切换为可用状态。

**验证：**

```powershell
npm run validate:repository
node --test scripts/repository-contracts.test.mjs
npm test
npm run build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
git diff --check
```

**提交：**`ci: verify npm launcher on node lts`

---

## 检查点 A：实现完成但保持未公开

任务 1～9 完成后暂停并提交以下证据：

- 全部离线 launcher 测试通过；
- `npm pack --json` 精确文件清单和 `.tgz` SHA-256；
- Node 22.22 和 Node 24 Windows CI 均通过；
- 当前 npm 包仍未发布，GitHub Release 仍为草稿；
- 自动测试真实 Codex/Claude 调用次数为 0；
- 候选 `release-manifest.json` 对应现有草稿 portable ZIP，但仍需公开后的二次生成复核。

证据齐全后，把 `codex/ai-ability-radar-v02` 推送到 GitHub，创建 PR，等待 Node 22/24 launcher job 和现有完整 CI 全绿。代码审查通过后合并到 `main`，再次等待 main CI；只有这些步骤成功，才进入人工发布。PR 和合并不会触发真实模型调用，也不会自动发布 GitHub Release 或 npm 包。

---

## 任务 10：人工验收并公开 GitHub Release

**文件：**

- 修改 `docs/test-matrix.md`
- 修改 `docs/release-checklist.md`
- 不修改或移动既有 `v0.2.2` 标签

**人工门槛：**

1. 在干净 Windows 10 x64 验收 EXE 安装/卸载、MSI 安装/卸载和 portable ZIP 解压启动；
2. 在干净 Windows 11 x64重复同一验收；
3. 记录测试者、日期、OS build、资产 SHA-256 和通过/失败证据；
4. 核对外层 `SHA256SUMS.txt` 覆盖且匹配 EXE、MSI 和 ZIP；
5. 确认未签名警告、数据路径、无安装 portable 行为和离线客户端模式均如实；
6. 所有门槛通过后，才把现有草稿预发布公开；不得上传替换同名资产。

**公开后复验：**

```powershell
gh release view v0.2.2 --repo YuLeo926/ai-ability-radar
gh release download v0.2.2 --repo YuLeo926/ai-ability-radar --pattern "ability-radar_0.2.2_windows-x64-portable.zip" --pattern "SHA256SUMS.txt" --dir <全新临时目录>
npm run launcher:manifest -- --assets-dir <全新临时目录>
git diff --exit-code -- packages/launcher/release-manifest.json
```

若清单出现任何 diff，立即停止 npm 发布并调查，不修改 Release 资产来迁就 npm 清单。

**提交：**`docs: record v0.2.2 windows acceptance`

---

## 任务 11：登录 npm、公开发布并做真实 npx 验收

**前置条件：**

- GitHub `v0.2.2` Release 已公开；
- 任务 10 的清单二次生成无 diff；
- 工作树干净，main CI 通过；
- `npm view ai-ability-radar` 确认名称状态，用户确认其 npm 身份拥有发布权；
- 用户在本机交互完成 `npm login`，不得把 token 发给 Codex 或写入仓库。

**发布前命令：**

```powershell
npm whoami
npm run validate:repository
npm test
npm run test:launcher:package
npm audit --audit-level=high
npm pack --workspace packages/launcher --json
npm publish --workspace packages/launcher --access public
```

发布前人工查看 `npm pack --json` 的最终文件列表、包版本、完整性哈希和大小。`npm publish` 是不可逆的外部写入，执行当次必须再次向用户展示包名、版本和公开可见性并取得明确确认。

**发布后验收：**

1. 在不含仓库源码的全新目录运行 `npx --yes ai-ability-radar@0.2.2 --version`；
2. 运行 `npx --yes ai-ability-radar@0.2.2`，验证首次下载、哈希核对和桌面启动；
3. 断网后再次运行，验证缓存命中和桌面启动；
4. 篡改测试缓存中的一个副本文件，恢复联网后验证安全修复；
5. 运行 `--clear-cache`，确认只清除启动器缓存；
6. 记录 npm 页面、包 integrity、`.tgz` SHA-256、GitHub Release URL 和实测结果。

真实 `npx` 只下载/启动桌面程序，不自动开始能力测试，因此不应消耗 Codex/Claude 额度。若用户随后主动开始真实能力测试，仍需单独确认订阅额度消耗。

**回滚：**

- 不覆盖 `0.2.2`；严重问题使用 `npm deprecate ai-ability-radar@0.2.2 "<原因和替代版本>"` 标记；
- 修复只能发布新的补丁版本，并建立新的 Git 标签、Release 资产、清单和测试证据；
- GitHub Release 保留审计记录，不静默替换同名资产。

---

## 任务 12：开放公开入口并完成上线记录

**文件：**

- 修改 `README.md`
- 修改 `site/index.html`
- 修改 `scripts/validate-repository.mjs`
- 修改 `scripts/repository-contracts.test.mjs`
- 修改 `docs/release-checklist.md`

**RED：**

- README 和 Pages 只在 npm 发布和真实 `npx` 验收成功后显示可复制命令；
- 命令固定展示 `npx ai-ability-radar@0.2.2`，并解释不带版本的 `@latest` 升级语义；
- 下载入口同时保留 GitHub 安装包、MSI、portable ZIP、源码启动和 npm 启动器；
- 页面不得暗示 npm 包包含桌面程序、免首次联网、已签名或由维护者承担订阅费用；
- 仓库验证器要求公开 CTA、GitHub Release 和 npm 版本完全一致。

**GREEN：**

- 更新公开文档和 Pages；
- 完成 release checklist 的 npm 部分并保留验收证据链接；
- 推送后等待 main CI 和 Pages 部署成功，再从公开页面点击/复制入口复验。

**验证：**

```powershell
npm run validate:repository
npm test
npm run build
git diff --check
```

**提交：**`docs: launch npm distribution`

## 最终完成条件

1. `npx ai-ability-radar@0.2.2` 只会启动 GitHub `v0.2.2` 的已验证 portable 资产；
2. npm tarball 不含桌面二进制、ZIP、测试证书、缓存或用户数据；
3. 首次下载、离线、并发、损坏修复、清理和失败保留旧缓存均有自动证据；
4. 所有下载和解压文件由 npm 内置固定值验证；
5. Node 22.22/24 Windows CI、仓库验证、依赖审计和桌面回归测试通过；
6. Windows 10/11 x64 人工验收和 GitHub Release 公开完成；
7. npm 用户身份、公开发布和全新目录真实 `npx` 验收完成；
8. README 与 Pages 只在实际可用后开放入口；
9. 自动开发和 CI 真实 Codex/Claude 调用次数为零；
10. 既有 `v0.2.2` 标签和同名 Release 资产未被移动或静默替换。
