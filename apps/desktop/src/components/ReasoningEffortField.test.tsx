import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { ReasoningEffortField } from "./ReasoningEffortField";

test("renders ChatGPT levels and sends the canonical selection", async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(
    <ReasoningEffortField
      emptyLabel="未显示 / 不适用"
      id="effort"
      kind="chat_gpt_client"
      label="推理档位"
      onChange={onChange}
      onValidationChange={() => undefined}
      value=""
    />,
  );

  expect(screen.getByRole("option", { name: "极高" })).toHaveValue("xhigh");
  expect(screen.getByRole("option", { name: "最高" })).toHaveValue("max");
  expect(screen.getByRole("option", { name: "Ultra" })).toHaveValue("ultra");
  await user.selectOptions(screen.getByLabelText("推理档位"), "xhigh");
  expect(onChange).toHaveBeenLastCalledWith("xhigh");
});

test("custom mode preserves manual labels and reports validation", async () => {
  const user = userEvent.setup();
  function Harness() {
    const [value, setValue] = useState("");
    return (
      <ReasoningEffortField
        emptyLabel="未显示 / 不适用"
        id="effort"
        kind="claude_client"
        label="推理档位"
        onChange={setValue}
        onValidationChange={() => undefined}
        value={value}
      />
    );
  }
  render(<Harness />);
  await user.selectOptions(screen.getByLabelText("推理档位"), "__custom__");
  expect(screen.getByRole("alert")).toHaveTextContent("请填写自定义");
  await user.type(screen.getByLabelText("按界面原样填写"), "扩展思考");
  expect(screen.getByLabelText("按界面原样填写")).toHaveValue("扩展思考");
  await user.clear(screen.getByLabelText("按界面原样填写"));
  await user.type(screen.getByLabelText("按界面原样填写"), "想".repeat(41));
  expect(screen.getByRole("alert")).toHaveTextContent("40");
});

test("associates a custom validation error only with its text input", async () => {
  const user = userEvent.setup();
  function Harness() {
    const [value, setValue] = useState("");
    return (
      <ReasoningEffortField
        emptyLabel="未显示 / 不适用"
        id="effort"
        kind="chat_gpt_client"
        label="推理档位"
        onChange={setValue}
        onValidationChange={() => undefined}
        value={value}
      />
    );
  }
  render(<Harness />);

  const select = screen.getByLabelText("推理档位");
  await user.selectOptions(select, "__custom__");
  const input = screen.getByLabelText("按界面原样填写");
  const error = screen.getByRole("alert");

  expect(input).toHaveAttribute("aria-invalid", "true");
  expect(input).toHaveAttribute("aria-describedby", error.id);
  expect(select).not.toHaveAttribute("aria-invalid");
  expect(select).not.toHaveAttribute("aria-describedby");
});
