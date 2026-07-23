import { useEffect, useId, useRef, useState } from "react";
import type {
  ClientSelectionCandidate,
  ClientSelectionDetection,
} from "../api/backend";

export const CLIENT_AUTO_DETECT_KEY =
  "ai-ability-radar.client-selection-auto-detect";

const surfaceLabels = {
  chatgpt: "ChatGPT",
  codex_desktop: "Codex",
  claude: "Claude",
} as const;

const detectionSourceLabel = "Windows 可访问性";
const confidenceLabels = {
  visible_selector: "可见选择器",
  best_effort: "最佳努力",
} as const;

type ClientTarget = "chat_gpt_client" | "claude_client";
type AppliedSelection = {
  model?: string;
  reasoningEffort?: string;
};
type DetectSelection = (
  target: ClientTarget,
) => Promise<ClientSelectionDetection>;
type PendingAutomaticDetection = {
  key: ClientTarget;
  promise: Promise<ClientSelectionDetection>;
};

type PanelState =
  | { kind: "idle"; message: string }
  | { kind: "loading"; message: string }
  | {
      kind: "single";
      candidate: ClientSelectionCandidate;
      message: string;
      requiresApply: boolean;
    }
  | {
      kind: "multiple";
      candidates: ClientSelectionCandidate[];
      message: string;
      selectedKey: string | null;
    };

function readAutoDetectionPreference(): boolean {
  try {
    return window.localStorage.getItem(CLIENT_AUTO_DETECT_KEY) !== "false";
  } catch {
    return true;
  }
}

function candidateKey(candidate: ClientSelectionCandidate): string {
  return JSON.stringify([
    candidate.model ?? null,
    candidate.reasoningEffort ?? null,
    candidate.surface,
    candidate.source,
    candidate.confidence,
  ]);
}

function distinctCandidates(
  candidates: ClientSelectionCandidate[],
): ClientSelectionCandidate[] {
  const seen = new Set<string>();
  return candidates.filter((candidate) => {
    const key = candidateKey(candidate);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function appliedSelection(
  candidate: ClientSelectionCandidate,
): AppliedSelection {
  return {
    ...(candidate.model ? { model: candidate.model } : {}),
    ...(candidate.reasoningEffort
      ? { reasoningEffort: candidate.reasoningEffort }
      : {}),
  };
}

function fallbackMessage(status: ClientSelectionDetection["status"]): string {
  switch (status) {
    case "not_running":
      return "未检测到正在运行的客户端，可手动填写";
    case "not_exposed":
      return "客户端没有公开当前选择，可手动填写";
    case "unsupported":
      return "当前系统不支持自动读取，可手动填写";
    case "timed_out":
      return "读取客户端选择超时，可手动填写";
    case "failed":
      return "无法读取客户端选择，可手动填写";
    default:
      return "无法读取客户端选择，可手动填写";
  }
}

function CandidateDetails({
  candidate,
}: {
  candidate: ClientSelectionCandidate;
}) {
  return (
    <span className="selection-candidate-copy">
      <span className="selection-candidate-primary">
        <span>{candidate.model ?? "模型未显示"}</span>
        <span aria-hidden="true"> · </span>
        <span>{candidate.reasoningEffort ?? "推理档位未显示"}</span>
        <span aria-hidden="true"> · </span>
        <span>{surfaceLabels[candidate.surface]}</span>
      </span>
      <span className="selection-candidate-meta">
        <span>{detectionSourceLabel}</span>
        <span aria-hidden="true"> · </span>
        <span>{confidenceLabels[candidate.confidence]}</span>
      </span>
    </span>
  );
}

export function ClientSelectionPanel({
  detect,
  edited,
  enabled,
  formDirty,
  onApply,
  target,
}: {
  detect: DetectSelection;
  edited: boolean;
  enabled: boolean;
  formDirty: boolean;
  onApply(value: AppliedSelection): void;
  target: ClientTarget;
}) {
  const [autoDetectionEnabled, setAutoDetectionEnabled] = useState(
    readAutoDetectionPreference,
  );
  const [state, setState] = useState<PanelState>({
    kind: "idle",
    message: enabled
      ? "正在准备读取客户端选择…"
      : "当前页面不进行自动读取，可手动填写",
  });
  const titleId = useId();
  const mountedRef = useRef(false);
  const requestIdRef = useRef(0);
  const pendingAutomaticDetectionRef =
    useRef<PendingAutomaticDetection | null>(null);
  const detectRef = useRef(detect);
  const enabledRef = useRef(enabled);
  const formDirtyRef = useRef(formDirty);
  const onApplyRef = useRef(onApply);
  const autoDetectionEnabledRef = useRef(autoDetectionEnabled);
  const targetRef = useRef(target);
  detectRef.current = detect;
  enabledRef.current = enabled;
  formDirtyRef.current = formDirty;
  onApplyRef.current = onApply;
  autoDetectionEnabledRef.current = autoDetectionEnabled;
  targetRef.current = target;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      requestIdRef.current += 1;
    };
  }, []);

  async function runDetection(reusePendingAutomaticRequest = false) {
    if (!enabled) return;
    const requestTarget = target;
    const requestId = ++requestIdRef.current;
    setState({
      kind: "loading",
      message: "正在读取客户端可见选择器…",
    });

    try {
      let promise: Promise<ClientSelectionDetection>;
      const pending = pendingAutomaticDetectionRef.current;
      if (
        reusePendingAutomaticRequest &&
        pending?.key === requestTarget
      ) {
        promise = pending.promise;
      } else {
        promise = Promise.resolve().then(() =>
          detectRef.current(requestTarget),
        );
        if (reusePendingAutomaticRequest) {
          const nextPending = { key: requestTarget, promise };
          pendingAutomaticDetectionRef.current = nextPending;
          void promise
            .finally(() => {
              if (
                pendingAutomaticDetectionRef.current === nextPending
              ) {
                pendingAutomaticDetectionRef.current = null;
              }
            })
            .catch(() => undefined);
        }
      }
      const result = await promise;
      if (
        !mountedRef.current ||
        requestId !== requestIdRef.current ||
        !enabledRef.current ||
        targetRef.current !== requestTarget ||
        (reusePendingAutomaticRequest &&
          !autoDetectionEnabledRef.current)
      ) {
        return;
      }

      if (result.status === "detected") {
        const candidate = result.candidates[0];
        if (!candidate) return;
        const requiresApply = formDirtyRef.current;
        setState({
          kind: "single",
          candidate,
          message: `已从 ${surfaceLabels[candidate.surface]} 客户端界面读取，待确认`,
          requiresApply,
        });
        if (!requiresApply) {
          onApplyRef.current(appliedSelection(candidate));
        }
        return;
      }

      if (result.status === "multiple") {
        setState({
          kind: "multiple",
          candidates: distinctCandidates(result.candidates),
          message: "识别到多个客户端选择，请选择后应用",
          selectedKey: null,
        });
        return;
      }

      setState({
        kind: "idle",
        message: fallbackMessage(result.status),
      });
    } catch {
      if (
        !mountedRef.current ||
        requestId !== requestIdRef.current ||
        !enabledRef.current ||
        targetRef.current !== requestTarget ||
        (reusePendingAutomaticRequest &&
          !autoDetectionEnabledRef.current)
      ) {
        return;
      }
      setState({
        kind: "idle",
        message: "无法读取客户端选择，可手动填写",
      });
    }
  }

  useEffect(() => {
    requestIdRef.current += 1;
    if (enabled && autoDetectionEnabled) {
      void runDetection(true);
    } else {
      pendingAutomaticDetectionRef.current = null;
      setState({
        kind: "idle",
        message: enabled
          ? "自动读取已关闭，可手动填写"
          : "当前页面不进行自动读取，可手动填写",
      });
    }
  }, [autoDetectionEnabled, enabled, target]);

  function updateAutoDetection(next: boolean) {
    try {
      window.localStorage.setItem(CLIENT_AUTO_DETECT_KEY, String(next));
    } catch {
      // Preference persistence is optional; the current page remains usable.
    }
    autoDetectionEnabledRef.current = next;
    setAutoDetectionEnabled(next);
  }

  function applySingle(candidate: ClientSelectionCandidate) {
    setState((current) =>
      current.kind === "single"
        ? { ...current, requiresApply: false }
        : current,
    );
    onApplyRef.current(appliedSelection(candidate));
  }

  function applySelected() {
    if (state.kind !== "multiple" || state.selectedKey === null) return;
    const selected = state.candidates.find(
      (candidate) => candidateKey(candidate) === state.selectedKey,
    );
    if (selected) {
      onApplyRef.current(appliedSelection(selected));
    }
  }

  return (
    <section
      aria-labelledby={titleId}
      className="client-selection-panel"
    >
      <div className="selection-heading">
        <div>
          <p className="section-kicker">本地辅助识别</p>
          <h2 id={titleId}>确认客户端当前选择</h2>
        </div>
        <button
          className="secondary"
          disabled={!enabled}
          onClick={() => void runDetection()}
          type="button"
        >
          重新识别
        </button>
      </div>

      <label className="selection-setting">
        <input
          checked={autoDetectionEnabled}
          onChange={(event) => updateAutoDetection(event.target.checked)}
          type="checkbox"
        />
        <span>进入设置页时自动读取客户端可见选择器</span>
      </label>

      <p className="selection-status" role="status">
        {state.message}
      </p>

      {state.kind === "single" ? (
        <div className="selection-candidate">
          <CandidateDetails candidate={state.candidate} />
        </div>
      ) : null}

      {state.kind === "multiple" ? (
        <fieldset
          aria-label="客户端识别结果"
          className="selection-options"
          role="radiogroup"
        >
          <legend className="sr-only">客户端识别结果</legend>
          {state.candidates.map((candidate) => {
            const key = candidateKey(candidate);
            return (
              <label className="selection-option" key={key}>
                <input
                  checked={state.selectedKey === key}
                  name="client-selection-candidate"
                  onChange={() =>
                    setState((current) =>
                      current.kind === "multiple"
                        ? { ...current, selectedKey: key }
                        : current,
                    )
                  }
                  type="radio"
                />
                <CandidateDetails candidate={candidate} />
              </label>
            );
          })}
        </fieldset>
      ) : null}

      {state.kind === "single" &&
      (state.requiresApply || formDirty || edited) ? (
        <button
          onClick={() => applySingle(state.candidate)}
          type="button"
        >
          应用识别结果
        </button>
      ) : null}

      {state.kind === "multiple" ? (
        <button
          disabled={state.selectedKey === null}
          onClick={applySelected}
          type="button"
        >
          应用识别结果
        </button>
      ) : null}

      {edited ? (
        <p className="selection-edited">
          用户已修改，请确认当前填写值
        </p>
      ) : null}
    </section>
  );
}
