import axe from "axe-core";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, test, vi } from "vitest";
import { BackendProvider } from "../api/BackendContext";
import type {
  Backend,
  Bootstrap,
  RunDetail,
  RunRecord,
  TaskResult,
} from "../api/backend";
import { unsupportedBatchBackend } from "../api/backend";
import { App } from "../app/App";
import { AppRoutes } from "../app/routes";
import {
  applyTheme,
  readStoredTheme,
  THEME_STORAGE_KEY,
  ThemeToggle,
} from "../components/ThemeToggle";
import { messages, translate } from "../i18n/messages";
import indexHtml from "../../index.html?raw";

const runId = "task21-synthetic-run";
const sourceRoot = join(process.cwd(), "src");
const tokensCss = readFileSync(join(sourceRoot, "styles", "tokens.css"), "utf8");

function withThrowingLocalStorage<T>(run: () => T): T {
  const descriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
  if (!descriptor) {
    throw new Error("jsdom localStorage descriptor is required for this test");
  }
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    get() {
      throw new DOMException("blocked", "SecurityError");
    },
  });
  try {
    return run();
  } finally {
    Object.defineProperty(window, "localStorage", descriptor);
  }
}

function contrastRatio(first: string, second: string): number {
  const luminance = (color: string) => {
    const channels = color
      .trim()
      .replace(/^#/, "")
      .match(/.{2}/g)
      ?.map((channel) => Number.parseInt(channel, 16) / 255);
    if (!channels || channels.length !== 3 || channels.some(Number.isNaN)) {
      throw new Error(`Expected a six-digit CSS hex color, received ${color}`);
    }
    const [red, green, blue] = channels.map((channel) =>
      channel <= 0.04045
        ? channel / 12.92
        : ((channel + 0.055) / 1.055) ** 2.4,
    );
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
  };
  const lighter = Math.max(luminance(first), luminance(second));
  const darker = Math.min(luminance(first), luminance(second));
  return (lighter + 0.05) / (darker + 0.05);
}

function tokenValue(theme: "light" | "dark", name: string): string {
  const selector = theme === "light" ? ":root {" : ':root[data-theme="dark"] {';
  const start = tokensCss.indexOf(selector);
  const end = tokensCss.indexOf("}", start);
  const block = tokensCss.slice(start, end);
  const match = block.match(new RegExp(`${name}:\\s*(#[0-9a-f]{6})`, "i"));
  if (!match) {
    throw new Error(`Missing ${theme} ${name} token`);
  }
  return match[1];
}

function withoutCssUrlPayloads(source: string): string {
  const urlStart = /\burl\s*\(/gi;
  let cursor = 0;
  let output = "";
  let match: RegExpExecArray | null;

  while ((match = urlStart.exec(source))) {
    output += `${source.slice(cursor, match.index)}url()`;
    let index = urlStart.lastIndex;
    let depth = 1;
    let quote: '"' | "'" | null = null;

    while (index < source.length && depth > 0) {
      const character = source[index];
      if (character === "\\") {
        index += 2;
        continue;
      }
      if (quote) {
        if (character === quote) quote = null;
      } else if (character === '"' || character === "'") {
        quote = character;
      } else if (character === "(") {
        depth += 1;
      } else if (character === ")") {
        depth -= 1;
      }
      index += 1;
    }

    cursor = index;
    urlStart.lastIndex = index;
  }

  return output + source.slice(cursor);
}

function cssRuleBody(source: string, selector: string): string {
  const selectorIndex = source.indexOf(selector);
  if (selectorIndex < 0) {
    throw new Error(`Missing CSS selector: ${selector}`);
  }
  const openingBrace = source.indexOf("{", selectorIndex + selector.length);
  if (openingBrace < 0) {
    throw new Error(`Missing opening brace for CSS selector: ${selector}`);
  }

  let depth = 1;
  let cursor = openingBrace + 1;
  while (cursor < source.length && depth > 0) {
    if (source[cursor] === "{") depth += 1;
    if (source[cursor] === "}") depth -= 1;
    cursor += 1;
  }
  if (depth !== 0) {
    throw new Error(`Missing closing brace for CSS selector: ${selector}`);
  }
  return source.slice(openingBrace + 1, cursor - 1);
}

function tokenDeclarations(selector: string): Record<string, string> {
  const declarations: Record<string, string> = {};
  const block = cssRuleBody(tokensCss, selector);
  const declaration = /^\s*(--[\w-]+):\s*([\s\S]*?);\s*$/gm;
  let match: RegExpExecArray | null;
  while ((match = declaration.exec(block))) {
    declarations[match[1]] = match[2].replace(/\s+/g, " ").trim();
  }
  return declarations;
}

function bootstrap(): Bootstrap {
  return {
    clientPack: {
      id: "client-quick",
      version: "1.0.0",
      title: "客户端快速体检",
      taskCount: 8,
      estimatedMinutes: "10–15",
    },
    cliPack: {
      id: "cli-quick",
      version: "1.0.0",
      title: "CLI 快速体检",
      taskCount: 4,
      estimatedMinutes: "30–60",
    },
    batchCapabilities: ["guided_quick_v1", "cli_standard_v1"],
    targets: [
      {
        kind: "chat_gpt_client",
        installed: true,
        version: null,
        authState: "unknown",
        status: "ready",
        source: null,
        prerequisites: [],
      },
      {
        kind: "codex_cli",
        installed: true,
        version: "codex 1.2.3",
        authState: "ready",
        status: "ready",
        source: "reviewed_npm",
        prerequisites: [
          { name: "Node.js 22/24 LTS", available: true, version: "v22.22.0" },
        ],
      },
    ],
  };
}

function runRecord(): RunRecord {
  return {
    id: runId,
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
    totalTasks: 2,
    completedTasks: 2,
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
      abilityScore: 50,
      passedTasks: 1,
      validTasks: 2,
      totalTasks: 2,
      categoryScores: { logic: 50 },
    },
  };
}

function runDetail(): RunDetail {
  const taskResults: TaskResult[] = [
    {
      runId,
      taskId: "task-1",
      category: "logic",
      outcome: "passed",
      score: 100,
      failureKind: null,
      durationMs: 1_000,
      answerRelPath: "private-answer.txt",
    },
    {
      runId,
      taskId: "task-2",
      category: "logic",
      outcome: "failed",
      score: 0,
      failureKind: "wrong_answer",
      durationMs: 1_000,
      answerRelPath: "private-answer-2.txt",
    },
  ];
  return { run: runRecord(), taskResults };
}

function fakeBackend(overrides: Partial<Backend> = {}): Backend {
  return {
    ...unsupportedBatchBackend,
    getBootstrap: vi.fn(async () => bootstrap()),
    detectClientSelection: vi.fn<Backend["detectClientSelection"]>(
      async () => ({
        status: "not_running",
        candidates: [],
      }),
    ),
    startManualRun: vi.fn(async (input) => ({
      ...runRecord(),
      target: input.target,
      status: "running" as const,
      finishedAt: null,
      completedTasks: 0,
      score: undefined,
    })),
    nextManualStep: vi.fn(async () => ({
      runId,
      taskId: "task-1",
      taskNumber: 1,
      totalTasks: 2,
      prompt: "合成测试题目",
    })),
    submitManualAnswer: vi.fn(async () => runDetail().taskResults[0]),
    startCliRun: vi.fn(async () => runRecord()),
    resumeManualRun: vi.fn(async () => runRecord()),
    resumeCliRun: vi.fn(async () => runRecord()),
    cancelRun: vi.fn(async () => false),
    interruptManualRun: vi.fn(async () => false),
    listRuns: vi.fn(async () => [runRecord()]),
    getRunDetail: vi.fn(async () => runDetail()),
    exportPublicReport: vi.fn(async () => null),
    deleteRawArtifacts: vi.fn(async () => undefined),
    deleteRun: vi.fn(async () => false),
    deleteTargetHistory: vi.fn(async () => 0),
    getDataSettings: vi.fn(async () => ({
      rawRetentionDays: null,
      cleanupPending: false,
    })),
    setRawRetention: vi.fn(async () => 0),
    exportFullBackup: vi.fn(async () => false),
    onRunEvent: vi.fn(async () => () => undefined),
    onRunError: vi.fn(async () => () => undefined),
    ...overrides,
  };
}

function renderRoute(path: string, backend = fakeBackend()) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <BackendProvider backend={backend}>
        <AppRoutes />
      </BackendProvider>
    </MemoryRouter>,
  );
}

async function expectNoSeriousAxeViolations(container: HTMLElement) {
  const result = await axe.run(container, {
    runOnly: {
      type: "tag",
      values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"],
    },
  });
  const serious = result.violations.filter((violation) =>
      ["serious", "critical"].includes(violation.impact ?? ""),
    );
  expect(serious).toEqual([]);
}

describe("Task 21 accessibility baseline", () => {
  afterEach(() => {
    document.documentElement.removeAttribute("data-theme");
    localStorage.removeItem(THEME_STORAGE_KEY);
  });

  test.each([
    ["/", "选择要体检的 AI"],
    ["/manual/chat_gpt_client", "ChatGPT 客户端快速体检"],
    ["/manual/claude_client", "Claude 客户端快速体检"],
    ["/cli/codex_cli", "Codex CLI 快速体检"],
    ["/cli/claude_code", "Claude Code 快速体检"],
    ["/history", "严格同条件历史"],
    [`/results/${runId}`, "本次客观结果"],
    ["/missing", "没有找到这个页面"],
  ])("%s has one focusable named main landmark", async (path, heading) => {
    const { container } = renderRoute(path);
    await screen.findByRole("heading", { name: heading });
    const mains = container.querySelectorAll("main");
    expect(mains).toHaveLength(1);
    expect(mains[0]).toHaveAttribute("id", "page-content");
    expect(mains[0]).toHaveAttribute("tabindex", "-1");
  });

  test("home, history data, and result states have no serious axe violations", async () => {
    const home = renderRoute("/");
    await screen.findByRole("heading", { name: "选择要体检的 AI" });
    await expectNoSeriousAxeViolations(home.container);
    home.unmount();

    const history = renderRoute("/history");
    await screen.findByRole("heading", { name: "严格同条件历史" });
    await expectNoSeriousAxeViolations(history.container);
    history.unmount();

    const result = renderRoute(`/results/${runId}`);
    await screen.findByRole("heading", { name: "本次客观结果" });
    await expectNoSeriousAxeViolations(result.container);
  });

  test("CLI detection failures keep their precise accessible status name", async () => {
    const unavailable = bootstrap();
    unavailable.targets = unavailable.targets.map((target) =>
      target.kind === "codex_cli"
        ? {
            ...target,
            installed: false,
            version: null,
            authState: "unknown",
            status: "entry_inaccessible",
            source: null,
          }
        : target,
    );
    renderRoute("/", fakeBackend({ getBootstrap: vi.fn(async () => unavailable) }));

    expect(
      await screen.findByRole("status", {
        name: "Codex CLI 状态：入口不可访问",
      }),
    ).toBeInTheDocument();
  });

  test("manual setup, CLI setup, and report dialog have no serious axe violations", async () => {
    const manual = renderRoute("/manual/chat_gpt_client");
    await screen.findByRole("heading", {
      name: "ChatGPT 客户端快速体检",
    });
    await expectNoSeriousAxeViolations(manual.container);
    manual.unmount();

    const cli = renderRoute("/cli/codex_cli");
    await screen.findByRole("heading", { name: "Codex CLI 快速体检" });
    await expectNoSeriousAxeViolations(cli.container);
    cli.unmount();

    const user = userEvent.setup();
    const result = renderRoute(`/results/${runId}`);
    await screen.findByRole("heading", { name: "本次客观结果" });
    await user.click(
      screen.getByRole("button", { name: "检查并导出可分享报告" }),
    );
    const dialog = await screen.findByRole("dialog", { name: "导出前检查" });
    const dialogTitle = screen.getByRole("heading", { name: "导出前检查" });
    await waitFor(() => expect(dialogTitle).toHaveFocus());
    await user.tab({ shift: true });
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    await user.tab();
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    await expectNoSeriousAxeViolations(result.container);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "检查并导出可分享报告" }),
    ).toHaveFocus();
  });

  test("multiple client selection exposes labelled radio and checkbox controls without serious axe violations", async () => {
    const manual = renderRoute(
      "/manual/chat_gpt_client",
      fakeBackend({
        detectClientSelection: vi.fn<Backend["detectClientSelection"]>(
          async () => ({
            status: "multiple",
            candidates: [
              {
                model: "GPT-5.6",
                reasoningEffort: "high",
                surface: "chatgpt",
                source: "windows_accessibility",
                confidence: "visible_selector",
              },
              {
                model: "GPT-5.6 Codex",
                reasoningEffort: "max",
                surface: "codex_desktop",
                source: "windows_accessibility",
                confidence: "best_effort",
              },
            ],
          }),
        ),
      }),
    );

    const choices = await screen.findByRole("radiogroup", {
      name: "客户端识别结果",
    });
    expect(choices.querySelectorAll('input[type="radio"]')).toHaveLength(2);
    expect(
      screen.getByRole("checkbox", {
        name: "进入设置页时自动读取客户端可见选择器",
      }),
    ).toBeInTheDocument();
    await expectNoSeriousAxeViolations(manual.container);
  });

  test("manual task, CLI progress, history empty/confirmation, and not-found pass axe", async () => {
    const user = userEvent.setup();

    const manual = renderRoute("/manual/chat_gpt_client");
    await screen.findByRole("heading", {
      name: "ChatGPT 客户端快速体检",
    });
    await user.type(
      screen.getByRole("textbox", { name: "当前显示的模型" }),
      "GPT-X",
    );
    await user.click(
      screen.getByRole("checkbox", {
        name: "我会为每道题新建空白对话",
      }),
    );
    await user.click(screen.getByRole("button", { name: "开始快速体检" }));
    await screen.findByRole("heading", {
      name: "在新空白对话中完成这道题",
    });
    await expectNoSeriousAxeViolations(manual.container);
    manual.unmount();

    const cliRun = runRecord();
    cliRun.status = "running";
    cliRun.finishedAt = null;
    cliRun.score = null;
    cliRun.totalTasks = 4;
    cliRun.completedTasks = 1;
    cliRun.target = {
      kind: "codex_cli",
      reportedModel: "default",
      reasoningEffort: null,
      modelSource: "default_route",
      modelVerification: "unverified",
    };
    const cli = renderRoute(
      "/cli/codex_cli",
      fakeBackend({
        startCliRun: vi.fn(async () => cliRun),
        getRunDetail: vi.fn(async () => ({ run: cliRun, taskResults: [] })),
      }),
    );
    await screen.findByRole("heading", { name: "Codex CLI 快速体检" });
    await user.click(
      screen.getByRole("checkbox", {
        name: "我了解本次运行可能消耗自己的订阅额度或 API 余额",
      }),
    );
    await user.click(screen.getByRole("button", { name: "开始 4 个任务" }));
    await screen.findByRole("heading", { name: "正在完成本地微型项目" });
    await expectNoSeriousAxeViolations(cli.container);
    cli.unmount();

    const emptyHistory = renderRoute(
      "/history",
      fakeBackend({ listRuns: vi.fn(async () => []) }),
    );
    await screen.findByRole("heading", { name: "还没有体检记录" });
    await expectNoSeriousAxeViolations(emptyHistory.container);
    emptyHistory.unmount();

    const history = renderRoute("/history");
    await screen.findByRole("heading", { name: "严格同条件历史" });
    await user.click(
      screen.getByRole("button", { name: /删除该测试对象全部历史/ }),
    );
    await screen.findByRole("group", { name: /确认删除/ });
    await expectNoSeriousAxeViolations(history.container);
    history.unmount();

    const notFound = renderRoute("/missing");
    await screen.findByRole("heading", { name: "没有找到这个页面" });
    await expectNoSeriousAxeViolations(notFound.container);
  });

  test("the keyboard-visible skip link focuses the current main", async () => {
    const user = userEvent.setup();
    renderRoute("/");
    await screen.findByRole("heading", { name: "选择要体检的 AI" });
    const skip = screen.getByRole("link", { name: "跳到主要内容" });
    await user.tab();
    expect(skip).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("main")).toHaveFocus();
  });

  test("the theme control exposes all deterministic choices", async () => {
    const user = userEvent.setup();
    renderRoute("/");
    await screen.findByRole("heading", { name: "选择要体检的 AI" });
    const theme = screen.getByRole("combobox", { name: "配色主题" });
    expect(theme).toHaveValue("system");
    expect(theme).toHaveTextContent("跟随系统");
    expect(theme).toHaveTextContent("浅色");
    expect(theme).toHaveTextContent("深色");

    await user.selectOptions(theme, "light");
    expect(document.documentElement).toHaveAttribute("data-theme", "light");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");

    await user.selectOptions(theme, "dark");
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");

    await user.selectOptions(theme, "system");
    expect(document.documentElement).not.toHaveAttribute("data-theme");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  test("corrupt and blocked theme storage safely fall back to system", async () => {
    localStorage.setItem(THEME_STORAGE_KEY, "sepia");
    renderRoute("/");
    await screen.findByRole("heading", { name: "选择要体检的 AI" });
    expect(screen.getByRole("combobox", { name: "配色主题" })).toHaveValue(
      "system",
    );
    await waitFor(() =>
      expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull(),
    );

    const blockedRead = {
      getItem: () => {
        throw new DOMException("blocked", "SecurityError");
      },
    };
    expect(readStoredTheme(blockedRead)).toBe("system");

    const root = document.createElement("div");
    const blockedWrite = {
      setItem: () => {
        throw new DOMException("blocked", "SecurityError");
      },
      removeItem: () => {
        throw new DOMException("blocked", "SecurityError");
      },
    };
    expect(() => applyTheme("dark", root, blockedWrite)).not.toThrow();
    expect(root).toHaveAttribute("data-theme", "dark");
    expect(() => applyTheme("system", root, blockedWrite)).not.toThrow();
    expect(root).not.toHaveAttribute("data-theme");
  });

  test("default theme reads contain a throwing localStorage getter", () => {
    expect(withThrowingLocalStorage(() => readStoredTheme())).toBe("system");
  });

  test("default theme writes contain a throwing localStorage getter", () => {
    const root = document.createElement("div");
    expect(() =>
      withThrowingLocalStorage(() => applyTheme("dark", root)),
    ).not.toThrow();
    expect(root).toHaveAttribute("data-theme", "dark");
  });

  test("ThemeToggle renders when the global localStorage getter is blocked", () => {
    expect(() =>
      withThrowingLocalStorage(() => render(<ThemeToggle />)),
    ).not.toThrow();
    expect(
      screen.getByRole("combobox", { name: messages["theme.label"] }),
    ).toHaveValue("system");
  });

  test("the real App root renders when the global localStorage getter is blocked", () => {
    expect(() =>
      withThrowingLocalStorage(() => render(<App backend={fakeBackend()} />)),
    ).not.toThrow();
    expect(
      screen.getByRole("combobox", { name: messages["theme.label"] }),
    ).toBeInTheDocument();
  });

  test.each(["light", "dark"] as const)(
    "%s non-text tokens maintain 3:1 contrast against adjacent surfaces",
    (theme) => {
      document.documentElement.dataset.theme = theme;
      const surfaces = [
        "--canvas",
        "--panel",
        "--panel-raised",
        "--surface-muted",
      ];
      for (const token of ["--border-strong", "--focus"]) {
        for (const surface of surfaces) {
          expect(
            contrastRatio(
              tokenValue(theme, token),
              tokenValue(theme, surface),
            ),
            `${token} against ${surface}`,
          ).toBeGreaterThanOrEqual(3);
        }
      }
    },
  );

  test("shared tokens and shell background use the precision-radar palette", () => {
    const lightTokens = {
      "--canvas": "#f2f3ee",
      "--panel": "#fbfcf8",
      "--panel-raised": "#ffffff",
      "--surface-muted": "#e7eeea",
      "--surface-strong": "#d6e3de",
      "--text-primary": "#102824",
      "--text-muted": "#4c6660",
      "--border": "#b7c9c3",
      "--border-strong": "#607d76",
      "--brand": "#0b7067",
      "--brand-strong": "#07584f",
      "--brand-soft": "#d7ebe5",
      "--mineral": "#315e70",
      "--success": "#176c4f",
      "--success-soft": "#dceee4",
      "--warning": "#865400",
      "--warning-soft": "#f5e6bf",
      "--danger": "#96363b",
      "--danger-soft": "#f5dfe1",
      "--focus": "#145da0",
      "--on-brand": "#ffffff",
      "--scrim": "rgb(2 15 14 / 72%)",
      "--grid-pattern": "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='40' viewBox='0 0 40 40'%3E%3Cpath d='M40 0H0V40' fill='none' stroke='%230b7067' stroke-opacity='.055'/%3E%3C/svg%3E\")",
      "--shadow-sm": "0 1px 2px rgb(16 40 36 / 8%)",
      "--shadow-md": "0 14px 34px rgb(16 40 36 / 10%)",
      "--radius-sm": "0.4rem",
      "--radius-md": "0.75rem",
      "--space-1": "0.25rem",
      "--space-2": "0.5rem",
      "--space-3": "0.75rem",
      "--space-4": "1rem",
      "--space-5": "1.5rem",
      "--space-6": "2rem",
      "--space-7": "3rem",
      "--font-display": "\"Segoe UI Variable Display\", \"Microsoft YaHei UI\", \"Segoe UI\", sans-serif",
      "--font-body": "\"Segoe UI Variable Text\", \"Microsoft YaHei UI\", \"Segoe UI\", sans-serif",
    };
    const darkTokens = {
      "--canvas": "#0a1513",
      "--panel": "#10221f",
      "--panel-raised": "#152b27",
      "--surface-muted": "#1c3732",
      "--surface-strong": "#284a43",
      "--text-primary": "#edf6f2",
      "--text-muted": "#b6cbc5",
      "--border": "#3a5a53",
      "--border-strong": "#78978f",
      "--brand": "#78d4c5",
      "--brand-strong": "#a0e7dc",
      "--brand-soft": "#173e37",
      "--mineral": "#91bfd0",
      "--success": "#88d8af",
      "--success-soft": "#173c2f",
      "--warning": "#efc36d",
      "--warning-soft": "#45371c",
      "--danger": "#ff9ea4",
      "--danger-soft": "#47272b",
      "--focus": "#9dcaff",
      "--on-brand": "#071a17",
      "--grid-pattern": "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='40' viewBox='0 0 40 40'%3E%3Cpath d='M40 0H0V40' fill='none' stroke='%2378d4c5' stroke-opacity='.055'/%3E%3C/svg%3E\")",
    };

    expect(tokenDeclarations(":root")).toEqual(lightTokens);
    expect(tokenDeclarations(':root[data-theme="dark"]')).toEqual(darkTokens);
    expect(tokenDeclarations(":root:not([data-theme])")).toEqual(darkTokens);

    const appCss = readFileSync(join(sourceRoot, "styles", "app.css"), "utf8");
    expect(appCss).toMatch(
      /body\s*\{[\s\S]*background-color:\s*var\(--canvas\);[\s\S]*background-image:\s*var\(--grid-pattern\);[\s\S]*background-size:\s*2\.5rem 2\.5rem;/,
    );
    expect(appCss).toMatch(
      /\.topbar-inner\s*\{[\s\S]*width:\s*min\(76rem, calc\(100% - clamp\(1\.25rem, 4vw, 3rem\)\)\);/,
    );
    expect(appCss).not.toMatch(
      /(?:linear|radial|conic|repeating-radial)-gradient\s*\(/i,
    );
    expect(appCss).toMatch(
      /\.target-card::after\s*\{[\s\S]*border-radius:\s*50%;[\s\S]*box-shadow:/,
    );
    const appDeclarations = withoutCssUrlPayloads(appCss);
    expect(appDeclarations).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    expect(appDeclarations).not.toMatch(
      /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\s*\(/i,
    );

    const narrowTopbar = cssRuleBody(
      appCss,
      "@media (max-width: 48rem)",
    );
    expect(narrowTopbar).toMatch(
      /\.topbar-inner\s*\{\s*position:\s*relative;\s*grid-template-columns:\s*minmax\(0, 1fr\) auto;/,
    );
    expect(narrowTopbar).not.toMatch(/\.topbar\s*\{/);
  });

  test("the precision-radar surface uses no CSS gradients", () => {
    const styleFiles = [
      join(sourceRoot, "styles", "app.css"),
      join(sourceRoot, "pages", "ManualRunPage.css"),
      join(sourceRoot, "pages", "CliRunPage.css"),
      join(sourceRoot, "pages", "ResultsHistory.css"),
    ];
    for (const file of styleFiles) {
      const source = readFileSync(file, "utf8");
      expect(source, file).not.toMatch(
        /(?:linear|radial|conic|repeating-radial)-gradient\s*\(/i,
      );
    }
  });

  test("page styles consume shared color tokens instead of hard-coded colors", () => {
    const dataUrlFixture =
      ".fixture { background: url(\"data:image/svg+xml,<svg fill='rgb(1 2 3)' stroke='#fff'/>\"); }";
    expect(withoutCssUrlPayloads(dataUrlFixture)).toBe(
      ".fixture { background: url(); }",
    );

    for (const file of [
      "ManualRunPage.css",
      "CliRunPage.css",
      "ResultsHistory.css",
    ]) {
      const source = readFileSync(join(sourceRoot, "pages", file), "utf8");
      const declarations = withoutCssUrlPayloads(source);
      expect(declarations, file).not.toMatch(/#[0-9a-f]{3,8}\b/i);
      expect(declarations, file).not.toMatch(
        /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\s*\(/i,
      );
      expect(source, file).toMatch(/var\(--text-primary\)/);
      expect(source, file).toMatch(/var\(--border\)/);
    }

    const resultsHistory = readFileSync(
      join(sourceRoot, "pages", "ResultsHistory.css"),
      "utf8",
    );
    expect(resultsHistory).not.toMatch(
      /\b(?:gap|padding(?:-(?:top|right|bottom|left))?)\s*:\s*[^;]*\b\d+(?:\.\d+)?rem\b/,
    );
  });

  test("page focus rules use the semantic focus token and preserve forced colors", () => {
    const pageSources = [
      "ManualRunPage.css",
      "CliRunPage.css",
      "ResultsHistory.css",
    ].map((file) => readFileSync(join(sourceRoot, "pages", file), "utf8"));
    for (const source of pageSources) {
      expect(source).not.toMatch(/#f4aa49/i);
      expect(source).toMatch(/outline:\s*0\.2rem solid var\(--focus\)/);
    }
    const globalSource = readFileSync(
      join(sourceRoot, "styles", "app.css"),
      "utf8",
    );
    expect(globalSource).toMatch(/@media\s*\(forced-colors:\s*active\)/);
    expect(globalSource).toMatch(/outline-color:\s*Highlight/i);
  });

  test("an explicit stored theme is restored deterministically", async () => {
    localStorage.setItem(THEME_STORAGE_KEY, "dark");
    renderRoute("/");
    await screen.findByRole("heading", { name: "选择要体检的 AI" });
    expect(screen.getByRole("combobox", { name: "配色主题" })).toHaveValue(
      "dark",
    );
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
  });

  test("shared navigation labels come from the typed Chinese dictionary", async () => {
    renderRoute("/");
    await screen.findByRole("heading", { name: "选择要体检的 AI" });
    expect(
      screen.getByRole("link", { name: messages["app.name"] }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: messages["nav.start"] }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: messages["nav.history"] }),
    ).toBeInTheDocument();
    expect(translate("common.retry")).toBe("重试");
  });

  test("document metadata declares a local non-starter favicon", () => {
    const documentCopy = new DOMParser().parseFromString(indexHtml, "text/html");
    const favicon = documentCopy.querySelector<HTMLLinkElement>(
      'link[rel="icon"]',
    );
    expect(favicon).not.toBeNull();
    expect(favicon?.href).toMatch(/^data:image\/svg\+xml,/);
    expect(favicon?.href).not.toMatch(/vite|tauri/i);
  });

  test("home failure remains an accessible alert state", async () => {
    const backend = fakeBackend({
      getBootstrap: vi.fn(async () => {
        throw new Error("合成环境读取失败");
      }),
    });
    const { container } = renderRoute("/", backend);
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("合成环境读取失败"),
    );
    await expectNoSeriousAxeViolations(container);
  });
});
