import { Link } from "react-router-dom";
import { useT } from "../i18n/I18nContext";

export function NotFoundPage() {
  const t = useT();

  return (
    <main className="page placeholder-page" id="page-content" tabIndex={-1}>
      <p className="eyebrow">404</p>
      <h1>{t("notFound.title")}</h1>
      <p>地址可能已失效，或这个功能尚未开放。</p>
      <Link to="/">{t("common.backHome")}</Link>
    </main>
  );
}
