import { render, screen } from "@testing-library/react";
import { App } from "./App";

test("renders the product entry point", () => {
  render(<App />);
  expect(
    screen.getByRole("heading", { name: "AI 能力雷达" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "开始 AI 体检" }),
  ).toBeInTheDocument();
});
