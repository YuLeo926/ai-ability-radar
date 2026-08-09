# Codex CLI / Claude Code 本地计分回归雷达实施计划

> **执行要求：**逐任务实施，每个任务遵循 RED → GREEN → 回归验证 → 独立提交。真实模型调用必须放在所有离线契约测试之后，并再次取得用户的额度确认。

**目标：**在现有 Windows 本地应用中增加一套 30～60 分钟、5 道 JavaScript/TypeScript 工程题的正式计分测试，通过 Promptfoo 调用 Codex SDK 或 Claude Agent SDK，输出 0～100 原始能力分、0～150 雷达 IQ，以及与冻结本地基线的变化证据。

**设计依据：**`docs/superpowers/specs/2026-08-09-scored-cli-regression-radar-design.md`

**实现原则：**保留现有运行、存储、批次、取消、报告和 Windows 打包能力；新增版本化的 Promptfoo 执行契约、计分题包 schema、确定性分项验证和单目标历史系列。旧 `client-quick` 与 `cli-quick` 继续用于流程自检，但不得产生正式能力结论。

**技术栈：**Rust 2024、SQLite/rusqlite、Tauri 2、React 19、TypeScript 5.8、Node.js、Promptfoo、OpenAI Codex SDK、Claude Agent SDK、Vitest、Node test runner。

## 已核实的执行前提

计划编写时（2026-08-09）已核实：

- 本机 Node.js 为 `22.22.2`，满足 Promptfoo `>=22.22.0` 要求；
- npm registry 当前 Promptfoo 为 `0.122.0`；
- `@openai/codex-sdk` 当前为 `0.147.0`；
- `@anthropic-ai/claude-agent-sdk` 当前为 `0.3.226`，其许可证不是 Apache/MIT，必须保留独立许可证说明；
- 本机 Codex CLI 为 `0.142.5`；官方 Promptfoo 文档要求 GPT-5.6 使用 Codex SDK/Codex `0.144.0` 或更高，并建议通过追踪确认实际推理档位；
- Codex 首选 `openai:codex-sdk`，因为本产品测量自动编程能力，不依赖 app-server 的审批、插件或桌面事件；
- Claude 使用 `anthropic:claude-agent-sdk`，本地订阅登录时设置 `apiKeyRequired: false`；
- 两个 provider 均使用独立工作区、禁用 Promptfoo 缓存、串行执行；Codex 使用 `workspace-write`、关闭网络与搜索，Claude 仅开放完成任务所需的读取、编辑、写入和本地命令工具。

参考：

- <https://www.promptfoo.dev/docs/providers/openai-codex-sdk/>
- <https://www.promptfoo.dev/docs/providers/claude-agent-sdk/>
- <https://www.promptfoo.dev/docs/guides/evaluate-coding-agents/>

## 全局限制

- 除明确的真实验收任务外，所有自动化测试只使用假 provider、固定工作区和合成输出。
- 不在参数、日志或报告中暴露登录 token、API key、完整环境变量或用户目录。
- 不把模型自报名称当作已验证的底层模型身份。
- 不启用联网搜索、网络安装、MCP、skills、plugins、子 Agent 或用户全局规则。
- 每道正式题必须是全新会话和全新工作区，默认并发数固定为 1。
- Promptfoo 及 SDK 使用 lockfile 精确锁定；版本变化必须更新执行契约版本并重新校准。
- 能力分只来自确定性验证器。时间、Token、额度和工具步骤只作为独立指标。
- 基础设施失败不计 0 分；超时或 Agent 预算耗尽按工作区中已有的可验证结果计分。
- 正式总分要求同一次运行的 5 道题均形成有效结果。
- 旧数据库、旧报告和旧题包必须继续可读。
- 不自动安装或升级 Codex、Claude、Node、Promptfoo 或 SDK。
- 不推送、不发布、不签名，除非用户之后单独授权。

## 里程碑

1. **M1 执行底座：**Promptfoo 假 provider 和 Codex SDK 单题契约可用；
2. **M2 计分底座：**schema v2、分项验证、总分与雷达 IQ 可复算；
3. **M3 快速题包：**5 道正式候选题完成标准答案与缺陷变体测试；
4. **M4 产品接入：**预检、额度确认、串行运行、结果页和源码启动可用；
5. **M5 历史基线：**临时/冻结基线和变化信号可用；
6. **M6 校准发布：**完成真实配置重复校准后，候选题包才升级为正式题包。

---

## 任务 1：锁定 Promptfoo 运行依赖与仓库契约

**文件：**

- 修改 `package.json`
- 修改 `package-lock.json`
- 修改 `THIRD_PARTY_NOTICES.md`
- 修改 `scripts/generate-license-metadata.mjs`
- 修改 `scripts/repository-contracts.test.mjs`
- 修改 `docs/licenses/npm-dependencies.json`
- 新建 `tools/promptfoo-runner/README.md`

**RED：**

- 增加仓库测试，要求 Promptfoo、Codex SDK 和 Claude Agent SDK 使用精确版本并存在于 lockfile；
- 要求 Node engine 预检为 `>=22.22.0`；
- 要求 Claude Agent SDK 的非 Apache/MIT 许可证单独列入第三方说明；
- 要求生产运行不使用 `npx --yes` 或在线即时下载。

**GREEN：**

- 精确锁定经验证版本，不使用 `latest` 或宽泛范围；
- 增加 `test:promptfoo-runner` 脚本；
- 文档说明源码运行与便携运行使用同一份锁定依赖；
- 记录 Promptfoo provider ID 和项目执行契约版本 `promptfoo-agent-v1`。

**验证：**

```powershell
node --test scripts/repository-contracts.test.mjs
npm run licenses:generate
git diff --check
```

**提交：**`build: pin promptfoo agent runtime`

---

## 任务 2：建立无秘密的 Node Runner 协议

**文件：**

- 新建 `tools/promptfoo-runner/protocol.mjs`
- 新建 `tools/promptfoo-runner/run.mjs`
- 新建 `tools/promptfoo-runner/provider-config.mjs`
- 新建 `tools/promptfoo-runner/tests/protocol.test.mjs`
- 新建 `tools/promptfoo-runner/tests/provider-config.test.mjs`
- 修改 `crates/ability-adapters/src/process.rs`
- 修改 `crates/ability-adapters/tests/process_contract.rs`

**RED：**

- 请求 JSON 仅允许：provider、workspace、prompt、requested model、reasoning effort、时间预算、Claude 轮次预算、run ID；
- 拒绝未知字段、相对/不存在工作区、非有限数值、超限文本和不支持档位；
- 响应必须包含状态、最终文本、session ID、Token、工具摘要、可见模型证据和 provider 错误码；
- 进程参数和 Debug 输出不得包含 prompt、登录数据或工作区文件内容；
- Rust `ProcessRunner` 能通过 stdin 传入有界 JSON，并在取消/超时时终止整个进程树；
- runner stdout 只输出一条版本化结果 JSON，诊断写 stderr 且经过脱敏。

**GREEN：**

- 给 `ProcessSpec` 增加可选、有大小上限的 stdin；
- Node runner 从 stdin 读取请求，严格校验后调用 provider；
- 所有 provider 错误归一成稳定码：auth、quota、network、model unavailable、runtime、unknown；
- 默认关闭 Promptfoo cache；
- 输出契约版本固定为 `promptfoo-agent-v1`。

**验证：**

```powershell
npm run test:promptfoo-runner
cargo test -p ability-adapters --test process_contract
cargo fmt --all -- --check
git diff --check
```

**提交：**`feat: add promptfoo runner protocol`

---

## 任务 3：实现严格的 Codex 与 Claude provider 配置

**文件：**

- 修改 `tools/promptfoo-runner/provider-config.mjs`
- 修改 `tools/promptfoo-runner/run.mjs`
- 新建 `tools/promptfoo-runner/tests/codex-provider.test.mjs`
- 新建 `tools/promptfoo-runner/tests/claude-provider.test.mjs`
- 新建 `tools/promptfoo-runner/tests/fixtures/fake-provider.mjs`

**RED：**

- Codex 映射到 `openai:codex-sdk`，使用 `working_dir`、`workspace-write`、`approval_policy: never`、ephemeral thread、禁用网络/搜索；
- 请求的模型映射到 `config.model`，推理档位映射到 `model_reasoning_effort`，不自行压缩为低/中/高；
- Claude 映射到 `anthropic:claude-agent-sdk`，本地登录使用 `apiKeyRequired: false`、`permission_mode: dontAsk`、不持久化 session；
- Claude 只允许 Read/Grep/Glob/Edit/Write/Bash，禁止 WebSearch/WebFetch/MCP/AskUserQuestion；
- 两端均不得继承完整进程环境，只传递白名单运行变量和 provider 自己解析的本地登录状态；
- Promptfoo 或 SDK 返回未知字段时保留安全摘要，但不能把未验证字段升级为模型事实；
- 缓存命中被视为契约错误，而不是有效能力结果。

**GREEN：**

- 通过 Promptfoo 公开的 `loadApiProvider`/`callApi` 边界实现单任务调用；
- 将 provider 配置构建为纯函数，便于 snapshot 测试；
- 用假 provider 覆盖成功、部分工具轨迹、auth、quota、network、model unavailable、取消和畸形结果。

**验证：**

```powershell
npm run test:promptfoo-runner
git diff --check
```

**提交：**`feat: configure promptfoo coding agents`

---

## 任务 4：把 Promptfoo 接入 Rust AgentAdapter

**文件：**

- 新建 `crates/ability-adapters/src/promptfoo.rs`
- 新建 `crates/ability-adapters/tests/promptfoo_adapter.rs`
- 修改 `crates/ability-adapters/src/lib.rs`
- 修改 `crates/ability-adapters/src/classify.rs`
- 修改 `apps/desktop/src-tauri/src/app_state.rs`
- 修改 `apps/desktop/src-tauri/src/batch_runner.rs`

**RED：**

- `detect` 检查 Node、runner、Promptfoo、对应 SDK、本地 CLI/登录状态和版本；
- Codex `0.144.0` 以下对 GPT-5.6 显示“版本可能忽略档位”，不假装 Ready；
- adapter identity 包含 Promptfoo、SDK、provider ID 和契约版本，但不含绝对路径；
- execute 使用独立工作区、严格超时和取消 token；
- provider 错误映射到现有基础设施失败，时间耗尽映射 Agent 预算失败；
- 成功结果保留最终文本、Token、工具摘要、session ID 和请求/可见模型证据；
- 旧直接 CLI adapter 保留为回退实现，但正式计分题包拒绝其旧契约。

**GREEN：**

- 新增 `PromptfooAgentAdapter`；
- 正式题包使用 `promptfoo-agent-v1`；
- 流程自检题包仍可使用现有 `codex-cli-v1`/`claude-code-v1`；
- app state 根据题包执行契约选择 adapter。

**验证：**

```powershell
cargo test -p ability-adapters --test promptfoo_adapter
cargo test -p ability-adapters --all-targets
cargo test -p ability-radar --lib
cargo fmt --all -- --check
```

**提交：**`feat: bridge promptfoo agents into desktop runner`

---

## 检查点 A：先做一次轻量 Codex 执行底座验证

完成任务 1～4 后暂停。先运行免费预检，再向用户展示“1 次任务启动、最长 5 分钟、会消耗用户自己的 Codex/ChatGPT 订阅额度”。只有用户再次明确确认后，才用一个不计分的临时仓库任务验证：

- Promptfoo 能复用本机 Codex 登录；
- 请求模型与推理档位被写入执行配置；
- Agent 只能修改隔离工作区且网络关闭；
- 取消能终止进程树；
- 最终文本、文件修改、Token、工具摘要和 session ID 能按契约返回；
- 结果不进入能力分或历史基线。

若用户暂不授权额度，后续仍可继续开发，但正式题包保持 candidate，且不得宣称真实 Codex 接入已经验收。

---

## 任务 5：定义计分题包 schema v2

**文件：**

- 修改 `schemas/pack.schema.json`
- 修改 `crates/ability-core/src/packs.rs`
- 修改 `crates/ability-core/tests/pack_loading.rs`
- 修改 `crates/ability-core/tests/bundled_registry.rs`
- 修改 `crates/ability-core/src/bin/ability-pack-validator.rs`

**RED：**

- schema v1 保持兼容；
- schema v2 必须声明 `run_profile`、`release_status`、`scoring_rule_version` 和 `execution_contract_version`；
- `run_profile: scored_quick` 必须恰好 5 道任务；未来的 standard/full profile 使用各自的数量约束，不需要再次破坏 schema；
- 每题 grader 为 `scored_external_verifier`，声明 scorecard 文件和 verifier ID；
- scorecard 五维权重严格为 40/25/15/10/10；
- 每题至少一个不可替代核心断言；
- 所有分值为正整数且总计 100；
- 路径必须位于题包内，禁止链接、reparse point、越界和重复断言 ID；
- schema v2 题包只允许 `codex_cli` 与 `claude_code`。

**GREEN：**

- 增加 `ScoredExternalVerifier` 和版本化 scorecard 结构；
- registry/manifest 支持 `candidate` 与 `official`，未满足校准门槛时只能是 candidate；
- `LoadedTask` 加载但不向 Agent 工作区复制 scorecard 与 verifier；
- 题包内容哈希覆盖 starter、prompt、scorecard、verifier、许可证和快照元数据。

**验证：**

```powershell
cargo test -p ability-core --test pack_loading
cargo test -p ability-core --test bundled_registry
cargo run -p ability-core --bin ability-pack-validator -- --registry benchmark-packs/registry.json
```

**提交：**`feat: define scored benchmark pack schema`

---

## 任务 6：实现确定性分项评分与封顶规则

**文件：**

- 新建 `crates/ability-core/src/scorecard.rs`
- 新建 `crates/ability-core/tests/scorecard.rs`
- 修改 `crates/ability-core/src/domain.rs`
- 修改 `crates/ability-core/src/grading.rs`
- 修改 `crates/ability-core/src/lib.rs`
- 修改 `crates/ability-adapters/src/verifier.rs`
- 修改 `crates/ability-adapters/tests/verifier.rs`

**RED：**

- 分项分别产生 earned/possible/assertion evidence；
- 任一不可替代核心断言失败或核心断言通过率低于 50%，总分封顶 35；
- 构建失败或无法进入测试阶段，总分封顶 15；
- 篡改保护文件、越界读取、验证器绕过或写死隐藏答案，总分为 0；
- 多个封顶同时发生时取最低值；
- 基础设施/验证器自身故障返回 invalid 和 `score: null`；
- 同一工作区重复评分字节级一致；
- 非有限数值、未知断言、重复断言、缺失证据一律 fail closed。

**GREEN：**

- verifier 输出版本化 JSON，不再依赖 `TASK_PASSED` 文本；
- `TaskResult` 保存结构化分项摘要和证据 artifact 相对路径；
- 旧二元 verifier 继续通过旧解析器工作。

**验证：**

```powershell
cargo test -p ability-core --test scorecard
cargo test -p ability-core --test grading
cargo test -p ability-adapters --test verifier
```

**提交：**`feat: grade deterministic scorecards`

---

## 任务 7：修改总分、雷达 IQ 与旧分数兼容规则

**文件：**

- 修改 `crates/ability-core/src/domain.rs`
- 修改 `crates/ability-core/src/grading.rs`
- 修改 `crates/ability-core/tests/grading.rs`
- 修改 `crates/ability-core/tests/domain_contracts.rs`
- 修改 `crates/ability-core/src/report.rs`
- 修改 `crates/ability-core/tests/report.rs`
- 修改 `schemas/public-report.schema.json`

**RED：**

- schema v2 原始能力分等于 5 道有效题分的等权平均，而不是类别平均；
- 雷达 IQ 严格等于原始能力分乘 1.5，均保留一位小数；
- 分项维度按五题同名分项的已获分/可得分归一为 0～100；
- 5 题不完整时仅生成阶段成绩，不生成正式 `ScoreSummary`；
- 旧 schema v1 继续按旧类别规则读取和导出；
- 旧成绩不补造雷达 IQ 或新分项；
- 报告明确标注“本题包跑分刻度，不是心理测量 IQ”。

**GREEN：**

- 给 `ScoreSummary` 增加版本、可选雷达 IQ 和 dimension scores；
- 按评分规则版本选择 legacy/scored 聚合器；
- 所有报告验证器从任务证据重新计算分数，拒绝只信数据库总分。

**验证：**

```powershell
cargo test -p ability-core --test grading
cargo test -p ability-core --test report
cargo test -p ability-core --test report_schema
```

**提交：**`feat: calculate raw score and radar iq`

---

## 任务 8：建立正式候选题包骨架与隔离测试

**文件：**

- 新建 `benchmark-packs/cli-scored-quick-v1/manifest.json`
- 新建 `benchmark-packs/cli-scored-quick-v1/LICENSES/`
- 新建 `benchmark-packs/cli-scored-quick-v1/NOTICE.md`
- 新建 `benchmark-packs/cli-scored-quick-v1/tools/verify-task.mjs`
- 新建 `benchmark-packs/cli-scored-quick-v1/tools/scorecard.mjs`
- 新建 `scripts/scored-pack-contracts.test.mjs`
- 修改 `benchmark-packs/registry.json`

**RED：**

- 题包必须恰好 5 题、总预算不超过 60 分钟、每题不超过 12 分钟；
- starter 工作区不包含 hidden tests、scorecard、标准答案或 mutant；
- 所有依赖可离线运行且 lockfile 完整；
- 保护文件哈希在 Agent 运行前后可比较；
- 每题 verifier 只能读取自己的工作区和题包私有验证目录；
- 题包许可证元数据完整。

**GREEN：**

- 建立共享 verifier 协议和本地 fixture 工具；
- registry 中先标记为 `candidate`，未校准前 UI 不称“正式发布题包”。

**验证：**

```powershell
node --test scripts/scored-pack-contracts.test.mjs
cargo run -p ability-core --bin ability-pack-validator -- --registry benchmark-packs/registry.json
```

**提交：**`test: scaffold scored quick benchmark pack`

---

## 任务 9：题目一——异步并发与状态泄漏

**文件：**

- 新建 `benchmark-packs/cli-scored-quick-v1/tasks/async-state-leak/**`
- 修改 `scripts/scored-pack-contracts.test.mjs`

**RED：**

- 标准答案 >95；
- 空修改只得到低分；
- 至少 6 个 mutant 覆盖共享状态污染、竞态、异常清理、重复事件和取消路径；
- mutant 分数覆盖低/中/高三个区间；
- 连续评分 10 次结果一致。

**GREEN：**

- 从许可证兼容的真实开源项目裁剪最小可运行快照；
- 隐藏并发测试使用确定性屏障/假时钟，不依赖随机 sleep。

**验证：**运行该题标准答案、空修改、全部 mutant 和重复评分测试。

**提交：**`test: add async state leak benchmark`

---

## 任务 10：题目二——跨文件 API 功能

**文件：**

- 新建 `benchmark-packs/cli-scored-quick-v1/tasks/cross-file-api/**`
- 修改 `scripts/scored-pack-contracts.test.mjs`

**RED：**覆盖跨模块实现、输入验证、公共类型、旧调用方、错误契约和范围约束；标准答案 >95，mutant 覆盖低/中/高分。

**GREEN：**使用真实仓库裁剪快照，要求至少修改两个生产文件但禁止无关重写。

**验证：**标准答案、空修改、全部 mutant、10 次确定性复评分。

**提交：**`test: add cross file api benchmark`

---

## 任务 11：题目三——缓存配置与向后兼容

**文件：**

- 新建 `benchmark-packs/cli-scored-quick-v1/tasks/cache-config-compat/**`
- 修改 `scripts/scored-pack-contracts.test.mjs`

**RED：**覆盖缓存键、失效策略、旧配置默认值、序列化兼容、错误输入和并发访问；标准答案 >95，mutant 覆盖低/中/高分。

**GREEN：**使用真实仓库裁剪快照，隐藏测试固定时间与文件系统状态。

**验证：**标准答案、空修改、全部 mutant、10 次确定性复评分。

**提交：**`test: add cache compatibility benchmark`

---

## 任务 12：题目四——多约束重构

**文件：**

- 新建 `benchmark-packs/cli-scored-quick-v1/tasks/constrained-refactor/**`
- 修改 `scripts/scored-pack-contracts.test.mjs`

**RED：**覆盖行为保持、API 不变、禁止新增依赖、限制可改文件、类型检查和可维护交付；标准答案 >95，mutant 覆盖低/中/高分。

**GREEN：**构造接近真实项目规模的原创场景，避免单函数玩具题。

**验证：**标准答案、空修改、全部 mutant、10 次确定性复评分。

**提交：**`test: add constrained refactor benchmark`

---

## 任务 13：题目五——重试取消幂等与恢复

**文件：**

- 新建 `benchmark-packs/cli-scored-quick-v1/tasks/retry-cancel-recovery/**`
- 修改 `scripts/scored-pack-contracts.test.mjs`

**RED：**覆盖指数/固定策略约束、取消传播、重复请求幂等、部分失败恢复、错误分类和旧行为；标准答案 >95，mutant 覆盖低/中/高分。

**GREEN：**原创多模块任务使用假时钟和可控故障注入，禁止真实网络。

**验证：**标准答案、空修改、全部 mutant、10 次确定性复评分。

**提交：**`test: add recovery benchmark`

---

## 任务 14：持久化计分证据与历史系列身份

**文件：**

- 新建 `crates/ability-core/migrations/0004_scored_runs.sql`
- 修改 `crates/ability-core/src/storage.rs`
- 新建 `crates/ability-core/tests/scored_storage.rs`
- 修改 `crates/ability-core/tests/storage.rs`
- 修改 `crates/ability-core/tests/recovery.rs`

**RED：**

- 真实 v3 数据库可迁移且旧 JSON 不重写；
- task result 原子保存 scorecard JSON、证据 artifact 路径和执行指标；
- 历史系列 hash 包含目标、请求/可见模型、档位、题包 ID/版本/哈希、评分规则、Promptfoo 契约、clean/resumed；
- OS、CLI、账号路由和登录方式保存为环境变化证据，但不进入系列 hash；
- 保存运行开始时的本地日期、IANA 时区名称或可得的时区标识以及 UTC offset，基线的“不同日期”按这份冻结证据计算；
- 不完整、invalid、cancelled 或证据不一致的运行不能进入正式系列；
- 删除/恢复不会遗留 scorecard artifact 或破坏基线引用。

**GREEN：**

- 扩展 `task_results` 的结构化证据列；
- 新增 `ability_series`、`ability_series_runs`、`ability_baselines`；
- 所有状态转移使用事务并复用现有 artifact 两阶段删除规则。

**验证：**

```powershell
cargo test -p ability-core --test scored_storage
cargo test -p ability-core --test storage
cargo test -p ability-core --test recovery
```

**提交：**`feat: persist scored run evidence`

---

## 任务 15：实现临时基线、冻结基线和变化信号

**文件：**

- 新建 `crates/ability-core/src/regression.rs`
- 新建 `crates/ability-core/tests/regression.rs`
- 修改 `crates/ability-core/src/lib.rs`
- 修改 `crates/ability-core/src/storage.rs`

**RED：**

- 第 1～2 次显示尚无基线；
- 第 3 次建立临时基线，但不能产生“很可能退化”；
- 至少 5 次且覆盖 3 个不同的已存本地日期后冻结稳定基线，之后系统时区变化不能改写旧运行的日期归属；
- 新低分不会自动下移冻结基线；
- 最近 3 次中位数绝对下降 >=6、相对下降 >=8%、至少 3 题同向下降，才是持续异常；
- 稳定基线下 2,000 次固定种子分层 Bootstrap 的 95% 区间上界 <0，才是很可能退化；
- 题目为第一抽样层，运行在题目内重采样；
- 环境变化、排除过多或有效样本不足产生证据不足；
- 重建基线创建新快照和 hash，不删除旧快照。

**GREEN：**实现纯统计模块、可复现 RNG、冻结/重建存储接口和解释性 reason codes。

**验证：**

```powershell
cargo test -p ability-core --test regression
cargo test -p ability-core --all-targets
```

**提交：**`feat: analyze frozen local baselines`

---

## 任务 16：接入预检、额度确认、串行调度与恢复

**文件：**

- 修改 `apps/desktop/src-tauri/src/commands.rs`
- 修改 `apps/desktop/src-tauri/src/dto.rs`
- 修改 `apps/desktop/src-tauri/src/app_state.rs`
- 修改 `apps/desktop/src-tauri/src/batch_runner.rs`
- 修改 `apps/desktop/src-tauri/src/batch_commands.rs`
- 修改 `apps/desktop/src-tauri/src/batch_tests.rs`
- 修改 `crates/ability-adapters/src/cli_run.rs`
- 修改 `crates/ability-adapters/tests/cli_run.rs`

**RED：**

- 预检只做本地只读检查，不越过 provider 边界；
- 确认摘要精确显示 5 次任务启动、最长 60 分钟、模型/档位和额度归属；
- acknowledgement hash 绑定题包、目标、模型、档位、预算和执行契约；
- 无确认、过期确认或计划变化均不能调用 provider；
- 五题严格串行，每题独立 workspace/session；
- 当前题完成后立即保存；
- 取消阻止后续任务并杀死当前进程树；
- 已可能越过 provider 边界但结果未知的题不得自动重放；
- 恢复只启动确定未发送过请求的题，并重新确认剩余额度。

**GREEN：**在现有 batch/run 状态机上增加 `ScoredQuick` 能力级别和 Promptfoo adapter binding，不重写已有调度器。

**验证：**

```powershell
cargo test -p ability-adapters --test cli_run
cargo test -p ability-radar --lib
```

**提交：**`feat: orchestrate scored quick runs`

---

## 任务 17：更新前端流程与结果解释

**文件：**

- 修改 `apps/desktop/src/domain/batch.ts`
- 修改 `apps/desktop/src/api/backend.ts`
- 修改 `apps/desktop/src/api/tauriBackend.ts`
- 修改 `apps/desktop/src/api/runtimeValidation.ts`
- 修改对应 API/domain 测试
- 修改 `apps/desktop/src/pages/BatchSetupPage.tsx`
- 修改 `apps/desktop/src/pages/BatchRunPage.tsx`
- 修改 `apps/desktop/src/pages/BatchResultPage.tsx`
- 修改 `apps/desktop/src/pages/HistoryPage.tsx`
- 修改对应页面测试和 CSS
- 修改 `apps/desktop/src/i18n/messages.ts`

**RED：**

- 首页把旧题包标为“流程自检”，正式入口指向“CLI 计分快速测试”；
- 预检明确显示 Node、Promptfoo、SDK、CLI、登录和档位支持状态；
- 版本偏低时说明可能忽略档位，禁止误显示为已验证；
- 开始前必须勾选额度确认；
- 运行页显示当前题、已完成题、剩余预算和取消状态；
- 结果页并列显示原始能力分、雷达 IQ、5 道题、5 个分项和独立效率指标；
- 历史页区分尚无基线、波动、持续异常、很可能退化和证据不足；
- 环境变化与“不能证明官方换模”始终可见；
- 键盘、窄屏、缩放 200% 和 axe 可访问性测试通过。

**GREEN：**复用现有视觉 token 和批次页面结构，避免另建平行应用。

**验证：**

```powershell
npm run test --workspace apps/desktop
npm run build --workspace apps/desktop
```

**提交：**`feat: present scored cli radar results`

---

## 任务 18：更新报告、隐私、源码启动和 Windows 便携包

**文件：**

- 修改 `crates/ability-core/src/report.rs`
- 修改 `schemas/public-report.schema.json`
- 修改 `crates/ability-core/tests/report.rs`
- 修改 `crates/ability-core/tests/report_schema.rs`
- 修改 `scripts/package-portable.mjs`
- 修改 `scripts/package-portable.test.mjs`
- 修改 `README.md`
- 修改 `docs/privacy.md`
- 修改 `docs/methodology.md`
- 修改 `docs/troubleshooting.md`
- 修改 `docs/release-checklist.md`

**RED：**

- JSON/HTML 报告能复算 raw score、IQ、分项和基线信号；
- 报告不含绝对路径、用户名称、token、API key、完整 promptfoo 原始环境或隐藏测试正文；
- 便携包包含锁定 runner 及所需 Node 运行条件，离线假 provider 验收可运行；
- `npm start` 从源码直接启动同一后端和 index 页面；
- 缺依赖时给出可操作提示，不自动联网安装；
- 第三方许可证随便携包分发。

**GREEN：**升级 public schema，保留旧 schema 读取路径；扩展打包清单和用户文档。

**验证：**

```powershell
cargo test -p ability-core --test report
cargo test -p ability-core --test report_schema
node --test scripts/package-portable.test.mjs
npm run validate:repository
```

**提交：**`docs: ship scored radar runtime and reports`

---

## 任务 19：完成全仓离线验收

**文件：**按失败范围修正，不增加新功能。

**验收矩阵：**

1. 假 Codex：五题依次完成并产生预期部分分；
2. 假 Claude：同一题包与同一验证器完成；
3. auth、quota、network、runtime、verifier error 均不计 0 分；
4. 超时保留已有修改并部分计分；
5. 取消不启动下一题；
6. 崩溃恢复不重放不确定 provider 请求；
7. 旧数据库与旧报告仍可读取；
8. 源码启动与便携包使用相同题包 hash；
9. 连续运行不残留工作区、子进程或敏感日志。

**验证：**

```powershell
cargo test --workspace --all-targets
cargo fmt --all -- --check
npm test
npm run build
npm run validate:repository
git diff --check
```

**提交：**`test: verify scored radar end to end`

---

## 任务 20：一次受控的真实 Codex 单题验收

**前置人工门：**必须再次向用户显示并确认会消耗其 Codex/ChatGPT 订阅额度。没有确认就停止。

**步骤：**

1. 仅运行免费预检；
2. 确认 Codex SDK/CLI 满足 GPT-5.6 档位要求，或明确记录版本限制；
3. 选择候选题包中的一题、一个模型和一个档位；
4. 显示 1 次任务启动与最长 12 分钟；
5. 用户确认后执行；
6. 检查工作区修改、Promptfoo 结果、Token、工具轨迹、模型/档位证据、结构化评分和本地隐私；
7. 不把该单题结果写入正式基线。

**通过条件：**provider 边界只越过一次；结果可复算；取消有效；没有秘密或隐藏测试泄露。

**提交：**若只产生本地运行数据则不提交；仅在发现并修复代码问题后提交相应修复。

---

## 任务 21：完成候选题包校准并决定是否正式发布

**前置人工门：**这是大量真实调用，必须单独制定额度计划并得到用户明确授权；不得随着任务 20 自动开始。

**校准矩阵：**

- 至少 4 个可见模型/档位组合；
- 每个组合对每道题至少 3 次；
- 总计至少 60 次任务执行；
- 串行运行，分日期保存；
- Claude 没有可用账号时只完成 Codex 组合，题包保持 candidate，不虚构 Claude 校准。

**发布门槛：**

- 标准答案 >95；
- mutant 覆盖低/中/高；
- 原始能力分跨度 >=20；
- 没有普遍满分或普遍失败；
- 基础设施失败率 <5%；
- 中位总时长 30～60 分钟；
- 同一补丁复评分完全一致。

未满足任一门槛时，调整题目必须发布新的 candidate 版本并重新校准，不能只修改显示比例。全部通过后才将 registry 状态从 candidate 改为 official，并提交校准摘要（不含用户私密数据）。

---

## 后续阶段的进入条件

### 标准题包（约 12 题 + Python）

只有快速题包完成正式校准、至少积累一个稳定基线、用户确认 30～60 分钟体验可接受后开始。

### 完整题包（约 24 题 + JS/TS/Python/Go/Rust）

只有标准题包证明跨语言验证器稳定，并实现多次本地会话的安全暂停/恢复后开始。Windows 必须继续为一级支持平台。

### 可选公共生态

不在本计划授权范围内。任何上传、排行榜或匿名聚合都必须重新进行隐私、反作弊和费用设计评审。

## 最终完成定义

- 任务 1～19 全部通过且每项有独立提交；
- 任务 20 经用户授权后完成一次真实 Codex 单题验收；
- 任务 21 未获额度授权时允许保持 candidate，但界面和文档必须明确未完成正式校准；
- 没有真实 Claude 账号时，Claude 只宣称“契约已验证”，不能宣称“真实运行已验证”；
- 工作树干净，全仓测试通过，设计文档、实施计划、代码、题包 hash 和报告说明一致。
