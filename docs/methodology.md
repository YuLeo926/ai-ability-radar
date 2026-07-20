# v0.2 方法说明

## 测量目标

AI 能力雷达记录所选目标对固定快速题包的**客观通过表现**。它不是 IQ 测试，不推断模型内部状态，
也不能把一次波动归因于模型、路由、服务负载、账号方案或提示上下文中的某一个因素。

v0.2 明确不生成“降智”或退化裁决，也不生成置信度。历史页只在完整环境键相同的系列内展示原始
客观结果；用户仍需自行解释变化。

## v0.2 题包

- `client-quick` v1.0.0：面向 ChatGPT 与 Claude 客户端，共 8 道题。3 道指令遵循、3 道逻辑、
  2 道代码审查。
- `cli-quick` v1.0.0：面向 Codex CLI 与 Claude Code，共 2 个 JavaScript 微型项目。
  验证器规则标识为 `dedupe-events-v1` 与 `retry-schedule-v1`。

这些 v0.2 提示词和微型仓库是原创第一方快速检查，不复制 Codex Radar，也不复制或包含 DeepSWE
任务。固定任务一旦开源，长期可能被训练数据或人工记忆污染，因此它们只适合快速回归记录，
不能代表未见真实软件工程任务的能力。

## 客观评分与类别等权

客户端题使用精确文本、精确 JSON 或 JSON 字符串集合判分；CLI 题由固定的本地允许列表验证器判分。
每题先得到通过/未通过或基础设施无效状态，再在各类别内求平均，最终对有效类别做类别等权平均。
因此题数更多的类别不会自动获得更高权重。CLI 快速题包目前只有 `cli_coding` 一个类别。

基础设施无效（例如 CLI 缺失、登录无效、Node 版本错误、网络或验证器故障）不会计入能力分母。
agent-budget failure（超时、达到 turn 上限或受控输出预算耗尽）表示 agent 在既定预算内没有完成，
按任务失败计分；两者不能互换。

## 模型、推理档位与运行模式

CLI 模型字段为空白（输入留空）表示使用该 CLI 的默认路由，历史中规范化存为 `default`。显式提供模型时，模型名会
传给所选 CLI；选择推理档位时也会显式传递。模型、推理档位与 quick 运行模式都是历史键的一部分，
不能跨键拼接趋势。

客户端的“报告模型”由用户填写并去除首尾空白；它是标签，不是应用对服务端模型身份的验证。
该标签去除首尾空白后须为 1–120 个字符，并拒绝 Unicode `Cc`、`Cf`、
`Default_Ignorable_Code_Point` 与双向格式控制字符。前端启动、后端
启动/恢复和公开 JSON/HTML 报告使用同一规则。旧历史若含无效标签，界面
只显示稳定的“模型名称不可显示”占位符，不修改原始存储记录。

当前推理档位矩阵如下。实际可用档位仍取决于模型、客户端版本和账户权限；“自定义”只记录经过
校验的用户输入，不代表应用验证了服务端支持。

| 目标 | 可记录的预设 |
| --- | --- |
| ChatGPT 客户端 | 未显示/不适用、轻度、中、高、极高、最高、Ultra，以及“其他/按界面原样填写” |
| Claude 客户端 | 未显示/不适用、低、中、高、极高、最高，以及“其他/按界面原样填写” |
| Codex CLI | CLI 默认、`minimal`、`low`、`medium`、`high`、`xhigh`、`max`、`ultra`，以及自定义 |
| Claude Code | CLI 默认、`low`、`medium`、`high`、`xhigh`、`max`，以及自定义 |

规范值的中文标签由 `schemas/reasoning-effort-display.json` 中一份显式的
4 个目标 × 8 个值策略提供。策略允许目标覆盖，例如 ChatGPT 的 `low`
显示为“轻度”，其他目标的 `low` 显示为“低”；同一目标/值在设置、恢复、
历史、结果和导出 HTML 中必须一致，JSON 与数据库中的规范值保持不变。

ChatGPT/Codex 的 `none` 只能作为自定义值记录；Claude Code 的 `ultracode` 是组合运行模式，
不是额外的 effort 值。已有 `low`、`medium`、`high` 历史保持兼容。

推理档位写入历史、恢复运行核对和同条件比较时使用同一组规范化规则，沿用现有可选字符串字段，
不需要数据库迁移：已知推理值始终保持小写规范字符串；手动客户端自定义标签保留去除首尾空白后的
显示文本；CLI 自定义值规范化为小写安全 token（仅 ASCII 字母、数字、下划线或连字符）。
因此已有 `low`、`medium`、`high` 历史仍可读取、恢复和比较。

## 时长

客户端辅助时长包含人工复制题目、等待回答和粘贴原文的区间。CLI 时长包含受控进程与本地验证器
时间。两个轨道的人机组成完全不同，因此时长不做跨轨道比较；不同历史键之间也不比较时长。

## 可复现历史键

精确历史系列键按以下字段组成，任何字段变化都会建立独立系列：

`target kind, trimmed reported model, reasoning effort, run mode, suite ID/version/hash, scoring-rule version, OS family/version, app version, CLI version, Node verifier version, and clean-versus-resumed state`

其中 suite hash 是题包全部受控内容的 SHA-256；clean-versus-resumed state 防止恢复运行与全新运行
混在一起。CLI 版本和 Node verifier 版本只在适用时存在。

## 规则与格式版本

- 评分规则（scoring rule）：`ability-v1`
- pack schema：`1`
- bundled pack registry schema：`1`
- `client-quick` 与 `cli-quick` 题包版本：`1.0.0`
- public report schema：`1`
- full backup schema：`1`
- CLI 验证器规则：`dedupe-events-v1`、`retry-schedule-v1`

这些字符串进入持久化环境、报告或题包哈希。改变含义时必须发布新版本，不能静默复用旧字符串。

## 便携包的运行时解析一致性

便携包仍保留 Node 侧的精确 schema、引用关系、文件集合与内容哈希检查，
并额外构建第一方 `ability-pack-validator`。该 helper 直接复用运行时
`PackRegistry::parse` 与 `PackLoader::load`，在 source、staged、
pre-compression 和 extracted 四个检查点解析题包，因此 BOM、重复键、
整数的浮点/指数写法、溢出与非有限数等输入会按 Rust/Serde 运行时规则
失败。helper 只参与构建，不复制进应用或便携包 payload。

代价是 `package:portable:from-build` 会多执行一次离线 Rust release helper
构建，并在打包期间启动四次短生命周期的本地 helper 进程；它不会调用
模型服务或增加运行应用时的常驻成本。压缩后、解压前还会解析原始 ZIP
central directory，先验证 Windows 路径安全、规范化碰撞及精确文件成员
集合，再允许提取和发布。

## v0.5 计划边界

v0.5 计划引入经过审查的 baseline/calibration、最小样本规则和更强的 WSL/容器隔离门禁，再讨论
真实仓库或 DeepSWE。它们都不是 v0.2 已交付能力；在校准设计、再分发审查与隔离门禁完成之前，
项目不会声称提供统计退化判断。
