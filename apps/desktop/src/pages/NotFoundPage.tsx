import { Link } from "react-router-dom";

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
