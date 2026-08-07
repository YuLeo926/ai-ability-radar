import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type { ManualStep } from "../api/backend";
import type {
  NextGuidedMember,
  ScanBatchMemberRecord,
  ScanBatchRecord,
  ScanBatchTarget,
} from "../domain/batch";
import { formatModelProvenance } from "../domain/modelProvenance";
import { formatReasoningEffort } from "../domain/reasoningEffort";
import { formatReportedModel } from "../domain/reportedModel";
import "./BatchPages.css";

const ANSWER_LIMIT_BYTES = 256 * 1024;

type ProgressState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | {
      kind: "ready";
      batch: ScanBatchRecord;
      next: NextGuidedMember;
    }
  | {
      kind: "starting";
      batch: ScanBatchRecord;
      next: NextGuidedMember;
    }
  | {
      kind: "task";
      batch: ScanBatchRecord;
      member: ScanBatchMemberRecord;
      target: ScanBatchTarget;
      runId: string;
      step: ManualStep;
      submitError: string;
    }
  | {
      kind: "ambiguous";
      batch: ScanBatchRecord;
      next: NextGuidedMember;
      message: string;
    };

const targetLabels = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
} as const;

const memberStatusLabels: Record<ScanBatchMemberRecord["status"], string> = {
  planned: "待开始",
  reserved: "已预留",
  launching: "正在启动",
  running: "进行中",
  deferred: "已延期",
  completed: "已完成",
  invalid: "证据无效",
  unavailable: "不可用",
  cancelled: "已取消",
};

function visibleError(reason: unknown, fallback: string): string {
  const value = reason instanceof Error ? reason.message : String(reason);
  return (
    value
      .replace(/[\u0000-\u001f\u007f-\u009f]/g, " ")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 240) || fallback
  );
}

function answerBytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function targetForMember(
  batch: ScanBatchRecord,
  member: ScanBatchMemberRecord,
): ScanBatchTarget | null {
  return batch.plan.targets[member.targetPosition] ?? null;
}

function TargetFacts({ target }: { target: ScanBatchTarget }) {
  return (
    <dl className="batch-target-facts">
      <div>
        <dt>目标</dt>
        <dd>{targetLabels[target.target.kind]}</dd>
      </div>
      <div>
        <dt>模型</dt>
        <dd>
          {formatReportedModel(
            target.target.kind,
            target.target.reportedModel,
          )}
        </dd>
      </div>
      <div>
        <dt>推理档位</dt>
        <dd>
          {formatReasoningEffort(
            target.target.kind,
            target.target.reasoningEffort,
            "未显示 / 不适用",
          )}
        </dd>
      </div>
      <div>
        <dt>记录来源</dt>
        <dd>{formatModelProvenance(target.target)}</dd>
      </div>
    </dl>
  );
}

function ScheduleRail({ batch }: { batch: ScanBatchRecord }) {
  return (
    <aside aria-labelledby="batch-schedule-title" className="batch-schedule-rail">
      <div className="batch-rail-heading">
        <p className="section-kicker">持久化顺序</p>
        <h2 id="batch-schedule-title">本轮扫描</h2>
        <p>
          {batch.terminalMemberCount} / {batch.plannedMemberCount} 个目标已结束
        </p>
      </div>
      <ol className="batch-schedule-list">
        {batch.members.map((member) => {
          const target = targetForMember(batch, member);
          return (
            <li
              className={`batch-member batch-member-${member.status}`}
              key={member.ordinal}
            >
              <span aria-hidden="true" className="batch-member-index">
                {String(member.ordinal + 1).padStart(2, "0")}
              </span>
              <span>
                <strong>
                  {target ? targetLabels[target.target.kind] : "未知目标"}
                </strong>
                <small>
                  {target?.target.reportedModel ?? "目标元数据不可用"}
                </small>
              </span>
              <em>{memberStatusLabels[member.status]}</em>
            </li>
          );
        })}
      </ol>
      <div className="batch-isolation-legend">
        <span aria-hidden="true" className="batch-attestation-dot" />
        <p>
          新对话隔离由<strong>用户逐题确认</strong>，不是机器验证。
        </p>
      </div>
    </aside>
  );
}

function BatchResult({ batch }: { batch: ScanBatchRecord }) {
  return (
    <main className="page batch-page batch-result-page" id="page-content" tabIndex={-1}>
      <header className="batch-result-hero">
        <p className="eyebrow">批次结果 · 本地证据</p>
        <span className="batch-result-code">insufficient_data</span>
        <h1>证据不足，暂不判断是否降智</h1>
        <p className="hero-summary">
          轻量对比每个目标只有 1 次运行，只适合检查流程和发现异常线索，不能形成稳定的退化结论。
        </p>
      </header>

      <section aria-labelledby="result-overview-title" className="batch-result-layout">
        <ScheduleRail batch={batch} />
        <div className="batch-result-summary">
          <p className="section-kicker">解释边界</p>
          <h2 id="result-overview-title">这一页明确没有下“降智”结论</h2>
          <ul>
            <li>客户端隔离来自用户确认，不是机器可验证证据。</li>
            <li>模型名称和推理档位是当时显示或用户确认的产品元数据。</li>
            <li>需要更多重复次数和可靠统计后，才可讨论趋势或回归。</li>
          </ul>
          <dl className="batch-result-counts">
            <div><dt>计划目标</dt><dd>{batch.plannedMemberCount}</dd></div>
            <div><dt>结束目标</dt><dd>{batch.terminalMemberCount}</dd></div>
            <div><dt>批次状态</dt><dd>{batch.status}</dd></div>
          </dl>
          <div className="batch-result-actions">
            <Link className="button" to="/batch/setup">建立新的轻量对比</Link>
            <Link className="button secondary" to="/history">查看单次运行记录</Link>
          </div>
        </div>
      </section>
    </main>
  );
}

export function BatchRunPage({ resultMode = false }: { resultMode?: boolean }) {
  const backend = useBackend();
  const navigate = useNavigate();
  const { batchId = "" } = useParams();
  const mounted = useRef(false);
  const requestId = useRef(0);
  const pending = useRef(false);
  const [state, setState] = useState<ProgressState>({ kind: "loading" });
  const [resultBatch, setResultBatch] = useState<ScanBatchRecord | null>(null);
  const [answer, setAnswer] = useState("");
  const [attested, setAttested] = useState(false);
  const [declineConfirm, setDeclineConfirm] = useState(false);

  const loadProgress = useCallback(async () => {
    const id = ++requestId.current;
    pending.current = false;
    setState({ kind: "loading" });
    setAnswer("");
    setAttested(false);
    setDeclineConfirm(false);
    try {
      const batch = await backend.getBatch(batchId);
      if (!batch) {
        throw new Error("没有找到这次本地扫描。请返回设置页重新建立。");
      }
      if (!mounted.current || requestId.current !== id) return;
      if (
        batch.status === "completed" ||
        batch.status === "cancelled" ||
        batch.terminalMemberCount === batch.plannedMemberCount
      ) {
        navigate(`/batch/${batch.id}/result`, { replace: true });
        return;
      }

      const next = await backend.getNextGuidedMember(batch.id);
      if (!mounted.current || requestId.current !== id) return;
      if (next.decision === "exhausted") {
        navigate(`/batch/${batch.id}/result`, { replace: true });
        return;
      }
      if (next.decision === "runnable") {
        setState({ kind: "ready", batch, next });
        return;
      }

      const member = next.member;
      const target = next.target;
      if (
        member?.status === "running" &&
        member.runId &&
        target
      ) {
        try {
          const step = await backend.nextManualStep(member.runId);
          if (!mounted.current || requestId.current !== id) return;
          if (step) {
            setState({
              kind: "task",
              batch,
              member,
              target,
              runId: member.runId,
              step,
              submitError: "",
            });
            return;
          }
        } catch (reason: unknown) {
          if (!mounted.current || requestId.current !== id) return;
          setState({
            kind: "ambiguous",
            batch,
            next,
            message: visibleError(
              reason,
              "无法读取已运行成员；不会自动重新打开或重复这次运行。",
            ),
          });
          return;
        }
      }

      setState({
        kind: "ambiguous",
        batch,
        next,
        message:
          "检测到已预留、启动中或中断的成员。为避免重复消耗与覆盖证据，本工具不会自动重新打开或重做。",
      });
    } catch (reason: unknown) {
      if (mounted.current && requestId.current === id) {
        setState({
          kind: "error",
          message: visibleError(reason, "无法读取这次本地扫描。"),
        });
      }
    }
  }, [backend, batchId, navigate]);

  const loadResult = useCallback(async () => {
    const id = ++requestId.current;
    try {
      const batch = await backend.getBatch(batchId);
      if (!batch) throw new Error("没有找到这次本地扫描。");
      if (mounted.current && requestId.current === id) {
        setResultBatch(batch);
      }
    } catch (reason: unknown) {
      if (mounted.current && requestId.current === id) {
        setState({
          kind: "error",
          message: visibleError(reason, "无法读取批次结果。"),
        });
      }
    }
  }, [backend, batchId]);

  useEffect(() => {
    mounted.current = true;
    if (resultMode) {
      void loadResult();
    } else {
      void loadProgress();
    }
    return () => {
      mounted.current = false;
      requestId.current += 1;
      pending.current = false;
    };
  }, [loadProgress, loadResult, resultMode]);

  async function beginMember() {
    if (state.kind !== "ready" || pending.current) return;
    pending.current = true;
    const snapshot = state;
    setState({ kind: "starting", batch: state.batch, next: state.next });
    try {
      const run = await backend.beginGuidedBatchMember(state.batch.id);
      const step = await backend.nextManualStep(run.id);
      if (!mounted.current) return;
      if (!step || !snapshot.next.member || !snapshot.next.target) {
        pending.current = false;
        await loadProgress();
        return;
      }
      pending.current = false;
      setState({
        kind: "task",
        batch: snapshot.batch,
        member: { ...snapshot.next.member, runId: run.id, status: "running" },
        target: snapshot.next.target,
        runId: run.id,
        step,
        submitError: "",
      });
    } catch (reason: unknown) {
      if (mounted.current) {
        pending.current = false;
        setState({
          kind: "ambiguous",
          batch: snapshot.batch,
          next: snapshot.next,
          message: visibleError(
            reason,
            "成员启动状态不明确；不会自动重新启动。请重新读取持久化状态。",
          ),
        });
      }
    }
  }

  async function submitAnswer() {
    if (
      state.kind !== "task" ||
      pending.current ||
      !attested ||
      !answer.trim() ||
      answerBytes(answer) > ANSWER_LIMIT_BYTES
    ) {
      return;
    }
    pending.current = true;
    const snapshot = state;
    setState({ ...state, submitError: "" });
    try {
      await backend.submitGuidedBatchAnswer({
        batchId: state.batch.id,
        memberOrdinal: state.member.ordinal,
        runId: state.runId,
        taskId: state.step.taskId,
        answer,
        userAttestedNewConversation: true,
      });
      if (!mounted.current) return;
      pending.current = false;
      setAnswer("");
      setAttested(false);
      setDeclineConfirm(false);
      await loadProgress();
    } catch (reason: unknown) {
      if (mounted.current) {
        pending.current = false;
        setState({
          kind: "ambiguous",
          batch: snapshot.batch,
          next: {
            decision: "blocked_by_active",
            member: snapshot.member,
            target: snapshot.target,
          },
          message: visibleError(
            reason,
            "本题保存状态不明确；不会自动重复提交。请重新读取持久化状态。",
          ),
        });
      }
    }
  }

  async function declineAttestation() {
    if (state.kind !== "task" || pending.current) return;
    pending.current = true;
    try {
      await backend.declineGuidedBatchAttestation({
        batchId: state.batch.id,
        memberOrdinal: state.member.ordinal,
        runId: state.runId,
        taskId: state.step.taskId,
      });
      if (mounted.current) {
        pending.current = false;
        await loadProgress();
      }
    } catch (reason: unknown) {
      if (mounted.current) {
        pending.current = false;
        setState({
          ...state,
          submitError: visibleError(reason, "无法记录本次拒绝，请重试。"),
        });
      }
    }
  }

  if (resultMode && resultBatch) {
    return <BatchResult batch={resultBatch} />;
  }

  if (state.kind === "loading" || (resultMode && !resultBatch)) {
    return (
      <main aria-busy="true" className="page batch-page batch-state-page" id="page-content" tabIndex={-1}>
        <p className="eyebrow">批量扫描 · 本地状态</p>
        <h1>正在读取持久化进度</h1>
        <p aria-live="polite" role="status">不会启动模型，也不会重复任何成员。</p>
      </main>
    );
  }

  if (state.kind === "error") {
    return (
      <main className="page batch-page batch-state-page" id="page-content" tabIndex={-1}>
        <p className="eyebrow">批量扫描 · 读取失败</p>
        <h1>无法读取这次扫描</h1>
        <p className="form-error" role="alert">{state.message}</p>
        <button onClick={() => void (resultMode ? loadResult() : loadProgress())} type="button">
          重新读取本地状态
        </button>
      </main>
    );
  }

  const batch = state.batch;

  return (
    <main className="page batch-page batch-run-page" id="page-content" tabIndex={-1}>
      <header className="batch-run-header">
        <div>
          <p className="eyebrow">Quick Comparison · 批次 {batch.id.slice(0, 8)}</p>
          <h1>按持久化顺序完成每个客户端</h1>
        </div>
        <span className="batch-live-status">
          <span aria-hidden="true" /> 本地批次 {batch.status}
        </span>
      </header>

      <div className="batch-console-layout">
        <ScheduleRail batch={batch} />

        <section aria-label="当前扫描步骤" className="batch-workspace">
          {state.kind === "ready" && state.next.member && state.next.target ? (
            <>
              <p className="batch-coordinate">
                下一坐标 · {String(state.next.member.ordinal + 1).padStart(2, "0")}
              </p>
              <h2>准备开始 {targetLabels[state.next.target.target.kind]}</h2>
              <TargetFacts target={state.next.target} />
              <div className="batch-user-boundary">
                <strong>点击后只建立本地运行</strong>
                <p>
                  工具不会控制客户端，也不会自动复制文字。你将在每道题前单独确认新空白对话。
                </p>
              </div>
              <button onClick={() => void beginMember()} type="button">
                开始这个目标
              </button>
            </>
          ) : null}

          {state.kind === "starting" ? (
            <div aria-live="polite" className="batch-workspace-state" role="status">
              <p className="batch-coordinate">正在锁定下一坐标</p>
              <h2>建立同一个持久化运行</h2>
              <p>不会创建第二条运行记录，请稍候…</p>
            </div>
          ) : null}

          {state.kind === "ambiguous" ? (
            <div className="batch-workspace-state batch-ambiguous-state">
              <p className="batch-coordinate">安全暂停</p>
              <h2>没有自动重开或重复</h2>
              <p role="alert">{state.message}</p>
              {state.next.member?.runId ? (
                <code>运行 {state.next.member.runId}</code>
              ) : null}
              <button className="secondary" onClick={() => void loadProgress()} type="button">
                重新读取持久化状态
              </button>
            </div>
          ) : null}

          {state.kind === "task" ? (
            <>
              <div className="batch-task-heading">
                <div>
                  <p className="batch-coordinate">
                    目标 {state.member.ordinal + 1} · 题目 {state.step.taskNumber} / {state.step.totalTasks}
                  </p>
                  <h2>在新的空白对话中完成本题</h2>
                </div>
                <span className="batch-user-attested">用户确认隔离</span>
              </div>
              <TargetFacts target={state.target} />
              <div className="batch-attestation-note">
                <strong>隔离证据边界</strong>
                <p>
                  本工具只记录你的逐题确认；它不会读取或机器验证客户端里的对话是否为空白。
                </p>
              </div>
              <pre aria-label="当前题目，可选中后手动复制" className="batch-prompt" tabIndex={0}>
                {state.step.prompt}
              </pre>
              <p className="batch-copy-boundary">
                请手动选中上方题目并复制。本页面不会自动访问剪贴板。
              </p>
              <label className="field batch-answer-field">
                <span>粘贴 AI 的完整原始回答</span>
                <textarea
                  aria-describedby="batch-answer-count"
                  aria-invalid={answerBytes(answer) > ANSWER_LIMIT_BYTES ? "true" : undefined}
                  disabled={pending.current}
                  onChange={(event) => setAnswer(event.target.value)}
                  rows={11}
                  value={answer}
                />
              </label>
              <p
                className={answerBytes(answer) > ANSWER_LIMIT_BYTES ? "byte-count byte-count-over" : "byte-count"}
                id="batch-answer-count"
              >
                {answerBytes(answer)} / {ANSWER_LIMIT_BYTES} 字节
              </p>
              <label className="batch-task-attestation">
                <input
                  checked={attested}
                  disabled={pending.current}
                  onChange={(event) => setAttested(event.target.checked)}
                  type="checkbox"
                />
                <span>
                  我确认：这道题是在刚新建的空白对话中完成，且未沿用上一题上下文。
                </span>
              </label>
              {state.submitError ? <p className="form-error" role="alert">{state.submitError}</p> : null}
              <div className="batch-task-actions">
                <button
                  disabled={
                    pending.current ||
                    !attested ||
                    !answer.trim() ||
                    answerBytes(answer) > ANSWER_LIMIT_BYTES
                  }
                  onClick={() => void submitAnswer()}
                  type="button"
                >
                  保存本题并读取下一步
                </button>
                {!declineConfirm ? (
                  <button
                    className="secondary"
                    disabled={pending.current}
                    onClick={() => setDeclineConfirm(true)}
                    type="button"
                  >
                    无法确认新空白对话
                  </button>
                ) : (
                  <div aria-label="确认放弃当前目标" className="batch-decline-confirm" role="group">
                    <p>当前目标将标记为证据无效，且本阶段不会原地重试。</p>
                    <button className="danger" onClick={() => void declineAttestation()} type="button">
                      确认标记为无效
                    </button>
                    <button className="secondary" onClick={() => setDeclineConfirm(false)} type="button">
                      返回本题
                    </button>
                  </div>
                )}
              </div>
            </>
          ) : null}
        </section>
      </div>
    </main>
  );
}
