# Runtime and Portable Corrections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship v0.2.1 with provider-aware reasoning levels, reliable Windows npm CLI discovery, `npm start` source execution, and a Windows x64 no-install ZIP.

**Architecture:** Keep Tauri as the only complete runtime and preserve the existing React-to-Rust IPC boundary. Add a shared frontend reasoning-effort component, family-aware Rust validation, and a provider-specific Windows process launcher that resolves reviewed npm package layouts without passing user data through a shell. Build the portable archive from the same release EXE and sealed benchmark resources used by the installers.

**Tech Stack:** React 19, TypeScript 5.8, Vitest/Testing Library, Rust 2024, Tokio, Tauri 2, Node.js 22/24, PowerShell `Compress-Archive`, GitHub Actions.

## Global Constraints

- Work only in `C:\Users\zhouy\Desktop\降智检测\.worktrees\ai-ability-radar-v02` on branch `codex/ai-ability-radar-v02`.
- Target version is exactly `0.2.1`; release tag is exactly `v0.2.1`.
- Windows release scope remains Windows 10/11 x64.
- `npm start` starts the complete Tauri development runtime; it does not advertise the Vite URL as a standalone browser product.
- The no-install ZIP keeps data in `%APPDATA%\com.aiability.radar`; it is not a portable-data mode.
- Do not add a localhost HTTP server, Tauri shell permission, generic opener, updater, telemetry, remote scoring, credential reader, or API proxy.
- Never pass prompts, model names, effort values, or workspaces through `cmd.exe`, PowerShell, `.cmd`, `.bat`, or `.ps1`.
- Automated tests and GitHub Actions must use fake provider CLIs only and must never call a real Codex or Claude service.
- Known reasoning values remain lowercase canonical strings; manual custom labels preserve trimmed display text; CLI custom values are lowercase safe tokens.
- Existing `low`, `medium`, and `high` history remains readable without a database migration.
- Use `apply_patch` for repository edits and preserve unrelated user changes.

---

## File Structure

### New files

- `apps/desktop/src/components/ReasoningEffortField.tsx` — provider-aware select/custom-input UI.
- `apps/desktop/src/components/ReasoningEffortField.test.tsx` — option, custom mode, validation, and accessibility tests.
- `apps/desktop/src/domain/reasoningEffort.ts` — presets, display labels, and frontend validation.
- `apps/desktop/src/domain/reasoningEffort.test.ts` — pure provider matrix and formatting tests.
- `crates/ability-adapters/src/command_locator.rs` — shell-free Windows native/npm provider resolution.
- `scripts/package-portable.mjs` — stage, hash, and orchestrate the portable archive.
- `scripts/package-portable.test.mjs` — package layout, hash, and path-safety tests.
- `scripts/compress-portable.ps1` — argument-safe Windows ZIP compression.
- `packaging/windows-portable/README.txt` — no-install usage, data, fee, and unsigned-build notice.

### Modified files

- `apps/desktop/src/pages/ManualRunPage.tsx` — use the shared effort field and local validation.
- `apps/desktop/src/pages/CliRunPage.tsx` — use CLI-specific effort presets/custom values.
- `apps/desktop/src/pages/HistoryPage.tsx` — display known canonical efforts in Chinese.
- `apps/desktop/src/pages/ResultPage.tsx` — display known canonical efforts in Chinese.
- `apps/desktop/src/pages/HomePage.tsx` — re-detect button and precise executable-entry status.
- Corresponding page tests and `apps/desktop/src/test/accessibility.test.tsx`.
- `apps/desktop/src-tauri/src/commands.rs` — family-aware effort normalization and resume validation.
- `crates/ability-core/src/report.rs` and `crates/ability-core/tests/report.rs` — human-readable effort labels in HTML reports while JSON stays canonical.
- `crates/ability-adapters/src/process.rs` and `crates/ability-adapters/src/lib.rs` — invoke the resolved launch command.
- `package.json`, both npm manifests/lockfile, all first-party Cargo manifests/lockfile, and `tauri.conf.json` — scripts and v0.2.1.
- `.github/workflows/ci.yml` and `.github/workflows/release.yml` — exact v0.2.1 artifacts and portable upload.
- `scripts/validate-repository.mjs` and `scripts/repository-contracts.test.mjs` — seal the new scripts and release sequence.
- `README.md`, `docs/methodology.md`, `docs/troubleshooting.md`, `docs/release-checklist.md`, `docs/test-matrix.md`, `site/index.html`, and the bug template — truthful v0.2.1 instructions.

---

### Task 1: Rust Reasoning-Effort Validation and Report Labels

**Files:**
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `crates/ability-core/src/report.rs`
- Test: `apps/desktop/src-tauri/src/commands.rs`
- Test: `crates/ability-core/tests/report.rs`

**Interfaces:**
- Consumes: `StartFamily`, `TargetKind`, `TargetSelection`, and the existing optional `reasoning_effort: Option<String>`.
- Produces: `normalize_reasoning_effort(value: Option<String>, family: StartFamily) -> Result<Option<String>, String>`.
- Produces: private `reasoning_effort_display(kind: TargetKind, value: Option<&str>) -> &str` for HTML rendering.
- Storage and public JSON continue to contain canonical/custom source values, not translated labels.

- [ ] **Step 1: Write failing start and resume validation tests**

Add focused tests beside the current `target_values_are_normalized_before_use` test:

```rust
#[test]
fn manual_reasoning_accepts_all_known_values_and_preserves_custom_labels() {
    for value in [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ] {
        let padded = format!(" {value} ");
        let start = validate_start(
            start_input(
                TargetKind::ChatGptClient,
                "GPT-5.6",
                Some(&padded),
                RunMode::Quick,
            ),
            StartFamily::Manual,
        )
        .unwrap();
        assert_eq!(start.target.reasoning_effort.as_deref(), Some(value));
    }

    let custom = validate_start(
        start_input(
            TargetKind::ClaudeClient,
            "Claude",
            Some("  扩展思考（实验）  "),
            RunMode::Quick,
        ),
        StartFamily::Manual,
    )
    .unwrap();
    assert_eq!(
        custom.target.reasoning_effort.as_deref(),
        Some("扩展思考（实验）")
    );
}

#[test]
fn cli_reasoning_accepts_known_and_safe_custom_tokens() {
    for value in [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        "frontier_2", "deep-preview",
    ] {
        let padded = format!(" {value} ");
        let start = validate_start(
            start_input(
                TargetKind::CodexCli,
                "default",
                Some(&padded),
                RunMode::Quick,
            ),
            StartFamily::Cli,
        )
        .unwrap();
        assert_eq!(start.target.reasoning_effort.as_deref(), Some(value));
    }
}

#[test]
fn reasoning_rejects_control_overflow_and_unsafe_cli_values() {
    let manual_overflow = "思".repeat(41);
    for value in ["bad\nvalue".to_owned(), manual_overflow] {
        assert!(validate_start(
            start_input(
                TargetKind::ChatGptClient,
                "GPT",
                Some(&value),
                RunMode::Quick,
            ),
            StartFamily::Manual,
        )
        .is_err());
    }

    let cli_overflow = "a".repeat(33);
    for value in ["极高", "high;calc", "high value", cli_overflow.as_str()] {
        assert!(validate_start(
            start_input(
                TargetKind::CodexCli,
                "default",
                Some(value),
                RunMode::Quick,
            ),
            StartFamily::Cli,
        )
        .is_err());
    }
}

#[test]
fn resume_requires_the_already_normalized_family_specific_value() {
    let valid = validate_resume_target(
        ResumeTargetSelectionInput {
            kind: TargetKind::ClaudeClient,
            reported_model: "Claude".into(),
            reasoning_effort: Some("扩展思考".into()),
        },
        StartFamily::Manual,
    )
    .unwrap();
    assert_eq!(valid.reasoning_effort.as_deref(), Some("扩展思考"));

    for value in [" XHIGH ", "high;calc"] {
        assert!(validate_resume_target(
            ResumeTargetSelectionInput {
                kind: TargetKind::CodexCli,
                reported_model: "default".into(),
                reasoning_effort: Some(value.into()),
            },
            StartFamily::Cli,
        )
        .is_err());
    }
}
```

- [ ] **Step 2: Run the Rust command tests and verify RED**

Run:

```powershell
cargo test -p ability-radar target_values_are_normalized_before_use
cargo test -p ability-radar manual_reasoning_accepts_all_known_values_and_preserves_custom_labels
```

Expected: the existing normalization test passes; the new test fails because `xhigh`, `max`, `ultra`, and custom manual labels are rejected.

- [ ] **Step 3: Implement family-aware normalization**

Replace the three-value allowlist with these helpers and call them from both `validate_start` and `validate_resume_target`:

```rust
const KNOWN_REASONING_EFFORTS: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

fn normalize_reasoning_effort(
    value: Option<String>,
    family: StartFamily,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.chars().any(char::is_control) {
        return Err("推理档位不能包含控制字符".into());
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let canonical = trimmed.to_ascii_lowercase();
    if KNOWN_REASONING_EFFORTS.contains(&canonical.as_str()) {
        return Ok(Some(canonical));
    }

    match family {
        StartFamily::Manual => {
            if trimmed.chars().count() > 40 {
                return Err("自定义推理档位必须是 1–40 个可见字符".into());
            }
            Ok(Some(trimmed.to_owned()))
        }
        StartFamily::Cli => {
            if canonical.len() > 32
                || !canonical
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return Err(
                    "CLI 推理档位只能包含 1–32 个 ASCII 字母、数字、下划线或连字符"
                        .into(),
                );
            }
            Ok(Some(canonical))
        }
    }
}

fn validate_stored_reasoning_effort(
    value: Option<String>,
    family: StartFamily,
) -> Result<Option<String>, String> {
    let normalized = normalize_reasoning_effort(value.clone(), family)?;
    if normalized != value {
        return Err("恢复目标包含未规范化的推理档位。".into());
    }
    Ok(normalized)
}
```

In `validate_start`, use:

```rust
let reasoning_effort =
    normalize_reasoning_effort(input.target.reasoning_effort, family)?;
```

In `validate_resume_target`, use:

```rust
let reasoning_effort =
    validate_stored_reasoning_effort(input.reasoning_effort, family)?;
```

and construct the target with that returned value.

- [ ] **Step 4: Run command tests and verify GREEN**

Run:

```powershell
cargo test -p ability-radar target_values_are_normalized_before_use
cargo test -p ability-radar reasoning_
cargo test -p ability-radar resume_requires_the_already_normalized_family_specific_value
```

Expected: all selected tests pass.

- [ ] **Step 5: Write failing HTML report label tests**

Add to `crates/ability-core/tests/report.rs`:

```rust
#[test]
fn html_report_translates_known_efforts_but_json_stays_canonical() {
    let (mut run, tasks) = sample_evidence("GPT-5.6");
    run.target.kind = TargetKind::ChatGptClient;
    run.target.reasoning_effort = Some("xhigh".into());

    let report = build_public_report(&run, &tasks).unwrap();
    assert_eq!(report.target.reasoning_effort.as_deref(), Some("xhigh"));
    let html = render_public_report_html(&report).unwrap();
    assert!(html.contains("推理档位：极高"));

    run.target.kind = TargetKind::ChatGptClient;
    run.target.reasoning_effort = Some("low".into());
    let report = build_public_report(&run, &tasks).unwrap();
    assert!(render_public_report_html(&report)
        .unwrap()
        .contains("推理档位：轻度"));
}

#[test]
fn html_report_preserves_and_escapes_custom_effort_labels() {
    let (mut run, tasks) = sample_evidence("Claude");
    run.target.kind = TargetKind::ClaudeClient;
    run.target.reasoning_effort = Some("<扩展思考>".into());

    let report = build_public_report(&run, &tasks).unwrap();
    let html = render_public_report_html(&report).unwrap();
    assert!(html.contains("推理档位：&lt;扩展思考&gt;"));
    assert!(!html.contains("推理档位：<扩展思考>"));
}
```

- [ ] **Step 6: Run the report tests and verify RED**

Run:

```powershell
cargo test -p ability-core --test report html_report_
```

Expected: the known-value test fails because the HTML currently prints `xhigh` and `low`.

- [ ] **Step 7: Implement report-only display translation**

Add a private helper to `report.rs`:

```rust
fn reasoning_effort_display(kind: TargetKind, value: Option<&str>) -> &str {
    match (kind, value) {
        (_, None | Some("")) => "未记录",
        (TargetKind::ChatGptClient, Some("low")) => "轻度",
        (_, Some("none")) => "无",
        (_, Some("minimal")) => "最小",
        (_, Some("low")) => "低",
        (_, Some("medium")) => "中",
        (_, Some("high")) => "高",
        (_, Some("xhigh")) => "极高",
        (_, Some("max")) => "最高",
        (_, Some("ultra")) => "Ultra",
        (_, Some(value)) => value,
    }
}
```

Keep `build_public_report` unchanged. In `render_public_report_html`, replace the current effort preparation with:

```rust
let effort = html_escape(reasoning_effort_display(
    report.target.kind,
    report.target.reasoning_effort.as_deref(),
));
```

- [ ] **Step 8: Run focused and package tests**

Run:

```powershell
cargo test -p ability-core --test report
cargo test -p ability-radar
```

Expected: both commands pass.

- [ ] **Step 9: Commit Task 1**

```powershell
git add -- apps/desktop/src-tauri/src/commands.rs crates/ability-core/src/report.rs crates/ability-core/tests/report.rs
git commit -m "feat: support current reasoning effort levels"
```

---

### Task 2: Provider-Aware Frontend Reasoning Field

**Files:**
- Create: `apps/desktop/src/domain/reasoningEffort.ts`
- Create: `apps/desktop/src/domain/reasoningEffort.test.ts`
- Create: `apps/desktop/src/components/ReasoningEffortField.tsx`
- Create: `apps/desktop/src/components/ReasoningEffortField.test.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.tsx`
- Modify: `apps/desktop/src/pages/CliRunPage.tsx`
- Modify: `apps/desktop/src/pages/HistoryPage.tsx`
- Modify: `apps/desktop/src/pages/ResultPage.tsx`
- Modify: `apps/desktop/src/styles/app.css`
- Test: corresponding page tests and `apps/desktop/src/test/accessibility.test.tsx`

**Interfaces:**
- Produces: `effortOptionsFor(kind: TargetKind): readonly EffortOption[]`.
- Produces: `formatReasoningEffort(kind: TargetKind, value?: string | null, emptyLabel?: string): string`.
- Produces: `reasoningEffortError(kind: TargetKind, value: string): string | null`.
- Produces: `normalizeReasoningEffortForTarget(kind: TargetKind, value: string): string`.
- Produces: `<ReasoningEffortField id kind label emptyLabel value onChange onValidationChange />`.
- Pages continue to send the actual canonical/custom value through `TargetSelection.reasoningEffort`.

- [ ] **Step 1: Write failing pure matrix tests**

Create `apps/desktop/src/domain/reasoningEffort.test.ts`:

```ts
import { describe, expect, test } from "vitest";
import {
  effortOptionsFor,
  formatReasoningEffort,
  normalizeReasoningEffortForTarget,
  reasoningEffortError,
} from "./reasoningEffort";

describe("provider effort matrices", () => {
  test("ChatGPT exposes the current UI levels and Ultra", () => {
    expect(effortOptionsFor("chat_gpt_client").map(({ value }) => value)).toEqual([
      "low", "medium", "high", "xhigh", "max", "ultra",
    ]);
  });

  test("Claude exposes the complete effort set without ultracode", () => {
    for (const kind of ["claude_client", "claude_code"] as const) {
      expect(effortOptionsFor(kind).map(({ value }) => value)).toEqual([
        "low", "medium", "high", "xhigh", "max",
      ]);
    }
  });

  test("Codex exposes model-dependent lower and upper levels", () => {
    expect(effortOptionsFor("codex_cli").map(({ value }) => value)).toEqual([
      "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ]);
  });
});

test("known labels are localized and custom labels are preserved", () => {
  expect(formatReasoningEffort("chat_gpt_client", "low")).toBe("轻度");
  expect(formatReasoningEffort("codex_cli", "xhigh")).toBe("极高");
  expect(formatReasoningEffort("claude_code", "max")).toBe("最高");
  expect(formatReasoningEffort("claude_client", "扩展思考")).toBe("扩展思考");
  expect(formatReasoningEffort("codex_cli", null, "CLI 默认")).toBe("CLI 默认");
});

test("custom validation mirrors the Rust family rules", () => {
  expect(reasoningEffortError("chat_gpt_client", "扩展思考")).toBeNull();
  expect(reasoningEffortError("chat_gpt_client", "思".repeat(41))).toMatch(/40/);
  expect(reasoningEffortError("codex_cli", "frontier_2")).toBeNull();
  expect(reasoningEffortError("codex_cli", "high value")).toMatch(/ASCII/);
  expect(reasoningEffortError("claude_code", "极高")).toMatch(/ASCII/);
});

test("known values normalize for every target and custom CLI values lowercase", () => {
  expect(normalizeReasoningEffortForTarget("chat_gpt_client", " XHIGH ")).toBe(
    "xhigh",
  );
  expect(normalizeReasoningEffortForTarget("claude_client", " 扩展思考 ")).toBe(
    "扩展思考",
  );
  expect(normalizeReasoningEffortForTarget("codex_cli", " Frontier_2 ")).toBe(
    "frontier_2",
  );
});
```

- [ ] **Step 2: Run the pure test and verify RED**

Run:

```powershell
npm run test --workspace apps/desktop -- reasoningEffort.test.ts
```

Expected: FAIL because `reasoningEffort.ts` does not exist.

- [ ] **Step 3: Implement the pure reasoning domain**

Create `apps/desktop/src/domain/reasoningEffort.ts` with:

```ts
import type { TargetKind } from "../api/backend";

export interface EffortOption {
  value: string;
  label: string;
}

const common = [
  { value: "low", label: "低" },
  { value: "medium", label: "中" },
  { value: "high", label: "高" },
  { value: "xhigh", label: "极高" },
  { value: "max", label: "最高" },
] as const;

const matrices: Record<TargetKind, readonly EffortOption[]> = {
  chat_gpt_client: [
    { value: "low", label: "轻度" },
    ...common.slice(1),
    { value: "ultra", label: "Ultra" },
  ],
  claude_client: common,
  codex_cli: [
    { value: "minimal", label: "最小" },
    ...common,
    { value: "ultra", label: "Ultra" },
  ],
  claude_code: common,
};

const CONTROL_CHARACTER = /\p{Cc}/u;
const SAFE_CLI_EFFORT = /^[A-Za-z0-9_-]{1,32}$/;
const KNOWN_EFFORTS = new Set([
  "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
]);

export function effortOptionsFor(kind: TargetKind): readonly EffortOption[] {
  return matrices[kind];
}

export function formatReasoningEffort(
  kind: TargetKind,
  value?: string | null,
  emptyLabel = "未记录",
): string {
  if (!value) return emptyLabel;
  return matrices[kind].find((option) => option.value === value)?.label ?? value;
}

export function reasoningEffortError(
  kind: TargetKind,
  value: string,
): string | null {
  if (!value) return null;
  if (CONTROL_CHARACTER.test(value)) return "推理档位不能包含控制字符";
  const trimmed = value.trim();
  const cli = kind === "codex_cli" || kind === "claude_code";
  if (cli) {
    return SAFE_CLI_EFFORT.test(trimmed)
      ? null
      : "自定义 CLI 档位只能包含 1–32 个 ASCII 字母、数字、下划线或连字符";
  }
  return Array.from(trimmed).length <= 40
    ? null
    : "自定义推理档位不能超过 40 个字符";
}

export function normalizeReasoningEffortForTarget(
  kind: TargetKind,
  value: string,
): string {
  const trimmed = value.trim();
  const lowered = trimmed.toLowerCase();
  if (
    KNOWN_EFFORTS.has(lowered) ||
    kind === "codex_cli" ||
    kind === "claude_code"
  ) {
    return lowered;
  }
  return trimmed;
}
```

- [ ] **Step 4: Run the pure test and verify GREEN**

Run:

```powershell
npm run test --workspace apps/desktop -- reasoningEffort.test.ts
```

Expected: PASS.

- [ ] **Step 5: Write failing component interaction tests**

Create `apps/desktop/src/components/ReasoningEffortField.test.tsx`:

```tsx
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
  expect(screen.getByRole("alert")).toHaveTextContent("填写自定义");
  await user.type(screen.getByLabelText("按界面原样填写"), "扩展思考");
  expect(screen.getByLabelText("按界面原样填写")).toHaveValue("扩展思考");
  await user.clear(screen.getByLabelText("按界面原样填写"));
  await user.type(
    screen.getByLabelText("按界面原样填写"),
    "思".repeat(41),
  );
  expect(screen.getByRole("alert")).toHaveTextContent("40");
});
```

- [ ] **Step 6: Implement the shared field**

Create `apps/desktop/src/components/ReasoningEffortField.tsx`:

```tsx
import { useState } from "react";
import type { TargetKind } from "../api/backend";
import {
  effortOptionsFor,
  reasoningEffortError,
} from "../domain/reasoningEffort";

const CUSTOM_VALUE = "__custom__";

export function ReasoningEffortField({
  emptyLabel,
  id,
  kind,
  label,
  onChange,
  onValidationChange,
  value,
}: {
  emptyLabel: string;
  id: string;
  kind: TargetKind;
  label: string;
  onChange(value: string): void;
  onValidationChange(error: string | null): void;
  value: string;
}) {
  const options = effortOptionsFor(kind);
  const preset = options.some((option) => option.value === value);
  const [customMode, setCustomMode] = useState(Boolean(value) && !preset);
  const custom = customMode || (Boolean(value) && !preset);
  const error = custom
    ? value.trim()
      ? reasoningEffortError(kind, value)
      : "请填写自定义推理档位"
    : null;
  const errorId = `${id}-error`;

  return (
    <div className="field reasoning-effort-field">
      <label htmlFor={id}>{label}</label>
      <select
        aria-describedby={error ? errorId : undefined}
        aria-invalid={error ? "true" : undefined}
        id={id}
        onChange={(event) => {
          const next = event.target.value;
          if (next === CUSTOM_VALUE) {
            setCustomMode(true);
            onChange("");
            onValidationChange("请填写自定义推理档位");
          } else {
            setCustomMode(false);
            onChange(next);
            onValidationChange(null);
          }
        }}
        value={custom ? CUSTOM_VALUE : value}
      >
        <option value="">{emptyLabel}</option>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
        <option value={CUSTOM_VALUE}>其他 / 按界面原样填写</option>
      </select>
      {custom ? (
        <label className="reasoning-custom">
          <span>按界面原样填写</span>
          <input
            autoComplete="off"
            onChange={(event) => {
              const next = event.target.value;
              onChange(next);
              onValidationChange(
                next.trim()
                  ? reasoningEffortError(kind, next)
                  : "请填写自定义推理档位",
              );
            }}
            value={value}
          />
        </label>
      ) : null}
      {error ? (
        <p className="form-error" id={errorId} role="alert">
          {error}
        </p>
      ) : null}
      <small className="hint">
        可用档位取决于模型、客户端版本和账户权限。
      </small>
    </div>
  );
}
```

- [ ] **Step 7: Run component tests and verify GREEN**

Run:

```powershell
npm run test --workspace apps/desktop -- ReasoningEffortField.test.tsx
```

Expected: PASS.

- [ ] **Step 8: Integrate the field and display formatter into all pages**

Make these exact behavioral changes:

```tsx
// ManualRunPage.tsx
const [reasoningEffort, setReasoningEffort] = useState("");
const [reasoningError, setReasoningError] = useState<string | null>(null);

<ReasoningEffortField
  emptyLabel="未显示 / 不适用"
  id="manual-reasoning"
  kind={kind}
  label="推理档位（没有显示可留空）"
  onChange={setReasoningEffort}
  onValidationChange={setReasoningError}
  value={reasoningEffort}
/>
```

Include `Boolean(reasoningError)` in the manual start button’s disabled expression and use:

```ts
formatReasoningEffort(kind, run.target.reasoningEffort, "未显示 / 不适用")
```

for resume display.

```tsx
// CliRunPage.tsx
const [reasoningEffort, setReasoningEffort] = useState("");
const [reasoningError, setReasoningError] = useState<string | null>(null);

<ReasoningEffortField
  emptyLabel="CLI 默认"
  id="cli-reasoning"
  kind={kind}
  label="推理档位（可选）"
  onChange={setReasoningEffort}
  onValidationChange={setReasoningError}
  value={reasoningEffort}
/>
```

Include `Boolean(reasoningError)` in the CLI start eligibility expression. Before constructing either requested target, call `normalizeReasoningEffortForTarget(kind, reasoningEffort)` and convert an empty result to `null`; this keeps frontend request equality aligned with Rust normalization. Use `formatReasoningEffort` in manual resume, CLI resume, history comparison conditions, result technical details, and result metadata.

Add this minimal layout rule to `apps/desktop/src/styles/app.css`:

```css
.reasoning-effort-field,
.reasoning-custom {
  display: grid;
  gap: var(--space-2);
}
```

- [ ] **Step 9: Extend page and accessibility tests**

Add assertions that:

```tsx
expect(screen.getByRole("option", { name: "极高" })).toHaveValue("xhigh");
expect(screen.getByRole("option", { name: "最高" })).toHaveValue("max");
```

For ChatGPT, select `xhigh` and expect `reasoningEffort: "xhigh"`. For Claude custom mode, enter `扩展思考` and expect that exact value. For Codex select `max`; for Claude Code assert that no `Ultra`/`ultracode` option is present. Add each setup route to the existing axe test table after the shared component is integrated.

- [ ] **Step 10: Run frontend tests and build**

Run:

```powershell
npm run test --workspace apps/desktop -- reasoningEffort.test.ts ReasoningEffortField.test.tsx ManualRunPage.test.tsx CliRunPage.test.tsx HistoryPage.ui.test.tsx ResultPage.test.tsx accessibility.test.tsx
npm run build --workspace apps/desktop
```

Expected: selected tests pass and TypeScript/Vite build succeeds.

- [ ] **Step 11: Commit Task 2**

```powershell
git add -- apps/desktop/src/domain apps/desktop/src/components/ReasoningEffortField.tsx apps/desktop/src/components/ReasoningEffortField.test.tsx apps/desktop/src/pages apps/desktop/src/styles/app.css apps/desktop/src/test/accessibility.test.tsx
git commit -m "feat: add provider-aware reasoning controls"
```

---

### Task 3: Shell-Free Windows Provider Command Locator

**Files:**
- Create: `crates/ability-adapters/src/command_locator.rs`
- Modify: `crates/ability-adapters/src/lib.rs`
- Modify: `crates/ability-adapters/src/process.rs`
- Test: `crates/ability-adapters/src/command_locator.rs`
- Test: `crates/ability-adapters/tests/process_contract.rs`

**Interfaces:**
- Produces: `LaunchCommand { program: PathBuf, prefix_args: Vec<String> }`.
- Produces: `resolve_launch_command(program: &str) -> io::Result<LaunchCommand>`.
- On Windows, `codex` and `claude` resolve to a native `.exe` or `node.exe` plus the reviewed official npm JavaScript entry.
- `ProcessSpec` remains unchanged so all adapters, fake runners, and command argument contracts keep their current interface.

- [ ] **Step 1: Write failing pure Windows locator tests**

In `command_locator.rs`, add a pure internal function:

```rust
#[cfg(windows)]
fn resolve_windows_provider_command(
    provider: &str,
    path: &std::ffi::OsStr,
) -> io::Result<LaunchCommand>;
```

Then add tests using `tempfile::tempdir()`:

```rust
#[cfg(windows)]
#[test]
fn npm_extensionless_shim_does_not_hide_the_reviewed_codex_package() {
    let temp = tempfile::tempdir().unwrap();
    let npm = temp.path().join("npm");
    let node_bin = temp.path().join("node-bin");
    std::fs::create_dir_all(
        npm.join("node_modules/@openai/codex/bin"),
    )
    .unwrap();
    std::fs::create_dir_all(&node_bin).unwrap();
    std::fs::write(npm.join("codex"), "#!/bin/sh").unwrap();
    std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
    std::fs::write(
        npm.join("node_modules/@openai/codex/bin/codex.js"),
        "console.log('fake')",
    )
    .unwrap();
    std::fs::write(node_bin.join("node.exe"), b"MZ").unwrap();
    let path = std::env::join_paths([&npm, &node_bin]).unwrap();

    let launch = resolve_windows_provider_command("codex", &path).unwrap();

    assert_eq!(launch.program, node_bin.join("node.exe"));
    assert_eq!(
        launch.prefix_args,
        [npm.join("node_modules/@openai/codex/bin/codex.js")
            .to_string_lossy()
            .into_owned()]
    );
}

#[cfg(windows)]
#[test]
fn native_exe_wins_without_executing_any_shim() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("npm");
    let second = temp.path().join("native");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("codex"), "not executable").unwrap();
    std::fs::write(first.join("codex.cmd"), "@echo off").unwrap();
    std::fs::write(second.join("codex.exe"), b"MZ").unwrap();
    let path = std::env::join_paths([&first, &second]).unwrap();

    let launch = resolve_windows_provider_command("codex", &path).unwrap();

    assert_eq!(launch.program, second.join("codex.exe"));
    assert!(launch.prefix_args.is_empty());
}

#[cfg(windows)]
#[test]
fn claude_uses_only_the_reviewed_npm_entry() {
    let temp = tempfile::tempdir().unwrap();
    let npm = temp.path().join("npm");
    let node_bin = temp.path().join("node");
    std::fs::create_dir_all(
        npm.join("node_modules/@anthropic-ai/claude-code"),
    )
    .unwrap();
    std::fs::create_dir_all(&node_bin).unwrap();
    std::fs::write(npm.join("claude.cmd"), "@echo off").unwrap();
    std::fs::write(
        npm.join("node_modules/@anthropic-ai/claude-code/cli.js"),
        "console.log('fake')",
    )
    .unwrap();
    std::fs::write(node_bin.join("node.exe"), b"MZ").unwrap();
    let path = std::env::join_paths([&npm, &node_bin]).unwrap();

    let launch = resolve_windows_provider_command("claude", &path).unwrap();

    assert_eq!(launch.program, node_bin.join("node.exe"));
    assert_eq!(launch.prefix_args.len(), 1);
    assert!(launch.prefix_args[0].ends_with("cli.js"));
}

#[cfg(windows)]
#[test]
fn unreviewed_or_incomplete_shims_are_not_executed() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("codex.cmd"), "@echo calc").unwrap();
    let path = std::env::join_paths([temp.path()]).unwrap();

    let error = resolve_windows_provider_command("codex", &path).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}
```

- [ ] **Step 2: Run the locator tests and verify RED**

Run:

```powershell
cargo test -p ability-adapters command_locator
```

Expected: FAIL because the module and resolver do not exist.

- [ ] **Step 3: Implement the locator**

Implement this file boundary:

```rust
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
}

impl LaunchCommand {
    fn direct(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            prefix_args: Vec::new(),
        }
    }
}

pub(crate) fn resolve_launch_command(program: &str) -> io::Result<LaunchCommand> {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        return Ok(LaunchCommand::direct(path));
    }

    #[cfg(windows)]
    if matches!(program, "codex" | "claude") {
        let inherited = std::env::var_os("PATH")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is unavailable"))?;
        return resolve_windows_provider_command(program, &inherited);
    }

    Ok(LaunchCommand::direct(program))
}

#[cfg(windows)]
fn path_directories(path: &OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(path)
}

#[cfg(windows)]
fn first_file(path: &OsStr, relative: &Path) -> Option<PathBuf> {
    path_directories(path)
        .map(|directory| directory.join(relative))
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
fn resolve_windows_provider_command(
    provider: &str,
    path: &OsStr,
) -> io::Result<LaunchCommand> {
    if let Some(executable) = first_file(path, Path::new(&format!("{provider}.exe"))) {
        return Ok(LaunchCommand::direct(executable));
    }

    let (package_entry, shim_name) = match provider {
        "codex" => (
            Path::new("node_modules/@openai/codex/bin/codex.js"),
            "codex.cmd",
        ),
        "claude" => (
            Path::new("node_modules/@anthropic-ai/claude-code/cli.js"),
            "claude.cmd",
        ),
        _ => return Err(io::Error::new(io::ErrorKind::NotFound, "unsupported provider")),
    };

    let script = path_directories(path).find_map(|directory| {
        let shim = directory.join(shim_name);
        let entry = directory.join(package_entry);
        (shim.is_file() && entry.is_file()).then_some(entry)
    });
    let node = first_file(path, Path::new("node.exe"));
    match (node, script) {
        (Some(node), Some(script)) => Ok(LaunchCommand {
            program: node,
            prefix_args: vec![script.to_string_lossy().into_owned()],
        }),
        _ => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "supported provider executable was not found",
        )),
    }
}
```

Export the module only internally from `lib.rs`:

```rust
mod command_locator;
```

- [ ] **Step 4: Apply the launch command inside `TokioProcessRunner`**

In `process.rs`, import the resolver and replace direct `Command::new(program)` construction:

```rust
let launch = if spec.environment == ProcessEnvironment::Clear {
    crate::command_locator::LaunchCommand {
        program: resolve_from_parent_path(&spec.program),
        prefix_args: Vec::new(),
    }
} else {
    crate::command_locator::resolve_launch_command(&spec.program)
        .map_err(ProcessError::Spawn)?
};
let mut command = Command::new(&launch.program);
if spec.environment == ProcessEnvironment::Clear {
    command.env_clear();
}
command
    .args(&launch.prefix_args)
    .args(&spec.args)
    .current_dir(&spec.current_dir)
    .envs(&spec.env);
```

Keep stdin null, output capture, job supervision, timeouts, and cancellation unchanged.

- [ ] **Step 5: Add the no-shell process contract**

In `process_contract.rs`, assert the source does not introduce provider shell execution:

```rust
#[test]
fn provider_resolution_never_routes_user_arguments_through_a_shell() {
    let source = include_str!("../src/command_locator.rs");
    assert!(!source.contains("cmd.exe"));
    assert!(!source.contains("powershell"));
    assert!(!source.contains(".bat"));
    assert!(!source.contains(".ps1"));
    assert!(source.contains("@openai/codex/bin/codex.js"));
    assert!(source.contains("@anthropic-ai/claude-code/cli.js"));
}
```

- [ ] **Step 6: Run resolver, adapter, and process tests**

Run:

```powershell
cargo test -p ability-adapters command_locator
cargo test -p ability-adapters --test process_contract
cargo test -p ability-adapters --test codex_adapter
cargo test -p ability-adapters --test claude_adapter
```

Expected: all commands pass; existing adapter argument snapshots remain unchanged.

- [ ] **Step 7: Commit Task 3**

```powershell
git add -- crates/ability-adapters/src/command_locator.rs crates/ability-adapters/src/lib.rs crates/ability-adapters/src/process.rs crates/ability-adapters/tests/process_contract.rs
git commit -m "fix: resolve Windows npm provider CLIs safely"
```

---

### Task 4: Home-Page CLI Re-Detection and Precise Status

**Files:**
- Modify: `apps/desktop/src/pages/HomePage.tsx`
- Modify: `apps/desktop/src/pages/HomePage.test.tsx`
- Modify: `apps/desktop/src/styles/app.css`
- Test: `apps/desktop/src/test/accessibility.test.tsx`

**Interfaces:**
- Reuses the existing `Backend.getBootstrap()` command; no new Tauri command or capability is added.
- Produces a visible `重新检测 CLI` button that increments the existing `attempt` state.
- A missing Node prerequisite takes precedence over the executable-entry status because both CLI verification and npm-backed CLIs require the runtime.

- [ ] **Step 1: Write failing UI tests**

Add:

```tsx
test("re-detects CLI availability without restarting the app", async () => {
  const first = readyBootstrap();
  first.targets = first.targets.map((target) =>
    target.kind === "codex_cli"
      ? { ...target, installed: false, version: null }
      : target,
  );
  const second = readyBootstrap();
  second.targets = second.targets.map((target) =>
    target.kind === "codex_cli"
      ? { ...target, version: "codex-cli 0.142.5" }
      : target,
  );
  const load = vi
    .fn<() => Promise<Bootstrap>>()
    .mockResolvedValueOnce(first)
    .mockResolvedValueOnce(second);
  const user = userEvent.setup();
  renderHome(backendFor(load));

  expect(await screen.findByText("未检测到可执行入口")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "重新检测 CLI" }));

  expect(await screen.findByText("版本：codex-cli 0.142.5")).toBeInTheDocument();
  expect(load).toHaveBeenCalledTimes(2);
});

test("missing Node takes precedence over an npm CLI detection failure", async () => {
  const bootstrap = readyBootstrap();
  bootstrap.targets = bootstrap.targets.map((target) =>
    target.kind === "codex_cli"
      ? {
          ...target,
          installed: false,
          prerequisites: [
            { name: "Node.js 22/24 LTS", available: false, version: null },
          ],
        }
      : target,
  );
  renderHome(backendFor(async () => bootstrap));

  expect(await screen.findByRole("status", {
    name: "Codex CLI 状态：缺少 Node.js 22/24 LTS",
  })).toBeInTheDocument();
});
```

- [ ] **Step 2: Run HomePage tests and verify RED**

Run:

```powershell
npm run test --workspace apps/desktop -- HomePage.test.tsx
```

Expected: FAIL because there is no ready-state re-detect button and the old status text/order remains.

- [ ] **Step 3: Implement ready-state re-detection**

Change `blocker`:

```ts
function blocker(target: TargetAvailability): string | null {
  const missing = target.prerequisites.find(
    (prerequisite) => !prerequisite.available,
  );
  if (missing) return `缺少 ${missing.name}`;
  if (!target.installed) return "未检测到可执行入口";
  if (target.authState === "needs_login") {
    return isCli(target.kind) ? "需要先在终端登录" : "需要先登录";
  }
  return null;
}
```

Extend `TargetGroup` with an optional `action`, render it beside the pack summary, and pass:

```tsx
action={
  <button
    className="secondary-action"
    onClick={() => setAttempt((value) => value + 1)}
    type="button"
  >
    重新检测 CLI
  </button>
}
```

only to the CLI group.

- [ ] **Step 4: Run HomePage and accessibility tests**

Run:

```powershell
npm run test --workspace apps/desktop -- HomePage.test.tsx accessibility.test.tsx
```

Expected: PASS with no axe violations.

- [ ] **Step 5: Commit Task 4**

```powershell
git add -- apps/desktop/src/pages/HomePage.tsx apps/desktop/src/pages/HomePage.test.tsx apps/desktop/src/styles/app.css apps/desktop/src/test/accessibility.test.tsx
git commit -m "feat: recheck local CLI availability"
```

---

### Task 5: Source Start Command and Portable Archive Builder

**Files:**
- Create: `scripts/package-portable.mjs`
- Create: `scripts/package-portable.test.mjs`
- Create: `scripts/compress-portable.ps1`
- Create: `packaging/windows-portable/README.txt`
- Modify: `package.json`
- Modify: `scripts/validate-repository.mjs`
- Modify: `scripts/repository-contracts.test.mjs`

**Interfaces:**
- Produces: `npm start` → `npm run tauri -- dev`.
- Produces: `npm run package:portable` → release no-bundle build plus archive staging.
- Produces: `npm run package:portable:from-build` → archive from existing `target/release`.
- Produces: `stagePortable({ repoRoot, targetDir, bundleDir, version }) -> Promise<{ archivePath, stageRoot }>` for tests and the CLI entry.

- [ ] **Step 1: Write failing package contract tests**

Create `scripts/package-portable.test.mjs` using only Node core modules:

```js
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { stagePortable } from "./package-portable.mjs";

test("stages one rooted no-install package with deterministic checksums", async () => {
  const root = await mkdtemp(join(tmpdir(), "ability-radar-portable-"));
  try {
    const repoRoot = join(root, "repo");
    const targetDir = join(root, "target", "release");
    const bundleDir = join(targetDir, "bundle", "portable");
    await mkdir(join(repoRoot, "packaging", "windows-portable"), {
      recursive: true,
    });
    await mkdir(join(targetDir, "benchmark-packs", "client-quick-v1"), {
      recursive: true,
    });
    await mkdir(join(targetDir, "benchmark-packs", "cli-quick-v1"), {
      recursive: true,
    });
    await writeFile(join(targetDir, "ability-radar.exe"), "fake-exe");
    await writeFile(
      join(targetDir, "benchmark-packs", "registry.json"),
      '{"schema_version":1,"packs":[]}\n',
    );
    await writeFile(
      join(targetDir, "benchmark-packs", "client-quick-v1", "manifest.json"),
      "{}\n",
    );
    await writeFile(
      join(targetDir, "benchmark-packs", "cli-quick-v1", "manifest.json"),
      "{}\n",
    );
    await writeFile(
      join(repoRoot, "packaging", "windows-portable", "README.txt"),
      "no install\n",
    );

    const result = await stagePortable({
      repoRoot,
      targetDir,
      bundleDir,
      version: "0.2.1",
    });

    assert.equal(
      result.archivePath,
      join(bundleDir, "ability-radar_0.2.1_windows-x64-portable.zip"),
    );
    const checksums = await readFile(
      join(result.stageRoot, "SHA256SUMS.txt"),
      "utf8",
    );
    assert.match(checksums, /  ability-radar\.exe$/m);
    assert.match(checksums, /  benchmark-packs\/registry\.json$/m);
    assert.doesNotMatch(checksums, /SHA256SUMS\.txt/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("refuses an output directory outside the selected target tree", async () => {
  await assert.rejects(
    stagePortable({
      repoRoot: "C:\\repo",
      targetDir: "C:\\repo\\target\\release",
      bundleDir: "C:\\outside",
      version: "0.2.1",
    }),
    /inside target directory/,
  );
});
```

Add negative repository-contract tests that mutate `package.json` so `start` points to `vite`, or so `package:portable` skips `tauri build --no-bundle`; both mutations must be rejected.

- [ ] **Step 2: Run tests and verify RED**

Run:

```powershell
node --test scripts/package-portable.test.mjs
node --test scripts/repository-contracts.test.mjs
```

Expected: the portable test fails because the module does not exist; the repository contracts fail after adding the negative expectations until the validator is updated.

- [ ] **Step 3: Implement safe staging and hashes**

Implement `scripts/package-portable.mjs` with these exported boundaries:

```js
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFile,
  cp,
  mkdir,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

function assertInside(root, candidate, label) {
  const from = resolve(root);
  const to = resolve(candidate);
  const child = relative(from, to);
  if (!child || child.startsWith(`..${sep}`) || child === ".." || isAbsolute(child)) {
    throw new Error(`${label} must stay inside target directory`);
  }
}

async function filesUnder(root, current = root) {
  const entries = await readdir(current, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) =>
    left.name.localeCompare(right.name, "en"),
  )) {
    const path = join(current, entry.name);
    if (entry.isDirectory()) files.push(...await filesUnder(root, path));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

async function sha256(path) {
  return createHash("sha256").update(await readFile(path)).digest("hex");
}

export async function stagePortable({
  repoRoot,
  targetDir,
  bundleDir,
  version,
}) {
  assertInside(targetDir, bundleDir, "portable bundle directory");
  const executable = join(targetDir, "ability-radar.exe");
  const packs = join(targetDir, "benchmark-packs");
  const readme = join(repoRoot, "packaging", "windows-portable", "README.txt");
  for (const required of [
    executable,
    join(packs, "registry.json"),
    join(packs, "client-quick-v1", "manifest.json"),
    join(packs, "cli-quick-v1", "manifest.json"),
    readme,
  ]) {
    if (!(await stat(required)).isFile()) {
      throw new Error(`required portable input is not a file: ${required}`);
    }
  }

  const stageParent = join(bundleDir, ".stage");
  const stageRoot = join(stageParent, "ability-radar-portable");
  assertInside(bundleDir, stageParent, "portable stage directory");
  await rm(stageParent, { recursive: true, force: true });
  await mkdir(stageRoot, { recursive: true });
  await copyFile(executable, join(stageRoot, "ability-radar.exe"));
  await cp(packs, join(stageRoot, "benchmark-packs"), { recursive: true });
  await copyFile(readme, join(stageRoot, "README.txt"));

  const files = await filesUnder(stageRoot);
  const lines = [];
  for (const file of files) {
    const name = relative(stageRoot, file).split(sep).join("/");
    lines.push(`${await sha256(file)}  ${name}`);
  }
  await writeFile(join(stageRoot, "SHA256SUMS.txt"), `${lines.join("\n")}\n`);

  return {
    archivePath: join(
      bundleDir,
      `ability-radar_${version}_windows-x64-portable.zip`,
    ),
    stageRoot,
  };
}
```

Use this CLI main to read the exact root version, require Windows, invoke the compressor with an argument array, and remove only the validated stage directory:

```js
async function main() {
  if (process.platform !== "win32") {
    throw new Error("portable packaging currently supports Windows only");
  }
  const scriptPath = fileURLToPath(import.meta.url);
  const repoRoot = resolve(dirname(scriptPath), "..");
  const packageManifest = JSON.parse(
    await readFile(join(repoRoot, "package.json"), "utf8"),
  );
  const version = packageManifest.version;
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
    throw new Error("root package version must be strict semantic version");
  }
  const targetDir = join(repoRoot, "target", "release");
  const bundleDir = join(targetDir, "bundle", "portable");
  const { archivePath, stageRoot } = await stagePortable({
    repoRoot,
    targetDir,
    bundleDir,
    version,
  });
  const stageParent = dirname(stageRoot);
  assertInside(bundleDir, stageParent, "portable stage directory");
  await rm(archivePath, { force: true });
  try {
    const result = spawnSync(
      "powershell.exe",
      [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        join(repoRoot, "scripts", "compress-portable.ps1"),
        "-Source",
        stageRoot,
        "-Destination",
        archivePath,
      ],
      { cwd: repoRoot, stdio: "inherit" },
    );
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error("portable ZIP compression failed");
    }
  } finally {
    await rm(stageParent, { recursive: true, force: true });
  }
  process.stdout.write(`${archivePath}\n`);
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  });
}
```

- [ ] **Step 4: Implement argument-safe ZIP compression**

Create `scripts/compress-portable.ps1`:

```powershell
param(
  [Parameter(Mandatory = $true)]
  [string]$Source,
  [Parameter(Mandatory = $true)]
  [string]$Destination
)

$ErrorActionPreference = "Stop"
$sourcePath = [System.IO.Path]::GetFullPath($Source)
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
if (-not (Test-Path -LiteralPath $sourcePath -PathType Container)) {
  throw "Portable source directory does not exist."
}
if ([System.IO.Path]::GetExtension($destinationPath) -cne ".zip") {
  throw "Portable destination must be a .zip file."
}
$destinationDirectory = Split-Path -Parent $destinationPath
New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null
Compress-Archive `
  -LiteralPath $sourcePath `
  -DestinationPath $destinationPath `
  -CompressionLevel Optimal `
  -Force
```

The Node entry invokes it with:

```js
const result = spawnSync(
  "powershell.exe",
  [
    "-NoLogo",
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    join(repoRoot, "scripts", "compress-portable.ps1"),
    "-Source",
    stageRoot,
    "-Destination",
    archivePath,
  ],
  { cwd: repoRoot, encoding: "utf8", stdio: "inherit" },
);
if (result.status !== 0) {
  throw new Error("portable ZIP compression failed");
}
```

- [ ] **Step 5: Add scripts and the portable README**

Set the root scripts exactly:

```json
{
  "start": "npm run tauri -- dev",
  "package:portable": "npm run tauri -- build --no-bundle && npm run package:portable:from-build",
  "package:portable:from-build": "node scripts/package-portable.mjs"
}
```

Include `node --test scripts/package-portable.test.mjs` in the root `test` command.

`packaging/windows-portable/README.txt` must say:

```text
AI 能力雷达 Windows x64 免安装预览版

1. 保持 ability-radar.exe 与 benchmark-packs 文件夹在同一目录。
2. 双击 ability-radar.exe 启动；本版本未签名，Windows SmartScreen 可能提示未知发布者。
3. 免安装仅表示不写入安装/卸载项。本地历史仍保存在 %APPDATA%\com.aiability.radar。
4. 手动客户端和真实 CLI 体检可能消耗运行者自己的订阅额度；维护者不承担费用。
5. 使用本目录 SHA256SUMS.txt 校验解压后的文件。不要运行缺失文件或校验不一致的副本。
```

- [ ] **Step 6: Seal the new entry points**

Update `validate-repository.mjs` so the new files are required and exact script strings are checked. Add an explicit rejection if `package-portable.mjs` or `compress-portable.ps1` contains provider invocations, network upload commands, or writes outside `target/release/bundle/portable`.

- [ ] **Step 7: Run package and repository tests**

Run:

```powershell
node --test scripts/package-portable.test.mjs
node --test scripts/repository-contracts.test.mjs
npm run validate:repository
```

Expected: all commands pass.

- [ ] **Step 8: Commit Task 5**

```powershell
git add -- package.json scripts/package-portable.mjs scripts/package-portable.test.mjs scripts/compress-portable.ps1 packaging/windows-portable/README.txt scripts/validate-repository.mjs scripts/repository-contracts.test.mjs
git commit -m "feat: add source and portable run paths"
```

---

### Task 6: v0.2.1 Release, Contracts, and User Documentation

**Files:**
- Modify: `package.json`, `package-lock.json`, `apps/desktop/package.json`
- Modify: `apps/desktop/src-tauri/Cargo.toml`, `crates/ability-core/Cargo.toml`, `crates/ability-adapters/Cargo.toml`, `Cargo.lock`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- Modify: `scripts/validate-repository.mjs`, `scripts/repository-contracts.test.mjs`
- Modify: `README.md`, `docs/methodology.md`, `docs/troubleshooting.md`, `docs/release-checklist.md`, `docs/test-matrix.md`
- Modify: `site/index.html`, `.github/ISSUE_TEMPLATE/bug.yml`
- Regenerate: `docs/licenses/npm-dependencies.json`, `docs/licenses/rust-dependencies.json`

**Interfaces:**
- All first-party manifests and the release tag seal use exactly `0.2.1`.
- Release assets are NSIS, MSI, `ability-radar_0.2.1_windows-x64-portable.zip`, and one outer `SHA256SUMS.txt`.
- Tauri action remains the sole installer uploader; one exact `gh release upload` step uploads the one portable archive and checksum file.

- [ ] **Step 1: Update negative contracts for v0.2.1 and portable ownership**

Change the CTA negative test to require `/releases/tag/v0.2.1`. Add mutations proving rejection when:

- the release checksum set omits `.zip`;
- the portable archive upload uses a wildcard or uploads raw `target/release`;
- the portable step appears before the Tauri draft release exists;
- the uploaded ZIP filename differs from the manifest version;
- the CI debug installer path remains `0.2.0`.

- [ ] **Step 2: Run repository contracts and verify RED**

Run:

```powershell
node --test scripts/repository-contracts.test.mjs
```

Expected: FAIL until validator/workflow/version sources are updated together.

- [ ] **Step 3: Bump only first-party release versions**

Set `0.2.1` in:

```text
package.json
apps/desktop/package.json
apps/desktop/src-tauri/Cargo.toml
crates/ability-core/Cargo.toml
crates/ability-adapters/Cargo.toml
apps/desktop/src-tauri/tauri.conf.json
```

Then regenerate locks without adding dependencies:

```powershell
npm install --package-lock-only --ignore-scripts
cargo check --workspace
```

Do not blanket-replace `0.2.0` in fixture records or third-party packages inside `Cargo.lock`.

- [ ] **Step 4: Add the exact release portable sequence**

After `Build unsigned draft prerelease`, add:

```yaml
      - name: Build portable archive from reviewed release output
        run: npm run package:portable:from-build
```

Change checksum selection to:

```powershell
$files = Get-ChildItem target/release/bundle -Recurse -File |
  Where-Object { $_.Extension -in ".exe", ".msi", ".zip" } |
  Sort-Object FullName
```

Replace the final upload step with an exact path derived from the verified tag:

```yaml
      - name: Upload portable archive and checksums to the draft prerelease
        shell: pwsh
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          $version = $env:RELEASE_TAG.Substring(1)
          $portable = "target/release/bundle/portable/ability-radar_${version}_windows-x64-portable.zip"
          if (-not (Test-Path -LiteralPath $portable -PathType Leaf)) {
            throw "The exact portable archive is missing."
          }
          gh release upload $env:RELEASE_TAG $portable SHA256SUMS.txt --clobber
```

Update release notes to say “安装程序和免安装 ZIP 均未签名” and “校验所有下载文件”.

- [ ] **Step 5: Update the workflow and repository source seals**

Update `exactReleaseBody`, `exactChecksumRun`, `exactReleaseSteps`, CI debug installer path, the normalized workflow source hashes/seals, exact version, exact scripts, required files, and site CTA validation in `validate-repository.mjs`. Update every corresponding negative fixture expectation in `repository-contracts.test.mjs`.

- [ ] **Step 6: Update user-facing documentation**

Document these exact commands in `README.md`:

```powershell
npm ci
npm start
```

and:

```powershell
npm run package:portable
```

State explicitly that `npm start` opens a Tauri desktop development window and that opening `http://localhost:1420` in a normal browser is not a complete product.

In `docs/troubleshooting.md`, document the confirmed Windows npm-shim symptom and safe read-only checks:

```powershell
Get-Command codex -All
where.exe codex
codex.cmd --version
```

Clarify that `--version` does not send a model request. Add the “重新检测 CLI” path.

Update methodology effort matrices, release checklist portable gates, Windows 10/11 test matrix rows, site download copy/link, and bug-template example version to v0.2.1.

- [ ] **Step 7: Regenerate license metadata and run contracts**

Run:

```powershell
npm run licenses:generate
node --test scripts/repository-contracts.test.mjs
npm run validate:repository
git diff --check
```

Expected: all commands pass and `git diff --check` prints nothing.

- [ ] **Step 8: Commit Task 6**

```powershell
git add -- package.json package-lock.json apps/desktop/package.json apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/tauri.conf.json crates/ability-core/Cargo.toml crates/ability-adapters/Cargo.toml Cargo.lock .github/workflows/ci.yml .github/workflows/release.yml .github/ISSUE_TEMPLATE/bug.yml scripts/validate-repository.mjs scripts/repository-contracts.test.mjs README.md docs/methodology.md docs/troubleshooting.md docs/release-checklist.md docs/test-matrix.md docs/licenses/npm-dependencies.json docs/licenses/rust-dependencies.json site/index.html packaging/windows-portable/README.txt
git commit -m "chore: prepare v0.2.1 Windows release"
```

---

### Task 7: Full Verification and Local Release Artifacts

**Files:**
- Modify only if a gate exposes a scoped defect in Tasks 1–6.
- Produce (untracked): `target/release/bundle/nsis/ability-radar_0.2.1_x64-setup.exe`
- Produce (untracked): `target/release/bundle/msi/ability-radar_0.2.1_x64_en-US.msi`
- Produce (untracked): `target/release/bundle/portable/ability-radar_0.2.1_windows-x64-portable.zip`

**Interfaces:**
- Consumes all completed task commits.
- Produces evidence for every automated gate, artifact hashes, and an explicit clean-VM status.

- [ ] **Step 1: Read the completion-verification skill**

Read `superpowers:verification-before-completion` fully before running or reporting final gates.

- [ ] **Step 2: Run repository, frontend, and Rust gates**

Run:

```powershell
npm run validate:repository
npm test
npm run build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

Expected: every command exits 0.

- [ ] **Step 3: Run security and dependency gates**

Run:

```powershell
npm audit --offline --audit-level=high
cargo audit
```

Expected: npm reports zero high vulnerabilities; cargo reports zero vulnerabilities, with only the already reviewed allowed warnings.

- [ ] **Step 4: Run the sealed fake CLI E2E**

Use only copied fake executables:

```powershell
cargo build -p ability-radar-fake-cli --locked
$fakeBin = Join-Path $env:TEMP "ability-radar-v021-fake-bin"
$resolvedFakeBin = [System.IO.Path]::GetFullPath($fakeBin)
$resolvedTemp = [System.IO.Path]::GetFullPath($env:TEMP)
if (-not $resolvedFakeBin.StartsWith($resolvedTemp, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Fake CLI directory escaped TEMP."
}
New-Item -ItemType Directory -Force -Path $resolvedFakeBin | Out-Null
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $resolvedFakeBin "codex.exe")
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $resolvedFakeBin "claude.exe")
$oldPath = $env:PATH
try {
  $env:PATH = "$resolvedFakeBin;$oldPath"
  $env:ABILITY_RADAR_FAKE_CLI_E2E = "1"
  cargo test -p ability-radar --test fake_cli_e2e --locked -- --ignored
} finally {
  $env:PATH = $oldPath
  Remove-Item Env:ABILITY_RADAR_FAKE_CLI_E2E -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $resolvedFakeBin -Recurse -Force
}
```

Expected: the ignored fake CLI E2E passes and no fake provider process remains.

- [ ] **Step 5: Build all local Windows release forms**

Run:

```powershell
npm run tauri -- build
npm run package:portable:from-build
```

Expected: NSIS, MSI, and the portable ZIP exist under the exact v0.2.1 paths.

- [ ] **Step 6: Inspect the portable archive and hashes**

Run:

```powershell
$archive = Resolve-Path 'target/release/bundle/portable/ability-radar_0.2.1_windows-x64-portable.zip'
$extract = Join-Path $env:TEMP 'ability-radar-v021-portable-check'
$resolvedExtract = [System.IO.Path]::GetFullPath($extract)
$resolvedTemp = [System.IO.Path]::GetFullPath($env:TEMP)
if (-not $resolvedExtract.StartsWith($resolvedTemp, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Portable check directory escaped TEMP."
}
Remove-Item -LiteralPath $resolvedExtract -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive -LiteralPath $archive -DestinationPath $resolvedExtract
Get-ChildItem -LiteralPath $resolvedExtract -Recurse -File
Push-Location (Join-Path $resolvedExtract 'ability-radar-portable')
try {
  Get-Content -LiteralPath SHA256SUMS.txt | ForEach-Object {
    $hash, $name = $_ -split '  ', 2
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $name).Hash.ToLowerInvariant()
    if ($actual -cne $hash) { throw "Portable checksum mismatch: $name" }
  }
} finally {
  Pop-Location
  Remove-Item -LiteralPath $resolvedExtract -Recurse -Force
}
```

Expected: the rooted directory contains the EXE, README, internal checksum file, registry, and both complete pack directories; every internal hash matches.

- [ ] **Step 7: Record release artifact hashes**

Run:

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath `
  'target/release/bundle/nsis/ability-radar_0.2.1_x64-setup.exe', `
  'target/release/bundle/msi/ability-radar_0.2.1_x64_en-US.msi', `
  'target/release/bundle/portable/ability-radar_0.2.1_windows-x64-portable.zip' |
  Select-Object Path, Hash
```

Expected: three non-empty SHA-256 values.

- [ ] **Step 8: Check source start wiring without invoking a real provider**

Run:

```powershell
npm start -- --help
```

Expected: Tauri CLI development help or a clean argument error that proves root argument forwarding; do not start a real provider test. Then run the command normally only long enough to confirm the Tauri window opens, and close it without starting any benchmark.

- [ ] **Step 9: Check final Git state and clean-VM status**

Run:

```powershell
git status --short --branch
git log --oneline --decorate -10
```

Expected: working tree clean. Report Windows 10/11 clean-VM launch/install/uninstall evidence as pending unless it was actually performed; do not infer it from the current Windows 11 host.

- [ ] **Step 10: Use the branch-finishing skill**

Read and follow `superpowers:finishing-a-development-branch`. Do not push, merge, open a PR, install the unsigned artifact, or publish a release without explicit user authorization.
