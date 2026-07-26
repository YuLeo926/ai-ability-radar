import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { StrictMode } from "react";
import { afterEach, expect, test, vi } from "vitest";
import type {
  ClientSelectionCandidate,
  ClientSelectionDetection,
} from "../api/backend";
import {
  CLIENT_AUTO_DETECT_KEY,
  ClientSelectionPanel,
} from "./ClientSelectionPanel";

function candidate(
  overrides: Partial<ClientSelectionCandidate> = {},
): ClientSelectionCandidate {
  return {
    model: "GPT-5.6",
    reasoningEffort: "max",
    surface: "codex_desktop",
    source: "windows_accessibility",
    confidence: "visible_selector",
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

function panel(
  detect: (
    target: "chat_gpt_client" | "claude_client",
  ) => Promise<ClientSelectionDetection>,
  overrides: Partial<{
    edited: boolean;
    enabled: boolean;
    formDirty: boolean;
    onApply: (value: {
      model?: string;
      reasoningEffort?: string;
    }) => void;
    target: "chat_gpt_client" | "claude_client";
  }> = {},
) {
  return (
    <ClientSelectionPanel
      detect={detect}
      edited={false}
      enabled
      formDirty={false}
      onApply={() => undefined}
      target="chat_gpt_client"
      {...overrides}
    />
  );
}

afterEach(() => {
  localStorage.removeItem(CLIENT_AUTO_DETECT_KEY);
  vi.restoreAllMocks();
});

test("auto-runs once and applies one detected candidate", async () => {
  const detect = vi.fn(async () => ({
    status: "detected" as const,
    candidates: [candidate()],
  }));
  const onApply = vi.fn();

  const view = render(panel(detect, { onApply }));

  expect(
    await screen.findByText("已从 Codex 客户端界面读取，待确认"),
  ).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent(
    "已从 Codex 客户端界面读取，待确认",
  );
  expect(onApply).toHaveBeenCalledWith({
    model: "GPT-5.6",
    reasoningEffort: "max",
  });
  expect(detect).toHaveBeenCalledTimes(1);
  expect(detect).toHaveBeenCalledWith("chat_gpt_client");

  view.rerender(panel(detect, { onApply }));
  expect(detect).toHaveBeenCalledTimes(1);
});

test("uses the compact selection panel layout without changing its controls", async () => {
  const detect = vi.fn(async () => ({
    status: "multiple" as const,
    candidates: [candidate(), candidate({ model: "GPT-5.6 Codex" })],
  }));

  const { container } = render(panel(detect));

  await screen.findByRole("radiogroup");
  const root = container.querySelector(".selection-panel");
  expect(root).not.toBeNull();
  expect(root?.querySelector(".selection-panel-header")).not.toBeNull();
  expect(root?.querySelector(".selection-status")).toHaveAttribute(
    "role",
    "status",
  );
  expect(root?.querySelector(".selection-candidates")).toHaveAttribute(
    "role",
    "radiogroup",
  );
});

test("StrictMode mount reuses the same in-flight automatic request", async () => {
  const pending = deferred<ClientSelectionDetection>();
  const detect = vi.fn(() => pending.promise);
  const onApply = vi.fn();

  render(
    <StrictMode>
      {panel(detect, { onApply })}
    </StrictMode>,
  );
  await waitFor(() => expect(detect).toHaveBeenCalledOnce());
  await act(async () => {
    pending.resolve({
      status: "detected",
      candidates: [candidate()],
    });
    await pending.promise;
  });

  expect(detect).toHaveBeenCalledOnce();
  expect(onApply).toHaveBeenCalledOnce();
});

test("multiple renders distinct radio choices and applies only a chosen result", async () => {
  const detect = vi.fn(async () => ({
    status: "multiple" as const,
    candidates: [
      candidate(),
      candidate(),
      candidate({
        model: "GPT-5.6 Codex",
        reasoningEffort: "high",
        surface: "chatgpt" as const,
        confidence: "best_effort" as const,
      }),
    ],
  }));
  const onApply = vi.fn();
  const user = userEvent.setup();

  render(panel(detect, { onApply }));

  expect(
    await screen.findByText("识别到多个客户端选择，请选择后应用"),
  ).toBeInTheDocument();
  const choices = screen.getByRole("radiogroup", {
    name: "客户端识别结果",
  });
  expect(within(choices).getAllByRole("radio")).toHaveLength(2);
  expect(onApply).not.toHaveBeenCalled();
  expect(
    screen.getByRole("button", { name: "应用识别结果" }),
  ).toBeDisabled();

  await user.click(
    within(choices).getByRole("radio", {
      name: /GPT-5\.6 Codex.*high.*ChatGPT/,
    }),
  );
  expect(onApply).not.toHaveBeenCalled();
  await user.click(
    screen.getByRole("button", { name: "应用识别结果" }),
  );
  expect(onApply).toHaveBeenCalledOnce();
  expect(onApply).toHaveBeenCalledWith({
    model: "GPT-5.6 Codex",
    reasoningEffort: "high",
  });
});

test("duplicate-only multiple follows the clean single path", async () => {
  const first = candidate({ reasoningEffort: undefined });
  const detect = vi.fn(async () => ({
    status: "multiple" as const,
    candidates: [first, { ...first, reasoningEffort: null }],
  }));
  const onApply = vi.fn();

  render(panel(detect, { formDirty: false, onApply }));

  await waitFor(() =>
    expect(onApply).toHaveBeenCalledWith({ model: "GPT-5.6" }),
  );
  expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
  expect(
    screen.queryByText("识别到多个客户端选择，请选择后应用"),
  ).not.toBeInTheDocument();
});

test("duplicate-only multiple uses latest dirty state before explicit single apply", async () => {
  const pending = deferred<ClientSelectionDetection>();
  const first = candidate({ model: undefined });
  const detect = vi.fn(() => pending.promise);
  const onApply = vi.fn();
  const user = userEvent.setup();
  const view = render(panel(detect, { formDirty: false, onApply }));

  view.rerender(panel(detect, { formDirty: true, onApply }));
  await act(async () => {
    pending.resolve({
      status: "multiple",
      candidates: [first, { ...first, model: null }],
    });
    await pending.promise;
  });

  expect(onApply).not.toHaveBeenCalled();
  expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
  await user.click(
    await screen.findByRole("button", { name: "应用识别结果" }),
  );
  expect(onApply).toHaveBeenCalledWith({ reasoningEffort: "max" });
});

test("a single result arriving after typing requires explicit apply", async () => {
  const pending = deferred<ClientSelectionDetection>();
  const detect = vi.fn(() => pending.promise);
  const onApply = vi.fn();
  const user = userEvent.setup();
  const view = render(panel(detect, { formDirty: false, onApply }));

  view.rerender(panel(detect, { formDirty: true, onApply }));
  await act(async () => {
    pending.resolve({
      status: "detected",
      candidates: [candidate()],
    });
    await pending.promise;
  });

  expect(onApply).not.toHaveBeenCalled();
  await user.click(
    await screen.findByRole("button", { name: "应用识别结果" }),
  );
  expect(onApply).toHaveBeenCalledWith({
    model: "GPT-5.6",
    reasoningEffort: "max",
  });
});

test("candidate rows show only reviewed surface, source, and confidence labels", async () => {
  const detect = vi.fn(async () => ({
    status: "multiple" as const,
    candidates: [
      {
        ...candidate(),
        windowTitle: "private conversation title",
        processPath: "private-process-path",
        rawControls: ["private conversation body"],
      },
      candidate({
        model: "Claude Sonnet",
        reasoningEffort: null,
        surface: "claude",
        confidence: "best_effort",
      }),
    ],
  })) as unknown as (
    target: "chat_gpt_client" | "claude_client",
  ) => Promise<ClientSelectionDetection>;

  render(panel(detect));

  await screen.findByRole("radiogroup", { name: "客户端识别结果" });
  expect(screen.getAllByText("Windows 可访问性")).toHaveLength(2);
  expect(screen.getByText("可见选择器")).toBeInTheDocument();
  expect(screen.getByText("最佳努力")).toBeInTheDocument();
  expect(screen.getByText("Codex")).toBeInTheDocument();
  expect(screen.getByText("Claude")).toBeInTheDocument();
  expect(document.body).not.toHaveTextContent("private conversation title");
  expect(document.body).not.toHaveTextContent("private-process-path");
  expect(document.body).not.toHaveTextContent("private conversation body");
});

test.each([
  ["not_running", "未检测到正在运行的客户端，可手动填写"],
  ["not_exposed", "客户端没有公开当前选择，可手动填写"],
  ["unsupported", "当前系统不支持自动读取，可手动填写"],
  ["timed_out", "读取客户端选择超时，可手动填写"],
  ["failed", "无法读取客户端选择，可手动填写"],
] as const)("%s preserves manual fallback", async (status, copy) => {
  const detect = vi.fn(async () => ({ status, candidates: [] }));

  render(
    <>
      <input aria-label="外部模型字段" />
      {panel(detect)}
    </>,
  );

  expect(await screen.findByRole("status")).toHaveTextContent(copy);
  expect(screen.getByLabelText("外部模型字段")).toBeEnabled();
});

test.each(["async", "sync"] as const)(
  "%s detection failure preserves manual fallback",
  async (mode) => {
    const detect =
      mode === "sync"
        ? vi.fn(() => {
            throw new Error("同步失败");
          })
        : vi.fn(async () => {
            throw new Error("异步失败");
          });

    render(panel(detect));

    expect(await screen.findByRole("status")).toHaveTextContent(
      "无法读取客户端选择，可手动填写",
    );
  },
);

test("a stale first request cannot overwrite a newer refresh", async () => {
  const first = deferred<ClientSelectionDetection>();
  const second = deferred<ClientSelectionDetection>();
  const detect = vi
    .fn()
    .mockImplementationOnce(() => first.promise)
    .mockImplementationOnce(() => second.promise);
  const onApply = vi.fn();
  const user = userEvent.setup();

  render(panel(detect, { onApply }));
  await user.click(screen.getByRole("button", { name: "重新识别" }));
  await act(async () => {
    second.resolve({
      status: "detected",
      candidates: [
        candidate({
          model: "GPT-New",
          surface: "chatgpt",
        }),
      ],
    });
    await second.promise;
  });
  await act(async () => {
    first.resolve({
      status: "detected",
      candidates: [candidate({ model: "GPT-Stale" })],
    });
    await first.promise;
  });

  expect(detect).toHaveBeenCalledTimes(2);
  expect(onApply).toHaveBeenCalledTimes(1);
  expect(onApply).toHaveBeenCalledWith({
    model: "GPT-New",
    reasoningEffort: "max",
  });
  expect(document.body).not.toHaveTextContent("GPT-Stale");
});

test("unmount ignores request completion", async () => {
  const pending = deferred<ClientSelectionDetection>();
  const onApply = vi.fn();
  const view = render(panel(vi.fn(() => pending.promise), { onApply }));

  view.unmount();
  await act(async () => {
    pending.resolve({
      status: "detected",
      candidates: [candidate()],
    });
    await pending.promise;
  });

  expect(onApply).not.toHaveBeenCalled();
});

test("disabling automatic detection writes the setting without calling the backend", async () => {
  const detect = vi.fn(async () => ({
    status: "not_running" as const,
    candidates: [],
  }));
  const user = userEvent.setup();

  render(panel(detect, { enabled: false }));
  await user.click(
    screen.getByRole("checkbox", {
      name: "进入设置页时自动读取客户端可见选择器",
    }),
  );

  expect(localStorage.getItem(CLIENT_AUTO_DETECT_KEY)).toBe("false");
  expect(detect).not.toHaveBeenCalled();
  expect(screen.getByRole("status")).toHaveTextContent("可手动填写");
});

test("only exact stored false disables mount detection and re-enabling runs once", async () => {
  localStorage.setItem(CLIENT_AUTO_DETECT_KEY, "false");
  const detect = vi.fn(async () => ({
    status: "not_running" as const,
    candidates: [],
  }));
  const user = userEvent.setup();
  const view = render(panel(detect));

  expect(detect).not.toHaveBeenCalled();
  expect(screen.getByRole("status")).toHaveTextContent(
    "自动读取已关闭，可手动填写",
  );
  await user.click(
    screen.getByRole("checkbox", {
      name: "进入设置页时自动读取客户端可见选择器",
    }),
  );
  await waitFor(() => expect(detect).toHaveBeenCalledOnce());
  expect(localStorage.getItem(CLIENT_AUTO_DETECT_KEY)).toBe("true");

  view.rerender(panel(detect));
  expect(detect).toHaveBeenCalledOnce();
});

test("non-false storage and throwing storage access fall back to enabled", async () => {
  localStorage.setItem(CLIENT_AUTO_DETECT_KEY, "corrupt");
  const firstDetect = vi.fn(async () => ({
    status: "not_running" as const,
    candidates: [],
  }));
  const first = render(panel(firstDetect));
  await waitFor(() => expect(firstDetect).toHaveBeenCalledOnce());
  first.unmount();

  const descriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
  if (!descriptor) {
    throw new Error("jsdom localStorage descriptor is required");
  }
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    get() {
      throw new DOMException("blocked", "SecurityError");
    },
  });
  try {
    const secondDetect = vi.fn(async () => ({
      status: "not_running" as const,
      candidates: [],
    }));
    expect(() => render(panel(secondDetect))).not.toThrow();
    await waitFor(() => expect(secondDetect).toHaveBeenCalledOnce());
  } finally {
    Object.defineProperty(window, "localStorage", descriptor);
  }
});

test("throwing storage writes do not crash or start detection while disabled", async () => {
  const setItem = vi
    .spyOn(Storage.prototype, "setItem")
    .mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });
  const detect = vi.fn(async () => ({
    status: "not_running" as const,
    candidates: [],
  }));
  const user = userEvent.setup();

  render(panel(detect, { enabled: false }));
  await user.click(
    screen.getByRole("checkbox", {
      name: "进入设置页时自动读取客户端可见选择器",
    }),
  );

  expect(setItem).toHaveBeenCalledWith(CLIENT_AUTO_DETECT_KEY, "false");
  expect(detect).not.toHaveBeenCalled();
});

test("target changes invalidate the old request and detect only the current target", async () => {
  const first = deferred<ClientSelectionDetection>();
  const second = deferred<ClientSelectionDetection>();
  const detect = vi
    .fn()
    .mockImplementationOnce(() => first.promise)
    .mockImplementationOnce(() => second.promise);
  const onApply = vi.fn();
  const view = render(
    panel(detect, { onApply, target: "chat_gpt_client" }),
  );

  view.rerender(
    panel(detect, { onApply, target: "claude_client" }),
  );
  await act(async () => {
    second.resolve({
      status: "detected",
      candidates: [
        candidate({
          model: "Claude Sonnet",
          surface: "claude",
        }),
      ],
    });
    await second.promise;
    first.resolve({
      status: "detected",
      candidates: [candidate({ model: "GPT-Stale" })],
    });
    await first.promise;
  });

  expect(detect.mock.calls).toEqual([
    ["chat_gpt_client"],
    ["claude_client"],
  ]);
  expect(onApply).toHaveBeenCalledOnce();
  expect(onApply).toHaveBeenCalledWith({
    model: "Claude Sonnet",
    reasoningEffort: "max",
  });
});

test("disabling the route or preference invalidates an in-flight result", async () => {
  const routePending = deferred<ClientSelectionDetection>();
  const preferencePending = deferred<ClientSelectionDetection>();
  const routeApply = vi.fn();
  const route = render(
    panel(vi.fn(() => routePending.promise), { onApply: routeApply }),
  );
  route.rerender(
    panel(vi.fn(() => routePending.promise), {
      enabled: false,
      onApply: routeApply,
    }),
  );
  await act(async () => {
    routePending.resolve({
      status: "detected",
      candidates: [candidate()],
    });
    await routePending.promise;
  });
  expect(routeApply).not.toHaveBeenCalled();
  route.unmount();

  const preferenceApply = vi.fn();
  const user = userEvent.setup();
  render(
    panel(vi.fn(() => preferencePending.promise), {
      onApply: preferenceApply,
    }),
  );
  await user.click(
    screen.getByRole("checkbox", {
      name: "进入设置页时自动读取客户端可见选择器",
    }),
  );
  await act(async () => {
    preferencePending.resolve({
      status: "detected",
      candidates: [candidate()],
    });
    await preferencePending.promise;
  });
  expect(preferenceApply).not.toHaveBeenCalled();
});

test("edited state asks the user to confirm current values", async () => {
  render(
    panel(
      vi.fn(async () => ({
        status: "not_running" as const,
        candidates: [],
      })),
      { edited: true },
    ),
  );

  expect(
    await screen.findByText("用户已修改，请确认当前填写值"),
  ).toBeInTheDocument();
});
