# Agent 执行证据实施计划

日期：2026-08-11
依据：[Agent 执行证据与验证状态分离设计](../specs/2026-08-11-agent-execution-evidence-design.md)

## 实施边界

- 只为 Codex CLI / Claude Code 自动运行增加 Agent 证据；手动客户端体检保持原状。
- 自动化测试全部使用假 Provider，不调用真实 Codex 或 Claude，不消耗订阅额度。
- 验证器仍是任务结论和分数的唯一来源；Agent 完成状态只用于诊断。
- 私密正文只写入本地 artifact，摘要进入 SQLite，公开报告只含聚合状态和计数。
- 保持 Windows 可用，并保留现有旧数据库、旧报告、批次恢复和原始数据清理语义。

## 任务 1：Promptfoo Agent v2 协议

文件：

- `tools/promptfoo-runner/protocol.mjs`
- `tools/promptfoo-runner/provider-config.mjs`
- `tools/promptfoo-runner/run.mjs`
- `tools/promptfoo-runner/README.md`
- `tools/promptfoo-runner/tests/*.test.mjs`

步骤：

1. 先增加失败测试，固定 `promptfoo-agent-v2` 的精确字段集合、大小限制和成功/错误互斥规则。
2. 将 Codex `raw.items` 映射为：命令成功/失败/未知计数、受限退出码分布、文件变更事件数、脱敏工具错误摘要。
3. 使用 SDK `finalResponse` 对应的 `result.output` 作为最终回答；保存前执行凭据、绝对路径、控制字符和长度脱敏。
4. 不保存命令正文、聚合输出、补丁、文件路径和完整 session ID，只保留 `session_present`。
5. Claude 只使用固定 Provider 明确暴露的元数据；不可证明的计数保持未知。
6. 补充恶意对象、超长文本、URL 与本地路径区分、错误摘要截断、未知 item 类型测试。
7. 运行 `npm run test:promptfoo-runner`。

## 任务 2：Rust 严格解析和领域模型

文件：

- `crates/ability-adapters/src/lib.rs`
- `crates/ability-adapters/src/promptfoo.rs`
- `crates/ability-adapters/tests/promptfoo_adapter.rs`
- `crates/ability-core/src/domain.rs`
- `crates/ability-core/src/lib.rs`

步骤：

1. 先增加 v2 正常、矛盾、越界、未知字段和 Provider 错误测试。
2. 扩展 `AgentExecutionEvidence`，承载命令摘要、退出码、工具错误摘要、文件变更数和 `session_present`。
3. Rust 二次检查所有数量、退出码、数组长度、文本长度、控制字符与模型证据一致性。
4. 在核心领域层增加 `AgentExecutionStatus`、数据库安全摘要和本地详情封装。
5. 明确映射：成功响应为 `completed`；Provider/Runner 错误为 `provider_error`；固定预算耗尽为 `timed_out`；取消为 `cancelled`。

## 任务 3：SQLite 原子检查点与恢复

文件：

- `crates/ability-core/migrations/0004_agent_execution_evidence.sql`
- `crates/ability-core/src/storage.rs`
- `crates/ability-core/tests/*`
- `crates/ability-adapters/src/cli_run.rs`
- `crates/ability-adapters/tests/cli_run.rs`

步骤：

1. 新增 `task_agent_evidence` 表，以 `(run_id, task_id)` 关联 `task_results`，正文相关字段不进入表。
2. 增加摘要读取、单次运行原子保存、批次运行原子保存和清理路径置空 API。
3. CLI 每个新终态任务都生成摘要；只有成功且存在结构化证据时才生成私密详情文件。
4. 将详情原子写入 `runs/<run-id>/evidence/<task-id>.json`；数据库提交失败时删除本次新文件。
5. 扩展恢复检查：新检查点必须同时具有任务结果和 Agent 摘要；旧记录缺少摘要仍可读取。
6. 为写文件失败、事务失败、重试标记、批次隔离、旧库迁移和重复任务补充测试。

## 任务 4：artifact 安全策略与保留期

文件：

- `crates/ability-core/src/artifact_store.rs`
- `crates/ability-core/tests/artifact_deletion.rs`
- `apps/desktop/src-tauri/src/data_management.rs`
- `apps/desktop/src-tauri/src/data_management_tests.rs`

步骤：

1. 将 `evidence/*.json` 纳入 CLI artifact 白名单、备份、删除和恢复布局。
2. 本地读取只接受数据库中已验证的规范相对路径，并拒绝绝对路径、父目录、重解析点、符号链接和非 JSON 文件。
3. 原始数据删除/到期清理同时移除详情文件并将 `evidence_rel_path` 置空，保留摘要。
4. 增加逃逸路径、链接替换、缺失文件、清理后读取和完整备份测试。

## 任务 5：桌面命令、DTO 与运行时校验

文件：

- `apps/desktop/src-tauri/src/dto.rs`
- `apps/desktop/src-tauri/src/commands.rs`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src/api/backend.ts`
- `apps/desktop/src/api/tauriBackend.ts`
- `apps/desktop/src/api/runtimeValidation.ts`
- 对应 Rust / Vitest 测试

步骤：

1. `RunDetailDto` 增加每题非敏感 Agent 摘要，不返回正文。
2. 新增按 `runId + taskId` 读取本地诊断详情的命令；后端从数据库取得路径，调用 artifact 安全读取。
3. DTO 和 TypeScript 运行时校验拒绝未知枚举、非整数计数、超长正文、不一致状态和路径泄漏。
4. 缺失、过期、损坏详情返回稳定可显示状态，不把本机路径放入错误消息。

## 任务 6：结果页双状态诊断卡

文件：

- `apps/desktop/src/pages/ResultPage.tsx`
- `apps/desktop/src/pages/ResultPage.test.tsx`
- `apps/desktop/src/pages/ResultsHistory.css`
- `apps/desktop/src/test/accessibility.test.tsx`

视觉方向：沿用现有“本地证据档案”式编辑排版，以一条清晰的双轨状态带区分 `Agent 执行` 与 `任务验证`；详情像封存的运行记录，默认折叠，避免再增加大面积仪表盘噪音。

步骤：

1. CLI 每题卡增加两个独立状态；验证结论继续使用最醒目的颜色和措辞。
2. 增加默认折叠的“本地诊断详情”，首次展开时按需加载。
3. 展示脱敏最终回答、命令状态计数、退出码分布、工具错误摘要、文件修改事件、Token 和模型证据。
4. 对过期、损坏、无证据和旧版本分别显示稳定说明。
5. 补充键盘、焦点、窄屏、200% 缩放和 axe 测试；不改变手动客户端结果页。

## 任务 7：公开报告安全摘要

文件：

- `crates/ability-core/src/report.rs`
- `crates/ability-core/tests/report_contract.rs`
- `schemas/public-report.schema.json`
- `apps/desktop/src/domain/modelProvenance.ts`
- `scripts/validate-repository.mjs`

步骤：

1. 将单次公开报告升级为 schema v3，新增 Agent 状态聚合、命令状态计数、工具错误数和文件修改事件数。
2. 不加入最终回答、错误正文、命令、输出、session、artifact 路径或用户标识。
3. 更新 HTML 模板、JSON Schema、敏感文本扫描和前端导出说明。
4. 保持历史 v2 报告文件可独立打开；当前代码只生成并严格验证 v3。

## 任务 8：全量验证与提交

顺序：

1. `npm run test:promptfoo-runner`
2. `cargo fmt --all -- --check`
3. `cargo test --workspace --locked --offline`
4. `npm test`
5. `npm run build`
6. `npm run validate:repository`
7. `git diff --check`

验收门槛：

- 假 Provider 可复现“Agent 已结束、文件修改为 0、验证失败”，结果页同时显示两个事实。
- 所有自动测试期间真实模型调用数为 0。
- 工作区、数据库、公开报告和错误消息均通过隐私回归测试。
- 全部检查通过后提交实现，并提供本地启动与验收说明。
