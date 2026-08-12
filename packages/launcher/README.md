# AI 能力雷达 npm 启动器

这是 AI 能力雷达 Windows 便携版的轻量启动器。npm 包本身不包含桌面程序；它只下载、验证、缓存并启动与 npm 包版本严格对应的 GitHub Release 资产。

正式启动功能仍在接入中。当前包结构只开放帮助和版本命令，尚未发布到 npm。

## 计划中的使用方式

```powershell
npx ai-ability-radar@0.2.2
```

- 初版只支持 Windows 10/11 x64；
- 需要 Node.js 22.22 或更新的 Node 22，或者 Node.js 24 LTS；
- 首次运行需要联网，后续可使用经过验证的本地缓存；
- 不要求管理员权限，不安装服务，不接收登录凭据；
- 原始测试结果和模型回答仍只保存在本机；
- 下载的 Windows 程序尚未签名，可能触发 SmartScreen 提示；
- 真实能力测试消耗运行者自己的 Codex/Claude 订阅额度，启动器不会替用户承担费用。

项目主页：https://yuleo926.github.io/ai-ability-radar/

源码与问题反馈：https://github.com/YuLeo926/ai-ability-radar
