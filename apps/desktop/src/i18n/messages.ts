export const messages = {
  "app.name": "AI 能力雷达",
  "nav.label": "主导航",
  "nav.start": "开始体检",
  "nav.history": "历史记录",
  "skip.main": "跳到主要内容",
  "theme.label": "配色主题",
  "theme.system": "跟随系统",
  "theme.light": "浅色",
  "theme.dark": "深色",
  "common.loading": "正在读取本机数据…",
  "common.retry": "重试",
  "common.reload": "重新读取",
  "common.cancel": "取消",
  "common.backHome": "返回开始页",
  "common.backHistory": "返回历史记录",
  "home.loading": "正在检查本机环境…",
  "home.retry": "重新检查",
  "manual.loadingFirst": "正在读取第一题…",
  "cli.loadingEnvironment": "正在检查本机环境…",
  "history.loading": "正在读取本地历史…",
  "result.loading": "正在读取本地结果…",
  "notFound.title": "没有找到这个页面",
  "result.boundary":
    "v0.2 只展示本题包的客观结果，不生成降智结论。",
} as const;

export type MessageKey = keyof typeof messages;
export type Translator = (key: MessageKey) => (typeof messages)[MessageKey];

export function translate(key: MessageKey): (typeof messages)[MessageKey] {
  return messages[key];
}
