import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type {
  ManualStep,
  RunRecord,
  TargetKind,
} from "../api/backend";
import "./ManualRunPage.css";

const ANSWER_LIMIT_BYTES = 256 * 1024;
const CONTROL_CHARACTER = /\p{Cc}/u;

type ClientTargetKind = Extract<
  TargetKind,
  "chat_gpt_client" | "claude_client"
>;
type ReasoningEffort = "" | "low" | "medium" | "high";
type CopyStatus = "idle" | "copied" | "manual";

type WizardState =
  | { kind: "setup"; error: string }
  | { kind: "starting" }
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

function validateModel(value: string): string | null {
  if (CONTROL_CHARACTER.test(value)) {
    return "模型名称不能包含控制字符";
  }
  const trimmed = value.trim();
  if (!trimmed || Array.from(trimmed).length > 120) {
    return "模型名称必须是 1–120 个可见字符";
  }
  return null;
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
  reasoningEffort,
  onFreshChatChange,
  onModelChange,
  onReasoningEffortChange,
  onStart,
}: {
  busy: boolean;
  error: string;
  freshChat: boolean;
  kind: ClientTargetKind;
  model: string;
  modelTouched: boolean;
  reasoningEffort: ReasoningEffort;
  onFreshChatChange(value: boolean): void;
  onModelChange(value: string): void;
  onReasoningEffortChange(value: ReasoningEffort): void;
  onStart(): void;
}) {
  const modelError = validateModel(model);
  const showModelError = modelTouched && modelError;
  const label = clientLabels[kind];

  return (
    <main className="page run-page manual-setup-page">
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

          <label className="field">
            <span>推理档位（没有显示可留空）</span>
            <select
              onChange={(event) =>
                onReasoningEffortChange(
                  event.target.value as ReasoningEffort,
                )
              }
              value={reasoningEffort}
            >
              <option value="">未显示 / 不适用</option>
              <option value="low">低</option>
              <option value="medium">中</option>
              <option value="high">高</option>
            </select>
          </label>

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
          disabled={busy || !freshChat || Boolean(modelError)}
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
  error,
  first,
  run,
  onRetry,
}: {
  error?: string;
  first: boolean;
  run: RunRecord;
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
    </main>
  );
}

function TaskPage({
  answer,
  busy,
  copyStatus,
  error,
  step,
  onAnswerChange,
  onCopy,
  onSubmit,
}: {
  answer: string;
  busy: boolean;
  copyStatus: CopyStatus;
  error: string;
  step: ManualStep;
  onAnswerChange(value: string): void;
  onCopy(): void;
  onSubmit(): void;
}) {
  const bytes = answerBytes(answer);
  const overLimit = bytes > ANSWER_LIMIT_BYTES;
  const canSubmit = Boolean(answer.trim()) && !overLimit && !busy;
  const completedTasks = step.taskNumber - 1;

  return (
    <main className="page run-page manual-task-page">
      <header className="progress-copy">
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
      </section>
    </main>
  );
}

function ManualWizard({
  kind,
}: {
  kind: ClientTargetKind;
}) {
  const backend = useBackend();
  const navigate = useNavigate();
  const mounted = useRef(true);
  const pending = useRef(false);
  const [model, setModel] = useState("");
  const [modelTouched, setModelTouched] = useState(false);
  const [reasoningEffort, setReasoningEffort] =
    useState<ReasoningEffort>("");
  const [freshChat, setFreshChat] = useState(false);
  const [state, setState] = useState<WizardState>({
    kind: "setup",
    error: "",
  });

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      pending.current = false;
    };
  }, []);

  function beginOperation(): boolean {
    if (pending.current) {
      return false;
    }
    pending.current = true;
    return true;
  }

  function completeNavigation(run: RunRecord) {
    pending.current = false;
    navigate(`/results/${run.id}`, {
      replace: true,
      state: { manualRunCompleted: true },
    });
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
        return;
      }
      showStep(run, step);
    } catch (reason) {
      if (!mounted.current) {
        return;
      }
      pending.current = false;
      setState({
        kind: "first-step-error",
        run,
        error: errorMessage(reason),
      });
    }
  }

  async function start() {
    const modelError = validateModel(model);
    setModelTouched(true);
    if (modelError || !freshChat || state.kind !== "setup") {
      return;
    }
    if (!beginOperation()) {
      return;
    }

    setState({ kind: "starting" });
    let run: RunRecord;
    try {
      run = await Promise.resolve().then(() =>
        backend.startManualRun({
          target: {
            kind,
            reportedModel: model.trim(),
            reasoningEffort: reasoningEffort || null,
          },
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

    if (!mounted.current) {
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
        return;
      }
      showStep(run, step);
    } catch (reason) {
      if (!mounted.current) {
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

  if (state.kind === "setup" || state.kind === "starting") {
    return (
      <SetupPage
        busy={state.kind === "starting"}
        error={state.kind === "setup" ? state.error : ""}
        freshChat={freshChat}
        kind={kind}
        model={model}
        modelTouched={modelTouched}
        onFreshChatChange={setFreshChat}
        onModelChange={(value) => {
          setModelTouched(true);
          setModel(value);
        }}
        onReasoningEffortChange={setReasoningEffort}
        onStart={() => void start()}
        reasoningEffort={reasoningEffort}
      />
    );
  }

  if (state.kind === "loading-first") {
    return <PendingStepPage first run={state.run} />;
  }

  if (state.kind === "first-step-error") {
    return (
      <PendingStepPage
        error={state.error}
        first
        onRetry={() => void retryFirstStep()}
        run={state.run}
      />
    );
  }

  if (state.kind === "loading-next") {
    return <PendingStepPage first={false} run={state.run} />;
  }

  if (state.kind === "next-step-error") {
    return (
      <PendingStepPage
        error={state.error}
        first={false}
        onRetry={() => void retryNextStep()}
        run={state.run}
      />
    );
  }

  return (
    <TaskPage
      answer={state.answer}
      busy={state.kind === "submitting"}
      copyStatus={state.copyStatus}
      error={state.kind === "answering" ? state.submitError : ""}
      onAnswerChange={(answer) =>
        setState((current) =>
          current.kind === "answering"
            ? { ...current, answer, submitError: "" }
            : current,
        )
      }
      onCopy={() => void copyPrompt()}
      onSubmit={() => void submit()}
      step={state.step}
    />
  );
}

export function ManualRunPage() {
  const { target = "" } = useParams();

  if (!isClientTarget(target)) {
    return (
      <main className="page placeholder-page unsupported-manual-page">
        <p className="eyebrow">不支持的地址</p>
        <h1>不支持的客户端体检</h1>
        <p>这个地址不是 ChatGPT 或 Claude 客户端体检。</p>
        <Link to="/">返回开始页</Link>
      </main>
    );
  }

  return <ManualWizard key={target} kind={target} />;
}
