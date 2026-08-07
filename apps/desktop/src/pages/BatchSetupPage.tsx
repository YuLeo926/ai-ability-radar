import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type { ModelSource, TargetKind } from "../api/backend";
import { ClientSelectionPanel } from "../components/ClientSelectionPanel";
import { ReasoningEffortField } from "../components/ReasoningEffortField";
import type {
  BatchEstimate,
  BatchPlanInput,
  BatchTargetInput,
} from "../domain/batch";
import { reportedModelError } from "../domain/reportedModel";
import "./BatchPages.css";

type ClientKind = Extract<
  TargetKind,
  "chat_gpt_client" | "claude_client"
>;

interface TargetDraft {
  kind: ClientKind;
  label: string;
  model: string;
  reasoningEffort: string;
  reasoningError: string | null;
  source: ModelSource;
  dirty: boolean;
  detectionApplied: boolean;
}

type EstimateState =
  | { kind: "idle" }
  | { kind: "loading"; fingerprint: string }
  | { kind: "ready"; fingerprint: string; estimate: BatchEstimate }
  | { kind: "error"; fingerprint: string; message: string };

const initialTargets: TargetDraft[] = [
  {
    kind: "chat_gpt_client",
    label: "ChatGPT 客户端",
    model: "",
    reasoningEffort: "",
    reasoningError: null,
    source: "manual",
    dirty: false,
    detectionApplied: false,
  },
  {
    kind: "claude_client",
    label: "Claude 客户端",
    model: "",
    reasoningEffort: "",
    reasoningError: null,
    source: "manual",
    dirty: false,
    detectionApplied: false,
  },
];

function providerFamily(kind: ClientKind): "openai" | "anthropic" {
  return kind === "chat_gpt_client" ? "openai" : "anthropic";
}

function toTargetInput(draft: TargetDraft): BatchTargetInput {
  return {
    target: {
      kind: draft.kind,
      reportedModel: draft.model.trim(),
      reasoningEffort: draft.reasoningEffort.trim() || null,
      modelSource: draft.source,
      modelVerification: "user_confirmed",
    },
    executionSurface: "guided_client",
    executionAdapterIdentity: {
      executionSurface: "guided_client",
      providerFamily: providerFamily(draft.kind),
      launchKind: "guided_client",
      publicVersion: null,
      adapterContractVersion: "guided-client-v1",
    },
  };
}

function visibleError(reason: unknown): string {
  const value = reason instanceof Error ? reason.message : String(reason);
  return (
    value
      .replace(/[\u0000-\u001f\u007f-\u009f]/g, " ")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 240) || "无法生成本地扫描估算，请重试。"
  );
}

function durationMinutes(seconds: number): string {
  return `${Math.ceil(seconds / 60)} 分钟`;
}

function TargetEditor({
  draft,
  index,
  onChange,
}: {
  draft: TargetDraft;
  index: number;
  onChange(index: number, patch: Partial<TargetDraft>): void;
}) {
  const modelError = reportedModelError(draft.model);
  const modelErrorId = `batch-model-${index}-error`;

  return (
    <article className="batch-target-editor">
      <header className="batch-target-header">
        <div>
          <p className="batch-coordinate">目标 {String(index + 1).padStart(2, "0")}</p>
          <h2>{draft.label}</h2>
        </div>
        <span className="batch-surface-tag">用户引导</span>
      </header>

      <div className="batch-target-fields">
        <label className="field" htmlFor={`batch-model-${index}`}>
          <span>客户端当前显示的模型</span>
          <input
            aria-describedby={modelError ? modelErrorId : undefined}
            aria-invalid={modelError ? "true" : undefined}
            autoComplete="off"
            id={`batch-model-${index}`}
            onChange={(event) =>
              onChange(index, {
                model: event.target.value,
                source: "manual",
                dirty: true,
              })
            }
            placeholder={
              draft.kind === "chat_gpt_client"
                ? "例如 GPT-5.6"
                : "例如 Claude Sonnet 4.5"
            }
            value={draft.model}
          />
        </label>
        {modelError ? (
          <p className="form-error" id={modelErrorId} role="alert">
            {modelError}
          </p>
        ) : null}

        <ReasoningEffortField
          emptyLabel="未显示 / 不适用"
          id={`batch-reasoning-${index}`}
          kind={draft.kind}
          label="推理档位（按客户端原样记录）"
          onChange={(value) =>
            onChange(index, {
              reasoningEffort: value,
              source: "manual",
              dirty: true,
            })
          }
          onValidationChange={(reasoningError) =>
            onChange(index, { reasoningError })
          }
          value={draft.reasoningEffort}
        />
      </div>

      <ClientSelectionPanel
        detect={useBackend().detectClientSelection}
        edited={draft.detectionApplied && draft.dirty}
        enabled
        formDirty={draft.dirty}
        onApply={(selection) =>
          onChange(index, {
            ...(selection.model ? { model: selection.model } : {}),
            ...(selection.reasoningEffort
              ? { reasoningEffort: selection.reasoningEffort }
              : {}),
            source: "windows_accessibility",
            dirty: false,
            detectionApplied: true,
            reasoningError: null,
          })
        }
        target={draft.kind}
      />
    </article>
  );
}

export function BatchSetupPage() {
  const backend = useBackend();
  const navigate = useNavigate();
  const mounted = useRef(false);
  const requestId = useRef(0);
  const startPending = useRef(false);
  const seed = useRef(Date.now()).current;
  const [targets, setTargets] = useState(initialTargets);
  const [estimateState, setEstimateState] = useState<EstimateState>({
    kind: "idle",
  });
  const [acknowledged, setAcknowledged] = useState(false);
  const [startError, setStartError] = useState("");
  const [starting, setStarting] = useState(false);

  const plan = useMemo<BatchPlanInput | null>(() => {
    if (
      targets.some(
        (target) =>
          Boolean(reportedModelError(target.model)) ||
          Boolean(target.reasoningError),
      )
    ) {
      return null;
    }
    return {
      mode: "quick_comparison",
      seed,
      targets: targets.map(toTargetInput),
    };
  }, [seed, targets]);
  const fingerprint = plan ? JSON.stringify(plan) : "";
  const currentEstimate =
    estimateState.kind === "ready" &&
    estimateState.fingerprint === fingerprint
      ? estimateState.estimate
      : null;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      requestId.current += 1;
    };
  }, []);

  useEffect(() => {
    const id = ++requestId.current;
    setAcknowledged(false);
    setStartError("");
    if (!plan) {
      setEstimateState({ kind: "idle" });
      return;
    }
    setEstimateState({ kind: "loading", fingerprint });
    void Promise.resolve()
      .then(() => backend.estimateBatch(plan))
      .then((estimate) => {
        if (mounted.current && requestId.current === id) {
          setEstimateState({ kind: "ready", fingerprint, estimate });
        }
      })
      .catch((reason: unknown) => {
        if (mounted.current && requestId.current === id) {
          setEstimateState({
            kind: "error",
            fingerprint,
            message: visibleError(reason),
          });
        }
      });
  }, [backend, fingerprint, plan]);

  function updateTarget(index: number, patch: Partial<TargetDraft>) {
    setAcknowledged(false);
    setStartError("");
    setTargets((current) =>
      current.map((target, targetIndex) =>
        targetIndex === index ? { ...target, ...patch } : target,
      ),
    );
  }

  async function startBatch() {
    if (
      startPending.current ||
      !acknowledged ||
      !plan ||
      !currentEstimate
    ) {
      return;
    }
    startPending.current = true;
    setStarting(true);
    setStartError("");
    try {
      const batch = await backend.createAcknowledgedBatch({
        plan,
        estimateIssuedAt: currentEstimate.plan.costEstimate.issuedAt,
        acknowledgementHash: currentEstimate.plan.acknowledgementHash,
      });
      await backend.authorizeBatchExecution({
        batchId: batch.id,
        acknowledgementHash: batch.plan.acknowledgementHash,
      });
      await backend.startBatch(batch.id);
      if (mounted.current) {
        navigate(`/batch/${batch.id}`);
      }
    } catch (reason: unknown) {
      if (mounted.current) {
        startPending.current = false;
        setStarting(false);
        setAcknowledged(false);
        setStartError(visibleError(reason));
      }
    }
  }

  const staleEstimate =
    estimateState.kind === "ready" &&
    estimateState.fingerprint !== fingerprint;

  return (
    <main className="page batch-page batch-setup-page" id="page-content" tabIndex={-1}>
      <header className="batch-hero">
        <div>
          <p className="eyebrow">Phase B · 用户引导对比</p>
          <h1>建立一次可复核的双客户端扫描</h1>
          <p className="hero-summary">
            同一题包、同一评分规则、交错顺序。每道题都要求你确认使用了新的空白对话。
          </p>
        </div>
        <div aria-label="扫描方法" className="batch-method-stamp">
          <span>方法</span>
          <strong>Quick Comparison</strong>
          <small>2 个目标 · 每目标 1 次</small>
        </div>
      </header>

      <section aria-labelledby="batch-mode-title" className="batch-mode-panel">
        <div>
          <p className="section-kicker">扫描强度</p>
          <h2 id="batch-mode-title">当前阶段仅开放轻量对比</h2>
        </div>
        <div aria-label="批量扫描模式" className="batch-mode-options" role="radiogroup">
          <label>
            <input checked readOnly type="radio" />
            <span><strong>轻量对比</strong><small>每目标 1 轮</small></span>
          </label>
          <label aria-disabled="true">
            <input disabled type="radio" />
            <span><strong>标准</strong><small>后续阶段开放</small></span>
          </label>
          <label aria-disabled="true">
            <input disabled type="radio" />
            <span><strong>完整</strong><small>需可靠统计后开放</small></span>
          </label>
        </div>
      </section>

      <section aria-labelledby="batch-targets-title" className="batch-target-section">
        <header className="batch-section-heading">
          <div>
            <p className="section-kicker">目标坐标</p>
            <h2 id="batch-targets-title">确认两个客户端的实际选择</h2>
          </div>
          <p>自动识别结果只作辅助；最终以你确认的文字为准。</p>
        </header>
        <div className="batch-target-grid">
          {targets.map((target, index) => (
            <TargetEditor
              draft={target}
              index={index}
              key={target.kind}
              onChange={updateTarget}
            />
          ))}
        </div>
      </section>

      <aside className="batch-comparability-note">
        <strong>为什么这里不能混入 CLI？</strong>
        <span>
          客户端依赖用户确认的新对话，CLI 使用机器隔离的临时会话；两种执行面不能当作同条件样本直接比较。
        </span>
      </aside>

      <section aria-labelledby="batch-estimate-title" className="batch-estimate-panel">
        <div className="batch-estimate-copy">
          <p className="section-kicker">本地预算封印</p>
          <h2 id="batch-estimate-title">开始前核对预计消耗</h2>
          <p>
            维护者不承担费用。实际客户端使用会消耗你自己的订阅额度；本工具不接收登录凭据。
          </p>
        </div>

        {currentEstimate ? (
          <dl className="batch-cost-grid">
            <div><dt>目标</dt><dd>{currentEstimate.plan.costEstimate.targetCount}</dd></div>
            <div><dt>总任务交互</dt><dd>{currentEstimate.plan.costEstimate.guidedInteractions}</dd></div>
            <div><dt>预计最短</dt><dd>{durationMinutes(currentEstimate.plan.costEstimate.expectedElapsedSecsMin)}</dd></div>
            <div><dt>预计最长</dt><dd>{durationMinutes(currentEstimate.plan.costEstimate.expectedElapsedSecsMax)}</dd></div>
          </dl>
        ) : (
          <div className="batch-estimate-state" role="status">
            {estimateState.kind === "loading"
              ? "模型或档位已变化，正在重新计算本地估算…"
              : estimateState.kind === "error"
                ? estimateState.message
                : "填写两个客户端的模型后生成估算。"}
          </div>
        )}

        {staleEstimate ? (
          <p className="form-error" role="alert">估算已过期，请等待重新计算。</p>
        ) : null}
        <label className="batch-acknowledgement">
          <input
            checked={acknowledged}
            disabled={!currentEstimate || starting}
            onChange={(event) => setAcknowledged(event.target.checked)}
            type="checkbox"
          />
          <span>
            我已核对这次扫描的目标、题量和预计时间，并接受使用自己的订阅额度。
          </span>
        </label>
        {startError ? <p className="form-error" role="alert">{startError}</p> : null}
        <button
          disabled={!currentEstimate || !acknowledged || starting}
          onClick={() => void startBatch()}
          type="button"
        >
          {starting ? "正在建立本地扫描…" : "确认并建立扫描"}
        </button>
      </section>
    </main>
  );
}
