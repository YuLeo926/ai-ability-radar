import { Link, useParams } from "react-router-dom";

const labels: Record<string, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

function useSelectedTarget(fallback: string): string {
  const { target } = useParams();
  return (target && labels[target]) || fallback;
}

export function ManualRunPage() {
  const target = useSelectedTarget("客户端");
  return (
    <main className="page placeholder-page">
      <p className="eyebrow">手动快速体检</p>
      <h1>{target}体检</h1>
      <p>当前版本使用逐题复制与粘贴流程。</p>
      <Link to="/">返回开始页</Link>
    </main>
  );
}

export function CliRunPage() {
  const target = useSelectedTarget("CLI");
  return (
    <main className="page placeholder-page">
      <p className="eyebrow">自动快速体检</p>
      <h1>{target} 体检</h1>
      <p>自动运行流程将在后续任务中接入。</p>
      <Link to="/">返回开始页</Link>
    </main>
  );
}

export function HistoryPage() {
  return (
    <main className="page placeholder-page">
      <p className="eyebrow">仅保存在本机</p>
      <h1>历史记录</h1>
      <p>可比结果与分类分数将在后续任务中接入。</p>
      <Link to="/">开始新的体检</Link>
    </main>
  );
}

export function ResultPage() {
  const { runId } = useParams();
  return (
    <main className="page placeholder-page">
      <p className="eyebrow">本地结果</p>
      <h1>体检结果</h1>
      <p>测试编号：{runId}</p>
      <Link to="/history">查看历史记录</Link>
    </main>
  );
}

export function NotFoundPage() {
  return (
    <main className="page placeholder-page">
      <p className="eyebrow">404</p>
      <h1>没有找到这个页面</h1>
      <p>地址可能已失效，或这个功能尚未开放。</p>
      <Link to="/">返回开始页</Link>
    </main>
  );
}
