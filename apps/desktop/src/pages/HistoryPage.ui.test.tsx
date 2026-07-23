import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, test, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type { Backend, RunRecord } from "../api/backend";
import {
  CANONICAL_EFFORT_DISPLAY_CASES,
  DEFAULT_MODEL_DISPLAY_CASES,
  INVALID_LEGACY_EFFORT_CASES,
} from "../test/reasoningEffortCases";
import { HistoryPage } from "./HistoryPage";

function makeRun(overrides: Partial<RunRecord> = {}): RunRecord {
  return {
    id: "private-run-1",
    target: {
      kind: "chat_gpt_client",
      reportedModel: "GPT-X",
      reasoningEffort: "high",
      modelSource: "windows_accessibility",
      modelVerification: "user_confirmed",
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
      suiteContentSha256: "a".repeat(64),
      scoringRuleVersion: "ability-v1",
      resumed: false,
    },
    score: {
      abilityScore: 75,
      passedTasks: 6,
      validTasks: 8,
      totalTasks: 8,
      categoryScores: {
        instruction_following: 75,
        logic: 75,
        code_review: 75,
      },
    },
    ...overrides,
  };
}

function makeBackend(listRuns: Backend["listRuns"]): Backend {
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
    listRuns,
    getRunDetail: async () => null,
    exportPublicReport: async () => null,
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

test.each([
  ["chat_gpt_client", `/manual/chat_gpt_client?resume=private-run-1`],
  ["claude_client", `/manual/claude_client?resume=private-run-1`],
  ["codex_cli", `/cli/codex_cli?resume=private-run-1`],
  ["claude_code", `/cli/claude_code?resume=private-run-1`],
] as const)(
  "interrupted %s history exposes a resume route without treating it as a result",
  async (kind, expectedHref) => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          target: { ...makeRun().target, kind },
          status: "interrupted",
          completedTasks: 1,
          score: null,
        }),
      ]),
    );

    const link = await screen.findByRole("link", { name: /继续/ });
    expect(link).toHaveAttribute("href", expectedHref);
  },
);

test("target history deletion is two-step, cancel makes zero calls, and confirmation binds exact ids", async () => {
  const user = userEvent.setup();
  const first = makeRun({ id: "run-one" });
  const second = makeRun({ id: "run-two" });
  const listRuns = vi
    .fn<Backend["listRuns"]>()
    .mockResolvedValueOnce([first, second])
    .mockResolvedValueOnce([]);
  const pending = deferred<number>();
  const deleteTargetHistory = vi.fn(() => pending.promise);
  const backend = {
    ...makeBackend(listRuns),
    deleteTargetHistory,
  };
  renderHistory(backend);

  const open = await screen.findByRole("button", {
    name: /删除该测试对象全部历史/,
  });
  await user.click(open);
  expect(screen.getByRole("group", { name: /确认删除/ })).toHaveTextContent(
    "2",
  );
  await user.click(screen.getByRole("button", { name: "取消" }));
  expect(deleteTargetHistory).not.toHaveBeenCalled();

  await user.click(open);
  const confirm = screen.getByRole("button", { name: /确认删除 2 条记录/ });
  await user.dblClick(confirm);
  expect(deleteTargetHistory).toHaveBeenCalledTimes(1);
  expect(deleteTargetHistory).toHaveBeenCalledWith("chat_gpt_client", [
    "run-one",
    "run-two",
  ]);
  pending.resolve(2);
  expect(
    await screen.findByRole("heading", { name: /还没有体检记录/ }),
  ).toBeInTheDocument();
  expect(listRuns).toHaveBeenCalledTimes(2);
});

test("failed stale target confirmation keeps history visible and never claims deletion", async () => {
  const user = userEvent.setup();
  const deleteTargetHistory = vi.fn(async () => {
    throw new Error("C:\\Users\\Alice\\private.db");
  });
  const backend = {
    ...makeBackend(async () => [makeRun({ id: "still-here" })]),
    deleteTargetHistory,
  };
  renderHistory(backend);
  await user.click(
    await screen.findByRole("button", {
      name: /删除该测试对象全部历史/,
    }),
  );
  await user.click(
    screen.getByRole("button", { name: /确认删除 1 条记录/ }),
  );

  expect(await screen.findByRole("alert")).toBeInTheDocument();
  expect(document.body.textContent).not.toContain("Alice");
  expect(
    screen.getByRole("link", { name: /查看本次结果/ }),
  ).toBeInTheDocument();
  expect(document.body.textContent).not.toMatch(/已删除/);
});

function renderHistory(backend: Backend) {
  return render(historyTree(backend));
}

function historyTree(backend: Backend) {
  return (
    <MemoryRouter initialEntries={["/history"]}>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/history" element={<HistoryPage />} />
          <Route path="/results/:runId" element={<h1>结果页</h1>} />
          <Route path="/" element={<h1>开始页</h1>} />
        </Routes>
      </BackendProvider>
    </MemoryRouter>
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

test("renders a stable safe placeholder for an invalid legacy model name", async () => {
  const unsafeModel = "GPT\u200B-X";
  renderHistory(
    makeBackend(async () => [
      makeRun({
        target: { ...makeRun().target, reportedModel: unsafeModel },
      }),
    ]),
  );

  await screen.findByRole("link", { name: /查看本次结果/ });
  expect(document.body.textContent).toContain("模型名称不可显示");
  expect(document.body.textContent).not.toContain(unsafeModel);
});

test.each(DEFAULT_MODEL_DISPLAY_CASES)(
  "history displays default for %s as target-aware model text",
  async (kind, targetLabel, expectedLabel, expectsRoutingWarning) => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          target: {
            ...makeRun().target,
            kind,
            reportedModel: "default",
          },
        }),
      ]),
    );

    expect(
      await screen.findByRole("heading", {
        name: `${targetLabel} · ${expectedLabel}`,
      }),
    ).toBeInTheDocument();
    const warning = screen.queryByText(/默认路由可能在服务侧切换实际模型/);
    expect(Boolean(warning)).toBe(expectsRoutingWarning);
  },
);

test.each(INVALID_LEGACY_EFFORT_CASES)(
  "history safely hides legacy effort containing %s",
  async (_name, effort) => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          target: {
            ...makeRun().target,
            reasoningEffort: effort,
          },
        }),
      ]),
    );

    await screen.findByRole("link", { name: /查看本次结果/ });
    const effortTerm = screen.getByText("推理档位");
    expect(effortTerm.parentElement).toHaveTextContent("推理档位不可显示");
    expect(effortTerm.parentElement?.textContent).not.toContain(effort);
  },
);

test.each(CANONICAL_EFFORT_DISPLAY_CASES)(
  "history displays %s/%s as %s",
  async (kind, effort, expectedLabel) => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          target: {
            ...makeRun().target,
            kind,
            reasoningEffort: effort,
          },
        }),
      ]),
    );

    await screen.findByRole("link", { name: /查看本次结果/ });
    const effortTerm = screen.getByText("推理档位");
    expect(effortTerm.parentElement).toHaveTextContent(expectedLabel);
  },
);

describe("HistoryPage states", () => {
  test("shows an explicit loading state", () => {
    const pending = deferred<RunRecord[]>();
    renderHistory(makeBackend(() => pending.promise));

    expect(
      screen.getByRole("status", { name: "正在读取本地历史" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveAttribute("aria-busy", "true");
  });

  test("shows a safe error and retries without exposing backend text", async () => {
    const user = userEvent.setup();
    const listRuns = vi
      .fn<Backend["listRuns"]>()
      .mockRejectedValueOnce(
        new Error("C:\\Users\\private\\credential=secret"),
      )
      .mockResolvedValueOnce([]);
    renderHistory(makeBackend(listRuns));

    expect(
      await screen.findByRole("heading", { name: "暂时无法读取历史" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "本地历史读取失败，请稍后重试。",
    );
    expect(document.body.textContent).not.toContain("credential=secret");

    await user.click(screen.getByRole("button", { name: "重新读取" }));
    expect(
      await screen.findByRole("heading", { name: "还没有体检记录" }),
    ).toBeInTheDocument();
    expect(listRuns).toHaveBeenCalledTimes(2);
  });

  test("fails closed when persisted history has a malformed record", async () => {
    renderHistory(
      makeBackend(async () => [
        {
          id: "C:\\Users\\private\\credential=secret",
          status: "completed",
        } as RunRecord,
      ]),
    );

    expect(
      await screen.findByRole("heading", { name: "暂时无法读取历史" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "本地历史读取失败，请稍后重试。",
    );
    expect(document.body.textContent).not.toContain("credential=secret");
  });

  test("fails closed when completed history has unaccounted tasks", async () => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          completedTasks: 7,
          score: null,
        }),
      ]),
    );

    expect(
      await screen.findByRole("heading", { name: "暂时无法读取历史" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: /历史系列/ }),
    ).not.toBeInTheDocument();
  });

  test("shows a useful empty state with a route home", async () => {
    renderHistory(makeBackend(async () => []));

    expect(
      await screen.findByRole("heading", { name: "还没有体检记录" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "开始第一次体检" }),
    ).toHaveAttribute("href", "/");
  });

  test("ignores a stale list completion after the backend changes", async () => {
    const first = deferred<RunRecord[]>();
    const oldBackend = makeBackend(() => first.promise);
    const newBackend = makeBackend(async () => [
      makeRun({
        id: "current-private-id",
        target: {
          kind: "claude_client",
          reportedModel: "CURRENT MODEL",
          reasoningEffort: "medium",
          modelSource: "manual",
          modelVerification: "user_confirmed",
        },
      }),
    ]);
    const view = render(
      <MemoryRouter>
        <BackendProvider backend={oldBackend}>
          <HistoryPage />
        </BackendProvider>
      </MemoryRouter>,
    );

    view.rerender(
      <MemoryRouter>
        <BackendProvider backend={newBackend}>
          <HistoryPage />
        </BackendProvider>
      </MemoryRouter>,
    );
    expect(
      await screen.findByRole("heading", {
        name: "Claude 客户端 · CURRENT MODEL",
      }),
    ).toBeInTheDocument();

    first.resolve([
      makeRun({
        target: {
          kind: "chat_gpt_client",
          reportedModel: "STALE MODEL",
          reasoningEffort: "high",
          modelSource: "manual",
          modelVerification: "user_confirmed",
        },
      }),
    ]);
    await waitFor(() => {
      expect(screen.queryByText(/STALE MODEL/)).not.toBeInTheDocument();
    });
    expect(screen.getByText(/CURRENT MODEL/)).toBeInTheDocument();
  });
});

describe("HistoryPage local data controls", () => {
  test("remain available when there are no history records", async () => {
    renderHistory(makeBackend(async () => []));

    expect(
      await screen.findByRole("heading", { name: "还没有体检记录" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("heading", { name: "本地数据" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("combobox", { name: "原始数据保留期限" }),
    ).toBeInTheDocument();
  });

  test("retention selection and cancellation mutate nothing until explicit shortening confirmation", async () => {
    const user = userEvent.setup();
    const setRawRetention = vi.fn(async () => 1);
    const backend = {
      ...makeBackend(async () => [makeRun()]),
      setRawRetention,
    };
    renderHistory(backend);

    expect(
      await screen.findByRole("heading", { name: "本地数据" }),
    ).toBeInTheDocument();
    const select = await screen.findByRole("combobox", {
      name: "原始数据保留期限",
    });
    await user.selectOptions(select, "7");
    expect(setRawRetention).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "应用保留期限" }));
    const confirmation = screen.getByRole("group", {
      name: "确认缩短原始数据保留期限",
    });
    await user.click(within(confirmation).getByRole("button", { name: "取消" }));
    expect(setRawRetention).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "应用保留期限" }));
    await user.click(
      screen.getByRole("button", { name: "确认应用并清理过期原始数据" }),
    );
    await waitFor(() => expect(setRawRetention).toHaveBeenCalledTimes(1));
    expect(setRawRetention).toHaveBeenCalledWith(7);
  });

  test("retention confirmation suppresses duplicate mutation clicks", async () => {
    const user = userEvent.setup();
    const pending = deferred<number>();
    const setRawRetention = vi.fn(() => pending.promise);
    renderHistory({
      ...makeBackend(async () => [makeRun()]),
      setRawRetention,
    });
    const select = await screen.findByRole("combobox", {
      name: "原始数据保留期限",
    });
    await user.selectOptions(select, "7");
    await user.click(screen.getByRole("button", { name: "应用保留期限" }));
    const confirmation = screen.getByRole("group", {
      name: "确认缩短原始数据保留期限",
    });

    await user.dblClick(
      within(confirmation).getByRole("button", {
        name: "确认应用并清理过期原始数据",
      }),
    );

    expect(setRawRetention).toHaveBeenCalledTimes(1);
    pending.resolve(1);
    await waitFor(() => expect(select).not.toBeDisabled());
  });

  test("backup acknowledgement gates exact payload, suppresses duplicates, and cancel is silent", async () => {
    const user = userEvent.setup();
    const pending = deferred<boolean>();
    const exportFullBackup = vi.fn(() => pending.promise);
    renderHistory({
      ...makeBackend(async () => [makeRun()]),
      exportFullBackup,
    });

    const backup = await screen.findByRole("button", {
      name: "导出完整本地备份",
    });
    expect(backup).toBeDisabled();
    await user.click(
      screen.getByRole("checkbox", {
        name: "我知道此 ZIP 未加密，并包含原始回答和日志",
      }),
    );
    await user.dblClick(backup);
    expect(exportFullBackup).toHaveBeenCalledTimes(1);
    expect(exportFullBackup).toHaveBeenCalledWith({
      acknowledgedUnencryptedRawData: true,
    });
    pending.resolve(false);
    await waitFor(() => expect(backup).not.toBeDisabled());
    expect(screen.queryByText(/备份已导出/)).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  test("backup failures expose only the fixed safe UI error", async () => {
    const user = userEvent.setup();
    renderHistory({
      ...makeBackend(async () => [makeRun()]),
      exportFullBackup: async () => {
        throw new Error("C:\\Users\\Alice\\private.zip SQL sk-ant-secret");
      },
    });
    await user.click(
      await screen.findByRole("checkbox", {
        name: "我知道此 ZIP 未加密，并包含原始回答和日志",
      }),
    );
    await user.click(
      screen.getByRole("button", { name: "导出完整本地备份" }),
    );

    expect(
      await screen.findByRole("alert"),
    ).toHaveTextContent("无法完成本地备份，请稍后重试。");
    expect(document.body.textContent).not.toContain("Alice");
    expect(document.body.textContent).not.toContain("SQL");
    expect(document.body.textContent).not.toContain("sk-ant");
  });

  test("backup cleanup-incomplete failures render the distinct private-data warning", async () => {
    const user = userEvent.setup();
    renderHistory({
      ...makeBackend(async () => [makeRun()]),
      exportFullBackup: async () => {
        throw new Error(
          "备份未完成，临时私密数据可能尚未清理；请关闭应用并联系支持。",
        );
      },
    });
    await user.click(
      await screen.findByRole("checkbox", {
        name: "我知道此 ZIP 未加密，并包含原始回答和日志",
      }),
    );
    await user.click(
      screen.getByRole("button", { name: "导出完整本地备份" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "备份未完成，临时私密数据可能尚未清理；请关闭应用并联系支持。",
    );
    expect(screen.queryByText("无法完成本地备份，请稍后重试。")).not.toBeInTheDocument();
  });

  test("stale settings completion cannot overwrite a newer backend view", async () => {
    const oldSettings = deferred<{
      rawRetentionDays: number | null;
      cleanupPending: boolean;
    }>();
    const oldBackend = {
      ...makeBackend(async () => [makeRun()]),
      getDataSettings: () => oldSettings.promise,
    };
    const view = renderHistory(oldBackend);
    await screen.findByRole("heading", { name: "本地数据" });

    const newBackend = {
      ...makeBackend(async () => [makeRun()]),
      getDataSettings: async () => ({
        rawRetentionDays: 90,
        cleanupPending: false,
      }),
    };
    view.rerender(historyTree(newBackend));
    const select = await screen.findByRole("combobox", {
      name: "原始数据保留期限",
    });
    await waitFor(() => expect(select).toHaveValue("90"));
    oldSettings.resolve({ rawRetentionDays: 7, cleanupPending: false });
    await Promise.resolve();
    expect(select).toHaveValue("90");
  });

  test("a stale retention operation cannot leave a newer backend view busy", async () => {
    const user = userEvent.setup();
    const oldRetention = deferred<number>();
    const oldBackend = {
      ...makeBackend(async () => [makeRun()]),
      setRawRetention: () => oldRetention.promise,
    };
    const view = renderHistory(oldBackend);
    const select = await screen.findByRole("combobox", {
      name: "原始数据保留期限",
    });
    await user.selectOptions(select, "7");
    await user.click(screen.getByRole("button", { name: "应用保留期限" }));
    await user.click(
      screen.getByRole("button", { name: "确认应用并清理过期原始数据" }),
    );
    expect(select).toBeDisabled();

    const newSetRawRetention = vi.fn(async () => 0);
    view.rerender(
      historyTree({
        ...makeBackend(async () => [makeRun()]),
        setRawRetention: newSetRawRetention,
      }),
    );

    const currentSelect = await screen.findByRole("combobox", {
      name: "原始数据保留期限",
    });
    await waitFor(() => expect(currentSelect).not.toBeDisabled());
    await user.selectOptions(currentSelect, "7");
    await user.click(screen.getByRole("button", { name: "应用保留期限" }));
    await user.click(
      screen.getByRole("button", { name: "确认应用并清理过期原始数据" }),
    );
    await waitFor(() => expect(newSetRawRetention).toHaveBeenCalledTimes(1));
    oldRetention.resolve(1);
    await Promise.resolve();
  });

  test("a stale backup operation cannot leave a newer backend view busy or acknowledged", async () => {
    const user = userEvent.setup();
    const oldBackup = deferred<boolean>();
    const oldBackend = {
      ...makeBackend(async () => [makeRun()]),
      exportFullBackup: () => oldBackup.promise,
    };
    const view = renderHistory(oldBackend);
    const acknowledgement = await screen.findByRole("checkbox", {
      name: "我知道此 ZIP 未加密，并包含原始回答和日志",
    });
    await user.click(acknowledgement);
    await user.click(
      screen.getByRole("button", { name: "导出完整本地备份" }),
    );
    expect(acknowledgement).toBeDisabled();

    const newExportFullBackup = vi.fn(async () => false);
    view.rerender(
      historyTree({
        ...makeBackend(async () => [makeRun()]),
        exportFullBackup: newExportFullBackup,
      }),
    );

    const currentAcknowledgement = await screen.findByRole("checkbox", {
      name: "我知道此 ZIP 未加密，并包含原始回答和日志",
    });
    await waitFor(() => expect(currentAcknowledgement).not.toBeDisabled());
    expect(currentAcknowledgement).not.toBeChecked();
    oldBackup.resolve(true);
    await Promise.resolve();
    expect(screen.queryByText("完整本地备份已导出。")).not.toBeInTheDocument();
  });

  test("partial cleanup failure reloads effective setting and shows only safe pending copy", async () => {
    const user = userEvent.setup();
    const getDataSettings = vi
      .fn<Backend["getDataSettings"]>()
      .mockResolvedValueOnce({
        rawRetentionDays: null,
        cleanupPending: false,
      })
      .mockResolvedValueOnce({
        rawRetentionDays: 7,
        cleanupPending: true,
      });
    const setRawRetention = vi.fn(async () => {
      throw new Error("C:\\Users\\Alice\\ability.db SQL secret");
    });
    renderHistory({
      ...makeBackend(async () => [makeRun()]),
      getDataSettings,
      setRawRetention,
    });
    const select = await screen.findByRole("combobox", {
      name: "原始数据保留期限",
    });
    await user.selectOptions(select, "7");
    await user.click(screen.getByRole("button", { name: "应用保留期限" }));
    await user.click(
      screen.getByRole("button", { name: "确认应用并清理过期原始数据" }),
    );

    expect(
      await screen.findByText("保留期限已生效，但原始数据清理尚未完成；稍后可重试。"),
    ).toBeInTheDocument();
    expect(select).toHaveValue("7");
    expect(getDataSettings).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).not.toContain("Alice");
    expect(document.body.textContent).not.toContain("SQL");
  });
});

describe("HistoryPage comparable series", () => {
  test("keeps incompatible client, CLI, default, and resumed records visibly separate", async () => {
    const records = [
      makeRun({
        id: "chat-new",
        startedAt: "2026-07-18T09:00:00Z",
        finishedAt: "2026-07-18T09:10:00Z",
      }),
      makeRun({
        id: "chat-old",
        startedAt: "2026-07-17T09:00:00Z",
        finishedAt: "2026-07-17T09:10:00Z",
      }),
      makeRun({
        id: "claude-client",
        target: {
          kind: "claude_client",
          reportedModel: "Claude Y",
          reasoningEffort: "medium",
          modelSource: "manual",
          modelVerification: "user_confirmed",
        },
        startedAt: "2026-07-16T09:00:00Z",
      }),
      makeRun({
        id: "codex-default",
        target: {
          kind: "codex_cli",
          reportedModel: "default",
          reasoningEffort: "xhigh",
          modelSource: "default_route",
          modelVerification: "unverified",
        },
        suiteId: "cli-quick",
        totalTasks: 2,
        completedTasks: 2,
        score: {
          abilityScore: 75,
          passedTasks: 1,
          validTasks: 2,
          totalTasks: 2,
          categoryScores: { cli_coding: 75 },
        },
        environment: {
          ...makeRun().environment,
          suiteId: "cli-quick",
          cliVersion: "codex 1.0.0",
          verifierRuntimeVersion: "node v22.0.0",
          suiteContentSha256: "b".repeat(64),
        },
        startedAt: "2026-07-15T09:00:00Z",
      }),
      makeRun({
        id: "claude-code-resumed",
        target: {
          kind: "claude_code",
          reportedModel: "default",
          reasoningEffort: null,
          modelSource: "default_route",
          modelVerification: "unverified",
        },
        suiteId: "cli-quick",
        totalTasks: 2,
        completedTasks: 1,
        environment: {
          ...makeRun().environment,
          suiteId: "cli-quick",
          cliVersion: "claude 2.0.0",
          verifierRuntimeVersion: "node v24.0.0",
          suiteContentSha256: "c".repeat(64),
          resumed: true,
        },
        status: "interrupted",
        score: null,
        startedAt: "2026-07-14T09:00:00Z",
      }),
    ];
    renderHistory(makeBackend(async () => records));

    expect(await screen.findByText("极高")).toBeInTheDocument();

    expect(
      await screen.findByRole("heading", { name: "严格同条件历史" }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("region", { name: /历史系列/ })).toHaveLength(
      4,
    );
    expect(
      screen.getByRole("heading", { name: "ChatGPT 客户端 · GPT-X" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Claude 客户端 · Claude Y" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", {
        name: "Codex CLI · 默认路由（未固定）",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", {
        name: "Claude Code · 默认路由（未固定）",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText(/默认路由可能在服务侧切换实际模型/),
    ).toHaveLength(2);
    expect(screen.getByText("恢复运行 · 单独系列")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(
      /平均|差值|上升|下降|降智|基线|可信度|anomaly|confidence/i,
    );
  });

  test("sorts groups and records newest-first with deterministic visible dates", async () => {
    const records = [
      makeRun({
        id: "older",
        startedAt: "2026-07-16T08:00:00Z",
        finishedAt: "2026-07-16T08:10:00Z",
      }),
      makeRun({
        id: "newer",
        startedAt: "2026-07-18T08:00:00Z",
        finishedAt: "2026-07-18T08:10:00Z",
      }),
      makeRun({
        id: "middle-other-series",
        target: {
          kind: "claude_client",
          reportedModel: "Claude Y",
          reasoningEffort: "medium",
          modelSource: "manual",
          modelVerification: "user_confirmed",
        },
        startedAt: "2026-07-17T08:00:00Z",
        finishedAt: "2026-07-17T08:10:00Z",
      }),
    ];
    renderHistory(makeBackend(async () => records));

    const groups = await screen.findAllByRole("region", {
      name: /历史系列/,
    });
    expect(
      within(groups[0]).getByRole("heading", { level: 2 }),
    ).toHaveTextContent("ChatGPT 客户端");
    expect(
      within(groups[1]).getByRole("heading", { level: 2 }),
    ).toHaveTextContent("Claude 客户端");

    const chatTimes = within(groups[0]).getAllByRole("time");
    expect(chatTimes.map((time) => time.textContent)).toEqual([
      "2026-07-18 08:00 UTC",
      "2026-07-16 08:00 UTC",
    ]);
    expect(chatTimes[0]).toHaveAttribute(
      "datetime",
      "2026-07-18T08:00:00Z",
    );
  });

  test("shows a score only for completed scored records and localizes every other status", async () => {
    const sameKey = {
      target: makeRun().target,
      environment: makeRun().environment,
    };
    const records = [
      makeRun({
        id: "completed-scored",
        ...sameKey,
        status: "completed",
        startedAt: "2026-07-18T05:00:00Z",
      }),
      makeRun({
        id: "completed-empty",
        ...sameKey,
        status: "completed",
        score: null,
        startedAt: "2026-07-18T04:00:00Z",
      }),
      makeRun({
        id: "running",
        ...sameKey,
        status: "running",
        score: null,
        startedAt: "2026-07-18T03:00:00Z",
      }),
      makeRun({
        id: "created",
        ...sameKey,
        status: "created",
        score: null,
        startedAt: "2026-07-18T02:00:00Z",
      }),
      makeRun({
        id: "cancelled",
        ...sameKey,
        status: "cancelled",
        score: null,
        startedAt: "2026-07-18T01:00:00Z",
      }),
      makeRun({
        id: "interrupted",
        ...sameKey,
        status: "interrupted",
        score: null,
        startedAt: "2026-07-18T00:00:00Z",
      }),
    ];
    renderHistory(makeBackend(async () => records));

    expect(await screen.findAllByText("75.0 分")).toHaveLength(1);
    expect(
      screen.getByText("已完成 · 没有可计分样本"),
    ).toBeInTheDocument();
    expect(screen.getByText("进行中 · 尚未形成结果")).toBeInTheDocument();
    expect(screen.getByText("尚未开始")).toBeInTheDocument();
    expect(screen.getByText("已取消")).toBeInTheDocument();
    expect(screen.getByText("已中断")).toBeInTheDocument();
  });

  test("uses a safe semantic fallback for invalid timestamps", async () => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          id: "invalid-time-private-id",
          startedAt: "not-a-date /Users/private",
          finishedAt: null,
        }),
      ]),
    );

    const time = await screen.findByRole("time");
    expect(time).toHaveTextContent("时间记录无效");
    expect(time).not.toHaveAttribute("datetime");
    expect(document.body.textContent).not.toContain("/Users/private");
  });

  test("uses an internal deterministic fallback when comparable timestamps are invalid", async () => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          id: "fallback-b",
          startedAt: "invalid-b",
          finishedAt: null,
        }),
        makeRun({
          id: "fallback-a",
          startedAt: "invalid-a",
          finishedAt: null,
        }),
      ]),
    );

    const group = await screen.findByRole("region", {
      name: /历史系列：ChatGPT 客户端/,
    });
    const links = within(group).getAllByRole("link", {
      name: "查看本次结果",
    });
    expect(links.map((link) => link.getAttribute("href"))).toEqual([
      "/results/fallback-a",
      "/results/fallback-b",
    ]);
  });

  test("always orders a valid record timestamp before an invalid one in the same series", async () => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          id: "a-invalid",
          startedAt: "invalid",
          finishedAt: null,
        }),
        makeRun({
          id: "z-valid",
          startedAt: "2026-07-20T08:00:00Z",
          finishedAt: null,
        }),
      ]),
    );

    const group = await screen.findByRole("region", {
      name: /历史系列：ChatGPT 客户端/,
    });
    expect(
      within(group)
        .getAllByRole("link", { name: "查看本次结果" })
        .map((link) => link.getAttribute("href")),
    ).toEqual(["/results/z-valid", "/results/a-invalid"]);
  });

  test("derives each series newest timestamp from every record before ordering groups", async () => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          id: "a-invalid-claude",
          target: {
            kind: "claude_client",
            reportedModel: "Claude Y",
            reasoningEffort: "medium",
            modelSource: "manual",
            modelVerification: "user_confirmed",
          },
          startedAt: "invalid",
          finishedAt: null,
        }),
        makeRun({
          id: "z-newest-claude",
          target: {
            kind: "claude_client",
            reportedModel: "Claude Y",
            reasoningEffort: "medium",
            modelSource: "manual",
            modelVerification: "user_confirmed",
          },
          startedAt: "2026-07-20T08:00:00Z",
          finishedAt: null,
        }),
        makeRun({
          id: "chat-older",
          startedAt: "2026-07-19T08:00:00Z",
          finishedAt: null,
        }),
      ]),
    );

    const groups = await screen.findAllByRole("region", {
      name: /历史系列/,
    });
    expect(
      within(groups[0]).getByRole("heading", { level: 2 }),
    ).toHaveTextContent("Claude 客户端 · Claude Y");
    expect(
      within(groups[1]).getByRole("heading", { level: 2 }),
    ).toHaveTextContent("ChatGPT 客户端 · GPT-X");
  });

  test("uses explicit code-unit order for equal or invalid timestamps", async () => {
    renderHistory(
      makeBackend(async () => [
        makeRun({
          id: "a",
          startedAt: "invalid-a",
          finishedAt: null,
        }),
        makeRun({
          id: "Z",
          startedAt: "invalid-z",
          finishedAt: null,
        }),
      ]),
    );

    const group = await screen.findByRole("region", {
      name: /历史系列：ChatGPT 客户端/,
    });
    expect(
      within(group)
        .getAllByRole("link", { name: "查看本次结果" })
        .map((link) => link.getAttribute("href")),
    ).toEqual(["/results/Z", "/results/a"]);
  });

  test("result links use human labels rather than run identifiers", async () => {
    const user = userEvent.setup();
    renderHistory(
      makeBackend(async () => [
        makeRun({ id: "do-not-show-this-private-id" }),
      ]),
    );

    const link = await screen.findByRole("link", { name: "查看本次结果" });
    expect(link).toHaveAttribute(
      "href",
      "/results/do-not-show-this-private-id",
    );
    expect(link).not.toHaveTextContent("do-not-show-this-private-id");
    expect(document.body.textContent).not.toContain(
      "do-not-show-this-private-id",
    );
    await user.click(link);
    expect(
      screen.getByRole("heading", { name: "结果页" }),
    ).toBeInTheDocument();
  });
});
