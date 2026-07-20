import { useLayoutEffect } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  Link,
  MemoryRouter,
  Route,
  Routes,
  useLocation,
} from "react-router-dom";
import { describe, expect, test, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type {
  Backend,
  FailureKind,
  RunDetail,
  RunRecord,
  TaskResult,
} from "../api/backend";
import { CANONICAL_EFFORT_DISPLAY_CASES } from "../test/reasoningEffortCases";
import { ResultPage } from "./ResultPage";

const runId = "a8ecbc64-9160-448d-9426-e21c6839d219";

function makeRun(overrides: Partial<RunRecord> = {}): RunRecord {
  return {
    id: runId,
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-X",
      reasoningEffort: "high",
    },
    mode: "quick",
    suiteId: "client-quick",
    suiteVersion: "1.0.0",
    status: "completed",
    startedAt: "2026-07-17T00:00:00Z",
    finishedAt: "2026-07-17T00:12:00Z",
    totalTasks: 8,
    completedTasks: 8,
    environment: {
      osFamily: "Windows",
      osVersion: "11",
      appVersion: "0.2.0",
      cliVersion: null,
      verifierRuntimeVersion: "embedded-verifier 1.0.0",
      suiteId: "client-quick",
      suiteVersion: "1.0.0",
      suiteContentSha256: "e".repeat(64),
      scoringRuleVersion: "ability-v1",
      resumed: false,
    },
    score: {
      abilityScore: 66.7,
      passedTasks: 5,
      validTasks: 6,
      totalTasks: 8,
      categoryScores: {
        instruction_following: 100,
        logic: 100,
        code_review: 0,
      },
    },
    ...overrides,
  };
}

function makeTask(
  index: number,
  overrides: Partial<TaskResult> = {},
): TaskResult {
  return {
    runId,
    taskId: `private-task-${index}`,
    category: "logic",
    outcome: "passed",
    score: 100,
    failureKind: null,
    durationMs: 1_250,
    answerRelPath: `answers/private-${index}.txt`,
    ...overrides,
  };
}

function canonicalTasks(detailRunId = runId): TaskResult[] {
  return [
    makeTask(1, { runId: detailRunId, category: "instruction_following" }),
    makeTask(2, { runId: detailRunId, category: "instruction_following" }),
    makeTask(3, { runId: detailRunId, category: "instruction_following" }),
    makeTask(4, { runId: detailRunId }),
    makeTask(5, { runId: detailRunId }),
    makeTask(6, {
      runId: detailRunId,
      outcome: "failed",
      score: 0,
      failureKind: "network",
    }),
    makeTask(7, {
      runId: detailRunId,
      category: "code_review",
      outcome: "failed",
      score: 0,
      failureKind: "wrong_answer",
    }),
    makeTask(8, {
      runId: detailRunId,
      category: "code_review",
      outcome: "invalid",
      score: 0,
      failureKind: "network",
    }),
  ];
}

function fullCoverageTasks(detailRunId = runId): TaskResult[] {
  return [
    makeTask(1, { runId: detailRunId, category: "instruction_following" }),
    makeTask(2, { runId: detailRunId, category: "instruction_following" }),
    makeTask(3, {
      runId: detailRunId,
      category: "instruction_following",
      outcome: "failed",
      score: 0,
      failureKind: "wrong_answer",
    }),
    makeTask(4, { runId: detailRunId }),
    makeTask(5, { runId: detailRunId }),
    makeTask(6, {
      runId: detailRunId,
      outcome: "failed",
      score: 0,
      failureKind: null,
    }),
    makeTask(7, { runId: detailRunId, category: "code_review" }),
    makeTask(8, { runId: detailRunId, category: "code_review" }),
  ];
}

function makeDetail(
  runOverrides: Partial<RunRecord> = {},
  taskResults?: TaskResult[],
): RunDetail {
  const run = makeRun(runOverrides);
  return {
    run,
    taskResults: taskResults ?? canonicalTasks(run.id),
  };
}

function makeBackend(
  getRunDetail: Backend["getRunDetail"],
  exportPublicReport: Backend["exportPublicReport"] = async () => null,
): Backend {
  return {
    getBootstrap: async () => {
      throw new Error("unused fake getBootstrap");
    },
    startManualRun: async () => {
      throw new Error("unused fake startManualRun");
    },
    nextManualStep: async () => null,
    submitManualAnswer: async () => {
      throw new Error("unused fake submitManualAnswer");
    },
    startCliRun: async () => {
      throw new Error("unused fake startCliRun");
    },
    resumeManualRun: async () => {
      throw new Error("unused fake resumeManualRun");
    },
    resumeCliRun: async () => {
      throw new Error("unused fake resumeCliRun");
    },
    cancelRun: async () => false,
    interruptManualRun: async () => false,
    listRuns: async () => [],
    getRunDetail,
    exportPublicReport,
    deleteRawArtifacts: async () => undefined,
    deleteRun: async () => false,
    deleteTargetHistory: async () => 0,
    getDataSettings: async () => ({
      rawRetentionDays: null,
      cleanupPending: false,
    }),
    setRawRetention: async () => 0,
    exportFullBackup: async () => false,
    onRunEvent: async () => () => undefined,
    onRunError: async () => () => undefined,
  };
}

describe("ResultPage local data controls", () => {
  test("raw-only deletion has a separate confirmation, cancel calls nothing, and success refreshes evidence", async () => {
    const user = userEvent.setup();
    const initial = makeDetail();
    const refreshed = makeDetail(
      {},
      initial.taskResults.map((result) => ({
        ...result,
        answerRelPath: null,
      })),
    );
    const getRunDetail = vi
      .fn<Backend["getRunDetail"]>()
      .mockResolvedValueOnce(initial)
      .mockResolvedValueOnce(refreshed);
    const pending = deferred<void>();
    const deleteRawArtifacts = vi.fn(() => pending.promise);
    const backend = {
      ...makeBackend(getRunDetail),
      deleteRawArtifacts,
    };
    renderResult(backend);

    await user.click(
      await screen.findByText("数据管理"),
    );
    const rawButton = screen.getByRole("button", {
      name: /只删除原始回答和 CLI 日志/,
    });
    await user.click(rawButton);
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(deleteRawArtifacts).not.toHaveBeenCalled();

    await user.click(rawButton);
    const confirm = screen.getByRole("button", {
      name: /确认只删除原始数据/,
    });
    await user.dblClick(confirm);
    expect(deleteRawArtifacts).toHaveBeenCalledTimes(1);
    expect(deleteRawArtifacts).toHaveBeenCalledWith(runId);
    pending.resolve();
    expect(
      await screen.findByRole("status", {
        name: /原始数据已删除，分数和客观证据已保留/,
      }),
    ).toBeInTheDocument();
    await waitFor(() => expect(getRunDetail).toHaveBeenCalledTimes(2));
  });

  test("whole-run deletion is distinct, cancel is inert, and success navigates to history", async () => {
    const user = userEvent.setup();
    const deleteRun = vi.fn(async () => true);
    const backend = {
      ...makeBackend(async () => makeDetail()),
      deleteRun,
    };
    renderResult(backend);
    await user.click(await screen.findByText("数据管理"));
    const deleteButton = screen.getByRole("button", {
      name: /删除本次记录和原始数据/,
    });
    await user.click(deleteButton);
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(deleteRun).not.toHaveBeenCalled();

    await user.click(deleteButton);
    await user.click(
      screen.getByRole("button", { name: /确认删除本次记录/ }),
    );
    expect(deleteRun).toHaveBeenCalledWith(runId);
    expect(
      await screen.findByRole("heading", { name: "历史页" }),
    ).toBeInTheDocument();
  });

  test("failed deletion keeps the result and never exposes or claims backend details", async () => {
    const user = userEvent.setup();
    const deleteRun = vi.fn(async () => {
      throw new Error("C:\\Users\\Alice\\ability-radar.db");
    });
    const backend = {
      ...makeBackend(async () => makeDetail()),
      deleteRun,
    };
    renderResult(backend);
    await user.click(await screen.findByText("数据管理"));
    await user.click(
      screen.getByRole("button", { name: /删除本次记录和原始数据/ }),
    );
    await user.click(
      screen.getByRole("button", { name: /确认删除本次记录/ }),
    );

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("Alice");
    expect(document.body.textContent).not.toMatch(/本次记录已删除/);
    expect(
      screen.getByRole("heading", { name: "本次客观结果" }),
    ).toBeInTheDocument();
  });
});

function renderResult(
  backend: Backend,
  path = `/results/${runId}`,
) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/results/:runId" element={<ResultPage />} />
          <Route path="/" element={<h1>开始页</h1>} />
          <Route path="/history" element={<h1>历史页</h1>} />
        </Routes>
      </BackendProvider>
    </MemoryRouter>,
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function RouteCommitProbe({
  onCommit,
}: {
  onCommit: (pathname: string, visibleText: string) => void;
}) {
  const location = useLocation();
  useLayoutEffect(() => {
    onCommit(location.pathname, document.body.textContent ?? "");
  }, [location.pathname, onCommit]);
  return null;
}

describe("ResultPage objective semantics", () => {
  test("shows loading before persisted detail arrives", () => {
    const pending = deferred<RunDetail | null>();
    renderResult(makeBackend(() => pending.promise));

    expect(
      screen.getByRole("status", { name: "正在读取本地结果" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveAttribute("aria-busy", "true");
  });

  test("renders a stable safe placeholder for an invalid legacy model name", async () => {
    const unsafeModel = "GPT\u200B-X";
    renderResult(
      makeBackend(async () =>
        makeDetail({
          target: { ...makeRun().target, reportedModel: unsafeModel },
        }),
      ),
    );

    await screen.findByRole("heading", { name: "本次客观结果" });
    expect(document.body.textContent).toContain("模型名称不可显示");
    expect(document.body.textContent).not.toContain(unsafeModel);
  });

  test.each(CANONICAL_EFFORT_DISPLAY_CASES)(
    "result displays %s/%s as %s",
    async (kind, effort, expectedLabel) => {
      renderResult(
        makeBackend(async () =>
          makeDetail({
            target: {
              ...makeRun().target,
              kind,
              reasoningEffort: effort,
            },
          }),
        ),
      );

      await screen.findByRole("heading", { name: "本次客观结果" });
      const effortTerm = screen.getByText("推理档位");
      expect(effortTerm.parentElement).toHaveTextContent(expectedLabel);
    },
  );

  test("explains a partially invalid completed score with exact denominators", async () => {
    renderResult(makeBackend(async () => makeDetail()));

    expect(
      await screen.findByRole("heading", { name: "本次客观结果" }),
    ).toBeInTheDocument();
    expect(screen.getByText("66.7")).toBeInTheDocument();
    expect(screen.getByText("原始通过 5 / 6")).toBeInTheDocument();
    expect(screen.getByText("有效覆盖 6 / 8")).toBeInTheDocument();
    expect(screen.getByText("排除样本 2")).toBeInTheDocument();
    expect(
      screen.getByText(/先计算各个有有效题目的分类分，再对这些分类等权平均/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/0–100.*只代表这个题包里的客观结果.*不是 IQ/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/v0.5.*真实试运行校准.*配对变化结论/),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(
      /降智|疑似下降|明显下降|没有下降|基线|可信度|anomaly|confidence/i,
    );
  });

  test("shows full valid coverage without inventing excluded samples", async () => {
    renderResult(
      makeBackend(async () =>
        makeDetail({
          score: {
            abilityScore: 77.8,
            passedTasks: 6,
            validTasks: 8,
            totalTasks: 8,
            categoryScores: {
              instruction_following: 66.7,
              logic: 66.7,
              code_review: 100,
            },
          },
        }, fullCoverageTasks()),
      ),
    );

    expect(await screen.findByText("77.8")).toBeInTheDocument();
    expect(screen.getByText("原始通过 6 / 8")).toBeInTheDocument();
    expect(screen.getByText("有效覆盖 8 / 8")).toBeInTheDocument();
    expect(screen.getByText("排除样本 0")).toBeInTheDocument();
  });

  test("renders present category scores in fixed order and omits missing categories", async () => {
    renderResult(makeBackend(async () => makeDetail()));

    const chart = await screen.findByRole("list", {
      name: "各能力分类得分",
    });
    const labels = within(chart)
      .getAllByRole("listitem")
      .map((item) => within(item).getByTestId("category-label").textContent);
    expect(labels).toEqual(["指令遵循", "逻辑推理", "代码审查"]);
    expect(within(chart).queryByText("CLI 编码")).not.toBeInTheDocument();
    expect(within(chart).getAllByText("100.0 分")).toHaveLength(2);
    expect(within(chart).getByText("0.0 分")).toBeInTheDocument();
  });

  test.each([
    {
      status: "completed" as const,
      heading: "本次没有可计分样本",
      copy: /没有题目形成可计分结果.*运行环境样本/,
    },
    {
      status: "cancelled" as const,
      heading: "本次体检已取消",
      copy: /取消或未完成的题目不会进入成绩/,
    },
    {
      status: "interrupted" as const,
      heading: "本次体检被中断",
      copy: /应用退出或电脑重启.*不会作为能力失败/,
    },
    {
      status: "created" as const,
      heading: "体检尚未开始",
      copy: /本地记录已经建立.*还没有开始/,
    },
    {
      status: "running" as const,
      heading: "体检仍在进行",
      copy: /最终结果尚未形成/,
    },
  ])("gives $status its own honest no-score state", async ({
    status,
    heading,
    copy,
  }) => {
    const completedEvidence =
      status === "completed"
        ? Array.from({ length: 8 }, (_, index) =>
            makeTask(index + 1, {
              outcome: "invalid",
              score: null,
              failureKind: "network",
            }),
          )
        : [];
    renderResult(
      makeBackend(async () =>
        makeDetail({
          status,
          completedTasks: completedEvidence.length,
          score: null,
        }, completedEvidence),
      ),
    );

    expect(
      await screen.findByRole("heading", { name: heading }),
    ).toBeInTheDocument();
    expect(screen.getByText(copy)).toBeInTheDocument();
    expect(screen.queryByText("66.7")).not.toBeInTheDocument();
  });

  test("keeps an all-invalid completed run out of the ability denominator", async () => {
    const invalidTasks = [
      makeTask(1, {
        outcome: "invalid",
        score: null,
        failureKind: "network",
      }),
      makeTask(2, {
        outcome: "invalid",
        score: null,
        failureKind: "verifier_error",
      }),
    ];
    renderResult(
      makeBackend(async () =>
        makeDetail(
          {
            status: "completed",
            score: null,
            totalTasks: 2,
            completedTasks: 2,
          },
          invalidTasks,
        ),
      ),
    );

    expect(
      await screen.findByRole("heading", {
        name: "本次没有可计分样本",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText("运行无效（不计入成绩）"),
    ).toHaveLength(2);
    expect(screen.queryByText("0.0")).not.toBeInTheDocument();
  });

  test("treats failed infrastructure evidence as excluded and hides its raw score", async () => {
    const infrastructureFailure = makeTask(1, {
      outcome: "failed",
      score: 0,
      failureKind: "network",
    });
    renderResult(
      makeBackend(async () =>
        makeDetail(
          {
            status: "interrupted",
            score: null,
            totalTasks: 1,
            completedTasks: 1,
          },
          [infrastructureFailure],
        ),
      ),
    );

    const evidence = await screen.findByRole("list", {
      name: "逐题客观证据",
    });
    expect(
      within(evidence).getByText("运行无效（不计入成绩）"),
    ).toBeInTheDocument();
    expect(
      within(evidence).queryByText("未通过（计入本题包成绩）"),
    ).not.toBeInTheDocument();
    expect(within(evidence).queryByText("单题得分 0.0")).not.toBeInTheDocument();
    expect(
      within(evidence).getByText(/网络连接中断.*运行环境样本排除/),
    ).toBeInTheDocument();
  });

  test("renders coherent wrong-answer and budget failures as scored evidence", async () => {
    const failures = [
      makeTask(1, {
        outcome: "failed",
        score: 25,
        failureKind: "wrong_answer",
      }),
      makeTask(2, {
        outcome: "failed",
        score: 0,
        failureKind: "agent_budget_exceeded",
      }),
    ];
    renderResult(
      makeBackend(async () =>
        makeDetail(
          {
            totalTasks: 2,
            completedTasks: 2,
            score: {
              abilityScore: 12.5,
              passedTasks: 0,
              validTasks: 2,
              totalTasks: 2,
              categoryScores: { logic: 12.5 },
            },
          },
          failures,
        ),
      ),
    );

    const evidence = await screen.findByRole("list", {
      name: "逐题客观证据",
    });
    expect(
      within(evidence).getAllByText("未通过（计入本题包成绩）"),
    ).toHaveLength(2);
    expect(within(evidence).getByText("单题得分 25.0")).toBeInTheDocument();
    expect(within(evidence).getByText("单题得分 0.0")).toBeInTheDocument();
    expect(
      within(evidence).getByText(/答案未通过确定性检查.*客观规则计为未通过/),
    ).toBeInTheDocument();
    expect(
      within(evidence).getByText(/固定代理预算内未完成.*客观规则计为未通过/),
    ).toBeInTheDocument();
  });

  test("maps every safe task outcome and failure without exposing identifiers or paths", async () => {
    const failures: FailureKind[] = [
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
    const tasks = [
      makeTask(1, {
        category: "instruction_following",
        outcome: "passed",
        score: 100,
      }),
      makeTask(2, { category: "logic", outcome: "failed", score: 1 }),
      makeTask(3, { category: "code_review", outcome: "invalid" }),
      makeTask(4, { category: "cli_coding", outcome: "cancelled" }),
      ...failures.map((failureKind, index) =>
        makeTask(index + 5, {
          outcome:
            failureKind === "agent_budget_exceeded" ||
            failureKind === "wrong_answer"
              ? "failed"
              : "invalid",
          score:
            failureKind === "agent_budget_exceeded" ||
            failureKind === "wrong_answer"
              ? 1
              : 100,
          failureKind,
        }),
      ),
    ];
    renderResult(
      makeBackend(async () =>
        makeDetail(
          {
            status: "interrupted",
            score: null,
            totalTasks: tasks.length,
            completedTasks: tasks.length,
          },
          tasks,
        ),
      ),
    );

    const evidence = await screen.findByRole("list", {
      name: "逐题客观证据",
    });
    expect(within(evidence).getByText("第 1 题")).toBeInTheDocument();
    expect(within(evidence).getAllByText("通过").length).toBeGreaterThan(0);
    expect(
      within(evidence).getAllByText("未通过（计入本题包成绩）").length,
    ).toBeGreaterThan(0);
    expect(
      within(evidence).getAllByText("运行无效（不计入成绩）").length,
    ).toBeGreaterThan(0);
    expect(
      within(evidence).getByText("未完成（不计入成绩）"),
    ).toBeInTheDocument();
    expect(
      within(evidence).getByText(/固定代理预算内未完成.*客观规则计为未通过/),
    ).toBeInTheDocument();
    expect(
      within(evidence).getByText(/答案未通过确定性检查.*客观规则计为未通过/),
    ).toBeInTheDocument();
    expect(
      within(evidence).getByText(/网络连接中断.*运行环境样本排除/),
    ).toBeInTheDocument();
    expect(within(evidence).getByText("单题得分 100.0")).toBeInTheDocument();
    expect(within(evidence).getAllByText("单题得分 1.0")).toHaveLength(3);
    expect(within(evidence).getAllByText("耗时 1.3 秒").length).toBe(
      tasks.length,
    );

    const visible = document.body.textContent ?? "";
    expect(visible).not.toContain(runId);
    expect(visible).not.toContain("private-task-");
    expect(visible).not.toContain("answers/private-");
    for (const failure of failures) {
      expect(visible).not.toContain(failure);
    }
  });

  test("keeps reproducibility facts collapsed, formats effort, and hides local paths", async () => {
    renderResult(
      makeBackend(async () =>
        makeDetail({
          target: {
            kind: "codex_cli",
            reportedModel: "default",
            reasoningEffort: "xhigh",
          },
        }),
      ),
    );

    const summary = await screen.findByText("技术与复现信息");
    const details = summary.closest("details");
    expect(details).not.toHaveAttribute("open");
    expect(details).toHaveTextContent("题包 client-quick · 1.0.0");
    expect(details).toHaveTextContent("内容封印 eeeeeeeeeeee");
    expect(details).toHaveTextContent("评分规则 ability-v1");
    expect(details).toHaveTextContent("应用 0.2.0");
    expect(details).toHaveTextContent("系统 Windows 11");
    expect(details).toHaveTextContent("验证器 embedded-verifier 1.0.0");
    expect(details).toHaveTextContent("完整运行");
    expect(details).toHaveTextContent("极高");
    expect(details?.textContent).not.toMatch(/[A-Z]:\\|\/Users\/|\/home\//);
  });

  test("offers safe retry without rendering a backend exception", async () => {
    const user = userEvent.setup();
    const getRunDetail = vi
      .fn<Backend["getRunDetail"]>()
      .mockRejectedValueOnce(
        new Error("C:\\Users\\private\\secret token=abc"),
      )
      .mockResolvedValueOnce(makeDetail());
    renderResult(makeBackend(getRunDetail));

    expect(
      await screen.findByRole("heading", { name: "暂时无法读取结果" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "本地结果读取失败，请稍后重试。",
    );
    expect(document.body.textContent).not.toContain("secret token");

    await user.click(screen.getByRole("button", { name: "重新读取" }));
    expect(
      await screen.findByRole("heading", { name: "本次客观结果" }),
    ).toBeInTheDocument();
    expect(getRunDetail).toHaveBeenCalledTimes(2);
  });

  test.each([
    ["missing", null],
    ["malformed", {} as RunDetail],
  ])("handles a %s result without exposing the route identifier", async (
    _case,
    detail,
  ) => {
    renderResult(makeBackend(async () => detail));

    expect(
      await screen.findByRole("heading", {
        name: "没有找到这次体检",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "返回开始页" }),
    ).toHaveAttribute("href", "/");
    expect(
      screen.getByRole("link", { name: "查看历史记录" }),
    ).toHaveAttribute("href", "/history");
    expect(document.body.textContent).not.toContain(runId);
  });

  test("rejects a completed detail that has not accounted for every task", async () => {
    renderResult(
      makeBackend(async () =>
        makeDetail(
          {
            completedTasks: 1,
            score: {
              abilityScore: 100,
              passedTasks: 1,
              validTasks: 1,
              totalTasks: 8,
              categoryScores: { logic: 100 },
            },
          },
          [makeTask(1)],
        ),
      ),
    );

    expect(
      await screen.findByRole("heading", {
        name: "没有找到这次体检",
      }),
    ).toBeInTheDocument();
    expect(screen.queryByText("本次客观结果")).not.toBeInTheDocument();
  });

  test("rejects persisted detail whose run does not match the requested route", async () => {
    const mismatchedId = "different-private-run";
    renderResult(
      makeBackend(async () =>
        makeDetail({
          id: mismatchedId,
          target: {
            kind: "claude_client",
            reportedModel: "MISMATCHED MODEL",
            reasoningEffort: "medium",
          },
        }),
      ),
    );

    expect(
      await screen.findByRole("heading", {
        name: "没有找到这次体检",
      }),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("MISMATCHED MODEL");
    expect(document.body.textContent).not.toContain(mismatchedId);
  });

  test("renders loading in the same commit as a route id replacement", async () => {
    const user = userEvent.setup();
    const second = deferred<RunDetail | null>();
    const commits: Array<{ pathname: string; visibleText: string }> = [];
    const recordCommit = (pathname: string, visibleText: string) => {
      commits.push({ pathname, visibleText });
    };
    const backend = makeBackend((requestedId) =>
      requestedId === runId ? Promise.resolve(makeDetail()) : second.promise,
    );

    render(
      <MemoryRouter initialEntries={[`/results/${runId}`]}>
        <BackendProvider backend={backend}>
          <Link to="/results/second-private-id">切换结果</Link>
          <Routes>
            <Route path="/results/:runId" element={<ResultPage />} />
          </Routes>
          <RouteCommitProbe onCommit={recordCommit} />
        </BackendProvider>
      </MemoryRouter>,
    );

    expect(
      await screen.findByText("ChatGPT 客户端 · GPT-X"),
    ).toBeInTheDocument();
    commits.length = 0;
    await user.click(screen.getByRole("link", { name: "切换结果" }));

    expect(
      screen.getByRole("status", { name: "正在读取本地结果" }),
    ).toBeInTheDocument();
    expect(
      commits.some(
        ({ pathname, visibleText }) =>
          pathname === "/results/second-private-id" &&
          visibleText.includes("ChatGPT 客户端 · GPT-X"),
      ),
    ).toBe(false);
  });

  test("ignores a stale result completion after the route ID changes", async () => {
    const user = userEvent.setup();
    const first = deferred<RunDetail | null>();
    const second = makeDetail({
      id: "second-private-id",
      target: {
        kind: "claude_client",
        reportedModel: "Claude Y",
        reasoningEffort: "medium",
      },
    });
    const backend = makeBackend((requestedId) =>
      requestedId === runId ? first.promise : Promise.resolve(second),
    );

    render(
      <MemoryRouter initialEntries={[`/results/${runId}`]}>
        <BackendProvider backend={backend}>
          <Link to="/results/second-private-id">切换结果</Link>
          <Routes>
            <Route path="/results/:runId" element={<ResultPage />} />
          </Routes>
        </BackendProvider>
      </MemoryRouter>,
    );

    await user.click(screen.getByRole("link", { name: "切换结果" }));
    expect(
      await screen.findByText("Claude 客户端 · Claude Y"),
    ).toBeInTheDocument();

    first.resolve(
      makeDetail({
        target: {
          kind: "chat_gpt_client",
          reportedModel: "STALE MODEL",
          reasoningEffort: "high",
        },
      }),
    );
    await waitFor(() => {
      expect(screen.queryByText(/STALE MODEL/)).not.toBeInTheDocument();
    });
    expect(screen.getByText("Claude 客户端 · Claude Y")).toBeInTheDocument();
  });

  test("keeps home and history actions authoritative", async () => {
    const user = userEvent.setup();
    renderResult(makeBackend(async () => makeDetail()));

    await screen.findByRole("heading", { name: "本次客观结果" });
    await user.click(screen.getByRole("link", { name: "开始新的体检" }));
    expect(
      screen.getByRole("heading", { name: "开始页" }),
    ).toBeInTheDocument();
  });

  test("opens an accessible review dialog containing only the exact public allowlist", async () => {
    const user = userEvent.setup();
    const exportReport = vi.fn<Backend["exportPublicReport"]>(async () => null);
    renderResult(makeBackend(async () => makeDetail(), exportReport));

    const opener = await screen.findByRole("button", {
      name: "检查并导出可分享报告",
    });
    await user.click(opener);

    const dialog = screen.getByRole("dialog", { name: "导出前检查" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    expect(within(dialog).getByText("报告格式版本 1")).toBeInTheDocument();
    expect(
      within(dialog).getByText(/匿名报告编号和生成时间将在确认并选择位置后创建/),
    ).toBeInTheDocument();
    expect(within(dialog).getByText("ChatGPT 客户端")).toBeInTheDocument();
    expect(within(dialog).getByText("GPT-X")).toBeInTheDocument();
    expect(within(dialog).getByText("高")).toBeInTheDocument();
    expect(within(dialog).getByText("Windows")).toBeInTheDocument();
    expect(within(dialog).getByText("0.2.0")).toBeInTheDocument();
    expect(
      within(dialog).getByText("embedded-verifier 1.0.0"),
    ).toBeInTheDocument();
    expect(within(dialog).getByText("client-quick · 1.0.0")).toBeInTheDocument();
    expect(within(dialog).getByText("e".repeat(64))).toBeInTheDocument();
    expect(within(dialog).getByText("ability-v1")).toBeInTheDocument();
    expect(within(dialog).getByText("completed")).toBeInTheDocument();
    expect(within(dialog).getByText("66.7")).toBeInTheDocument();
    expect(within(dialog).getByText("5 / 6 / 8")).toBeInTheDocument();
    expect(within(dialog).getByText(/passed 5.*failed 2.*invalid 1/)).toBeInTheDocument();
    expect(within(dialog).getByText(/network 2.*wrong_answer 1/)).toBeInTheDocument();
    expect(within(dialog).getByText("10.0 秒")).toBeInTheDocument();
    expect(within(dialog).getByText("not_evaluated")).toBeInTheDocument();
    expect(within(dialog).getByText(/v0.2 不生成降智结论/)).toBeInTheDocument();
    for (const excluded of [
      "原始回答",
      "题目提示词",
      "CLI 日志",
      "逐题详情文本",
      "用户名",
      "主机名",
      "操作系统构建号",
      "绝对路径",
      "本地 run ID",
      "本地 task ID",
      "相对 artifact / answer path",
      "凭据",
      "保存位置",
    ]) {
      expect(within(dialog).getByText(excluded)).toBeInTheDocument();
    }
    expect(dialog).not.toHaveTextContent(runId);
    expect(dialog).not.toHaveTextContent("private-task");
    expect(dialog).not.toHaveTextContent("answers/private");
    expect(dialog).not.toHaveTextContent("Windows 11");
    expect(exportReport).not.toHaveBeenCalled();
  });

  test("gates export behind review confirmation and restores focus on cancel and Escape", async () => {
    const user = userEvent.setup();
    const exportReport = vi.fn<Backend["exportPublicReport"]>(async () => null);
    renderResult(makeBackend(async () => makeDetail(), exportReport));

    const opener = await screen.findByRole("button", {
      name: "检查并导出可分享报告",
    });
    await user.click(opener);
    const confirm = screen.getByRole("button", {
      name: "选择位置并导出",
    });
    expect(confirm).toBeDisabled();

    await user.click(
      screen.getByRole("checkbox", { name: "我已检查以上公开字段" }),
    );
    expect(confirm).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(exportReport).not.toHaveBeenCalled();
    expect(opener).toHaveFocus();

    await user.click(opener);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(exportReport).not.toHaveBeenCalled();
    expect(opener).toHaveFocus();
  });

  test("prevents duplicate export while busy and announces the anonymous report id", async () => {
    const user = userEvent.setup();
    const exported = deferred<string | null>();
    const exportReport = vi.fn<Backend["exportPublicReport"]>(
      () => exported.promise,
    );
    renderResult(makeBackend(async () => makeDetail(), exportReport));

    await user.click(
      await screen.findByRole("button", {
        name: "检查并导出可分享报告",
      }),
    );
    await user.click(
      screen.getByRole("checkbox", { name: "我已检查以上公开字段" }),
    );
    const confirm = screen.getByRole("button", {
      name: "选择位置并导出",
    });
    await user.click(confirm);
    await user.click(confirm);

    expect(exportReport).toHaveBeenCalledTimes(1);
    expect(exportReport).toHaveBeenCalledWith(runId);
    expect(confirm).toBeDisabled();
    expect(confirm).toHaveAttribute("aria-busy", "true");

    exported.resolve("019f-report-anonymous");
    expect(
      await screen.findByRole("status", { name: "报告导出状态" }),
    ).toHaveTextContent("019f-report-anonymous");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  test("treats native picker cancellation as normal and never exposes backend errors", async () => {
    const user = userEvent.setup();
    const exportReport = vi
      .fn<Backend["exportPublicReport"]>()
      .mockResolvedValueOnce(null)
      .mockRejectedValueOnce(
        new Error(
          String.raw`C:\Users\Alice\report.html sk-ant-private-token`,
        ),
      );
    renderResult(makeBackend(async () => makeDetail(), exportReport));

    const opener = await screen.findByRole("button", {
      name: "检查并导出可分享报告",
    });
    await user.click(opener);
    await user.click(
      screen.getByRole("checkbox", { name: "我已检查以上公开字段" }),
    );
    await user.click(
      screen.getByRole("button", { name: "选择位置并导出" }),
    );

    expect(
      await screen.findByRole("status", { name: "报告导出状态" }),
    ).toHaveTextContent("已取消保存，未生成报告");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    await user.click(opener);
    await user.click(
      screen.getByRole("checkbox", { name: "我已检查以上公开字段" }),
    );
    await user.click(
      screen.getByRole("button", { name: "选择位置并导出" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "无法导出报告，请检查公开字段后重试",
    );
    expect(document.body).not.toHaveTextContent("Alice");
    expect(document.body).not.toHaveTextContent("sk-ant-private-token");
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "选择位置并导出" }),
    ).toBeEnabled();
  });

  test("surfaces only an allowlisted public field label for scanner failures", async () => {
    const user = userEvent.setup();
    const exportReport = vi.fn<Backend["exportPublicReport"]>(async () => {
      throw "无法导出：公开字段 reportedModel 可能包含敏感信息。";
    });
    renderResult(makeBackend(async () => makeDetail(), exportReport));

    await user.click(
      await screen.findByRole("button", {
        name: "检查并导出可分享报告",
      }),
    );
    await user.click(
      screen.getByRole("checkbox", { name: "我已检查以上公开字段" }),
    );
    await user.click(
      screen.getByRole("button", { name: "选择位置并导出" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "公开字段“用户填写模型”可能包含敏感信息",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("reportedModel");
  });

  test.each(["created", "running", "cancelled", "interrupted"] as const)(
    "does not offer export for a %s run",
    async (status) => {
      renderResult(
        makeBackend(async () =>
          makeDetail(
            {
              status,
              score: null,
              completedTasks: 0,
            },
            [],
          ),
        ),
      );

      await screen.findByRole("heading", { level: 1 });
      expect(
        screen.queryByRole("button", {
          name: "检查并导出可分享报告",
        }),
      ).not.toBeInTheDocument();
    },
  );
});
