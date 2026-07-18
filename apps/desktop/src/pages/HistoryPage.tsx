import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import { isSafeRunRecordList } from "../api/runtimeValidation";
import type {
  RunRecord,
  RunStatus,
  TargetKind,
} from "../api/backend";
import "./ResultsHistory.css";

const targetLabels: Record<TargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

type HistoryState =
  | { kind: "loading" }
  | { kind: "error" }
  | { kind: "ready"; runs: RunRecord[] };

interface HistoryGroup {
  key: string;
  records: RunRecord[];
  newestTimestamp: number | null;
}

export function comparableSeriesKey(run: RunRecord): string {
  return JSON.stringify([
    run.target.kind,
    run.target.reportedModel,
    run.target.reasoningEffort ?? null,
    run.mode,
    run.suiteId,
    run.suiteVersion,
    run.environment.suiteId,
    run.environment.suiteVersion,
    run.environment.suiteContentSha256,
    run.environment.scoringRuleVersion,
    run.environment.osFamily,
    run.environment.osVersion,
    run.environment.appVersion,
    run.environment.cliVersion ?? null,
    run.environment.verifierRuntimeVersion ?? null,
    run.environment.resumed,
    run.totalTasks,
  ]);
}

function validTimestamp(value: string | null | undefined): number | null {
  if (!value) return null;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : null;
}

function recordTimestamp(run: RunRecord): number | null {
  const timestamps = [
    validTimestamp(run.startedAt),
    validTimestamp(run.finishedAt),
  ].filter((value): value is number => value !== null);
  return timestamps.length > 0 ? Math.max(...timestamps) : null;
}

function compareCodeUnits(left: string, right: string): number {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

function compareOptionalTimestampDescending(
  left: number | null,
  right: number | null,
): number {
  if (left === null) return right === null ? 0 : 1;
  if (right === null) return -1;
  if (left === right) return 0;
  return right - left;
}

function compareRecords(left: RunRecord, right: RunRecord): number {
  const timestampOrder = compareOptionalTimestampDescending(
    recordTimestamp(left),
    recordTimestamp(right),
  );
  return timestampOrder !== 0
    ? timestampOrder
    : compareCodeUnits(left.id, right.id);
}

function groupRuns(runs: RunRecord[]): HistoryGroup[] {
  const grouped = new Map<string, RunRecord[]>();
  for (const run of runs) {
    const key = comparableSeriesKey(run);
    const existing = grouped.get(key);
    if (existing) {
      existing.push(run);
    } else {
      grouped.set(key, [run]);
    }
  }

  return [...grouped.entries()]
    .map(([key, records]) => {
      const sortedRecords = [...records].sort(compareRecords);
      const newestTimestamp = records.reduce<number | null>(
        (newest, record) => {
          const timestamp = recordTimestamp(record);
          if (timestamp === null) return newest;
          return newest === null || timestamp > newest ? timestamp : newest;
        },
        null,
      );
      return {
        key,
        records: sortedRecords,
        newestTimestamp,
      };
    })
    .sort((left, right) => {
      const timestampOrder = compareOptionalTimestampDescending(
        left.newestTimestamp,
        right.newestTimestamp,
      );
      return timestampOrder !== 0
        ? timestampOrder
        : compareCodeUnits(left.key, right.key);
    });
}

function statusLabel(run: RunRecord): string {
  if (run.status === "completed" && run.score) {
    return `${run.score.abilityScore.toFixed(1)} 分`;
  }
  const labels: Record<RunStatus, string> = {
    created: "尚未开始",
    running: "进行中 · 尚未形成结果",
    completed: "已完成 · 没有可计分样本",
    cancelled: "已取消",
    interrupted: "已中断",
  };
  return labels[run.status];
}

function modelLabel(run: RunRecord): string {
  return run.target.reportedModel === "default"
    ? "默认路由（未固定）"
    : run.target.reportedModel;
}

function deterministicDate(value: string): {
  dateTime?: string;
  label: string;
} {
  const timestamp = validTimestamp(value);
  if (timestamp === null) {
    return { label: "时间记录无效" };
  }
  const date = new Date(timestamp);
  const pad = (part: number) => String(part).padStart(2, "0");
  return {
    dateTime: value,
    label:
      `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-` +
      `${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:` +
      `${pad(date.getUTCMinutes())} UTC`,
  };
}

function optionalFact(value: string | null | undefined): string {
  return value ?? "未记录";
}

function HistorySeries({
  group,
  index,
}: {
  group: HistoryGroup;
  index: number;
}) {
  const representative = group.records[0];
  const titleId = `history-series-${index}`;
  const defaultRouting = representative.target.reportedModel === "default";

  return (
    <section
      aria-label={`历史系列：${targetLabels[representative.target.kind]} · ${modelLabel(representative)}`}
      className="history-series"
      role="region"
    >
      <header className="history-series-header">
        <div>
          <p className="section-kicker">历史系列 {index + 1}</p>
          <h2 id={titleId}>
            {targetLabels[representative.target.kind]} ·{" "}
            {modelLabel(representative)}
          </h2>
        </div>
        <span className="record-count">{group.records.length} 次记录</span>
      </header>

      {defaultRouting ? (
        <p className="routing-warning">
          默认路由可能在服务侧切换实际模型，因此这里只按原样记录，不能视为固定模型。
        </p>
      ) : null}
      {representative.environment.resumed ? (
        <p className="resumed-marker">恢复运行 · 单独系列</p>
      ) : null}

      <details className="technical-details comparison-details">
        <summary>本系列比较条件</summary>
        <dl>
          <div>
            <dt>推理档位</dt>
            <dd>{optionalFact(representative.target.reasoningEffort)}</dd>
          </div>
          <div>
            <dt>模式</dt>
            <dd>{representative.mode === "quick" ? "快速体检" : "深度检测"}</dd>
          </div>
          <div>
            <dt>运行题包</dt>
            <dd>
              {representative.suiteId} · {representative.suiteVersion}
            </dd>
          </div>
          <div>
            <dt>环境题包</dt>
            <dd>
              {representative.environment.suiteId} ·{" "}
              {representative.environment.suiteVersion}
            </dd>
          </div>
          <div>
            <dt>内容封印</dt>
            <dd>
              {representative.environment.suiteContentSha256.slice(0, 12)}
            </dd>
          </div>
          <div>
            <dt>评分规则</dt>
            <dd>{representative.environment.scoringRuleVersion}</dd>
          </div>
          <div>
            <dt>系统</dt>
            <dd>
              {representative.environment.osFamily}{" "}
              {representative.environment.osVersion}
            </dd>
          </div>
          <div>
            <dt>应用</dt>
            <dd>{representative.environment.appVersion}</dd>
          </div>
          <div>
            <dt>CLI</dt>
            <dd>{optionalFact(representative.environment.cliVersion)}</dd>
          </div>
          <div>
            <dt>验证器</dt>
            <dd>
              {optionalFact(
                representative.environment.verifierRuntimeVersion,
              )}
            </dd>
          </div>
          <div>
            <dt>任务数</dt>
            <dd>{representative.totalTasks}</dd>
          </div>
        </dl>
      </details>

      <ol
        aria-label={`${targetLabels[representative.target.kind]} ${modelLabel(representative)} 历史记录`}
        className="history-records"
      >
        {group.records.map((run) => {
          const displayedDate = deterministicDate(run.startedAt);
          return (
            <li className="history-record" key={run.id}>
              <time
                dateTime={displayedDate.dateTime}
                role="time"
              >
                {displayedDate.label}
              </time>
              <strong>{statusLabel(run)}</strong>
              <Link to={`/results/${run.id}`}>查看本次结果</Link>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

export function HistoryPage() {
  const backend = useBackend();
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<HistoryState>({ kind: "loading" });

  useEffect(() => {
    let current = true;
    setState({ kind: "loading" });
    void Promise.resolve()
      .then(() => backend.listRuns())
      .then((runs) => {
        if (!current) return;
        setState(
          isSafeRunRecordList(runs)
            ? { kind: "ready", runs }
            : { kind: "error" },
        );
      })
      .catch(() => {
        if (current) setState({ kind: "error" });
      });
    return () => {
      current = false;
    };
  }, [attempt, backend]);

  const groups = useMemo(
    () => (state.kind === "ready" ? groupRuns(state.runs) : []),
    [state],
  );

  if (state.kind === "loading") {
    return (
      <main aria-busy="true" className="evidence-page evidence-state">
        <p aria-label="正在读取本地历史" role="status">
          正在读取本地历史…
        </p>
      </main>
    );
  }

  if (state.kind === "error") {
    return (
      <main className="evidence-page evidence-state">
        <section aria-labelledby="history-error-title">
          <p className="eyebrow">仅保存在本机</p>
          <h1 id="history-error-title">暂时无法读取历史</h1>
          <p role="alert">本地历史读取失败，请稍后重试。</p>
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

  if (groups.length === 0) {
    return (
      <main className="evidence-page evidence-state">
        <section aria-labelledby="history-empty-title">
          <p className="eyebrow">仅保存在本机</p>
          <h1 id="history-empty-title">还没有体检记录</h1>
          <p>完成一次客户端或 CLI 快速体检后，客观结果会出现在这里。</p>
          <Link className="evidence-button" to="/">
            开始第一次体检
          </Link>
        </section>
      </main>
    );
  }

  return (
    <main className="evidence-page history-page">
      <header className="evidence-hero">
        <p className="eyebrow">条件完全一致才放在同一组</p>
        <h1>严格同条件历史</h1>
        <p className="hero-summary">
          ChatGPT、Claude、Codex CLI 和 Claude Code 各自记录；配置、题包或
          运行环境不同也会另起一组。
        </p>
        <p>
          本页不跨系列合并分数，也不根据少量记录推断能力变化。
        </p>
      </header>

      <div className="history-series-list">
        {groups.map((group, index) => (
          <HistorySeries group={group} index={index} key={group.key} />
        ))}
      </div>
    </main>
  );
}
