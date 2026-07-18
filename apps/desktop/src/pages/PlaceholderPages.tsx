import { Link, useLocation } from "react-router-dom";

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
  const location = useLocation();
  const navigationState = location.state as
    | { manualRunCompleted?: boolean }
    | null;
  return (
    <main className="page placeholder-page">
      <p className="eyebrow">本地结果</p>
      <h1>体检结果</h1>
      {navigationState?.manualRunCompleted ? (
        <p role="status">全部任务已完成，结果已保存到本机。</p>
      ) : null}
      <p>正在读取这次体检的本地结果。</p>
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
