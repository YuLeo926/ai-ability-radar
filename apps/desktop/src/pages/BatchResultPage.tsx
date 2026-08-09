import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type { TargetKind } from "../api/backend";
import type {
  BatchAnalysis,
  BatchMemberStatus,
  ScanBatchRecord,
  ScanBatchTarget,
  TargetBatchAnalysis,
} from "../domain/batch";
import { formatReportedModel } from "../domain/reportedModel";
import "./BatchPages.css";

type LoadState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; batch: ScanBatchRecord; analysis: BatchAnalysis };

type EvidenceState =
  | "queued"
  | "running"
  | "completed"
  | "invalid"
  | "unavailable"
  | "insufficient";

const targetLabels: Record<TargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

const evidenceStateLabels: Record<EvidenceState, string> = {
  queued: "排队中",
  running: "运行中",
  completed: "证据已完成",
  invalid: "证据无效",
  unavailable: "目标不可用",
  insufficient: "证据不足",
};

const signalLabels = {
  insufficient_data: "证据不足",
  stable: "表现稳定",
  watch: "值得复测",
  likely_regression: "值得复测",
} as const;

const categoryLabels = {
  instruction_following: "指令遵循",
  logic: "逻辑",
  code_review: "代码审查",
  cli_coding: "CLI 编程",
} as const;

function safeError(reason: unknown): string {
  const raw = reason instanceof Error ? reason.message : String(reason);
  return (
    raw
      .replace(/[\u0000-\u001f\u007f-\u009f]/g, " ")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 240) || "无法读取这次批次结果。"
  );
}

function routeSourceLabel(target: ScanBatchTarget): string {
  switch (target.target.modelSource) {
    case "windows_accessibility":
      return "界面可见模型";
    case "cli_requested":
      return "请求模型";
    case "default_route":
      return "提供方默认路由";
    case "manual":
      return "用户确认模型";
    case "cli_reported":
      return "CLI 报告模型";
    case "legacy_unknown":
      return "历史来源未确认";
  }
}

function evidenceState(statuses: BatchMemberStatus[]): EvidenceState {
  if (statuses.some((status) => status === "running" || status === "launching")) {
    return "running";
  }
  if (statuses.some((status) => status === "planned" || status === "reserved")) {
    return "queued";
  }
  if (statuses.some((status) => status === "completed")) return "completed";
  if (statuses.some((status) => status === "invalid")) return "invalid";
  if (statuses.some((status) => status === "unavailable")) return "unavailable";
  return "insufficient";
}

function number(value: number | null | undefined, suffix = ""): string {
  return value == null ? "—" : `${value >= 0 ? "" : "−"}${Math.abs(value).toFixed(1)}${suffix}`;
}

function EvidenceButton({
  label,
  value,
  sampleCount,
  detail,
  selected,
  onSelect,
}: {
  label: string;
  value: string;
  sampleCount: number;
  detail: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      aria-expanded={selected}
      aria-label={`${label}：${value}；样本 ${sampleCount}；打开证据明细`}
      className="batch-matrix-cell"
      onClick={onSelect}
      type="button"
    >
      <strong>{value}</strong>
      <small>n = {sampleCount}</small>
      <span>{detail}</span>
    </button>
  );
}

function TargetMatrixRow({
  batch,
  target,
  position,
  analysis,
  selectedCell,
  onSelect,
}: {
  batch: ScanBatchRecord;
  target: ScanBatchTarget;
  position: number;
  analysis: TargetBatchAnalysis | undefined;
  selectedCell: string | null;
  onSelect: (id: string, title: string, body: string) => void;
}) {
  const members = batch.members.filter((member) => member.targetPosition === position);
  const completed = members.filter((member) => member.status === "completed").length;
  const state = evidenceState(members.map((member) => member.status));
  const candidateCount = analysis?.candidate?.count ?? analysis?.candidateMemberCount ?? completed;
  const baselineCount = analysis?.baseline?.count ?? analysis?.baselineBatchCount ?? 0;
  const signal = analysis?.signal ?? "insufficient_data";
  const cells = [
    {
      id: `${position}-candidate`,
      label: `${targetLabels[target.target.kind]} 当前中位数`,
      value: number(analysis?.candidate?.median),
      count: candidateCount,
      note: analysis?.candidate
        ? `MAD ${analysis.candidate.medianAbsoluteDeviation.toFixed(1)}`
        : evidenceStateLabels[state],
      body: `本批次纳入 ${candidateCount} 个完整成员；另有 ${analysis?.excludedCandidateMemberOrdinals.length ?? Math.max(0, members.length - completed)} 个成员未进入聚合。`,
    },
    {
      id: `${position}-baseline`,
      label: `${targetLabels[target.target.kind]} 历史基线`,
      value: number(analysis?.baseline?.median),
      count: baselineCount,
      note: analysis?.baseline
        ? `${analysis.baselineUtcDayCount} 个 UTC 日期`
        : "无兼容基线",
      body: `历史基线使用 ${analysis?.baselineBatchCount ?? 0} 个兼容批次，覆盖 ${analysis?.baselineUtcDayCount ?? 0} 个 UTC 日期。`,
    },
    {
      id: `${position}-delta`,
      label: `${targetLabels[target.target.kind]} 与基线差值`,
      value: number(analysis?.delta),
      count: candidateCount,
      note: analysis?.deltaConfidenceInterval
        ? `${Math.round(analysis.deltaConfidenceInterval.confidenceLevel * 100)}% CI`
        : "区间未形成",
      body: analysis?.deltaConfidenceInterval
        ? `置信区间 ${number(analysis.deltaConfidenceInterval.lower)} 至 ${number(analysis.deltaConfidenceInterval.upper)}；差值来自批次聚合，不是单条回答。`
        : "当前样本或兼容历史不足，未形成可解释的置信区间。",
    },
    {
      id: `${position}-signal`,
      label: `${targetLabels[target.target.kind]} 判读`,
      value: signalLabels[signal],
      count: candidateCount,
      note: `${analysis?.matchedTaskDeltas.length ?? 0} 道匹配题`,
      body: `判读版本 ${batch.plan.scoringRuleVersion}；共有 ${analysis?.matchedTaskDeltas.length ?? 0} 道题形成候选与历史的同题匹配差值。`,
    },
  ];

  return (
    <tr>
      <th scope="row">
        <span className="batch-target-coordinate">T{String(position + 1).padStart(2, "0")}</span>
        <strong>{targetLabels[target.target.kind]}</strong>
        <span>{formatReportedModel(target.target.kind, target.target.reportedModel)}</span>
        <small>{routeSourceLabel(target)}</small>
        <em className={`batch-evidence-state state-${state}`}>{evidenceStateLabels[state]}</em>
      </th>
      {cells.map((cell) => (
        <td key={cell.id}>
          <EvidenceButton
            detail={cell.note}
            label={cell.label}
            onSelect={() => onSelect(cell.id, cell.label, cell.body)}
            sampleCount={cell.count}
            selected={selectedCell === cell.id}
            value={cell.value}
          />
        </td>
      ))}
    </tr>
  );
}

export function BatchResultPage() {
  const backend = useBackend();
  const { batchId = "" } = useParams();
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const [detail, setDetail] = useState<{ id: string; title: string; body: string } | null>(null);
  const [exportState, setExportState] = useState<"idle" | "busy" | "done" | "error">("idle");
  const detailRef = useRef<HTMLElement>(null);

  useEffect(() => {
    let active = true;
    void Promise.all([backend.getBatch(batchId), backend.getBatchAnalysis(batchId)])
      .then(([batch, analysis]) => {
        if (!active) return;
        if (!batch) throw new Error("没有找到这次本地批次体检。");
        setState({ kind: "ready", batch, analysis });
      })
      .catch((reason: unknown) => {
        if (active) setState({ kind: "error", message: safeError(reason) });
      });
    return () => {
      active = false;
    };
  }, [backend, batchId]);

  useEffect(() => {
    if (detail) detailRef.current?.focus();
  }, [detail]);

  const analysisByTarget = useMemo(() => {
    if (state.kind !== "ready") return new Map<number, TargetBatchAnalysis>();
    return new Map(state.analysis.targets.map((target) => [target.targetPosition, target]));
  }, [state]);

  if (state.kind === "loading") {
    return <main className="page batch-state-page" id="page-content"><p role="status">正在整理批次证据矩阵…</p></main>;
  }
  if (state.kind === "error") {
    return <main className="page batch-state-page" id="page-content"><h1>无法打开批次结果</h1><p role="alert">{state.message}</p><Link className="button secondary" to="/history">返回历史记录</Link></main>;
  }

  const { batch, analysis } = state;
  const surface = batch.plan.costEstimate.executionSurface;
  const surfaceLabel = surface === "guided_client" ? "客户端证据矩阵" : "CLI 证据矩阵";
  const baseline = batch.baselineSnapshot;

  async function exportReport() {
    if (exportState === "busy") return;
    setExportState("busy");
    try {
      const reportId = await backend.exportPublicBatchReport(batch.id);
      setExportState(reportId ? "done" : "idle");
    } catch {
      setExportState("error");
    }
  }

  return (
    <main className="page batch-page batch-matrix-page" id="page-content" tabIndex={-1}>
      <header className="batch-matrix-hero">
        <div>
          <p className="eyebrow">批次结果 · 本地聚合证据</p>
          <h1>{surfaceLabel}</h1>
          <p className="hero-summary">每个数字都带样本量并可展开证据说明；客户端与 CLI 使用不同执行面，不会合并排名或直接比较。</p>
        </div>
        <div className="batch-signal-stamp" data-signal={analysis.signal}>
          <span>本轮判读</span>
          <strong>{signalLabels[analysis.signal]}</strong>
          <small>analysis v{analysis.analysisVersion}</small>
        </div>
      </header>

      <section aria-labelledby="batch-provenance-title" className="batch-provenance-strip">
        <div><p className="section-kicker">可比性护栏</p><h2 id="batch-provenance-title">{surfaceLabel}单独成组</h2><p>另一个执行面的成绩不会进入这张矩阵，也不会参与本轮基线。</p></div>
        <dl>
          <div><dt>题包</dt><dd>{batch.plan.suiteId} · {batch.plan.suiteVersion}</dd></div>
          <div><dt>题包哈希</dt><dd><code>{batch.plan.suiteContentSha256.slice(0, 12)}…</code></dd></div>
          <div><dt>评分规则</dt><dd>{batch.plan.scoringRuleVersion}</dd></div>
          <div><dt>隔离方式</dt><dd>{surface === "guided_client" ? "逐题用户确认新对话" : "机器强制新会话与独立工作区"}</dd></div>
        </dl>
      </section>

      <section aria-labelledby="batch-matrix-title" className="batch-matrix-panel">
        <div className="batch-matrix-heading">
          <div><p className="section-kicker">Evidence ledger</p><h2 id="batch-matrix-title">目标 × 聚合证据</h2></div>
          <p>{baseline ? `兼容基线已冻结 · ${baseline.selectedBatchIds.length} 个历史批次 · ${baseline.contentSha256.slice(0, 10)}…` : "未冻结兼容历史基线；当前仅展示本批次状态。"}</p>
        </div>
        <div className="batch-matrix-scroll" tabIndex={0} aria-label={`${surfaceLabel}，可横向滚动`}>
          <table className="batch-evidence-matrix">
            <caption>{surfaceLabel}；所有数据单元格都标明样本量并提供证据明细。</caption>
            <thead><tr><th scope="col">目标 / 路由</th><th scope="col">当前中位数</th><th scope="col">兼容基线</th><th scope="col">差值 / 区间</th><th scope="col">谨慎判读</th></tr></thead>
            <tbody>
              {batch.plan.targets.map((target, position) => (
                <TargetMatrixRow
                  analysis={analysisByTarget.get(position)}
                  batch={batch}
                  key={`${target.target.kind}-${position}`}
                  onSelect={(id, title, body) => setDetail({ id, title, body })}
                  position={position}
                  selectedCell={detail?.id ?? null}
                  target={target}
                />
              ))}
            </tbody>
          </table>
        </div>
      </section>

      {detail ? (
        <section aria-labelledby="batch-detail-title" className="batch-evidence-detail" ref={detailRef} tabIndex={-1}>
          <div><p className="section-kicker">证据明细</p><h2 id="batch-detail-title">{detail.title}</h2></div>
          <p>{detail.body}</p>
          <button className="button secondary" onClick={() => setDetail(null)} type="button">关闭明细</button>
        </section>
      ) : null}

      <section aria-labelledby="batch-category-title" className="batch-category-ledger">
        <div><p className="section-kicker">同题匹配</p><h2 id="batch-category-title">分类与题目证据</h2><p>只有题包、评分规则、路由、来源和执行适配器全部兼容的历史，才会进入基线。</p></div>
        <div className="batch-category-grid">
          {analysis.targets.flatMap((target) => target.matchedTaskDeltas).length === 0 ? (
            <p className="batch-empty-evidence">当前没有足够的同题历史差值；这不等于表现稳定。</p>
          ) : analysis.targets.flatMap((target) => target.matchedTaskDeltas).map((task) => (
            <article key={`${task.taskId}-${task.category}`}><span>{categoryLabels[task.category]}</span><strong>{task.taskId}</strong><p>候选 {task.candidateMedian.toFixed(1)} · 基线 {task.baselineMedian.toFixed(1)} · Δ {number(task.delta)}</p></article>
          ))}
        </div>
      </section>

      <footer className="batch-result-footer">
        <div><strong>公开导出不含原始回答</strong><p>仅导出聚合值、版本、哈希、样本量、排除计数与不确定性。</p></div>
        <div className="batch-result-actions">
          <button className="button" disabled={exportState === "busy" || batch.status !== "completed"} onClick={() => void exportReport()} type="button">{exportState === "busy" ? "正在导出…" : "导出匿名批次证据"}</button>
          <Link className="button secondary" to="/history">查看批次历史</Link>
        </div>
        {exportState === "done" ? <p role="status">匿名批次证据已导出。</p> : null}
        {exportState === "error" ? <p role="alert">无法导出；请确认批次已完整结束后重试。</p> : null}
      </footer>
    </main>
  );
}
