import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import { useT } from "../i18n/I18nContext";
import type {
  AvailabilityStatus,
  Bootstrap,
  LaunchSource,
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

const statusCopy: Record<AvailabilityStatus, string | null> = {
  ready: null,
  needs_login: "需要先在终端登录",
  not_found: "未检测到受支持入口",
  runtime_missing: "缺少 Node.js 运行时",
  entry_inaccessible: "入口不可访问",
  version_probe_failed: "版本检测失败",
};

const sourceCopy: Record<LaunchSource, string> = {
  native_exe: "原生安装",
  reviewed_npm: "npm 安装",
};

const REFRESH_ERROR_COPY = "无法重新检测 CLI，请重试。";

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

function visibleError(reason: unknown, fallback: string): string {
  const value =
    reason instanceof Error ? reason.message : typeof reason === "string" ? reason : "";
  const sanitized = value
    .replace(/[\u0000-\u001f\u007f-\u009f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 200);
  return sanitized || fallback;
}

function blocker(target: TargetAvailability): string | null {
  if (isCli(target.kind)) {
    const status = statusCopy[target.status];
    if (status) return status;
  }
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
  const source =
    isCli(target.kind) &&
    target.source &&
    (target.status === "ready" || target.status === "needs_login")
      ? sourceCopy[target.source]
      : null;
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
      {source ? (
        <p className="target-source">
          入口来源：<span>{source}</span>
        </p>
      ) : null}
      <p
        aria-label={statusLabel}
        className={
          reason
            ? "target-status status-warning"
            : "target-status status-ready"
        }
        role="status"
      >
        <span aria-hidden="true" className="status-indicator" />
        <span>{status}</span>
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
  guidance,
}: {
  title: string;
  description: string;
  targets: TargetAvailability[];
  pack: PackSummary;
  id: string;
  action?: ReactNode;
  guidance?: ReactNode;
}) {
  const titleId = `${id}-title`;

  return (
    <section aria-labelledby={titleId} className="target-section">
      <header className="section-heading">
        <div className="section-heading-copy">
          <p className="section-kicker">{description}</p>
          <h2 id={titleId}>{title}</h2>
          <div className="section-pack-meta">
            <span>{pack.title} · v{pack.version}</span>
            <span>{pack.taskCount} 道任务 · 预计 {pack.estimatedMinutes} 分钟</span>
          </div>
        </div>
        {action ? <div className="section-action">{action}</div> : null}
      </header>
      {guidance}
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
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null);
  const [initialError, setInitialError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const mounted = useRef(false);
  const requestId = useRef(0);
  const refreshInFlight = useRef(false);

  const loadBootstrap = useCallback(() => {
    const id = ++requestId.current;
    refreshInFlight.current = false;
    setBootstrap(null);
    setInitialError(null);
    setRefreshing(false);
    setRefreshError(null);
    void Promise.resolve()
      .then(() => backend.getBootstrap())
      .then((data) => {
        if (mounted.current && requestId.current === id) {
          setBootstrap(data);
        }
      })
      .catch((reason: unknown) => {
        if (mounted.current && requestId.current === id) {
          setInitialError(visibleError(reason, "无法读取本机环境，请重试。"));
        }
      });
  }, [backend]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      requestId.current += 1;
    };
  }, []);

  useEffect(() => {
    loadBootstrap();
  }, [loadBootstrap]);

  const refreshBootstrap = () => {
    if (!bootstrap || refreshInFlight.current) return;

    const id = ++requestId.current;
    refreshInFlight.current = true;
    setRefreshing(true);
    setRefreshError(null);
    void Promise.resolve()
      .then(() => backend.getBootstrap())
      .then((data) => {
        if (mounted.current && requestId.current === id) {
          setBootstrap(data);
          setRefreshing(false);
          refreshInFlight.current = false;
        }
      })
      .catch((reason: unknown) => {
        if (mounted.current && requestId.current === id) {
          setRefreshError(visibleError(reason, REFRESH_ERROR_COPY));
          setRefreshing(false);
          refreshInFlight.current = false;
        }
      });
  };

  if (!bootstrap && !initialError) {
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

  if (initialError) {
    return (
      <main className="page bootstrap-state" id="page-content" tabIndex={-1}>
        <section aria-labelledby="bootstrap-error-title" role="alert">
          <p className="eyebrow">本地环境检查</p>
          <h1 id="bootstrap-error-title">无法读取本机环境</h1>
          <p>{initialError}</p>
          <button type="button" onClick={loadBootstrap}>
            {t("home.retry")}
          </button>
        </section>
      </main>
    );
  }

  if (!bootstrap) {
    return null;
  }

  const clients = bootstrap.targets.filter((target) => !isCli(target.kind));
  const clis = bootstrap.targets.filter((target) => isCli(target.kind));

  return (
    <main className="page home-page" id="page-content" tabIndex={-1}>
      <section
        aria-labelledby="home-title"
        className="hero home-hero"
        data-testid="home-hero"
      >
        <p className="eyebrow">本地优先 · 结果按目标分别记录</p>
        <h1 id="home-title">选择要体检的 AI</h1>
        <p className="hero-summary">
          客户端逐题复制粘贴，CLI 在专用临时目录自动执行。
        </p>
        <div className="hero-data-strip" aria-label="体检边界">
          <span>原始数据仅存本机</span>
          <span>使用你自己的订阅额度</span>
          <span>衡量端到端产品表现</span>
        </div>
      </section>

      <div className="target-sections">
        <TargetGroup
          description="手动复制与粘贴"
          id="client-targets"
          pack={bootstrap.clientPack}
          targets={clients}
          title="聊天客户端"
        />
        <TargetGroup
          description="本机自动执行"
          id="cli-targets"
          pack={bootstrap.cliPack}
          targets={clis}
          title="编程 CLI"
          action={
            <div className="cli-refresh-control">
              <button
                className="secondary-action"
                disabled={refreshing}
                onClick={refreshBootstrap}
                type="button"
              >
                {refreshing ? "正在重新检测…" : "重新检测 CLI"}
              </button>
              {refreshing ? (
                <span className="sr-only" role="status">
                  正在重新检测 CLI
                </span>
              ) : null}
              {refreshError ? (
                <p className="inline-error" role="alert">
                  {refreshError}
                </p>
              ) : null}
            </div>
          }
          guidance={
            <p className="cli-guidance" data-testid="cli-guidance">
              已继承 PATH 目录内的变化可以立即刷新；安装程序新增 PATH
              目录后请重启应用，再重新检测。
            </p>
          }
        />
      </div>

      <aside aria-labelledby="cost-privacy-title" className="notice">
        <h2 id="cost-privacy-title">费用和隐私说明</h2>
        <p>手动客户端体检和自动 CLI 体检都可能消耗你自己的订阅额度。</p>
        <p>维护者不会承担这些费用，也不会接收你的登录凭据。</p>
        <p>原始回答和运行日志只保存在本机。</p>
        <p>CLI 自动任务使用专用临时任务目录。</p>
        <p>体检衡量端到端产品表现，不直接测量底层模型的“智商”。</p>
      </aside>
    </main>
  );
}
