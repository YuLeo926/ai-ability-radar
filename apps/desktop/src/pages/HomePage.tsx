import { useEffect, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import { useT } from "../i18n/I18nContext";
import type {
  Bootstrap,
  PackSummary,
  TargetAvailability,
  TargetKind,
} from "../api/backend";

const targetLabels: Record<TargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

type BootstrapState =
  | { kind: "loading" }
  | { kind: "ready"; data: Bootstrap }
  | { kind: "error"; message: string };

function isCli(kind: TargetKind): boolean {
  return kind === "codex_cli" || kind === "claude_code";
}

function visibleVersion(value: string | null | undefined): string | null {
  const sanitized = value
    ?.replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 120);
  return sanitized || null;
}

function blocker(target: TargetAvailability): string | null {
  const missing = target.prerequisites.find(
    (prerequisite) => !prerequisite.available,
  );
  if (missing) return `缺少 ${missing.name}`;
  if (!target.installed) return "未检测到可执行入口";
  if (target.authState === "needs_login") {
    return isCli(target.kind) ? "需要先在终端登录" : "需要先登录";
  }
  return null;
}

function availableStatus(target: TargetAvailability): string {
  return isCli(target.kind) ? "本机环境检查通过" : "可开始手动体检";
}

function TargetCard({
  target,
  pack,
}: {
  target: TargetAvailability;
  pack: PackSummary;
}) {
  const label = targetLabels[target.kind];
  const reason = blocker(target);
  const status = reason ?? availableStatus(target);
  const version = visibleVersion(target.version);
  const destination = isCli(target.kind) ? "cli" : "manual";
  const actionLabel = isCli(target.kind)
    ? `开始 ${label} 自动体检`
    : `开始 ${label}手动体检`;
  const statusLabel = isCli(target.kind)
    ? `${label} 状态：${status}`
    : `${label}状态：${status}`;
  const headingId = `target-${target.kind}`;

  return (
    <article aria-labelledby={headingId} className="target-card">
      <div className="target-card-header">
        <h3 id={headingId}>{label}</h3>
        <span className="target-kind">
          {isCli(target.kind) ? "自动 CLI" : "客户端"}
        </span>
      </div>
      {version ? <p className="target-version">版本：{version}</p> : null}
      <p
        aria-label={statusLabel}
        className={reason ? "target-status status-warning" : "target-status status-ready"}
        role="status"
      >
        {status}
      </p>
      {reason ? (
        <button
          aria-label={`${label} 暂时无法开始`}
          className="target-action target-action-disabled"
          disabled
          type="button"
        >
          暂时无法开始
        </button>
      ) : (
        <Link
          aria-label={actionLabel}
          className="target-action"
          to={`/${destination}/${target.kind}`}
        >
          {actionLabel}
        </Link>
      )}
      <p className="target-pack-reference">
        使用 {pack.title} v{pack.version}
      </p>
    </article>
  );
}

function TargetGroup({
  title,
  description,
  targets,
  pack,
  id,
  action,
}: {
  title: string;
  description: string;
  targets: TargetAvailability[];
  pack: PackSummary;
  id: string;
  action?: ReactNode;
}) {
  const titleId = `${id}-title`;

  return (
    <section aria-labelledby={titleId} className="target-section">
      <header className="section-heading">
        <div>
          <p className="section-kicker">{description}</p>
          <h2 id={titleId}>{title}</h2>
        </div>
        <div className="section-heading-actions">
          <div className="pack-summary">
            <p>{pack.title} · v{pack.version}</p>
            <p>{pack.taskCount} 道任务 · 预计 {pack.estimatedMinutes} 分钟</p>
          </div>
          {action}
        </div>
      </header>
      <div className="target-grid">
        {targets.map((target) => (
          <TargetCard key={target.kind} pack={pack} target={target} />
        ))}
      </div>
    </section>
  );
}

export function HomePage() {
  const backend = useBackend();
  const t = useT();
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<BootstrapState>({ kind: "loading" });

  useEffect(() => {
    let current = true;
    setState({ kind: "loading" });
    void Promise.resolve()
      .then(() => backend.getBootstrap())
      .then((data) => {
        if (current) {
          setState({ kind: "ready", data });
        }
      })
      .catch((reason: unknown) => {
        if (current) {
          setState({
            kind: "error",
            message: reason instanceof Error ? reason.message : String(reason),
          });
        }
      });
    return () => {
      current = false;
    };
  }, [attempt, backend]);

  if (state.kind === "loading") {
    return (
      <main
        aria-busy="true"
        className="page bootstrap-state"
        id="page-content"
        tabIndex={-1}
      >
        <p aria-label="正在检查本机环境" role="status">
          {t("home.loading")}
        </p>
      </main>
    );
  }

  if (state.kind === "error") {
    return (
      <main className="page bootstrap-state" id="page-content" tabIndex={-1}>
        <section aria-labelledby="bootstrap-error-title" role="alert">
          <p className="eyebrow">本地环境检查</p>
          <h1 id="bootstrap-error-title">无法读取本机环境</h1>
          <p>{state.message}</p>
          <button type="button" onClick={() => setAttempt((value) => value + 1)}>
            {t("home.retry")}
          </button>
        </section>
      </main>
    );
  }

  const clients = state.data.targets.filter((target) => !isCli(target.kind));
  const clis = state.data.targets.filter((target) => isCli(target.kind));

  return (
    <main className="page home-page" id="page-content" tabIndex={-1}>
      <section className="hero" aria-labelledby="home-title">
        <p className="eyebrow">本地优先 · 四条结果序列分别记录</p>
        <h1 id="home-title">选择要体检的 AI</h1>
        <p className="hero-summary">
          客户端采用逐题复制粘贴。
        </p>
        <p>CLI 自动任务使用专用临时任务目录。</p>
        <p>体检衡量端到端产品表现，不直接测量底层模型的“智商”。</p>
      </section>

      <div className="target-sections">
        <TargetGroup
          description="手动复制与粘贴"
          id="client-targets"
          pack={state.data.clientPack}
          targets={clients}
          title="聊天客户端"
        />
        <TargetGroup
          description="本机自动执行"
          id="cli-targets"
          pack={state.data.cliPack}
          targets={clis}
          title="编程 CLI"
          action={
            <button
              className="secondary-action"
              onClick={() => setAttempt((value) => value + 1)}
              type="button"
            >
              重新检测 CLI
            </button>
          }
        />
      </div>

      <aside aria-labelledby="cost-privacy-title" className="notice">
        <h2 id="cost-privacy-title">费用和隐私说明</h2>
        <p>手动客户端体检和自动 CLI 体检都可能消耗你自己的订阅额度。</p>
        <p>维护者不会承担这些费用，也不会接收你的登录凭据。</p>
        <p>原始回答和运行日志只保存在本机。</p>
      </aside>
    </main>
  );
}
