import { useEffect, useRef, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import { isSafeRunRecord } from "../api/runtimeValidation";
import { ClientSelectionPanel } from "../components/ClientSelectionPanel";
import { ReasoningEffortField } from "../components/ReasoningEffortField";
import type {
  ModelSource,
  ModelVerification,
} from "../domain/modelProvenance";
import { formatModelProvenance } from "../domain/modelProvenance";
import {
  formatReasoningEffort,
  normalizeReasoningEffortForTarget,
} from "../domain/reasoningEffort";
import {
  formatReportedModel,
  reportedModelError,
} from "../domain/reportedModel";
import { useT } from "../i18n/I18nContext";
import type {
  Backend,
  ManualStep,
  RunRecord,
  TargetKind,
  TargetSelection,
} from "../api/backend";
import "./ManualRunPage.css";

const ANSWER_LIMIT_BYTES = 256 * 1024;

type ClientTargetKind = Extract<
  TargetKind,
  "chat_gpt_client" | "claude_client"
>;
type CopyStatus = "idle" | "copied" | "manual";

type WizardState =
  | { kind: "setup"; error: string }
  | { kind: "starting" }
  | { kind: "resume-loading" }
  | { kind: "resume-review"; run: RunRecord }
  | { kind: "resuming"; run: RunRecord }
  | { kind: "resume-error"; message: string }
  | { kind: "loading-first"; run: RunRecord }
  | { kind: "first-step-error"; run: RunRecord; error: string }
  | {
      kind: "answering";
      run: RunRecord;
      step: ManualStep;
      answer: string;
      copyStatus: CopyStatus;
      submitError: string;
    }
  | {
      kind: "submitting";
      run: RunRecord;
      step: ManualStep;
      answer: string;
      copyStatus: CopyStatus;
    }
  | { kind: "loading-next"; run: RunRecord }
  | { kind: "next-step-error"; run: RunRecord; error: string };

const clientLabels: Record<ClientTargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
};

function isClientTarget(value: string): value is ClientTargetKind {
  return value === "chat_gpt_client" || value === "claude_client";
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function sameTarget(left: TargetSelection, right: TargetSelection): boolean {
  return (
    left.kind === right.kind &&
    left.reportedModel === right.reportedModel &&
    (left.reasoningEffort ?? null) === (right.reasoningEffort ?? null) &&
    left.modelSource === right.modelSource &&
    left.modelVerification === right.modelVerification
  );
}

function answerBytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function SetupPage({
  busy,
  error,
  freshChat,
  kind,
  model,
  modelTouched,
  reasoningError,
  reasoningEffort,
  onFreshChatChange,
  onModelChange,
  onReasoningEffortChange,
  onReasoningValidationChange,
  onSelectionApply,
  onStart,
  selectionEdited,
  selectionFormDirty,
  detectClientSelection,
}: {
  busy: boolean;
  detectClientSelection: Backend["detectClientSelection"];
  error: string;
  freshChat: boolean;
  kind: ClientTargetKind;
  model: string;
  modelTouched: boolean;
  reasoningError: string | null;
  reasoningEffort: string;
  onFreshChatChange(value: boolean): void;
  onModelChange(value: string): void;
  onReasoningEffortChange(value: string): void;
  onReasoningValidationChange(error: string | null): void;
  onSelectionApply(value: {
    model?: string;
    reasoningEffort?: string;
  }): void;
  onStart(): void;
  selectionEdited: boolean;
  selectionFormDirty: boolean;
}) {
  const modelError = reportedModelError(model);
  const showModelError = modelTouched && modelError;
  const label = clientLabels[kind];

  return (
    <main
      className="page run-page manual-setup-page"
      id="page-content"
      tabIndex={-1}
    >
      <section aria-labelledby="manual-setup-title" className="manual-setup">
        <p className="eyebrow">客户端 · 快速体检 · 约 10–15 分钟</p>
        <h1 id="manual-setup-title">{label}快速体检</h1>
        <p className="hero-summary">
          一次只处理一道题：复制题目到客户端的新空白对话，再把完整回答原样粘贴回来。
        </p>

        <aside aria-labelledby="manual-boundary-title" className="notice">
          <h2 id="manual-boundary-title">开始前请确认</h2>
          <p>客户端使用可能消耗你自己的订阅额度。</p>
          <p>维护者不会承担费用，也不会接收你的登录凭据。</p>
          <p>这里测量端到端客户端表现，不是底层模型的“智商”。</p>
          <p>原始回答只保存在本机，不会自动上传。</p>
        </aside>

        {!busy ? (
          <ClientSelectionPanel
            detect={detectClientSelection}
            edited={selectionEdited}
            enabled
            formDirty={selectionFormDirty}
            onApply={onSelectionApply}
            target={kind}
          />
        ) : null}

        <div className="manual-fields">
          <label className="field">
            <span>当前显示的模型</span>
            <input
              aria-describedby={showModelError ? "model-error" : undefined}
              aria-invalid={showModelError ? "true" : undefined}
              autoComplete="off"
              onChange={(event) => onModelChange(event.target.value)}
              placeholder="例如 GPT-5、Claude Sonnet"
              value={model}
            />
          </label>
          {showModelError ? (
            <p
              aria-label={showModelError}
              className="form-error"
              id="model-error"
              role="alert"
            >
              {showModelError}
            </p>
          ) : null}

          <ReasoningEffortField
            emptyLabel="未显示 / 不适用"
            id="manual-reasoning"
            kind={kind}
            label="推理档位（没有显示可留空）"
            onChange={onReasoningEffortChange}
            onValidationChange={onReasoningValidationChange}
            value={reasoningEffort}
          />

          <label className="check-row">
            <input
              checked={freshChat}
              onChange={(event) => onFreshChatChange(event.target.checked)}
              type="checkbox"
            />
            <span>我会为每道题新建空白对话</span>
          </label>
        </div>

        <p className="hint">
          除非题目明确允许，否则关闭联网搜索、工具和连接器；不要追加解释性提示，
          也不要把评分结果发回给 AI。
        </p>
        {error ? (
          <p className="form-error" role="alert">
            {error}
          </p>
        ) : null}
        {busy ? (
          <p aria-live="polite" role="status">
            正在创建本地体检…
          </p>
        ) : null}
        <button
          disabled={
            busy || !freshChat || Boolean(modelError) || Boolean(reasoningError)
          }
          onClick={onStart}
          type="button"
        >
          开始快速体检
        </button>
      </section>
    </main>
  );
}

function PendingStepPage({
  cancelBusy,
  cancelConfirm,
  cancelError,
  error,
  first,
  run,
  onBeginCancel,
  onConfirmCancel,
  onDismissCancel,
  onRetry,
}: {
  cancelBusy: boolean;
  cancelConfirm: boolean;
  cancelError: string;
  error?: string;
  first: boolean;
  run: RunRecord;
  onBeginCancel(): void;
  onConfirmCancel(): void;
  onDismissCancel(): void;
  onRetry?(): void;
}) {
  const submitted = !first;
  const heading = error
    ? submitted
      ? "上一题已经提交"
      : "体检已经创建"
    : submitted
      ? "正在继续体检"
      : "正在准备第一题";

  return (
    <main
      aria-busy={!error}
      className="page run-page manual-transition-page"
      id="page-content"
      tabIndex={-1}
    >
      <p className="eyebrow">
        {run.totalTasks} 道任务 · 本地运行已创建
      </p>
      <h1>{heading}</h1>
      {submitted ? (
        <p>上一题的回答已经成功提交，不需要也不会再次提交。</p>
      ) : (
        <p>已保留这次体检；如果读取失败，重试会继续使用同一个运行。</p>
      )}
      {error ? (
        <>
          <p className="form-error" role="alert">
            {error}
          </p>
          <button onClick={onRetry} type="button">
            {submitted ? "继续读取下一题" : "重试读取第一题"}
          </button>
        </>
      ) : (
        <p aria-live="polite" role="status">
          {submitted ? "本题已提交，正在读取下一题…" : "正在读取第一题…"}
        </p>
      )}
      <CancelControls
        busy={cancelBusy}
        confirm={cancelConfirm}
        error={cancelError}
        onBegin={onBeginCancel}
        onConfirm={onConfirmCancel}
        onDismiss={onDismissCancel}
      />
    </main>
  );
}

function RecoveryPage({ failed }: { failed: boolean }) {
  return (
    <main
      aria-busy={!failed}
      className="page run-page manual-transition-page"
      id="page-content"
      tabIndex={-1}
    >
      <p className="eyebrow">客户端 · 恢复未完成体检</p>
      <h1>{failed ? "无法恢复这次体检" : "正在恢复这次体检"}</h1>
      {failed ? (
        <>
          <p className="form-error" role="alert">
            无法恢复这次体检。记录可能已经完成、被取消或与当前环境不一致。
          </p>
          <Link to="/history">返回历史记录</Link>
        </>
      ) : (
        <p aria-live="polite" role="status">
          正在核对本地检查点，只会继续尚未完成的题目…
        </p>
      )}
    </main>
  );
}

function ResumeReviewPage({
  run,
  onResume,
}: {
  run: RunRecord;
  onResume(): void;
}) {
  return (
    <main
      className="page run-page manual-transition-page"
      id="page-content"
      tabIndex={-1}
    >
      <p className="eyebrow">客户端 · 恢复未完成体检</p>
      <h1>确认恢复原体检</h1>
      <p>恢复会继续下面这份本地记录，不会按当前网址创建或改写目标。</p>
      <dl className="run-metadata">
        <div>
          <dt>原目标</dt>
          <dd>{clientLabels[run.target.kind as ClientTargetKind]}</dd>
        </div>
        <div>
          <dt>原模型</dt>
          <dd>
            {formatReportedModel(
              run.target.kind,
              run.target.reportedModel,
            )}
          </dd>
        </div>
        <div>
          <dt>原推理档位</dt>
          <dd>
            {formatReasoningEffort(
              run.target.kind,
              run.target.reasoningEffort,
              "未显示 / 不适用",
            )}
          </dd>
        </div>
        <div>
          <dt>原模型来源与核验</dt>
          <dd>{formatModelProvenance(run.target)}</dd>
        </div>
      </dl>
      <button onClick={onResume} type="button">
        继续剩余题目
      </button>
    </main>
  );
}

function ResumeErrorPage({ message }: { message: string }) {
  return (
    <main
      className="page run-page manual-transition-page"
      id="page-content"
      tabIndex={-1}
    >
      <p className="eyebrow">客户端 · 恢复未完成体检</p>
      <h1>无法恢复这次体检</h1>
      <p className="form-error" role="alert">
        {message}
      </p>
      <Link to="/history">返回历史记录</Link>
    </main>
  );
}

function CancelControls({
  busy,
  confirm,
  error,
  onBegin,
  onConfirm,
  onDismiss,
}: {
  busy: boolean;
  confirm: boolean;
  error: string;
  onBegin(): void;
  onConfirm(): void;
  onDismiss(): void;
}) {
  return (
    <>
      {confirm ? (
        <div className="button-row">
          <button
            className="danger"
            disabled={busy}
            onClick={onConfirm}
            type="button"
          >
            {busy ? "正在取消…" : "确认取消"}
          </button>
          <button
            className="secondary"
            disabled={busy}
            onClick={onDismiss}
            type="button"
          >
            继续体检
          </button>
        </div>
      ) : (
        <button className="secondary" onClick={onBegin} type="button">
          取消本次体检
        </button>
      )}
      {error ? (
        <p className="form-error" role="alert">
          {error}
        </p>
      ) : null}
    </>
  );
}

function TaskPage({
  answer,
  busy,
  cancelBusy,
  cancelConfirm,
  cancelError,
  copyStatus,
  error,
  step,
  onAnswerChange,
  onBeginCancel,
  onConfirmCancel,
  onDismissCancel,
  onCopy,
  onSubmit,
}: {
  answer: string;
  busy: boolean;
  cancelBusy: boolean;
  cancelConfirm: boolean;
  cancelError: string;
  copyStatus: CopyStatus;
  error: string;
  step: ManualStep;
  onAnswerChange(value: string): void;
  onBeginCancel(): void;
  onConfirmCancel(): void;
  onDismissCancel(): void;
  onCopy(): void;
  onSubmit(): void;
}) {
  const bytes = answerBytes(answer);
  const overLimit = bytes > ANSWER_LIMIT_BYTES;
  const canSubmit = Boolean(answer.trim()) && !overLimit && !busy;
  const completedTasks = step.taskNumber - 1;

  return (
    <main
      className="page run-page manual-task-page"
      id="page-content"
      tabIndex={-1}
    >
      <header aria-live="polite" className="progress-copy">
        <p>
          第 {step.taskNumber} / {step.totalTasks} 题 · 已完成{" "}
          {completedTasks} 题
        </p>
        <progress
          aria-label={`已完成 ${completedTasks} / ${step.totalTasks} 题`}
          max={step.totalTasks}
          value={completedTasks}
        >
          {completedTasks}/{step.totalTasks}
        </progress>
      </header>

      <section aria-label="当前体检题目" className="manual-task">
        <p className="eyebrow">新对话检查点</p>
        <h1>在新空白对话中完成这道题</h1>
        <div className="task-reminders">
          <p>请为这道题新建空白对话。</p>
          <p>除非题目明确允许，否则不要使用联网搜索、工具或连接器。</p>
        </div>
        <pre
          aria-label="当前题目，可选中后手动复制"
          className="prompt-box"
          tabIndex={0}
        >
          {step.prompt}
        </pre>
        <button
          className="secondary"
          disabled={busy}
          onClick={onCopy}
          type="button"
        >
          {copyStatus === "copied" ? "再次复制题目" : "复制题目"}
        </button>
        {copyStatus !== "idle" ? (
          <p aria-live="polite" role="status">
            {copyStatus === "copied"
              ? "题目已复制，请粘贴到新的空白对话。"
              : "自动复制不可用，请选中题目文字后手动复制。"}
          </p>
        ) : null}

        <label className="field answer-field">
          <span>粘贴 AI 的完整回答</span>
          <textarea
            aria-describedby="answer-byte-count"
            aria-invalid={overLimit ? "true" : undefined}
            disabled={busy}
            onChange={(event) => onAnswerChange(event.target.value)}
            placeholder="不要修改、删减或补充回答"
            rows={10}
            value={answer}
          />
        </label>
        <p
          className={overLimit ? "byte-count byte-count-over" : "byte-count"}
          id="answer-byte-count"
        >
          {bytes} / {ANSWER_LIMIT_BYTES} 字节
        </p>
        {overLimit ? (
          <p className="form-error" role="alert">
            回答超过 256 KiB，请删减后再提交。
          </p>
        ) : null}
        {error ? (
          <p className="form-error" role="alert">
            {error}
          </p>
        ) : null}
        {busy ? (
          <p aria-live="polite" role="status">
            正在保存本题回答…
          </p>
        ) : null}
        <button disabled={!canSubmit} onClick={onSubmit} type="button">
          提交并进入下一题
        </button>
        <CancelControls
          busy={busy || cancelBusy}
          confirm={cancelConfirm}
          error={cancelError}
          onBegin={onBeginCancel}
          onConfirm={onConfirmCancel}
          onDismiss={onDismissCancel}
        />
      </section>
    </main>
  );
}

function ManualWizard({
  kind,
  resumeRunId,
}: {
  kind: ClientTargetKind;
  resumeRunId?: string;
}) {
  const backend = useBackend();
  const navigate = useNavigate();
  const mounted = useRef(true);
  const activeRunId = useRef<string | null>(null);
  const pending = useRef(false);
  const resumeStarted = useRef(false);
  const [cancelBusy, setCancelBusy] = useState(false);
  const [cancelConfirm, setCancelConfirm] = useState(false);
  const [cancelError, setCancelError] = useState("");
  const [model, setModel] = useState("");
  const [modelTouched, setModelTouched] = useState(false);
  const [modelSource, setModelSource] = useState<ModelSource>("manual");
  const [modelVerification] =
    useState<ModelVerification>("user_confirmed");
  const [formDirty, setFormDirty] = useState(false);
  const [selectionWasApplied, setSelectionWasApplied] = useState(false);
  const [reasoningEffort, setReasoningEffort] = useState("");
  const [reasoningError, setReasoningError] = useState<string | null>(null);
  const [freshChat, setFreshChat] = useState(false);
  const [state, setState] = useState<WizardState>(
    resumeRunId
      ? { kind: "resume-loading" }
      : {
          kind: "setup",
          error: "",
        },
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      pending.current = false;
      bestEffortInterruptActiveRun();
    };
  }, [backend]);

  useEffect(() => {
    if (!resumeRunId || resumeStarted.current) {
      return;
    }
    resumeStarted.current = true;
    void loadResumePreview();
  }, [resumeRunId]);

  function beginOperation(): boolean {
    if (pending.current) {
      return false;
    }
    pending.current = true;
    return true;
  }

  function completeNavigation(run: RunRecord) {
    pending.current = false;
    activeRunId.current = null;
    navigate(`/results/${run.id}`, {
      replace: true,
      state: { manualRunCompleted: true },
    });
  }

  function bestEffortInterruptActiveRun(runId = activeRunId.current) {
    if (runId) {
      void backend.interruptManualRun(runId).catch(() => undefined);
    }
  }

  function adoptActiveRun(run: RunRecord): boolean {
    if (!mounted.current) {
      bestEffortInterruptActiveRun(run.id);
      return false;
    }
    activeRunId.current = run.id;
    return true;
  }

  async function cancelActiveRun() {
    const runId = activeRunId.current;
    if (!runId || cancelBusy) {
      return;
    }
    setCancelBusy(true);
    setCancelError("");
    try {
      const cancelled = await backend.cancelRun(runId);
      if (!mounted.current) {
        if (!cancelled) {
          bestEffortInterruptActiveRun(runId);
        }
        return;
      }
      if (!cancelled || activeRunId.current !== runId) {
        setCancelBusy(false);
        setCancelError("无法取消这次体检，请稍后重试。");
        return;
      }
      activeRunId.current = null;
      pending.current = false;
      navigate(`/results/${runId}`, {
        replace: true,
        state: { manualRunCancelled: true },
      });
    } catch {
      if (!mounted.current) {
        bestEffortInterruptActiveRun(runId);
        return;
      }
      setCancelBusy(false);
      setCancelError("无法取消这次体检，请稍后重试。");
    }
  }

  function showStep(run: RunRecord, step: ManualStep | null) {
    if (!step) {
      completeNavigation(run);
      return;
    }
    pending.current = false;
    setState({
      kind: "answering",
      run,
      step,
      answer: "",
      copyStatus: "idle",
      submitError: "",
    });
  }

  async function readFirstStep(run: RunRecord) {
    setState({ kind: "loading-first", run });
    try {
      const step = await Promise.resolve().then(() =>
        backend.nextManualStep(run.id),
      );
      if (!mounted.current) {
        bestEffortInterruptActiveRun(run.id);
        return;
      }
      showStep(run, step);
    } catch (reason) {
      if (!mounted.current) {
        bestEffortInterruptActiveRun(run.id);
        return;
      }
      pending.current = false;
      setState({
        kind: "first-step-error",
        run,
        error: run.environment.resumed
          ? "无法继续读取剩余题目，请稍后重试。"
          : errorMessage(reason),
      });
    }
  }

  async function loadResumePreview() {
    if (!resumeRunId || !beginOperation()) {
      return;
    }
    setState({ kind: "resume-loading" });
    try {
      const detail = await Promise.resolve().then(() =>
        backend.getRunDetail(resumeRunId),
      );
      if (!mounted.current) {
        return;
      }
      if (
        !detail ||
        !isSafeRunRecord(detail.run) ||
        detail.run.id !== resumeRunId ||
        detail.run.status !== "interrupted"
      ) {
        pending.current = false;
        setState({
          kind: "resume-error",
          message: "无法恢复这次体检。本地记录可能已经结束或无法安全读取。",
        });
        return;
      }
      if (detail.run.target.kind !== kind) {
        pending.current = false;
        setState({
          kind: "resume-error",
          message: "恢复链接与原体检目标不一致。",
        });
        return;
      }
      pending.current = false;
      setState({ kind: "resume-review", run: detail.run });
    } catch {
      if (!mounted.current) {
        return;
      }
      pending.current = false;
      setState({
        kind: "resume-error",
        message: "无法恢复这次体检。本地记录可能已经结束或无法安全读取。",
      });
    }
  }

  async function resume() {
    if (
      !resumeRunId ||
      state.kind !== "resume-review" ||
      !beginOperation()
    ) {
      return;
    }
    const reviewed = state.run;
    setState({ kind: "resuming", run: reviewed });
    try {
      const recovered = await Promise.resolve().then(() =>
        backend.resumeManualRun({
          runId: resumeRunId,
          expectedTarget: {
            ...reviewed.target,
            reasoningEffort: reviewed.target.reasoningEffort ?? null,
          },
        }),
      );
      if (
        !isSafeRunRecord(recovered) ||
        recovered.id !== resumeRunId ||
        recovered.target.kind !== kind ||
        !sameTarget(recovered.target, reviewed.target) ||
        recovered.status !== "running" ||
        !recovered.environment.resumed
      ) {
        bestEffortInterruptActiveRun(resumeRunId);
        if (!mounted.current) {
          return;
        }
        pending.current = false;
        setState({
          kind: "resume-error",
          message: "无法恢复这次体检。原体检配置或本地检查点已经变化。",
        });
        return;
      }
      if (!adoptActiveRun(recovered)) {
        return;
      }
      await readFirstStep(recovered);
    } catch {
      bestEffortInterruptActiveRun(resumeRunId);
      if (!mounted.current) {
        return;
      }
      pending.current = false;
      setState({
        kind: "resume-error",
        message: "无法恢复这次体检。原体检配置或本地检查点已经变化。",
      });
    }
  }

  async function start() {
    const modelError = reportedModelError(model);
    setModelTouched(true);
    if (modelError || reasoningError || !freshChat || state.kind !== "setup") {
      return;
    }
    if (!beginOperation()) {
      return;
    }

    setState({ kind: "starting" });
    const normalizedReasoningEffort = normalizeReasoningEffortForTarget(
      kind,
      reasoningEffort,
    );
    const requestedTarget: TargetSelection = {
      kind,
      reportedModel: model.trim(),
      reasoningEffort: normalizedReasoningEffort || null,
      modelSource,
      modelVerification,
    };
    let run: RunRecord;
    try {
      run = await Promise.resolve().then(() =>
        backend.startManualRun({
          target: requestedTarget,
          mode: "quick",
        }),
      );
    } catch (reason) {
      if (!mounted.current) {
        return;
      }
      pending.current = false;
      setState({ kind: "setup", error: errorMessage(reason) });
      return;
    }

    const isTrustedFreshRun =
      isSafeRunRecord(run) &&
      run.status === "running" &&
      run.mode === "quick" &&
      run.completedTasks === 0 &&
      run.finishedAt == null &&
      run.score == null &&
      !run.environment.resumed &&
      sameTarget(run.target, requestedTarget);
    if (!isTrustedFreshRun) {
      if (!mounted.current) {
        return;
      }
      pending.current = false;
      setState({
        kind: "setup",
        error: "无法创建这次体检，请稍后重试。",
      });
      return;
    }
    if (!mounted.current) {
      bestEffortInterruptActiveRun(run.id);
      return;
    }
    if (!adoptActiveRun(run)) {
      return;
    }
    await readFirstStep(run);
  }

  async function retryFirstStep() {
    if (state.kind !== "first-step-error" || !beginOperation()) {
      return;
    }
    await readFirstStep(state.run);
  }

  async function copyPrompt() {
    if (state.kind !== "answering") {
      return;
    }
    const { prompt } = state.step;
    const clipboard = navigator.clipboard;
    if (!clipboard || typeof clipboard.writeText !== "function") {
      setState({ ...state, copyStatus: "manual" });
      return;
    }
    try {
      await Promise.resolve().then(() => clipboard.writeText(prompt));
      if (mounted.current) {
        setState((current) =>
          current.kind === "answering" &&
          current.step.taskId === state.step.taskId
            ? { ...current, copyStatus: "copied" }
            : current,
        );
      }
    } catch {
      if (mounted.current) {
        setState((current) =>
          current.kind === "answering" &&
          current.step.taskId === state.step.taskId
            ? { ...current, copyStatus: "manual" }
            : current,
        );
      }
    }
  }

  async function readNextStep(run: RunRecord) {
    setState({ kind: "loading-next", run });
    try {
      const step = await Promise.resolve().then(() =>
        backend.nextManualStep(run.id),
      );
      if (!mounted.current) {
        bestEffortInterruptActiveRun(run.id);
        return;
      }
      showStep(run, step);
    } catch (reason) {
      if (!mounted.current) {
        bestEffortInterruptActiveRun(run.id);
        return;
      }
      pending.current = false;
      setState({
        kind: "next-step-error",
        run,
        error: errorMessage(reason),
      });
    }
  }

  async function submit() {
    if (state.kind !== "answering" || !beginOperation()) {
      return;
    }
    const bytes = answerBytes(state.answer);
    if (!state.answer.trim() || bytes > ANSWER_LIMIT_BYTES) {
      pending.current = false;
      return;
    }

    const answeringState = state;
    setState({
      kind: "submitting",
      run: state.run,
      step: state.step,
      answer: state.answer,
      copyStatus: state.copyStatus,
    });
    try {
      await Promise.resolve().then(() =>
        backend.submitManualAnswer({
          runId: answeringState.run.id,
          taskId: answeringState.step.taskId,
          answer: answeringState.answer,
        }),
      );
    } catch (reason) {
      if (!mounted.current) {
        bestEffortInterruptActiveRun(answeringState.run.id);
        return;
      }
      pending.current = false;
      setState({
        ...answeringState,
        submitError: errorMessage(reason),
      });
      return;
    }

    if (!mounted.current) {
      bestEffortInterruptActiveRun(answeringState.run.id);
      return;
    }
    await readNextStep(answeringState.run);
  }

  async function retryNextStep() {
    if (state.kind !== "next-step-error" || !beginOperation()) {
      return;
    }
    await readNextStep(state.run);
  }

  const beginCancel = () => {
    setCancelConfirm(true);
    setCancelError("");
  };
  const dismissCancel = () => {
    setCancelConfirm(false);
    setCancelError("");
  };
  const applyClientSelection = (selection: {
    model?: string;
    reasoningEffort?: string;
  }) => {
    if (selection.model) {
      setModel(selection.model);
      setModelTouched(false);
      setModelSource("windows_accessibility");
    }
    if (selection.reasoningEffort) {
      setReasoningEffort(selection.reasoningEffort);
      setReasoningError(null);
    }
    setFormDirty(false);
    setSelectionWasApplied(true);
  };

  if (state.kind === "setup" || state.kind === "starting") {
    return (
      <SetupPage
        busy={state.kind === "starting"}
        detectClientSelection={backend.detectClientSelection}
        error={state.kind === "setup" ? state.error : ""}
        freshChat={freshChat}
        kind={kind}
        model={model}
        modelTouched={modelTouched}
        onFreshChatChange={setFreshChat}
        onModelChange={(value) => {
          setModelTouched(true);
          setModel(value);
          setFormDirty(true);
          setModelSource("manual");
        }}
        onReasoningEffortChange={(value) => {
          setReasoningEffort(value);
          setFormDirty(true);
          setModelSource("manual");
        }}
        onReasoningValidationChange={setReasoningError}
        onSelectionApply={applyClientSelection}
        onStart={() => void start()}
        reasoningError={reasoningError}
        reasoningEffort={reasoningEffort}
        selectionEdited={selectionWasApplied && formDirty}
        selectionFormDirty={formDirty}
      />
    );
  }

  if (state.kind === "resume-loading" || state.kind === "resuming") {
    return <RecoveryPage failed={false} />;
  }

  if (state.kind === "resume-review") {
    return <ResumeReviewPage onResume={() => void resume()} run={state.run} />;
  }

  if (state.kind === "resume-error") {
    return <ResumeErrorPage message={state.message} />;
  }

  if (state.kind === "loading-first") {
    return (
      <PendingStepPage
        cancelBusy={cancelBusy}
        cancelConfirm={cancelConfirm}
        cancelError={cancelError}
        first
        onBeginCancel={beginCancel}
        onConfirmCancel={() => void cancelActiveRun()}
        onDismissCancel={dismissCancel}
        run={state.run}
      />
    );
  }

  if (state.kind === "first-step-error") {
    return (
      <PendingStepPage
        cancelBusy={cancelBusy}
        cancelConfirm={cancelConfirm}
        cancelError={cancelError}
        error={state.error}
        first
        onBeginCancel={beginCancel}
        onConfirmCancel={() => void cancelActiveRun()}
        onDismissCancel={dismissCancel}
        onRetry={() => void retryFirstStep()}
        run={state.run}
      />
    );
  }

  if (state.kind === "loading-next") {
    return (
      <PendingStepPage
        cancelBusy={cancelBusy}
        cancelConfirm={cancelConfirm}
        cancelError={cancelError}
        first={false}
        onBeginCancel={beginCancel}
        onConfirmCancel={() => void cancelActiveRun()}
        onDismissCancel={dismissCancel}
        run={state.run}
      />
    );
  }

  if (state.kind === "next-step-error") {
    return (
      <PendingStepPage
        cancelBusy={cancelBusy}
        cancelConfirm={cancelConfirm}
        cancelError={cancelError}
        error={state.error}
        first={false}
        onBeginCancel={beginCancel}
        onConfirmCancel={() => void cancelActiveRun()}
        onDismissCancel={dismissCancel}
        onRetry={() => void retryNextStep()}
        run={state.run}
      />
    );
  }

  return (
    <TaskPage
      answer={state.answer}
      busy={state.kind === "submitting"}
      cancelBusy={cancelBusy}
      cancelConfirm={cancelConfirm}
      cancelError={cancelError}
      copyStatus={state.copyStatus}
      error={state.kind === "answering" ? state.submitError : ""}
      onAnswerChange={(answer) =>
        setState((current) =>
          current.kind === "answering"
            ? { ...current, answer, submitError: "" }
            : current,
        )
      }
      onBeginCancel={beginCancel}
      onConfirmCancel={() => void cancelActiveRun()}
      onDismissCancel={dismissCancel}
      onCopy={() => void copyPrompt()}
      onSubmit={() => void submit()}
      step={state.step}
    />
  );
}

export function ManualRunPage() {
  const t = useT();
  const { target = "" } = useParams();
  const [searchParams] = useSearchParams();
  const resumeRunId = searchParams.get("resume") || undefined;

  if (!isClientTarget(target)) {
    return (
      <main
        className="page placeholder-page unsupported-manual-page"
        id="page-content"
        tabIndex={-1}
      >
        <p className="eyebrow">不支持的地址</p>
        <h1>不支持的客户端体检</h1>
        <p>这个地址不是 ChatGPT 或 Claude 客户端体检。</p>
        <Link to="/">{t("common.backHome")}</Link>
      </main>
    );
  }

  return (
    <ManualWizard
      key={`${target}:${resumeRunId ?? "new"}`}
      kind={target}
      resumeRunId={resumeRunId}
    />
  );
}
