# v0.2 安全说明

## 构建与更新

v0.2 Windows 预览安装程序未签名，SmartScreen 可能显示“未知发布者”。发布工作流只创建 draft
prerelease，并附带 SHA-256；校验值不能替代可信代码签名。Tauri updater 插件、更新配置、更新清单
与更新签名均不存在，应用不会自动下载或执行更新。

GitHub 工作流的第三方 action 固定到完整 commit SHA，checkout 不保留凭据，job 使用最小权限和
超时。普通 CI 只使用 fake CLI，不安装、登录或运行真实 AI CLI，也不定义提供商凭据。

## v0.2 进程隔离

每个 CLI 题目复制到独立任务目录。Codex 以 `workspace-write` 运行；Claude 只允许
Read/Edit/Write 工具，并使用 `dontAsk` 非交互权限模式。题包还限定固定时间与 turn 上限，Node
验证器只接受允许列表中的 verifier ID，并在受控环境中检查结果。

这些控制降低提示词或 agent 意外访问题目目录之外内容的概率，但不是容器、VM（虚拟机）或
malicious-code sandbox（恶意代码沙箱）。不要把秘密或真实仓库放进 v0.2 临时任务。更强 WSL/容器
隔离是 v0.5 计划中真实仓库与 DeepSWE 的门禁，不是当前能力。

## 数据、网络与凭据

应用没有遥测或上传 endpoint（端点），也不采集登录凭据。真实测试仍必须把提示词和临时基准代码
发给所选 AI 提供商；被调用的 CLI 和提供商可以实施自己的日志、内容保留与遥测政策。只在理解并
接受对应服务政策的账号上运行。

本地历史含原始回答和 CLI 日志；公开报告使用允许列表，但完整备份含原文且不加密。不要把备份上传到
不受信任的位置，不要在 issue 中粘贴日志、令牌、用户名、设备名或绝对路径。

## 删除不是安全擦除

应用通过文件系统正常删除 artifacts。SQLite 启用 `secure_delete`，完整运行删除后请求
`wal_checkpoint(TRUNCATE)` 截断 WAL（繁忙连接可能使回收延后）。这不是取证擦除：SSD 行为、
文件系统快照、杀毒软件隔离区和外部备份可能保留可恢复副本。高保证清除需要依赖全盘加密与操作系统、
存储和备份设施的专门流程。

## 报告漏洞

请按仓库根目录的[安全政策](../SECURITY.md)，使用 GitHub Security Advisory 的私密
“Report a vulnerability”入口。不要公开未修复漏洞或敏感复现材料。
