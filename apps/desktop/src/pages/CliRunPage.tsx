import {
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import { useT } from "../i18n/I18nContext";
import { isSafeRunRecord } from "../api/runtimeValidation";
import type {
  Bootstrap,
  RunEvent,
  RunRecord,
  TargetAvailability,
  TargetKind,
  TargetSelection,
  Unlisten,
} from "../api/backend";
import "./CliRunPage.css";

type CliTargetKind = Extract<TargetKind, "codex_cli" | "claude_code">;
type ReasoningEffort = "" | "low" | "medium" | "high";
type BootstrapState =
  | { kind: "loading" }
  | { kind: "ready"; value: Bootstrap }
  | { kind: "error" };
type Connectivity = "connecting" | "live" | "fallback";
type SyncState = "ready" | "missing" | "failed";
type StopState = "idle" | "not-found" | "waiting";
type ResumePreviewState =
  | { kind: "loading" }
  | { kind: "ready"; run: RunRecord }
  | { kind: "error"; message: string };

const POLL_INTERVAL_MS = 2_000;
const MODEL_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:/-]*$/;
const CONTROL_CHARACTER = /\p{Cc}/u;
const cliLabels: Record<CliTargetKind, string> = {
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

function isCliTarget(value: string): value is CliTargetKind {
  return value === "codex_cli" || value === "claude_code";
}

function modelValidationError(value: string): string | null {
  if (CONTROL_CHARACTER.test(value)) {
    return "模型名称格式不正确";
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }
  if (Array.from(trimmed).length > 120 || !MODEL_PATTERN.test(trimmed)) {
    return "模型名称格式不正确";
  }
  return null;
}

function isTerminal(status: RunRecord["status"]): boolean {
  return (
    status === "completed" ||
    status === "cancelled" ||
    status === "interrupted"
  );
}

function sameTarget(left: TargetSelection, right: TargetSelection): boolean {
  return (
    left.kind === right.kind &&
    left.reportedModel === right.reportedModel &&
    (left.reasoningEffort ?? null) === (right.reasoningEffort ?? null)
  );
}

function reasoningEffortLabel(value?: string | null): string {
  if (value === "low") return "低";
  if (value === "medium") return "中";
  if (value === "high") return "高";
  return "CLI 默认";
}

function releaseUnlisten(unlisten: Unlisten): void {
  try {
    void Promise.resolve(unlisten()).catch(() => undefined);
  } catch {
    // Listener cleanup is best effort after the owning route is gone.
  }
}

function safeCount(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0;
}

function formatElapsed(startedAt: string, now: number): string {
  const started = Date.parse(startedAt);
  if (!Number.isFinite(started)) {
    return "已用时正在计算";
  }
  const elapsedSeconds = Math.max(0, Math.floor((now - started) / 1_000));
  const minutes = Math.floor(elapsedSeconds / 60);
  const seconds = elapsedSeconds % 60;
  return minutes > 0
    ? `已用时 ${minutes} 分 ${seconds} 秒`
    : `已用时 ${seconds} 秒`;
}

function availabilityBlocker(
  label: string,
  availability: TargetAvailability | undefined,
): string | null {
  if (!availability) {
    return `没有取得 ${label} 的环境状态，暂时无法开始。`;
  }
  if (!availability.installed) {
    return `未检测到 ${label}，暂时无法开始。`;
  }
  if (availability.authState === "needs_login") {
    return `需要先在终端完成 ${label} 登录。`;
  }
  const missing = availability.prerequisites.find(
    (prerequisite) => !prerequisite.available,
  );
  return missing ? `缺少 ${missing.name}，暂时无法开始。` : null;
}

function UnsupportedCliPage() {
  const t = useT();

  return (
    <main
      className="page placeholder-page unsupported-cli-page"
      id="page-content"
      tabIndex={-1}
    >
      <p className="eyebrow">不支持的地址</p>
      <h1>不支持的 CLI 体检</h1>
      <p>这个地址不是 Codex CLI 或 Claude Code 体检。</p>
      <Link to="/">{t("common.backHome")}</Link>
    </main>
  );
}

function CliWizard({
  kind,
  resumeRunId,
}: {
  kind: CliTargetKind;
  resumeRunId?: string;
}) {
  const backend = useBackend();
  const navigate = useNavigate();
  const label = cliLabels[kind];
  const mounted = useRef(true);
  const activeRunId = useRef<string | null>(null);
  const navigated = useRef(false);
  const startPending = useRef(false);
  const cancelPending = useRef(false);
  const pollInFlight = useRef(false);
  const pollQueued = useRef(false);
  const pollTimer = useRef<number | null>(null);
  const pollNowRef = useRef<() => void>(() => undefined);
  const listenerUnavailable = useRef(false);
  const progressRef = useRef({ completed: 0, total: 0 });

  const [bootstrapAttempt, setBootstrapAttempt] = useState(0);
  const [bootstrapState, setBootstrapState] = useState<BootstrapState>({
    kind: "loading",
  });
  const [connectivity, setConnectivity] =
    useState<Connectivity>("connecting");
  const [syncState, setSyncState] = useState<SyncState>("ready");
  const [model, setModel] = useState("");
  const [modelTouched, setModelTouched] = useState(false);
  const [reasoningEffort, setReasoningEffort] =
    useState<ReasoningEffort>("");
  const [acceptedCost, setAcceptedCost] = useState(false);
  const [starting, setStarting] = useState(false);
  const [setupError, setSetupError] = useState("");
  const [run, setRun] = useState<RunRecord | null>(null);
  const [progress, setProgress] = useState({ completed: 0, total: 0 });
  const [now, setNow] = useState(() => Date.now());
  const [runAlert, setRunAlert] = useState("");
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState("");
  const [stopState, setStopState] = useState<StopState>("idle");
  const [resumePreview, setResumePreview] =
    useState<ResumePreviewState | null>(
      resumeRunId ? { kind: "loading" } : null,
    );

  const clearPollTimer = useCallback(() => {
    if (pollTimer.current !== null) {
      window.clearTimeout(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      activeRunId.current = null;
      pollQueued.current = false;
      clearPollTimer();
    };
  }, [clearPollTimer]);

  const navigateToResult = useCallback(
    (runId: string) => {
      if (
        !mounted.current ||
        navigated.current ||
        activeRunId.current !== runId
      ) {
        return;
      }
      navigated.current = true;
      clearPollTimer();
      activeRunId.current = null;
      navigate(`/results/${runId}`, {
        replace: true,
        state: { cliRunCompleted: true },
      });
    },
    [clearPollTimer, navigate],
  );

  const reconcileProgress = useCallback(
    (completedValue: number, totalValue: number) => {
      const completed = safeCount(completedValue);
      const total = safeCount(totalValue);
      setProgress((current) => {
        const nextCompleted = Math.max(current.completed, completed);
        const nextTotal = Math.max(current.total, total, nextCompleted);
        const next = {
          completed: nextCompleted,
          total: nextTotal,
        };
        progressRef.current = next;
        return next;
      });
    },
    [],
  );

  const schedulePoll = useCallback(() => {
    clearPollTimer();
    if (
      !mounted.current ||
      navigated.current ||
      !activeRunId.current
    ) {
      return;
    }
    pollTimer.current = window.setTimeout(() => {
      pollTimer.current = null;
      pollNowRef.current();
    }, POLL_INTERVAL_MS);
  }, [clearPollTimer]);

  const pollNow = useCallback(() => {
    clearPollTimer();
    const runId = activeRunId.current;
    if (!mounted.current || navigated.current || !runId) {
      return;
    }
    if (pollInFlight.current) {
      pollQueued.current = true;
      return;
    }

    pollInFlight.current = true;
    void Promise.resolve()
      .then(() => backend.getRunDetail(runId))
      .then((detail) => {
        if (
          !mounted.current ||
          navigated.current ||
          activeRunId.current !== runId
        ) {
          return;
        }
        if (!detail) {
          setSyncState("missing");
          return;
        }

        setRun(detail.run);
        reconcileProgress(
          detail.run.completedTasks,
          detail.run.totalTasks,
        );
        setSyncState("ready");
        if (isTerminal(detail.run.status)) {
          navigateToResult(runId);
        }
      })
      .catch(() => {
        if (
          mounted.current &&
          !navigated.current &&
          activeRunId.current === runId
        ) {
          setSyncState("failed");
        }
      })
      .finally(() => {
        pollInFlight.current = false;
        if (
          !mounted.current ||
          navigated.current ||
          activeRunId.current !== runId
        ) {
          pollQueued.current = false;
          return;
        }
        if (pollQueued.current) {
          pollQueued.current = false;
          queueMicrotask(() => pollNowRef.current());
        } else {
          schedulePoll();
        }
      });
  }, [
    backend,
    clearPollTimer,
    navigateToResult,
    reconcileProgress,
    schedulePoll,
  ]);
  pollNowRef.current = pollNow;

  useEffect(() => {
    let disposed = false;
    setBootstrapState({ kind: "loading" });
    void Promise.resolve()
      .then(() => backend.getBootstrap())
      .then((bootstrap) => {
        if (!disposed && mounted.current) {
          setBootstrapState({ kind: "ready", value: bootstrap });
        }
      })
      .catch(() => {
        if (!disposed && mounted.current) {
          setBootstrapState({ kind: "error" });
        }
      });
    return () => {
      disposed = true;
    };
  }, [backend, bootstrapAttempt]);

  useEffect(() => {
    if (!resumeRunId) {
      setResumePreview(null);
      return;
    }
    let disposed = false;
    setResumePreview({ kind: "loading" });
    void Promise.resolve()
      .then(() => backend.getRunDetail(resumeRunId))
      .then((detail) => {
        if (disposed || !mounted.current) {
          return;
        }
        if (
          !detail ||
          !isSafeRunRecord(detail.run) ||
          detail.run.id !== resumeRunId ||
          detail.run.status !== "interrupted"
        ) {
          setResumePreview({
            kind: "error",
            message: "无法恢复这次 CLI 体检。本地记录无法安全读取。",
          });
          return;
        }
        if (detail.run.target.kind !== kind) {
          setResumePreview({
            kind: "error",
            message: "恢复链接与原体检目标不一致。",
          });
          return;
        }
        setResumePreview({ kind: "ready", run: detail.run });
      })
      .catch(() => {
        if (!disposed && mounted.current) {
          setResumePreview({
            kind: "error",
            message: "无法恢复这次 CLI 体检。本地记录无法安全读取。",
          });
        }
      });
    return () => {
      disposed = true;
    };
  }, [backend, kind, resumeRunId]);

  useEffect(() => {
    let disposed = false;
    let attached = 0;
    const unlisteners: Unlisten[] = [];

    function attach(factory: () => Promise<Unlisten>) {
      void Promise.resolve()
        .then(factory)
        .then((unlisten) => {
          if (disposed) {
            releaseUnlisten(unlisten);
            return;
          }
          unlisteners.push(unlisten);
          attached += 1;
          if (attached === 2 && !listenerUnavailable.current) {
            setConnectivity("live");
          }
        })
        .catch(() => {
          if (!disposed && mounted.current) {
            listenerUnavailable.current = true;
            setConnectivity("fallback");
          }
        });
    }

    attach(() =>
      backend.onRunEvent((event: RunEvent) => {
        if (
          !mounted.current ||
          navigated.current ||
          event.runId !== activeRunId.current
        ) {
          return;
        }
        reconcileProgress(event.completedTasks, event.totalTasks);
        if (!listenerUnavailable.current) {
          setConnectivity("live");
        }
        if (event.kind === "run_finished") {
          navigateToResult(event.runId);
        }
      }),
    );
    attach(() =>
      backend.onRunError((event) => {
        if (
          !mounted.current ||
          navigated.current ||
          event.runId !== activeRunId.current
        ) {
          return;
        }
        setRunAlert(
          "运行可能已中断，正在核对本地记录；这次不会按能力失败计分。",
        );
        pollNowRef.current();
      }),
    );

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        releaseUnlisten(unlisten);
      }
    };
  }, [backend, navigateToResult, reconcileProgress]);

  useEffect(() => {
    if (!run) {
      return;
    }
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [run?.id]);

  const bootstrap =
    bootstrapState.kind === "ready" ? bootstrapState.value : null;
  const availability = bootstrap?.targets.find(
    (target) => target.kind === kind,
  );
  const blocker = bootstrap
    ? availabilityBlocker(label, availability)
    : null;
  const modelError = modelValidationError(model);
  const startAllowed = Boolean(
    bootstrap &&
      availability &&
      !blocker &&
      acceptedCost &&
      (resumeRunId
        ? resumePreview?.kind === "ready"
        : !modelError) &&
      !starting,
  );

  async function start() {
    if (!resumeRunId) {
      setModelTouched(true);
    }
    if (
      startPending.current ||
      !startAllowed ||
      !bootstrap ||
      run
    ) {
      return;
    }
    startPending.current = true;
    setStarting(true);
    setSetupError("");

    try {
      const reviewed =
        resumeRunId && resumePreview?.kind === "ready"
          ? resumePreview.run
          : null;
      const created = await Promise.resolve().then(() =>
        reviewed
          ? backend.resumeCliRun({
              runId: reviewed.id,
              expectedTarget: {
                ...reviewed.target,
                reasoningEffort: reviewed.target.reasoningEffort ?? null,
              },
            })
          : backend.startCliRun({
              target: {
                kind,
                reportedModel: model.trim() || "default",
                reasoningEffort: reasoningEffort || null,
              },
              mode: "quick",
            }),
      );
      if (!mounted.current) {
        return;
      }
      if (
        !isSafeRunRecord(created) ||
        created.target.kind !== kind ||
        created.status !== "running" ||
        (resumeRunId &&
          (!reviewed ||
            created.id !== resumeRunId ||
            !sameTarget(created.target, reviewed.target) ||
            !created.environment.resumed))
      ) {
        throw new Error("unsafe run record");
      }
      activeRunId.current = created.id;
      navigated.current = false;
      const initialProgress = {
        completed: safeCount(created.completedTasks),
        total: Math.max(
          safeCount(created.totalTasks),
          safeCount(created.completedTasks),
        ),
      };
      progressRef.current = initialProgress;
      setProgress(initialProgress);
      setRun(created);
      setRunAlert("");
      setSyncState("ready");
      setStopState("idle");
      pollNowRef.current();
    } catch {
      if (mounted.current) {
        setSetupError(
          resumeRunId
            ? "无法恢复这次 CLI 体检，请返回历史记录后重试。"
            : "无法启动 CLI 体检，请检查安装和登录状态后重试。",
        );
      }
    } finally {
      startPending.current = false;
      if (mounted.current) {
        setStarting(false);
      }
    }
  }

  async function requestCancel() {
    const runId = activeRunId.current;
    if (!runId || cancelPending.current || navigated.current) {
      return;
    }
    cancelPending.current = true;
    setCancelling(true);
    setCancelError("");
    try {
      const requested = await Promise.resolve().then(() =>
        backend.cancelRun(runId),
      );
      if (
        !mounted.current ||
        navigated.current ||
        activeRunId.current !== runId
      ) {
        return;
      }
      setConfirmCancel(false);
      if (requested) {
        setStopState("waiting");
      } else {
        setStopState("not-found");
      }
      pollNowRef.current();
    } catch {
      if (
        mounted.current &&
        !navigated.current &&
        activeRunId.current === runId
      ) {
        setCancelError(
          "无法请求安全停止，请重试或继续运行。",
        );
      }
    } finally {
      cancelPending.current = false;
      if (mounted.current) {
        setCancelling(false);
      }
    }
  }

  if (bootstrapState.kind === "loading") {
    return (
      <main
        className="page run-page cli-run-page cli-loading-page"
        id="page-content"
        tabIndex={-1}
      >
        <p className="eyebrow">CLI · 本地预检</p>
        <h1>{label} 快速体检</h1>
        <p
          aria-label={`正在检查 ${label} 环境`}
          aria-live="polite"
          role="status"
        >
          正在检查 {label}、登录状态和本地验证器…
        </p>
      </main>
    );
  }

  if (bootstrapState.kind === "error") {
    return (
      <main
        className="page run-page cli-run-page cli-loading-page"
        id="page-content"
        tabIndex={-1}
      >
        <p className="eyebrow">CLI · 本地预检</p>
        <h1>{label} 快速体检</h1>
        <section
          aria-label={`无法检查 ${label} 环境`}
          className="cli-alert-card"
          role="alert"
        >
          <h2>无法检查本机环境</h2>
          <p>请确认本机环境后重试；技术错误不会作为能力结果。</p>
          <button
            onClick={() => setBootstrapAttempt((attempt) => attempt + 1)}
            type="button"
          >
            重新检查
          </button>
        </section>
      </main>
    );
  }

  if (!run && resumePreview?.kind === "error") {
    return (
      <main
        className="page run-page cli-run-page cli-loading-page"
        id="page-content"
        tabIndex={-1}
      >
        <p className="eyebrow">CLI · 恢复未完成体检</p>
        <h1>{label} 快速体检</h1>
        <section className="cli-alert-card" role="alert">
          <h2>无法恢复这次 CLI 体检</h2>
          <p>{resumePreview.message}</p>
          <Link to="/history">返回历史记录</Link>
        </section>
      </main>
    );
  }

  if (!run) {
    const pack = bootstrapState.value.cliPack;
    return (
      <main
        className="page run-page cli-run-page cli-setup-page"
        id="page-content"
        tabIndex={-1}
      >
        <section aria-labelledby="cli-setup-title" className="cli-setup">
          <p className="eyebrow">CLI · 自动快速体检</p>
          <h1 id="cli-setup-title">{label} 快速体检</h1>
          <p className="cli-pack-summary">
            {pack.taskCount} 个微型项目 · 预计 {pack.estimatedMinutes} 分钟（估计）
          </p>
          <p className="hint">
            预计时间不是上限；超过估计后仍会显示实际用时并继续等待安全结果。
          </p>

          <section
            aria-labelledby="cli-environment-title"
            className="cli-card"
          >
            <h2 id="cli-environment-title">本机预检</h2>
            <dl className="cli-environment-list">
              <div>
                <dt>CLI</dt>
                <dd>
                  {availability?.installed
                    ? availability.version || "已检测到，版本未报告"
                    : `未检测到 ${label}`}
                </dd>
              </div>
              <div>
                <dt>登录</dt>
                <dd>
                  {availability?.authState === "ready"
                    ? "本机 CLI 报告已就绪"
                    : availability?.authState === "needs_login"
                      ? `需要先登录 ${label}`
                      : "登录状态将在启动时复核，当前不代表已经登录。"}
                </dd>
              </div>
              {availability?.prerequisites.map((prerequisite) => (
                <div key={prerequisite.name}>
                  <dt>{prerequisite.name}</dt>
                  <dd>
                    {prerequisite.available
                      ? prerequisite.version || "可用"
                      : "不可用"}
                  </dd>
                </div>
              ))}
            </dl>
            {blocker ? <p className="form-error">{blocker}</p> : null}
          </section>

          <aside aria-labelledby="cli-boundary-title" className="notice">
            <h2 id="cli-boundary-title">费用、隐私与测量边界</h2>
            <p>
              运行使用你本机 CLI 的认证和计费配置，可能消耗订阅额度或 API
              余额；应用无法判断或保证具体扣费来源。
            </p>
            <p>
              维护者不会承担费用、提供共享密钥、接收凭据或检查认证文件。
            </p>
            <p>
              应用只在自己的数据目录创建隔离的临时微型项目，不会改写你的真实项目。
            </p>
            <p>
              结果衡量模型、CLI、配置和工具共同形成的端到端表现，不是底层模型的“智商”。
            </p>
          </aside>

          {resumeRunId && resumePreview?.kind === "ready" ? (
            <section
              aria-labelledby="cli-resume-title"
              className="resume-configuration"
            >
              <h2 id="cli-resume-title">确认恢复原体检</h2>
              <p>只继续原记录尚未完成的任务，并精确沿用以下配置。</p>
              <dl className="cli-environment-list">
                <div>
                  <dt>原目标</dt>
                  <dd>
                    {cliLabels[
                      resumePreview.run.target.kind as CliTargetKind
                    ]}
                  </dd>
                </div>
                <div>
                  <dt>原模型</dt>
                  <dd>{resumePreview.run.target.reportedModel}</dd>
                </div>
                <div>
                  <dt>原推理档位</dt>
                  <dd>
                    {reasoningEffortLabel(
                      resumePreview.run.target.reasoningEffort,
                    )}
                  </dd>
                </div>
              </dl>
            </section>
          ) : resumeRunId ? (
            <p aria-live="polite" role="status">
              正在读取原体检配置……
            </p>
          ) : null}

          <div className="cli-fields">
            {resumeRunId ? (
              <p className="resume-configuration">
                将沿用原运行的模型、推理档位与题包，只执行尚未完成的任务。
              </p>
            ) : (
              <>
                <div className="field">
                  <label htmlFor="cli-model">固定模型（可选）</label>
                  <input
                    aria-describedby={
                      modelTouched && modelError
                        ? "cli-model-hint cli-model-error"
                        : "cli-model-hint"
                    }
                    aria-invalid={
                      modelTouched && modelError ? "true" : undefined
                    }
                    autoComplete="off"
                    id="cli-model"
                    onChange={(event) => {
                      setModelTouched(true);
                      setModel(event.target.value);
                    }}
                    placeholder={
                      kind === "codex_cli"
                        ? "例如 gpt-5.4"
                        : "例如 sonnet"
                    }
                    value={model}
                  />
                  <small className="hint" id="cli-model-hint">
                    留空会使用 CLI 默认路由，并在记录中明确标为 default。
                  </small>
                </div>
                {modelTouched && modelError ? (
                  <p
                    aria-label={modelError}
                    className="form-error"
                    id="cli-model-error"
                    role="alert"
                  >
                    请输入 1–120 个字符；首字符须为英文字母或数字，其余只能使用英文字母、数字和
                    . _ : / -
                  </p>
                ) : null}

                <div className="field">
                  <label htmlFor="cli-reasoning">推理档位（可选）</label>
                  <select
                    id="cli-reasoning"
                    onChange={(event) =>
                      setReasoningEffort(
                        event.target.value as ReasoningEffort,
                      )
                    }
                    value={reasoningEffort}
                  >
                    <option value="">CLI 默认</option>
                    <option value="low">低</option>
                    <option value="medium">中</option>
                    <option value="high">高</option>
                  </select>
                </div>
              </>
            )}

            <label className="check-row">
              <input
                checked={acceptedCost}
                onChange={(event) => setAcceptedCost(event.target.checked)}
                type="checkbox"
              />
              <span>
                我了解本次运行可能消耗自己的订阅额度或 API 余额
              </span>
            </label>
          </div>

          {connectivity === "fallback" ? (
            <p
              aria-label="实时更新不可用，运行时将使用定时同步"
              className="hint"
              role="status"
            >
              实时更新不可用，运行时将使用定时同步。
            </p>
          ) : null}
          {setupError ? (
            <p className="form-error" role="alert">
              {setupError}
            </p>
          ) : null}
          {starting ? (
            <p aria-live="polite" role="status">
              {resumeRunId
                ? "正在恢复本地 CLI 体检…"
                : "正在启动本地 CLI 体检…"}
            </p>
          ) : null}
          <button
            disabled={!startAllowed}
            onClick={() => void start()}
            type="button"
          >
            {starting
              ? resumeRunId
                ? "正在恢复…"
                : "正在启动…"
              : resumeRunId
                ? "继续剩余任务"
                : `开始 ${pack.taskCount} 个任务`}
          </button>
        </section>
      </main>
    );
  }

  const currentPosition =
    progress.total > 0
      ? Math.min(progress.completed + 1, progress.total)
      : 0;
  const syncCopy =
    syncState === "missing"
      ? "同步暂未取得本地记录，正在自动重试。"
      : syncState === "failed"
        ? "进度同步暂时失败，正在自动重试。"
        : connectivity === "fallback"
          ? "实时更新不可用，正在使用定时同步。"
          : connectivity === "live"
            ? "实时事件已连接；定时同步会校验本地记录。"
            : "正在连接实时更新；定时同步已准备。";
  const syncLabel =
    syncState === "missing"
      ? "同步暂未取得本地记录，正在自动重试"
      : syncState === "failed"
        ? "进度同步暂时失败，正在自动重试"
        : undefined;

  return (
    <main
      className="page run-page cli-run-page cli-progress-page"
      id="page-content"
      tabIndex={-1}
    >
      <p className="eyebrow">{label} · 自动运行</p>
      <h1>正在完成本地微型项目</h1>
      <div className="cli-progress-card">
        <div
          aria-live="polite"
          className="cli-progress-announcement"
          role="status"
        >
          <p className="cli-progress-number">
            {progress.completed} / {progress.total} 已完成
          </p>
          <p>第 {currentPosition} / {progress.total} 个微型项目</p>
        </div>
        <progress
          aria-label={`已完成 ${progress.completed} / ${progress.total}`}
          max={Math.max(progress.total, 1)}
          value={Math.min(progress.completed, Math.max(progress.total, 1))}
        >
          {progress.completed}/{progress.total}
        </progress>
        <div className="cli-live-facts">
          <p>{formatElapsed(run.startedAt, now)}</p>
        </div>
      </div>

      <p aria-label={syncLabel} className="hint" role="status">
        {syncCopy}
      </p>
      <p className="hint">
        可以最小化窗口，但请不要关闭应用或所选 CLI
        的登录会话；应用只会处理本次隔离临时项目。
      </p>

      {runAlert ? (
        <p
          aria-label={runAlert.replace(/[。]$/, "")}
          className="form-error"
          role="alert"
        >
          {runAlert}
        </p>
      ) : null}

      {confirmCancel ? (
        <section
          aria-label="确认停止运行"
          className="cli-cancel-card"
          role="group"
        >
          <h2>确认停止本次运行？</h2>
          <p>
            停止请求会结束 CLI
            进程树，并把本次记录为已取消或无效，不会算作能力失败。
          </p>
          {cancelError ? (
            <p className="form-error" role="alert">
              {cancelError}
            </p>
          ) : null}
          <div className="cli-actions">
            <button
              className="danger"
              disabled={cancelling}
              onClick={() => void requestCancel()}
              type="button"
            >
              {cancelling ? "正在请求停止…" : "确认停止"}
            </button>
            <button
              className="secondary"
              disabled={cancelling}
              onClick={() => {
                setConfirmCancel(false);
                setCancelError("");
              }}
              type="button"
            >
              继续运行
            </button>
          </div>
        </section>
      ) : stopState === "waiting" ? (
        <div className="cli-stop-state">
          <p
            aria-label="正在安全停止"
            aria-live="polite"
            role="status"
          >
            正在安全停止；停止请求已发出，等待本地记录确认。
          </p>
          <button disabled type="button">
            正在安全停止
          </button>
        </div>
      ) : (
        <div className="cli-stop-state">
          {stopState === "not-found" ? (
            <p
              aria-label="没有找到活动的停止登记，正在重新同步"
              role="status"
            >
              没有找到活动的停止登记，正在重新同步本地记录。
            </p>
          ) : null}
          <button
            className="danger"
            onClick={() => {
              setCancelError("");
              setConfirmCancel(true);
            }}
            type="button"
          >
            {stopState === "not-found" ? "再次请求停止" : "停止运行"}
          </button>
        </div>
      )}
    </main>
  );
}

export function CliRunPage() {
  const { target = "" } = useParams();
  const [searchParams] = useSearchParams();
  const resumeRunId = searchParams.get("resume") || undefined;

  if (!isCliTarget(target)) {
    return <UnsupportedCliPage />;
  }

  return (
    <CliWizard
      key={`${target}:${resumeRunId ?? "new"}`}
      kind={target}
      resumeRunId={resumeRunId}
    />
  );
}
