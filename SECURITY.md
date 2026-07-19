# 安全政策

## 私密报告漏洞

请打开本仓库 GitHub 页面的 **Security → Advisories → Report a vulnerability**，通过私密
GitHub Security Advisory 的 **Report a vulnerability** 入口报告。不要在公开 issue、讨论或 Pull Request 中披露可利用细节、
原始日志、令牌、凭据、个人路径或未修复的复现数据。

报告可包含受影响的应用版本、Windows 版本、脱敏后的复现步骤、影响范围，以及不含私密数据的
最小概念验证。仓库没有公布专用安全邮箱，因此请不要猜测或向非官方地址发送材料。

## 当前支持范围

安全修复优先覆盖最新的 v0.2 Windows 预览分支。预览安装程序未签名，项目没有自动更新器；
用户应从仓库 Releases 获取安装程序并核对 SHA-256。关于运行时隔离、数据删除与提供商流量的
边界见[安全说明](docs/security.md)和[隐私说明](docs/privacy.md)。
