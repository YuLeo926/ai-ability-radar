import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import { useT } from "../i18n/I18nContext";
import {
  isSafeRunDetail,
  scoreableResultScore,
} from "../api/runtimeValidation";
import type {
  Category,
  FailureKind,
  RunDetail,
  RunRecord,
  RunStatus,
  TargetKind,
  TaskResult,
} from "../api/backend";
import { CategoryBars } from "../components/CategoryBars";
import {
  formatModelProvenance,
  PUBLIC_REPORT_SCHEMA_VERSION,
} from "../domain/modelProvenance";
import { formatReasoningEffort } from "../domain/reasoningEffort";
import { formatReportedModel } from "../domain/reportedModel";
import "./ResultsHistory.css";

const targetLabels: Record<TargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

const categoryLabels: Record<Category, string> = {
  instruction_following: "指令遵循",
  logic: "逻辑推理",
  code_review: "代码审查",
  cli_coding: "CLI 编码",
};

const categoryOrder: Category[] = [
  "instruction_following",
  "logic",
  "code_review",
  "cli_coding",
];

const outcomeOrder = ["passed", "failed", "invalid", "cancelled"] as const;
const failureOrder: FailureKind[] = [
  "cli_missing",
  "runtime_missing",
  "auth_expired",
  "quota_exhausted",
  "network",
  "user_cancelled",
  "app_interrupted",
  "infrastructure_timeout",
  "agent_budget_exceeded",
  "verifier_error",
  "wrong_answer",
];

const publicMethodology =
  "v0.2 不生成降智结论；仅展示本题包的客观结果，不是 IQ，也不代表模型退化。";

const publicFieldLabels: Record<string, string> = {
  reportedModel: "用户填写模型",
  reasoningEffort: "推理档位",
  modelSource: "模型来源",
  modelVerification: "模型核验",
  osFamily: "操作系统系列",
  appVersion: "应用版本",
  cliVersion: "CLI 版本",
  verifierRuntimeVersion: "Node / 验证器运行时",
  suiteId: "题包编号",
  suiteVersion: "题包版本",
  suiteContentSha256: "题包内容哈希",
  scoringRuleVersion: "评分规则版本",
};

type ResultState =
  | { kind: "loading"; requestedRunId: string }
  | { kind: "error"; requestedRunId: string }
  | { kind: "not-found"; requestedRunId: string }
  | { kind: "ready"; requestedRunId: string; detail: RunDetail };

function outcomeLabel(result: TaskResult): string {
  const scoreable = scoreableResultScore(result) !== null;

  switch (result.outcome) {
    case "passed":
      return scoreable ? "通过" : "记录不完整（不计入成绩）";
    case "failed":
      return scoreable
        ? "未通过（计入本题包成绩）"
        : "运行无效（不计入成绩）";
    case "invalid":
      return "运行无效（不计入成绩）";
    case "cancelled":
      return "未完成（不计入成绩）";
  }
}

function failureExplanation(failure: FailureKind): string {
  switch (failure) {
    case "cli_missing":
      return "未检测到所需 CLI，本题作为运行环境样本排除。";
    case "runtime_missing":
      return "本地验证器运行环境缺失，本题作为运行环境样本排除。";
    case "auth_expired":
      return "登录状态已失效，本题作为运行环境样本排除。";
    case "quota_exhausted":
      return "订阅额度不足，本题作为运行环境样本排除。";
    case "network":
      return "网络连接中断，本题作为运行环境样本排除。";
    case "user_cancelled":
      return "用户取消了运行，本题不进入成绩。";
    case "app_interrupted":
      return "应用或电脑中断了运行，本题作为运行环境样本排除。";
    case "infrastructure_timeout":
      return "运行基础设施未在时限内响应，本题作为运行环境样本排除。";
    case "agent_budget_exceeded":
      return "代理在固定代理预算内未完成，本题按客观规则计为未通过。";
    case "verifier_error":
      return "本地验证器未能完成，本题作为运行环境样本排除。";
    case "wrong_answer":
      return "答案未通过确定性检查，本题按客观规则计为未通过。";
  }
}

function statusPresentation(status: RunStatus): {
  heading: string;
  explanation: string;
} {
  switch (status) {
    case "created":
      return {
        heading: "体检尚未开始",
        explanation: "本地记录已经建立，但任务还没有开始。",
      };
    case "running":
      return {
        heading: "体检仍在进行",
        explanation: "本地记录显示任务仍在进行，最终结果尚未形成。",
      };
    case "completed":
      return {
        heading: "本次没有可计分样本",
        explanation:
          "没有题目形成可计分结果；登录、额度、网络和验证器问题只作为运行环境样本记录。",
      };
    case "cancelled":
      return {
        heading: "本次体检已取消",
        explanation: "取消或未完成的题目不会进入成绩，也不会作为能力失败。",
      };
    case "interrupted":
      return {
        heading: "本次体检被中断",
        explanation:
          "应用退出或电脑重启中断了运行；这些样本不会作为能力失败。",
      };
  }
}

function modelLabel(run: RunRecord): string {
  return formatReportedModel(
    run.target.kind,
    run.target.reportedModel,
  );
}

function technicalValue(value: string | null | undefined): string {
  return value && value.length > 0 ? value : "未记录";
}

function countBy<T extends string>(
  values: T[],
  order: readonly T[],
): string {
  const counts = new Map<T, number>();
  for (const value of values) {
    counts.set(value, (counts.get(value) ?? 0) + 1);
  }
  return order.map((value) => `${value} ${counts.get(value) ?? 0}`).join(" · ");
}

function totalDurationLabel(taskResults: TaskResult[]): string {
  const totalMs = taskResults.reduce(
    (sum, result) => sum + BigInt(result.durationMs),
    0n,
  );
  const seconds = totalMs / 1_000n;
  const tenths = (totalMs % 1_000n) / 100n;
  return `${seconds}.${tenths} 秒`;
}

function safeExportError(error: unknown): string {
  const message =
    typeof error === "string"
      ? error
      : error instanceof Error
        ? error.message
        : "";
  for (const [field, label] of Object.entries(publicFieldLabels)) {
    if (message.includes(`公开字段 ${field} `)) {
      return `无法导出：公开字段“${label}”可能包含敏感信息。`;
    }
  }
  return "无法导出报告，请检查公开字段后重试。";
}

function ReportExportControls({ detail }: { detail: RunDetail }) {
  const backend = useBackend();
  const { run, taskResults } = detail;
  const [open, setOpen] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const openerRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    if (open) titleRef.current?.focus();
  }, [open]);

  const restoreOpener = () => {
    openerRef.current?.focus();
  };

  const closeDialog = () => {
    if (busy) return;
    setOpen(false);
    setConfirmed(false);
    setError(null);
    restoreOpener();
  };

  const openDialog = () => {
    setConfirmed(false);
    setError(null);
    setStatus(null);
    setOpen(true);
  };

  const handleDialogKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      closeDialog();
      return;
    }
    if (event.key !== "Tab") return;

    const focusable = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), [href], [tabindex]:not([tabindex="-1"])',
      ) ?? [],
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (
      event.shiftKey &&
      (document.activeElement === first || document.activeElement === titleRef.current)
    ) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const exportReport = async () => {
    if (!confirmed || busy) return;
    setBusy(true);
    setError(null);
    try {
      const reportId = await backend.exportPublicReport(run.id);
      setOpen(false);
      setConfirmed(false);
      setStatus(
        reportId === null
          ? "已取消保存，未生成报告。"
          : `报告已导出。匿名报告编号：${reportId}`,
      );
      restoreOpener();
    } catch (cause) {
      setError(safeExportError(cause));
    } finally {
      setBusy(false);
    }
  };

  const score = run.score;
  const scoreCategories = categoryOrder
    .filter((category) => score?.categoryScores[category] !== undefined)
    .map(
      (category) =>
        `${category} ${score?.categoryScores[category]?.toFixed(1)}`,
    )
    .join(" · ");
  const outcomeCounts = countBy(
    taskResults.map((result) => result.outcome),
    outcomeOrder,
  );
  const failures = taskResults
    .map((result) => result.failureKind)
    .filter((failure): failure is FailureKind => failure != null);
  const failureCounts =
    failures.length === 0
      ? "无"
      : countBy(failures, failureOrder.filter((kind) => failures.includes(kind)));

  return (
    <section aria-labelledby="report-export-title" className="report-export">
      <div>
        <p className="section-kicker">完全离线 · 不会自动上传</p>
        <h2 id="report-export-title">导出可分享报告</h2>
        <p>
          先核对严格白名单，再由系统窗口选择一个新的本地 HTML 文件。
          v0.2 不会上传报告，也不会发布到 GitHub。
        </p>
      </div>
      <button
        className="evidence-button"
        onClick={openDialog}
        ref={openerRef}
        type="button"
      >
        检查并导出可分享报告
      </button>
      {status ? (
        <p aria-label="报告导出状态" className="export-status" role="status">
          {status}
        </p>
      ) : null}

      {open ? (
        <div className="report-review-backdrop">
          <div
            aria-labelledby="report-review-title"
            aria-modal="true"
            className="report-review-dialog"
            onKeyDown={handleDialogKeyDown}
            ref={dialogRef}
            role="dialog"
          >
            <header>
              <p className="section-kicker">发布前隐私检查</p>
              <h2 id="report-review-title" ref={titleRef} tabIndex={-1}>
                导出前检查
              </h2>
              <p>
                下面是报告会公开的全部字段。匿名报告编号和生成时间将在确认并选择位置后创建。
              </p>
            </header>

            <div className="report-review-columns">
              <div className="report-allowlist">
                <section aria-labelledby="public-meta-title">
                  <h3 id="public-meta-title">报告元数据</h3>
                  <dl>
                    <div>
                      <dt>格式</dt>
                      <dd>
                        报告格式版本 {PUBLIC_REPORT_SCHEMA_VERSION}
                      </dd>
                    </div>
                    <div>
                      <dt>新身份</dt>
                      <dd>新的匿名报告编号 · 生成时间</dd>
                    </div>
                  </dl>
                </section>

                <section aria-labelledby="public-target-title">
                  <h3 id="public-target-title">测试目标</h3>
                  <dl>
                    <div>
                      <dt>对象</dt>
                      <dd>{targetLabels[run.target.kind]}</dd>
                    </div>
                    <div>
                      <dt>用户填写模型</dt>
                      <dd>
                        {formatReportedModel(
                          run.target.kind,
                          run.target.reportedModel,
                        )}
                      </dd>
                    </div>
                    <div>
                      <dt>推理档位</dt>
                      <dd>
                        {formatReasoningEffort(
                          run.target.kind,
                          run.target.reasoningEffort?.trim(),
                        )}
                      </dd>
                    </div>
                    <div>
                      <dt>模型来源与核验</dt>
                      <dd>{formatModelProvenance(run.target)}</dd>
                    </div>
                  </dl>
                </section>

                <section aria-labelledby="public-repro-title">
                  <h3 id="public-repro-title">复现信息</h3>
                  <dl>
                    <div>
                      <dt>操作系统系列</dt>
                      <dd>{run.environment.osFamily}</dd>
                    </div>
                    <div>
                      <dt>应用版本</dt>
                      <dd>{run.environment.appVersion}</dd>
                    </div>
                    <div>
                      <dt>CLI 版本</dt>
                      <dd>{technicalValue(run.environment.cliVersion)}</dd>
                    </div>
                    <div>
                      <dt>Node / 验证器运行时</dt>
                      <dd>
                        {technicalValue(
                          run.environment.verifierRuntimeVersion,
                        )}
                      </dd>
                    </div>
                    <div>
                      <dt>题包</dt>
                      <dd>{run.suiteId} · {run.suiteVersion}</dd>
                    </div>
                    <div>
                      <dt>题包内容哈希</dt>
                      <dd>{run.environment.suiteContentSha256}</dd>
                    </div>
                    <div>
                      <dt>评分规则</dt>
                      <dd>{run.environment.scoringRuleVersion}</dd>
                    </div>
                    <div>
                      <dt>恢复状态</dt>
                      <dd>{run.environment.resumed ? "是（恢复运行）" : "否（完整运行）"}</dd>
                    </div>
                  </dl>
                </section>

                <section aria-labelledby="public-result-title">
                  <h3 id="public-result-title">客观结果</h3>
                  <dl>
                    <div>
                      <dt>运行状态</dt>
                      <dd>{run.status}</dd>
                    </div>
                    <div>
                      <dt>题包客观分</dt>
                      <dd>{score ? score.abilityScore.toFixed(1) : "无有效分"}</dd>
                    </div>
                    <div>
                      <dt>分类分数</dt>
                      <dd>{scoreCategories || "无可计分分类"}</dd>
                    </div>
                    <div>
                      <dt>通过 / 有效 / 总数</dt>
                      <dd>
                        {score?.passedTasks ?? 0} / {score?.validTasks ?? 0} /{" "}
                        {run.totalTasks}
                      </dd>
                    </div>
                    <div>
                      <dt>结果计数</dt>
                      <dd>{outcomeCounts}</dd>
                    </div>
                    <div>
                      <dt>失败分类计数</dt>
                      <dd>{failureCounts}</dd>
                    </div>
                    <div>
                      <dt>总耗时</dt>
                      <dd>{totalDurationLabel(taskResults)}</dd>
                    </div>
                  </dl>
                </section>

                <section aria-labelledby="public-method-title">
                  <h3 id="public-method-title">方法与解释边界</h3>
                  <dl>
                    <div>
                      <dt>解释状态</dt>
                      <dd>not_evaluated</dd>
                    </div>
                  </dl>
                  <p>{publicMethodology}</p>
                </section>
              </div>

              <aside aria-labelledby="excluded-fields-title" className="excluded-fields">
                <h3 id="excluded-fields-title">明确排除，不会写入报告</h3>
                <ul>
                  <li>原始回答</li>
                  <li>题目提示词</li>
                  <li>CLI 日志</li>
                  <li>逐题详情文本</li>
                  <li>用户名</li>
                  <li>主机名</li>
                  <li>操作系统构建号</li>
                  <li>绝对路径</li>
                  <li>本地 run ID</li>
                  <li>本地 task ID</li>
                  <li>相对 artifact / answer path</li>
                  <li>凭据</li>
                  <li>保存位置</li>
                </ul>
              </aside>
            </div>

            {error ? (
              <p className="export-error" role="alert">
                {error}
              </p>
            ) : null}

            <label className="report-confirmation">
              <input
                checked={confirmed}
                disabled={busy}
                onChange={(event) => setConfirmed(event.target.checked)}
                type="checkbox"
              />
              <span>我已检查以上公开字段</span>
            </label>
            <div className="report-dialog-actions">
              <button
                className="evidence-button secondary"
                disabled={busy}
                onClick={closeDialog}
                type="button"
              >
                取消
              </button>
              <button
                aria-busy={busy}
                className="evidence-button"
                disabled={!confirmed || busy}
                onClick={() => void exportReport()}
                type="button"
              >
                {busy ? "正在打开系统保存窗口…" : "选择位置并导出"}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}

function DataManagementControls({
  run,
  onRawDeleted,
}: {
  run: RunRecord;
  onRawDeleted(): void;
}) {
  const backend = useBackend();
  const navigate = useNavigate();
  const mounted = useRef(true);
  const operationPending = useRef(false);
  const [choice, setChoice] = useState<"raw" | "run" | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      operationPending.current = false;
    };
  }, []);

  function closeConfirmation() {
    if (operationPending.current) {
      return;
    }
    setChoice(null);
    setError("");
  }

  async function confirmDeletion() {
    if (!choice || operationPending.current) {
      return;
    }
    const requestedChoice = choice;
    operationPending.current = true;
    setBusy(true);
    setError("");
    try {
      if (requestedChoice === "raw") {
        await backend.deleteRawArtifacts(run.id);
        if (!mounted.current) {
          return;
        }
        setChoice(null);
        onRawDeleted();
        return;
      }

      const deleted = await backend.deleteRun(run.id);
      if (!mounted.current) {
        return;
      }
      if (!deleted) {
        setError(
          "未能确认删除本次记录，当前结果仍然保留。请重新读取后再试。",
        );
        return;
      }
      navigate("/history", { replace: true });
    } catch {
      if (mounted.current) {
        setError(
          requestedChoice === "raw"
            ? "无法删除原始数据，当前记录仍然保留。请稍后重试。"
            : "无法删除本次记录，当前结果仍然保留。请稍后重试。",
        );
      }
    } finally {
      operationPending.current = false;
      if (mounted.current) {
        setBusy(false);
      }
    }
  }

  return (
    <details className="technical-details data-management">
      <summary>数据管理</summary>
      <p>
        删除只作用于应用管理的本地数据。系统备份、外部同步副本和取证痕迹不在应用可验证范围内。
      </p>

      <div className="data-management-actions">
        <button
          className="evidence-button danger-outline"
          disabled={busy || choice !== null}
          onClick={() => {
            setError("");
            setChoice("raw");
          }}
          type="button"
        >
          只删除原始回答和 CLI 日志，保留分数
        </button>
          <button
            className="evidence-button danger"
          disabled={busy || choice !== null}
          onClick={() => {
            setError("");
            setChoice("run");
            }}
            type="button"
          >
            从应用中删除本次记录和原始数据
          </button>
      </div>

      {choice ? (
        <section
          aria-label={
            choice === "raw"
              ? "确认只删除原始数据"
              : "确认删除本次记录"
          }
          className="inline-confirmation"
          role="group"
        >
          <h3>
            {choice === "raw"
              ? "只删除原始回答和 CLI 日志？"
              : "删除本次记录和原始数据？"}
          </h3>
          <p>
            {choice === "raw"
              ? "分数、逐题客观证据和运行条件会继续保留；原始回答、CLI 日志及工作区将被删除。"
              : "本次分数、逐题证据、运行记录以及应用管理的原始数据都会被删除。"}
          </p>
          {error ? (
            <p className="form-error" role="alert">
              {error}
            </p>
          ) : null}
          <div className="inline-confirmation-actions">
            <button
              className="evidence-button secondary"
              disabled={busy}
              onClick={closeConfirmation}
              type="button"
            >
              取消
            </button>
            <button
              className="evidence-button danger"
              disabled={busy}
              onClick={() => void confirmDeletion()}
              type="button"
            >
              {busy
                ? "正在删除…"
                : choice === "raw"
                  ? "确认只删除原始数据"
                  : "确认删除本次记录"}
            </button>
          </div>
        </section>
      ) : null}
    </details>
  );
}

function ResultReady({
  dataStatus,
  detail,
  onRawDeleted,
}: {
  dataStatus: string;
  detail: RunDetail;
  onRawDeleted(): void;
}) {
  const { run, taskResults } = detail;
  const finalScore = run.status === "completed" ? run.score : null;
  const noScore = statusPresentation(run.status);

  return (
    <main
      className="evidence-page result-page"
      id="page-content"
      tabIndex={-1}
    >
      <header className="evidence-hero">
        <p className="eyebrow">
          {targetLabels[run.target.kind]} · {modelLabel(run)}
        </p>
        <h1>{finalScore ? "本次客观结果" : noScore.heading}</h1>
        {!finalScore ? (
          <p className="hero-summary">{noScore.explanation}</p>
        ) : null}
      </header>

      <aside aria-labelledby="result-boundary-title" className="evidence-note">
        <h2 id="result-boundary-title">怎样理解这组数字</h2>
        <p>
          0–100 分只代表这个题包里的客观结果，不是 IQ，也不直接测量底层模型。
        </p>
        <p>
          v0.2 只展示本次证据和严格同条件历史；v0.5
          将在真实试运行校准后提供配对变化结论。
        </p>
      </aside>

      {dataStatus ? (
        <p aria-label={dataStatus} className="data-status" role="status">
          {dataStatus}
        </p>
      ) : null}

      {finalScore ? (
        <>
          <section aria-labelledby="score-summary-title">
            <div className="section-heading-row">
              <div>
                <p className="section-kicker">有效样本单独计分</p>
                <h2 id="score-summary-title">结果摘要</h2>
              </div>
            </div>
            <div aria-label="本次得分摘要" className="score-cards">
              <article className="score-card score-card-primary">
                <span>题包客观分</span>
                <strong>{finalScore.abilityScore.toFixed(1)}</strong>
                <small>
                  先计算各个有有效题目的分类分，再对这些分类等权平均
                </small>
              </article>
              <article className="score-card">
                <span>原始通过</span>
                <strong>
                  {finalScore.passedTasks} / {finalScore.validTasks}
                </strong>
                <small>
                  原始通过 {finalScore.passedTasks} /{" "}
                  {finalScore.validTasks}
                </small>
              </article>
              <article className="score-card">
                <span>有效覆盖</span>
                <strong>
                  {finalScore.validTasks} / {finalScore.totalTasks}
                </strong>
                <small>
                  有效覆盖 {finalScore.validTasks} /{" "}
                  {finalScore.totalTasks}
                </small>
              </article>
              <article className="score-card">
                <span>排除样本</span>
                <strong>
                  {finalScore.totalTasks - finalScore.validTasks}
                </strong>
                <small>
                  排除样本 {finalScore.totalTasks - finalScore.validTasks}
                </small>
              </article>
            </div>
          </section>

          <section aria-labelledby="category-title">
            <div className="section-heading-row">
              <div>
                <p className="section-kicker">只显示有有效题目的分类</p>
                <h2 id="category-title">分类分数</h2>
              </div>
            </div>
            <CategoryBars scores={finalScore.categoryScores} />
          </section>
        </>
      ) : null}

      {taskResults.length > 0 ? (
        <section aria-labelledby="task-evidence-title">
          <div className="section-heading-row">
            <div>
              <p className="section-kicker">不显示回答、路径或内部编号</p>
              <h2 id="task-evidence-title">逐题客观证据</h2>
            </div>
          </div>
          <ol aria-label="逐题客观证据" className="task-evidence-list">
            {taskResults.map((result, index) => {
              const score = scoreableResultScore(result);
              return (
                <li className="task-evidence-card" key={index}>
                  <div>
                    <span>第 {index + 1} 题</span>
                    <strong>{categoryLabels[result.category]}</strong>
                  </div>
                  <p className={`outcome outcome-${result.outcome}`}>
                    {outcomeLabel(result)}
                  </p>
                  <dl>
                    {score !== null ? (
                      <div>
                        <dt>单题得分</dt>
                        <dd>单题得分 {score.toFixed(1)}</dd>
                      </div>
                    ) : null}
                    <div>
                      <dt>耗时</dt>
                      <dd>耗时 {(result.durationMs / 1000).toFixed(1)} 秒</dd>
                    </div>
                  </dl>
                  {result.failureKind ? (
                    <p className="failure-explanation">
                      {failureExplanation(result.failureKind)}
                    </p>
                  ) : null}
                </li>
              );
            })}
          </ol>
        </section>
      ) : null}

      <details className="technical-details">
        <summary>技术与复现信息</summary>
        <dl>
          <div>
            <dt>测试对象</dt>
            <dd>{targetLabels[run.target.kind]}</dd>
          </div>
          <div>
            <dt>推理档位</dt>
            <dd>
              {formatReasoningEffort(
                run.target.kind,
                run.target.reasoningEffort,
              )}
            </dd>
          </div>
          <div>
            <dt>模型来源与核验</dt>
            <dd>{formatModelProvenance(run.target)}</dd>
          </div>
          <div>
            <dt>题包</dt>
            <dd>题包 {run.suiteId} · {run.suiteVersion}</dd>
          </div>
          <div>
            <dt>环境题包</dt>
            <dd>
              {run.environment.suiteId} · {run.environment.suiteVersion}
            </dd>
          </div>
          <div>
            <dt>内容封印</dt>
            <dd>
              内容封印 {run.environment.suiteContentSha256.slice(0, 12)}
            </dd>
          </div>
          <div>
            <dt>评分规则</dt>
            <dd>评分规则 {run.environment.scoringRuleVersion}</dd>
          </div>
          <div>
            <dt>应用</dt>
            <dd>应用 {run.environment.appVersion}</dd>
          </div>
          <div>
            <dt>系统</dt>
            <dd>
              系统 {run.environment.osFamily} {run.environment.osVersion}
            </dd>
          </div>
          {run.environment.cliVersion ? (
            <div>
              <dt>CLI</dt>
              <dd>{run.environment.cliVersion}</dd>
            </div>
          ) : null}
          {run.environment.verifierRuntimeVersion ? (
            <div>
              <dt>验证器</dt>
              <dd>验证器 {run.environment.verifierRuntimeVersion}</dd>
            </div>
          ) : null}
          <div>
            <dt>运行方式</dt>
            <dd>{run.environment.resumed ? "恢复运行" : "完整运行"}</dd>
          </div>
        </dl>
      </details>

      {run.status === "completed" ? (
        <ReportExportControls detail={detail} />
      ) : null}

      <DataManagementControls
        onRawDeleted={onRawDeleted}
        run={run}
      />

      <nav aria-label="结果操作" className="evidence-actions">
        <Link className="evidence-button" to="/">
          开始新的体检
        </Link>
        <Link className="evidence-button secondary" to="/history">
          查看历史记录
        </Link>
      </nav>
    </main>
  );
}

export function ResultPage() {
  const { runId = "" } = useParams();
  const backend = useBackend();
  const t = useT();
  const [attempt, setAttempt] = useState(0);
  const [dataStatus, setDataStatus] = useState<{
    message: string;
    runId: string;
  } | null>(null);
  const [state, setState] = useState<ResultState>({
    kind: "loading",
    requestedRunId: runId,
  });

  useEffect(() => {
    let current = true;
    setState({ kind: "loading", requestedRunId: runId });
    void Promise.resolve()
      .then(() => backend.getRunDetail(runId))
      .then((detail) => {
        if (!current) return;
        setState(
          isSafeRunDetail(detail) && detail.run.id === runId
            ? { kind: "ready", requestedRunId: runId, detail }
            : { kind: "not-found", requestedRunId: runId },
        );
      })
      .catch(() => {
        if (current) setState({ kind: "error", requestedRunId: runId });
      });
    return () => {
      current = false;
    };
  }, [attempt, backend, runId]);

  const visibleState: ResultState =
    state.requestedRunId === runId
      ? state
      : { kind: "loading", requestedRunId: runId };

  if (visibleState.kind === "loading") {
    return (
      <main
        aria-busy="true"
        className="evidence-page evidence-state"
        id="page-content"
        tabIndex={-1}
      >
        <p aria-label="正在读取本地结果" role="status">
          {t("result.loading")}
        </p>
      </main>
    );
  }

  if (visibleState.kind === "error") {
    return (
      <main
        className="evidence-page evidence-state"
        id="page-content"
        tabIndex={-1}
      >
        <section aria-labelledby="result-error-title">
          <p className="eyebrow">本地结果</p>
          <h1 id="result-error-title">暂时无法读取结果</h1>
          <p role="alert">本地结果读取失败，请稍后重试。</p>
          <button
            className="evidence-button"
            onClick={() => setAttempt((value) => value + 1)}
            type="button"
          >
            {t("common.reload")}
          </button>
        </section>
      </main>
    );
  }

  if (visibleState.kind === "not-found") {
    return (
      <main
        className="evidence-page evidence-state"
        id="page-content"
        tabIndex={-1}
      >
        <section aria-labelledby="result-missing-title">
          <p className="eyebrow">本地结果</p>
          <h1 id="result-missing-title">没有找到这次体检</h1>
          <p>这条记录可能已被删除，或内容不完整，无法安全显示。</p>
          <div className="evidence-actions">
            <Link className="evidence-button" to="/">
              {t("common.backHome")}
            </Link>
            <Link className="evidence-button secondary" to="/history">
              查看历史记录
            </Link>
          </div>
        </section>
      </main>
    );
  }

  return (
    <ResultReady
      dataStatus={
        dataStatus?.runId === runId ? dataStatus.message : ""
      }
      detail={visibleState.detail}
      onRawDeleted={() => {
        setDataStatus({
          message: "原始数据已删除，分数和客观证据已保留。",
          runId,
        });
        setAttempt((value) => value + 1);
      }}
    />
  );
}
