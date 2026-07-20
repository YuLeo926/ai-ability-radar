# AI 能力雷达：推理档位、CLI 检测与免安装运行设计

日期：2026-07-19
目标版本：v0.2.1
状态：已选择方案 A，待书面规格复核

## 2026-07-20 用户决定 A 修订（控制性）

本节覆盖下文与之冲突的早期措辞。v0.2.1 最终修复必须：

- 在压缩前按应用相同的 registry、manifest、引用文件和内容哈希不变量校验源题包与暂存副本，并在压缩前再次校验暂存副本；
- 拒绝手动自定义档位中的 Unicode `Cc`、`Cf`、默认可忽略字符和双向格式控制字符，去除首尾空白后必须仍有 1–40 个可见字符；
- 为每次便携打包独占创建不可预测的暂存目录，只按已记录身份清理本次调用拥有的目录，绝不复用或删除未知的固定 `.stage`；
- 让外层 `SHA256SUMS.txt` 恰好且按固定顺序覆盖版本化 NSIS、MSI 和便携 ZIP；缺失、额外、重复叶名称或错误版本均失败；
- 在 clean Windows 10/11 x64 验收与真实公开发布完成前，把 README/站点描述为候选/待发布构建，所有公共下载 CTA 保持不可导航；
- 保留“重新检测 CLI”，但只承诺立即发现已继承 PATH 目录内的变化；安装程序新增 PATH 目录时必须先重启应用再检测。

## 1. 背景与目标

v0.2 预览版暴露了三个直接影响可用性的问题：

1. ChatGPT、Codex 和 Claude 的推理档位被统一写死为 `low`、`medium`、`high`，与当前产品界面和模型能力不一致。
2. Windows 上通过 npm 安装的 Codex CLI 明明存在，应用仍可能显示“未检测到安装”。
3. 项目只有安装包使用路径，缺少“下载源码后直接运行”和“解压即用”的低摩擦入口。

本次改动的目标是：

- 按目标产品提供当前推理档位预设，同时允许原样记录未来新增或改名的档位；
- 在 Windows 上可靠识别并安全执行 npm 安装和原生安装的 Codex CLI、Claude Code；
- 增加无需安装的源码运行入口和 Windows x64 免安装 ZIP；
- 保持现有本地存储、固定题包、费用边界和安全约束不变。

## 2. 范围与非目标

### 2.1 本次范围

- ChatGPT 客户端和 Claude 客户端手动体检的推理档位输入；
- Codex CLI 和 Claude Code 自动体检的推理档位输入与参数传递；
- Windows CLI 命令定位、版本检测、登录状态检测和实际执行；
- 根目录 `npm start` 源码运行入口；
- Windows x64 免安装 ZIP、校验值和 GitHub Release 产物；
- 相关界面提示、故障排查文档、自动化测试和发布验证。

### 2.2 非目标

- 本次不新增普通浏览器可访问的 localhost HTTP 后端；
- 不把 Vite 页面单独宣传为可工作的浏览器版；
- 不自动操作 ChatGPT 或 Claude 客户端；
- 不安装、更新或登录任何第三方 CLI；
- 不新增共享账号、API 代理、远程遥测、自动上传或云端评分；
- 不实现“数据也随 ZIP 移动”的全便携模式。免安装版仍把本地数据保存在现有 `%APPDATA%\com.aiability.radar` 目录；
- 不在本次处理代码签名、自动更新和 macOS/Linux 发布。

## 3. 推理档位设计

### 3.1 数据模型

继续使用现有可选字符串字段 `reasoning_effort`，不修改数据库结构。已知档位保存规范英文值，未知档位保存经过校验的用户原始输入。

已知规范值为：

```text
none
minimal
low
medium
high
xhigh
max
ultra
```

已知值在界面和报告中显示中文标签；自定义值按用户填写内容显示。历史记录中的 `low`、`medium`、`high` 保持兼容。

### 3.2 按产品显示预设

| 目标 | 默认预设 |
| --- | --- |
| ChatGPT 客户端 | 未显示/不适用、轻度、中、高、极高、最高、Ultra，以及“其他/按界面原样填写” |
| Claude 客户端 | 未显示/不适用、低、中、高、极高、最高，以及“其他/按界面原样填写” |
| Codex CLI | CLI 默认、`minimal`、`low`、`medium`、`high`、`xhigh`、`max`、`ultra`，以及自定义 |
| Claude Code | CLI 默认、`low`、`medium`、`high`、`xhigh`、`max`，以及自定义 |

ChatGPT/Codex 的 `none` 只在用户选择自定义时记录，因为它不是当前截图中常见的交互档位。Claude Code 的 `ultracode` 是一种组合运行模式，不作为额外 effort 值展示。

界面明确提示：实际可用档位取决于所选模型、客户端版本和账户权限。应用不会假装能够从客户端准确读取模型能力。

### 3.3 自定义输入与校验

- 手动客户端体检允许去除首尾空白后的 1–40 个可见字符；不允许 Unicode 控制字符、格式/默认可忽略字符或双向格式控制字符；
- CLI 自定义值统一转为小写，只允许 ASCII 字母、数字、下划线和连字符，长度为 1–32；
- 参数始终作为独立进程参数传递，不拼接进 shell 命令；
- 如果所选模型不支持该档位，保留 CLI 的真实错误并把本次运行归类为基础设施失败，不静默降级到其他档位。

## 4. Windows CLI 检测修复

### 4.1 已确认根因

本机 npm 安装生成了以下入口：

```text
%APPDATA%\npm\codex
%APPDATA%\npm\codex.cmd
%APPDATA%\npm\codex.ps1
```

应用当前直接启动裸名称 `codex`。Windows 在 PATH 中首先遇到无扩展名 npm 脚本时会返回“拒绝访问”，导致适配器把已安装的 CLI 误判为未安装。相同环境中，显式运行 `codex.cmd --version` 和 npm 包内的真实 `codex.exe --version` 均成功。

### 4.2 命令定位器

新增共享的提供方命令定位器，返回：

```text
program: 要直接执行的 EXE
prefix_args: 在业务参数之前追加的固定参数
source_kind: native_exe | npm_package
```

Windows 定位顺序：

1. 在 PATH 中查找可直接执行的 `codex.exe` 或 `claude.exe`；
2. 发现 npm shim 时，只匹配经过审核的官方包布局：
   - `node_modules/@openai/codex/bin/codex.js`
   - `node_modules/@anthropic-ai/claude-code/cli.js`
3. 对官方 npm 包使用解析到的 `node.exe` 直接运行 JavaScript 入口，并把入口路径放在固定前置参数中；
4. 找不到受支持入口时返回未安装。

非 Windows 平台保持当前 PATH 原生命令行为，为后续平台支持保留接口，但本次不发布其他平台产物。

### 4.3 安全边界

- 不通过 `cmd.exe`、PowerShell 或其他 shell 传递题目、模型名、档位或工作目录；
- 不执行任意 `.cmd`、`.bat` 或 `.ps1` 内容；
- 只接受存在的普通文件和已审核的官方 npm 包相对布局；
- 版本检测、登录状态检测和实际体检复用同一个已解析命令，避免“检测成功、运行失败”；
- 不把绝对安装路径写入历史、报告或界面；
- 应用不会替用户安装、升级或登录 CLI。

### 4.4 界面行为

- 修复后，当前本机应显示 `codex-cli 0.142.5`，登录状态仍由官方只读状态命令判断；
- 编程 CLI 区域增加“重新检测”操作；已继承 PATH 目录内的安装或更新可原地刷新，如果安装程序新增 User/Machine PATH 目录则先重启应用再检测；
- 未找到入口时显示“未检测到可执行入口”，故障排查文档列出 PowerShell 只读检查方法；
- 检测不发送提示词，不产生模型调用费用。

## 5. 源码运行与免安装版

### 5.1 源码运行

根目录新增统一入口：

```powershell
npm ci
npm start
```

`npm start` 启动 Tauri 开发模式：Vite 提供前端资源，Rust/Tauri 提供完整本地后端，并自动打开桌面开发窗口。它不要求安装 AI 能力雷达，但仍要求仓库 README 中列出的 Node.js、Rust、WebView2 和 Windows 构建工具。

Vite 的开发 URL 仅供 Tauri WebView 使用。直接在普通浏览器打开该 URL 不具备 Tauri IPC 后端，不能作为完整产品使用。

### 5.2 免安装 ZIP

新增 Windows 专用打包命令：

```powershell
npm run package:portable
```

该命令构建并生成：

```text
ability-radar_0.2.1_windows-x64-portable.zip
```

ZIP 顶层结构：

```text
ability-radar-portable/
├── ability-radar.exe
├── benchmark-packs/
├── README.txt
└── SHA256SUMS.txt
```

用户解压后直接运行 `ability-radar.exe`。`benchmark-packs` 必须与 EXE 保持相对位置；`README.txt` 明确未签名提示、数据目录、CLI 费用和完整性校验方法。

便携包不修改系统注册表、不创建卸载项，也不要求管理员权限。它与安装版共用应用标识和本地数据目录，因此两者会看到同一份历史记录。

打包器在复制前校验源题包，在复制后和压缩前校验暂存题包，并在解压验证阶段再次校验。每次调用使用独占 UUID 暂存目录；固定 `.stage` 或其他未知预存内容不属于本次调用，不能复用或删除。

### 5.3 发布产物

GitHub Release 同时提供：

- NSIS 安装程序；
- MSI 安装程序；
- Windows x64 免安装 ZIP；
- 恰好按 NSIS、MSI、Windows x64 免安装 ZIP 顺序列出这三个版本化文件的 `SHA256SUMS.txt`。

发布工作流仍只使用 fake CLI 做自动化验证，不调用真实 AI 服务。

v0.2.1 在 clean Windows 10/11 x64 验收和公开发布完成前仅为候选/待发布构建；README 与静态站点不提供活动下载链接。发布工作流可以继续准备草稿预发布，但不会据此宣称已有公共下载。

## 6. 错误处理

- 命令定位失败：目标显示不可用，不尝试猜测或执行未知脚本；
- npm 包存在但 Node.js 不可用：显示 Node.js 前置条件失败；
- 版本命令输出异常：不展示原始输出，避免把路径或敏感内容带入界面；
- 自定义档位格式错误：在启动前就地提示；
- 模型不支持所选档位：显示脱敏后的提供方错误类别，保留本地原始日志供用户自行排查；
- 便携包缺失题包：应用启动失败并给出恢复完整解压目录的说明，不使用不完整题包继续评分。

## 7. 测试与验收

实现遵循测试驱动顺序。

### 7.1 自动化测试

- Windows 命令定位器在临时 PATH 中复现“无扩展名脚本优先”的失败场景；
- 原生 EXE、Codex npm 包、Claude npm 包、缺少 Node、伪造 npm 布局和完全未安装场景；
- 检测和执行必须使用同一个解析结果；
- 任意包含 shell 元字符的题目仍作为单独参数传递；
- 四种目标分别显示正确预设和自定义输入；
- 前后端使用匹配规则接受新增安全档位，并拒绝控制、格式、默认可忽略、双向控制、不可见空值、超长值和非法 CLI 自定义值；
- 历史记录、恢复、比较和导出兼容新增及自定义档位；
- `npm start`、便携打包脚本和 GitHub Release 文件清单通过仓库契约测试；
- 便携 ZIP 解压后包含 EXE、完整且通过 registry 校验的题包、说明和内部校验文件。

### 7.2 完整验证

- 前端单元测试、无障碍测试和生产构建；
- Rust 格式检查、严格 clippy 和全部 workspace 测试；
- 现有 sealed fake CLI E2E；
- 仓库验证器、依赖许可元数据和离线 npm audit；
- Windows 10/11 x64 干净环境分别验证源码运行、安装版和免安装版；
- 手工确认“重新检测”不会触发模型调用；
- 仅在用户主动开始真实 CLI 体检时才可能消耗其订阅额度。

## 8. 依据

- OpenAI Codex 官方手册：不同模型可提供 Light、Medium、High、Extra High、Max，符合条件的模型/账户还可能提供 Ultra；底层 `model_reasoning_effort` 支持范围依模型而异。
  <https://developers.openai.com/codex/codex-manual.md>
- Anthropic 官方 effort 文档：当前完整 API effort 集合为 `low`、`medium`、`high`、`xhigh`、`max`；Claude Code 的 ultracode 不是额外 API effort 值。
  <https://platform.claude.com/docs/en/build-with-claude/effort>

## 9. 验收标准

1. 当前本机 npm 安装的 Codex CLI 被识别为已安装并显示公开版本号；
2. ChatGPT 截图中的轻度、中、高、极高、最高均可记录，Claude/Codex 的当前五档 effort 均可选择；
3. 未来新增档位可通过受限自定义输入记录，无需立即发布数据库迁移；
4. `npm ci` 后运行 `npm start` 能打开完整桌面开发版；
5. 免安装 ZIP 解压后可独立启动，且题包完整性校验通过；
6. 安装版、免安装版和源码版保持同一评分、隐私、费用和安全边界；
7. 自动化测试和发布流程不会调用真实 Codex CLI、Claude Code 或 AI 服务。
