import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
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
  return run.target.reportedModel === "default"
    ? "默认路由（未固定）"
    : run.target.reportedModel;
}

function technicalValue(value: string | null | undefined): string {
  return value && value.length > 0 ? value : "未记录";
}

function ResultReady({ detail }: { detail: RunDetail }) {
  const { run, taskResults } = detail;
  const finalScore = run.status === "completed" ? run.score : null;
  const noScore = statusPresentation(run.status);

  return (
    <main className="evidence-page result-page">
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
            <dd>{technicalValue(run.target.reasoningEffort)}</dd>
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
  const [attempt, setAttempt] = useState(0);
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
      <main aria-busy="true" className="evidence-page evidence-state">
        <p aria-label="正在读取本地结果" role="status">
          正在读取本地结果…
        </p>
      </main>
    );
  }

  if (visibleState.kind === "error") {
    return (
      <main className="evidence-page evidence-state">
        <section aria-labelledby="result-error-title">
          <p className="eyebrow">本地结果</p>
          <h1 id="result-error-title">暂时无法读取结果</h1>
          <p role="alert">本地结果读取失败，请稍后重试。</p>
          <button
            className="evidence-button"
            onClick={() => setAttempt((value) => value + 1)}
            type="button"
          >
            重新读取
          </button>
        </section>
      </main>
    );
  }

  if (visibleState.kind === "not-found") {
    return (
      <main className="evidence-page evidence-state">
        <section aria-labelledby="result-missing-title">
          <p className="eyebrow">本地结果</p>
          <h1 id="result-missing-title">没有找到这次体检</h1>
          <p>这条记录可能已被删除，或内容不完整，无法安全显示。</p>
          <div className="evidence-actions">
            <Link className="evidence-button" to="/">
              返回开始页
            </Link>
            <Link className="evidence-button secondary" to="/history">
              查看历史记录
            </Link>
          </div>
        </section>
      </main>
    );
  }

  return <ResultReady detail={visibleState.detail} />;
}
