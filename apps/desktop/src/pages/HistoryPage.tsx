import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import { isSafeRunRecordList } from "../api/runtimeValidation";
import type {
  DataSettings,
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

function resumePath(run: RunRecord): string {
  const route =
    run.target.kind === "chat_gpt_client" ||
    run.target.kind === "claude_client"
      ? "manual"
      : "cli";
  return `/${route}/${run.target.kind}?resume=${encodeURIComponent(run.id)}`;
}

function validDataSettings(value: DataSettings): boolean {
  return (
    (value.rawRetentionDays === null ||
      [7, 30, 90].includes(value.rawRetentionDays)) &&
    typeof value.cleanupPending === "boolean"
  );
}

function retentionValue(value: number | null): string {
  return value === null ? "forever" : String(value);
}

const BACKUP_CLEANUP_INCOMPLETE =
  "备份未完成，临时私密数据可能尚未清理；请关闭应用并联系支持。";

function backupFailureMessage(error: unknown): string {
  const message =
    typeof error === "string"
      ? error
      : error instanceof Error
        ? error.message
        : "";
  return message === BACKUP_CLEANUP_INCOMPLETE
    ? BACKUP_CLEANUP_INCOMPLETE
    : "无法完成本地备份，请稍后重试。";
}

function LocalDataControls() {
  const backend = useBackend();
  const mounted = useRef(false);
  const settingsGeneration = useRef(0);
  const operationGeneration = useRef(0);
  const retentionInFlight = useRef(false);
  const backupInFlight = useRef(false);
  const [settings, setSettings] = useState<DataSettings | null>(null);
  const [settingsError, setSettingsError] = useState(false);
  const [pendingRetention, setPendingRetention] = useState<number | null>(null);
  const [confirmRetention, setConfirmRetention] = useState(false);
  const [retentionBusy, setRetentionBusy] = useState(false);
  const [retentionMessage, setRetentionMessage] = useState("");
  const [backupAcknowledged, setBackupAcknowledged] = useState(false);
  const [backupBusy, setBackupBusy] = useState(false);
  const [backupMessage, setBackupMessage] = useState("");

  async function loadSettings(): Promise<DataSettings | null> {
    const generation = ++settingsGeneration.current;
    try {
      const loaded = await backend.getDataSettings();
      if (
        !mounted.current ||
        generation !== settingsGeneration.current ||
        !validDataSettings(loaded)
      ) {
        if (
          mounted.current &&
          generation === settingsGeneration.current &&
          !validDataSettings(loaded)
        ) {
          setSettingsError(true);
        }
        return null;
      }
      setSettings(loaded);
      setPendingRetention(loaded.rawRetentionDays);
      setSettingsError(false);
      return loaded;
    } catch {
      if (mounted.current && generation === settingsGeneration.current) {
        setSettingsError(true);
      }
      return null;
    }
  }

  useEffect(() => {
    mounted.current = true;
    void loadSettings();
    return () => {
      mounted.current = false;
      settingsGeneration.current += 1;
      operationGeneration.current += 1;
      retentionInFlight.current = false;
      backupInFlight.current = false;
    };
  }, [backend]);

  const shortening =
    settings !== null &&
    pendingRetention !== null &&
    (settings.rawRetentionDays === null ||
      pendingRetention < settings.rawRetentionDays);
  const changed =
    settings !== null && pendingRetention !== settings.rawRetentionDays;

  async function applyRetention() {
    if (
      !changed ||
      retentionInFlight.current ||
      backupInFlight.current
    ) {
      return;
    }
    if (shortening && !confirmRetention) {
      setConfirmRetention(true);
      return;
    }
    const generation = ++operationGeneration.current;
    retentionInFlight.current = true;
    setRetentionBusy(true);
    setConfirmRetention(false);
    setRetentionMessage("");
    try {
      await backend.setRawRetention(pendingRetention);
      const effective = await loadSettings();
      if (!mounted.current || generation !== operationGeneration.current) return;
      setRetentionMessage(
        effective?.cleanupPending
          ? "保留期限已生效，但原始数据清理尚未完成；稍后可重试。"
          : "原始数据保留期限已更新。",
      );
    } catch {
      const effective = await loadSettings();
      if (!mounted.current || generation !== operationGeneration.current) return;
      setRetentionMessage(
        effective?.cleanupPending
          ? "保留期限已生效，但原始数据清理尚未完成；稍后可重试。"
          : "无法完成本地数据设置操作，请稍后重试。",
      );
    } finally {
      retentionInFlight.current = false;
      if (mounted.current && generation === operationGeneration.current) {
        setRetentionBusy(false);
      }
    }
  }

  async function exportBackup() {
    if (
      !backupAcknowledged ||
      backupInFlight.current ||
      retentionInFlight.current
    ) {
      return;
    }
    const generation = ++operationGeneration.current;
    backupInFlight.current = true;
    setBackupBusy(true);
    setBackupMessage("");
    try {
      const exported = await backend.exportFullBackup({
        acknowledgedUnencryptedRawData: true,
      });
      if (!mounted.current || generation !== operationGeneration.current) return;
      if (exported) setBackupMessage("完整本地备份已导出。");
    } catch (error) {
      if (mounted.current && generation === operationGeneration.current) {
        setBackupMessage(backupFailureMessage(error));
      }
    } finally {
      backupInFlight.current = false;
      if (mounted.current && generation === operationGeneration.current) {
        setBackupBusy(false);
      }
    }
  }

  return (
    <section
      aria-labelledby="local-data-title"
      className="data-management local-data-management"
    >
      <p className="section-kicker">仅保存在本机</p>
      <h2 id="local-data-title">本地数据</h2>
      <p>
        到期只删除原始回答、CLI 日志和工作区副本；体检记录、任务证据、分数与摘要会保留。
      </p>
      {settingsError ? (
        <p role="alert">暂时无法读取本地数据设置，请稍后重试。</p>
      ) : null}
      {settings ? (
        <>
          <p className="data-status">
            当前生效：{settings.rawRetentionDays === null
              ? "永久保留"
              : `${settings.rawRetentionDays} 天`}
          </p>
          <label className="local-data-field">
            <span>原始数据保留期限</span>
            <select
              aria-label="原始数据保留期限"
              disabled={retentionBusy || backupBusy}
              onChange={(event) => {
                const value = event.currentTarget.value;
                setPendingRetention(
                  value === "forever" ? null : Number(value),
                );
                setConfirmRetention(false);
                setRetentionMessage("");
              }}
              value={retentionValue(pendingRetention)}
            >
              <option value="forever">永久（默认）</option>
              <option value="90">90 天</option>
              <option value="30">30 天</option>
              <option value="7">7 天</option>
            </select>
          </label>
          <button
            className="evidence-button"
            disabled={!changed || retentionBusy || backupBusy}
            onClick={() => void applyRetention()}
            type="button"
          >
            {retentionBusy ? "正在应用…" : "应用保留期限"}
          </button>
          {confirmRetention ? (
            <section
              aria-label="确认缩短原始数据保留期限"
              className="inline-confirmation"
              role="group"
            >
              <h3>确认缩短保留期限？</h3>
              <p>
                应用后会立即尝试清理已经到期的原始回答和日志，但不会删除分数或体检证据。
              </p>
              <div className="inline-confirmation-actions">
                <button
                  className="evidence-button secondary"
                  disabled={retentionBusy || backupBusy}
                  onClick={() => setConfirmRetention(false)}
                  type="button"
                >
                  取消
                </button>
                <button
                  className="evidence-button danger"
                  disabled={retentionBusy || backupBusy}
                  onClick={() => void applyRetention()}
                  type="button"
                >
                  确认应用并清理过期原始数据
                </button>
              </div>
            </section>
          ) : null}
          {settings.cleanupPending && !retentionMessage ? (
            <p className="data-status">
              保留期限已生效，但原始数据清理尚未完成；稍后可重试。
            </p>
          ) : null}
          {retentionMessage ? (
            <p className="data-status" role="status">
              {retentionMessage}
            </p>
          ) : null}
        </>
      ) : null}

      <div className="local-backup">
        <h3>完整本地备份</h3>
        <p>备份由原生保存窗口选择位置，ZIP 未加密，且不会上传。</p>
        <label className="report-confirmation">
          <input
            checked={backupAcknowledged}
            disabled={backupBusy || retentionBusy}
            onChange={(event) => {
              setBackupAcknowledged(event.currentTarget.checked);
              setBackupMessage("");
            }}
            type="checkbox"
          />
          <span>我知道此 ZIP 未加密，并包含原始回答和日志</span>
        </label>
        <button
          className="evidence-button"
          disabled={!backupAcknowledged || backupBusy || retentionBusy}
          onClick={() => void exportBackup()}
          type="button"
        >
          {backupBusy ? "正在准备备份…" : "导出完整本地备份"}
        </button>
        {backupMessage ? (
          <p
            className="data-status"
            role={backupMessage === "完整本地备份已导出。" ? "status" : "alert"}
          >
            {backupMessage}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function HistorySeries({
  group,
  index,
  onDeleted,
  targetRunIds,
}: {
  group: HistoryGroup;
  index: number;
  onDeleted(): void;
  targetRunIds: string[];
}) {
  const backend = useBackend();
  const representative = group.records[0];
  const titleId = `history-series-${index}`;
  const defaultRouting = representative.target.reportedModel === "default";
  const mounted = useRef(true);
  const deleting = useRef(false);
  const currentSnapshot = useRef(targetRunIds.join("\0"));
  const [confirmation, setConfirmation] = useState<string[] | null>(null);
  const [deleteError, setDeleteError] = useState("");
  const [deletePending, setDeletePending] = useState(false);
  currentSnapshot.current = targetRunIds.join("\0");

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      deleting.current = false;
    };
  }, []);

  async function deleteTargetRecords() {
    if (!confirmation || deleting.current) {
      return;
    }
    const expectedRunIds = [...confirmation];
    const snapshot = expectedRunIds.join("\0");
    deleting.current = true;
    setDeletePending(true);
    setDeleteError("");
    try {
      const deleted = await backend.deleteTargetHistory(
        representative.target.kind,
        expectedRunIds,
      );
      if (
        !mounted.current ||
        currentSnapshot.current !== snapshot
      ) {
        return;
      }
      if (deleted !== expectedRunIds.length) {
        setDeleteError(
          "未能确认删除全部历史，本页仍保留当前记录。请重新读取后再试。",
        );
        return;
      }
      onDeleted();
    } catch {
      if (
        mounted.current &&
        currentSnapshot.current === snapshot
      ) {
        setDeleteError(
          "无法删除该测试对象的历史，本页仍保留当前记录。请重新读取后再试。",
        );
      }
    } finally {
      deleting.current = false;
      if (mounted.current) {
        setDeletePending(false);
      }
    }
  }

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
              {run.status === "interrupted" ? (
                <Link to={resumePath(run)}>继续未完成体检</Link>
              ) : null}
              <Link to={`/results/${run.id}`}>查看本次结果</Link>
            </li>
          );
        })}
      </ol>

      <div className="history-data-controls">
        <button
          className="evidence-button danger-outline"
          disabled={deletePending || confirmation !== null}
          onClick={() => {
            setDeleteError("");
            setConfirmation([...targetRunIds]);
          }}
          type="button"
        >
          删除该测试对象全部历史
        </button>
        {confirmation ? (
          <section
            aria-label={`确认删除 ${targetLabels[representative.target.kind]} 全部历史`}
            className="inline-confirmation"
            role="group"
          >
            <h3>
              删除 {targetLabels[representative.target.kind]} 的全部本地历史？
            </h3>
            <p>
              将删除当前读取到的 {confirmation.length} 条记录及其原始数据。
              这不会影响其他测试对象，也不会承诺清除系统备份、同步副本或取证痕迹。
            </p>
            {deleteError ? (
              <p className="form-error" role="alert">
                {deleteError}
              </p>
            ) : null}
            <div className="inline-confirmation-actions">
              <button
                className="evidence-button secondary"
                disabled={deletePending}
                onClick={() => {
                  setConfirmation(null);
                  setDeleteError("");
                }}
                type="button"
              >
                取消
              </button>
              <button
                className="evidence-button danger"
                disabled={deletePending}
                onClick={() => void deleteTargetRecords()}
                type="button"
              >
                {deletePending
                  ? "正在删除…"
                  : `确认删除 ${confirmation.length} 条记录`}
              </button>
            </div>
          </section>
        ) : null}
      </div>
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
  const targetRunIds = useMemo(() => {
    const byTarget = new Map<TargetKind, Set<string>>();
    if (state.kind !== "ready") {
      return new Map<TargetKind, string[]>();
    }
    for (const run of state.runs) {
      const ids = byTarget.get(run.target.kind) ?? new Set<string>();
      ids.add(run.id);
      byTarget.set(run.target.kind, ids);
    }
    return new Map(
      [...byTarget.entries()].map(([target, ids]) => [
        target,
        [...ids].sort(compareCodeUnits),
      ]),
    );
  }, [state]);

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
      <main className="evidence-page history-page">
        <section
          aria-labelledby="history-empty-title"
          className="history-empty-state"
        >
          <p className="eyebrow">仅保存在本机</p>
          <h1 id="history-empty-title">还没有体检记录</h1>
          <p>完成一次客户端或 CLI 快速体检后，客观结果会出现在这里。</p>
          <Link className="evidence-button" to="/">
            开始第一次体检
          </Link>
        </section>
        <LocalDataControls />
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

      <LocalDataControls />

      <div className="history-series-list">
        {groups.map((group, index) => (
          <HistorySeries
            group={group}
            index={index}
            key={group.key}
            onDeleted={() => setAttempt((value) => value + 1)}
            targetRunIds={
              targetRunIds.get(group.records[0].target.kind) ?? []
            }
          />
        ))}
      </div>
    </main>
  );
}
