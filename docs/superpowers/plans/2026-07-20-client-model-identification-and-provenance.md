# Client Model Identification and Provenance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Best-effort read visible model and reasoning selectors from supported Windows clients, require user confirmation, and persist an honest source and verification state for every run.

**Architecture:** Add backward-compatible provenance fields to the core run target, then expose a narrow Tauri command backed by a Windows-only UI Automation adapter. A focused React identification panel auto-runs once, handles multiple/no candidates, and always preserves manual entry as the fallback.

**Tech Stack:** Rust 2024, `windows` 0.61.3, Windows UI Automation, Tauri 2, Tokio, React 19, TypeScript 5.8, Vitest, SQLite JSON fields.

## Global Constraints

- Run this plan only after `2026-07-20-trusted-windows-cli-detection.md` passes.
- Client identification is local, read-only, best-effort, and Windows-only.
- Never infer a model from answer text, writing style, or model self-report.
- Never click a client, switch a model, send a message, inspect credentials, or invoke an AI model.
- Inspect only allowlisted selector control types in the provider window's header/toolbar region.
- Never read `Document`, chat transcript, message-list, or text-input control values.
- Cap a scan at 512 UI Automation nodes, depth 24, 120 characters per label, and 1.5 seconds total.
- Do not persist raw window titles, process paths, package identities, control labels, or accessibility trees.
- Automatic results are prefill only; clicking “开始快速体检” confirms the final values.
- Detection failure must never disable valid manual entry.
- Old stored runs remain readable and are labeled `legacy_unknown`.
- An unspecified CLI model remains `default_route` plus `unverified`; never guess a concrete model.
- Automated tests use synthetic controls and fake backends only.

---

## File Structure

- `crates/ability-core/src/domain.rs` — source and verification enums stored with `TargetSelection`.
- `crates/ability-core/src/report.rs` — provenance validation, public report, and HTML display.
- `crates/ability-core/tests/storage.rs` — old JSON compatibility.
- `crates/ability-core/tests/recovery.rs` — provenance-bound resume behavior.
- `crates/ability-core/tests/report.rs` — public provenance display and safety.
- `crates/ability-core/tests/report_schema.rs` — public report schema v2.
- `schemas/public-report.schema.json` — required public provenance fields.
- `apps/desktop/src-tauri/src/client_selection.rs` — pure parser, window fingerprinting, platform dispatcher, and synthetic tests.
- `apps/desktop/src-tauri/src/client_selection/windows.rs` — Windows process/window enumeration and UI Automation traversal.
- `apps/desktop/src-tauri/src/dto.rs` — strict detection input and run target wire fields.
- `apps/desktop/src-tauri/src/commands.rs` — narrow `detect_client_selection` command and provenance validation.
- `apps/desktop/src-tauri/src/lib.rs` — reviewed command registration.
- `apps/desktop/src-tauri/Cargo.toml` — target-specific Windows API features and Tokio time support.
- `apps/desktop/src/api/backend.ts` — frontend detection/provenance types and backend method.
- `apps/desktop/src/api/tauriBackend.ts` — Tauri invocation.
- `apps/desktop/src/api/tauriBackend.test.ts` — exact invocation shape.
- `apps/desktop/src/api/runtimeValidation.ts` — safe run provenance and exact
  client-detection response validation.
- `apps/desktop/src/domain/modelProvenance.ts` — one source of Chinese labels.
- `apps/desktop/src/components/ClientSelectionPanel.tsx` — identification state and candidate interaction.
- `apps/desktop/src/components/ClientSelectionPanel.test.tsx` — auto-run, stale result, multiple candidate, and fallback tests.
- `apps/desktop/src/pages/ManualRunPage.tsx` — integrates the panel and submits confirmed provenance.
- `apps/desktop/src/pages/ManualRunPage.test.tsx` — start request and fallback behavior.
- `apps/desktop/src/pages/CliRunPage.tsx` — explicit/default CLI provenance.
- `apps/desktop/src/pages/HistoryPage.tsx` — local provenance label.
- `apps/desktop/src/pages/ResultPage.tsx` — result provenance label.

### Shared Interfaces

Core run target:

```rust
pub enum ModelSource {
    Manual,
    WindowsAccessibility,
    CliRequested,
    CliReported,
    DefaultRoute,
    LegacyUnknown,
}

pub enum ModelVerification {
    UserConfirmed,
    ProviderReported,
    Unverified,
    LegacyUnknown,
}
```

Detection wire response:

```rust
pub struct ClientSelectionDetection {
    pub status: ClientSelectionStatus,
    pub candidates: Vec<ClientSelectionCandidate>,
}

pub struct ClientSelectionCandidate {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub surface: ClientSurface,
    pub source: ModelSource,
    pub confidence: ClientSelectionConfidence,
}
```

---

### Task 1: Persist Backward-Compatible Model Provenance

**Files:**

- Modify: `crates/ability-core/src/domain.rs`
- Modify: `crates/ability-core/src/orchestration.rs`
- Modify: `crates/ability-core/tests/domain_contracts.rs`
- Modify: `crates/ability-core/tests/manual_run.rs`
- Modify: `crates/ability-core/tests/recovery.rs`
- Modify: `crates/ability-core/tests/report.rs`
- Modify: `crates/ability-core/tests/report_schema.rs`
- Modify: `crates/ability-core/tests/storage.rs`
- Modify: `crates/ability-adapters/src/cli_run.rs`
- Modify: `crates/ability-adapters/tests/cli_run.rs`
- Modify: `apps/desktop/src-tauri/src/app_state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/data_management_tests.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/tests/fake_cli_e2e.rs`
- Modify: `apps/desktop/src/api/backend.ts`
- Modify: `apps/desktop/src/api/runtimeValidation.ts`
- Modify: `apps/desktop/src/api/backend.test.ts`
- Test: `crates/ability-core/tests/storage.rs`
- Test: `apps/desktop/src/api/runtimeValidation.test.ts`
- Test: `apps/desktop/src/api/tauriBackend.test.ts`
- Modify: `apps/desktop/src/pages/CliRunPage.tsx`
- Modify: `apps/desktop/src/pages/CliRunPage.test.tsx`
- Modify: `apps/desktop/src/pages/HistoryPage.test.ts`
- Modify: `apps/desktop/src/pages/HistoryPage.ui.test.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.test.tsx`
- Modify: `apps/desktop/src/pages/ResultPage.test.tsx`
- Modify: `apps/desktop/src/test/accessibility.test.tsx`

**Interfaces:**

- Consumes: existing `TargetSelection`.
- Produces: `ModelSource`, `ModelVerification`, and required frontend
  `modelSource`/`modelVerification` fields.

- [ ] **Step 1: Write failing old-record and runtime-validation tests**

In `crates/ability-core/tests/storage.rs`, add:

```rust
#[test]
fn legacy_target_json_defaults_provenance_without_rewriting_model() {
    let target: TargetSelection = serde_json::from_str(
        r#"{"kind":"chat_gpt_client","reportedModel":"GPT-X","reasoningEffort":"high"}"#,
    )
    .unwrap();

    assert_eq!(target.reported_model, "GPT-X");
    assert_eq!(target.model_source, ModelSource::LegacyUnknown);
    assert_eq!(
        target.model_verification,
        ModelVerification::LegacyUnknown
    );
}
```

In `runtimeValidation.test.ts`, extend the safe fixture with:

```ts
modelSource: "windows_accessibility",
modelVerification: "user_confirmed",
```

and add:

```ts
test("run validation rejects unknown model provenance", () => {
  const run = safeRunRecord();
  run.target.modelSource = "answer_inference" as never;
  expect(isSafeRunRecord(run)).toBe(false);
});
```

In `CliRunPage.test.tsx`, assert an empty model sends
`default_route`/`unverified`, while an explicit model sends
`cli_requested`/`user_confirmed`. In `ManualRunPage.test.tsx`, assert a
manually entered model sends `manual`/`user_confirmed`. Extend DTO and bridge
fixtures so the two fields are required and preserved exactly.

- [ ] **Step 2: Run tests and verify the fields are absent**

Run:

```powershell
cargo test -p ability-core legacy_target_json_defaults_provenance_without_rewriting_model
npm test --workspace apps/desktop -- src/api/runtimeValidation.test.ts
```

Expected: Rust compilation and TypeScript tests fail because provenance types
do not exist.

- [ ] **Step 3: Add core enums with legacy defaults**

In `domain.rs`, add:

```rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    Manual,
    WindowsAccessibility,
    CliRequested,
    CliReported,
    DefaultRoute,
    #[default]
    LegacyUnknown,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelVerification {
    UserConfirmed,
    ProviderReported,
    Unverified,
    #[default]
    LegacyUnknown,
}
```

Extend `TargetSelection`:

```rust
#[serde(default)]
pub model_source: ModelSource,
#[serde(default)]
pub model_verification: ModelVerification,
```

Update every Rust initializer explicitly. Test fixtures that do not exercise
provenance use:

```rust
model_source: ModelSource::LegacyUnknown,
model_verification: ModelVerification::LegacyUnknown,
```

New manual-run fixtures use `Manual`/`UserConfirmed`; new default CLI
fixtures use `DefaultRoute`/`Unverified`.

- [ ] **Step 4: Add TypeScript types and runtime allowlists**

In `backend.ts`:

```ts
export type ModelSource =
  | "manual"
  | "windows_accessibility"
  | "cli_requested"
  | "cli_reported"
  | "default_route"
  | "legacy_unknown";

export type ModelVerification =
  | "user_confirmed"
  | "provider_reported"
  | "unverified"
  | "legacy_unknown";
```

Extend `TargetSelection` and `ResumeRunInput.expectedTarget`:

```ts
modelSource: ModelSource;
modelVerification: ModelVerification;
```

Extend `TargetSelectionInput` and `ResumeTargetSelectionInput` in `dto.rs`
with required `ModelSource` and `ModelVerification` fields, and copy them
into the core target in `commands.rs`. This task establishes wire transport;
Task 2 adds the strict combination matrix.

In `runtimeValidation.ts`, define sets and require membership:

```ts
const modelSources = new Set<ModelSource>([
  "manual",
  "windows_accessibility",
  "cli_requested",
  "cli_reported",
  "default_route",
  "legacy_unknown",
]);
const modelVerifications = new Set<ModelVerification>([
  "user_confirmed",
  "provider_reported",
  "unverified",
  "legacy_unknown",
]);
```

Add these predicates to `isSafeRunRecord`:

```ts
modelSources.has(value.target.modelSource as ModelSource) &&
modelVerifications.has(
  value.target.modelVerification as ModelVerification,
)
```

Update frontend run fixtures explicitly rather than making the fields
optional. New manual-run builders use `manual`/`user_confirmed`. New CLI
builders use `default_route`/`unverified` when the model is blank and
`cli_requested`/`user_confirmed` when it is explicit. Rust fixtures use the
semantically equivalent pair; only fixtures specifically representing old
stored data use `legacy_unknown`/`legacy_unknown`.

Before running tests, rerun:

```powershell
rg -n "TargetSelection \{" crates apps -g "*.rs"
rg -n "reportedModel:" apps/desktop/src -g "*.ts" -g "*.tsx"
```

Every complete target initializer must now carry both fields, directly or
through a spread from a complete fixture.

- [ ] **Step 5: Run core and frontend suites**

Run:

```powershell
cargo test -p ability-core --all-targets
npm test --workspace apps/desktop -- src/api/runtimeValidation.test.ts
npm test --workspace apps/desktop -- src/pages/CliRunPage.test.tsx src/pages/ManualRunPage.test.tsx src/api/tauriBackend.test.ts
npm run build --workspace apps/desktop
```

Expected: all commands pass and old stored target JSON deserializes to legacy
provenance.

- [ ] **Step 6: Commit the stored contract**

```powershell
git add crates/ability-core/src/domain.rs crates/ability-core/src/orchestration.rs crates/ability-core/tests/domain_contracts.rs crates/ability-core/tests/manual_run.rs crates/ability-core/tests/recovery.rs crates/ability-core/tests/report.rs crates/ability-core/tests/report_schema.rs crates/ability-core/tests/storage.rs crates/ability-adapters/src/cli_run.rs crates/ability-adapters/tests/cli_run.rs apps/desktop/src-tauri/src/app_state.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src-tauri/src/data_management_tests.rs apps/desktop/src-tauri/src/dto.rs apps/desktop/src-tauri/tests/fake_cli_e2e.rs apps/desktop/src/api/backend.ts apps/desktop/src/api/runtimeValidation.ts apps/desktop/src/api/backend.test.ts apps/desktop/src/api/runtimeValidation.test.ts apps/desktop/src/api/tauriBackend.test.ts apps/desktop/src/pages/CliRunPage.tsx apps/desktop/src/pages/CliRunPage.test.tsx apps/desktop/src/pages/HistoryPage.test.ts apps/desktop/src/pages/HistoryPage.ui.test.tsx apps/desktop/src/pages/ManualRunPage.tsx apps/desktop/src/pages/ManualRunPage.test.tsx apps/desktop/src/pages/ResultPage.test.tsx apps/desktop/src/test/accessibility.test.tsx
git commit -m "feat: persist model provenance"
```

Before committing, confirm `git diff --name-only` contains only files whose
`TargetSelection` fixture or provenance validation changed.

---

### Task 2: Validate Provenance at the Tauri Boundary

**Files:**

- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Test: `apps/desktop/src-tauri/src/dto.rs`
- Test: `apps/desktop/src-tauri/src/commands.rs`

**Interfaces:**

- Consumes: Task 1 model provenance enums.
- Produces: strict start/resume inputs and `validate_provenance`.

- [ ] **Step 1: Add failing DTO tests**

Update the valid manual input:

```json
{
  "kind": "chat_gpt_client",
  "reportedModel": "GPT-5",
  "reasoningEffort": "high",
  "modelSource": "windows_accessibility",
  "modelVerification": "user_confirmed"
}
```

Add invalid combinations to command tests:

```rust
let inferred = start_input(
    TargetKind::ChatGptClient,
    "GPT-5",
    ModelSource::CliReported,
    ModelVerification::ProviderReported,
);
assert!(validate_start(inferred, StartFamily::Manual).is_err());

let guessed_default = start_input(
    TargetKind::CodexCli,
    "default",
    ModelSource::CliRequested,
    ModelVerification::UserConfirmed,
);
assert!(validate_start(guessed_default, StartFamily::Cli).is_err());
```

Also assert a new CLI start using
`cli_reported`/`provider_reported` is rejected: provider-reported identity is
an internal post-run fact, never a start-command claim. Add a resume test
showing a stored legacy target with
`legacy_unknown`/`legacy_unknown` remains resumable when all other checkpoint
fields match.

- [ ] **Step 2: Run Tauri library tests and verify failure**

Run:

```powershell
cargo test -p ability-radar --lib dto
cargo test -p ability-radar --lib provenance
```

Expected: DTO parsing or assertions fail because fields and combination
validation are missing.

- [ ] **Step 3: Extend strict DTOs**

Extend both `TargetSelectionInput` and `ResumeTargetSelectionInput`:

```rust
pub model_source: ModelSource,
pub model_verification: ModelVerification,
```

Keep `deny_unknown_fields`; do not add defaults at the command boundary.

When building `TargetSelection`, copy both fields exactly after validation.

- [ ] **Step 4: Add exact combination validation**

Add:

```rust
#[derive(Clone, Copy)]
enum ProvenanceContext {
    NewRun,
    Resume,
}

fn validate_provenance(
    target: &TargetSelection,
    family: StartFamily,
    context: ProvenanceContext,
) -> Result<(), String> {
    let accepted_new = match family {
        StartFamily::Manual => matches!(
            (target.model_source, target.model_verification),
            (ModelSource::Manual, ModelVerification::UserConfirmed)
                | (
                    ModelSource::WindowsAccessibility,
                    ModelVerification::UserConfirmed
                )
        ),
        StartFamily::Cli if target.reported_model == "default" => matches!(
            (target.model_source, target.model_verification),
            (ModelSource::DefaultRoute, ModelVerification::Unverified)
        ),
        StartFamily::Cli => matches!(
            (target.model_source, target.model_verification),
            (ModelSource::CliRequested, ModelVerification::UserConfirmed)
        ),
    };
    let accepted_resume_only = matches!(context, ProvenanceContext::Resume)
        && (
            matches!(
                (target.model_source, target.model_verification),
                (ModelSource::LegacyUnknown, ModelVerification::LegacyUnknown)
            )
                || (
                    family == StartFamily::Cli
                        && matches!(
                            (target.model_source, target.model_verification),
                            (
                                ModelSource::CliReported,
                                ModelVerification::ProviderReported
                            )
                        )
                )
        );
    let accepted = accepted_new || accepted_resume_only;
    accepted
        .then_some(())
        .ok_or_else(|| "模型来源与所选体检方式不一致".into())
}
```

Call it with `NewRun` from start validation and `Resume` from resume-target
validation after model and effort normalization. A manual resume never
accepts the resume-only `CliReported` pair. Equality with the stored target
remains the final resume guard, so the command cannot upgrade or rewrite a
legacy record.

- [ ] **Step 5: Run DTO, command, and recovery tests**

Run:

```powershell
cargo test -p ability-radar --lib
cargo test -p ability-core --test recovery
```

Expected: both pass and resume equality includes provenance.

- [ ] **Step 6: Commit boundary validation**

```powershell
git add apps/desktop/src-tauri/src/dto.rs apps/desktop/src-tauri/src/commands.rs crates/ability-core/tests/recovery.rs
git commit -m "feat: validate model provenance at startup"
```

---

### Task 3: Build the Pure Client Selector Parser

**Files:**

- Create: `apps/desktop/src-tauri/src/client_selection.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Test: `apps/desktop/src-tauri/src/client_selection.rs`

**Interfaces:**

- Consumes: `TargetKind`, `ModelSource`, and existing display-text validation.
- Produces detection DTOs plus:

```rust
fn preliminary_provider(
    identity: &WindowIdentity,
) -> Option<ProviderFamily>;
fn confirm_provider(
    provider: ProviderFamily,
    controls: &[RawControl],
) -> bool;
fn classify_surface(
    provider: ProviderFamily,
    controls: &[RawControl],
    title_hint: &str,
) -> Option<(ClientSurface, ClientSelectionConfidence)>;
fn extract_candidates(
    target: TargetKind,
    controls: &[ObservedControl],
) -> ClientSelectionDetection;
```

- [ ] **Step 1: Write synthetic parser tests**

Add tests inside `client_selection.rs`:

```rust
#[test]
fn openai_selector_extracts_model_and_effort_without_document_text() {
    let controls = vec![
        control(ControlKind::Document, "private conversation GPT-Fake"),
        control(ControlKind::Button, "GPT-5.6"),
        control(ControlKind::ComboBox, "最高"),
    ];

    let result = extract_candidates(TargetKind::ChatGptClient, &controls);

    assert_eq!(result.status, ClientSelectionStatus::Detected);
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].model.as_deref(), Some("GPT-5.6"));
    assert_eq!(
        result.candidates[0].reasoning_effort.as_deref(),
        Some("max")
    );
}

#[test]
fn multiple_visible_models_are_returned_without_guessing() {
    let controls = vec![
        control(ControlKind::Button, "GPT-5.6"),
        control(ControlKind::Button, "GPT-5.6 Codex"),
    ];

    let result = extract_candidates(TargetKind::ChatGptClient, &controls);

    assert_eq!(result.status, ClientSelectionStatus::Multiple);
    assert_eq!(result.candidates.len(), 2);
}

#[test]
fn claude_parser_rejects_openai_labels_and_unsafe_text() {
    let controls = vec![
        control(ControlKind::Button, "GPT-5.6"),
        control(ControlKind::Button, "Claude Sonnet\u{202e}"),
    ];

    let result = extract_candidates(TargetKind::ClaudeClient, &controls);

    assert_eq!(result.status, ClientSelectionStatus::NotExposed);
    assert!(result.candidates.is_empty());
}
```

Add tests proving a safe unknown client effort label such as `扩展思考`
survives as a custom value, while a 41-character effort label, an Edit
control, and a 121-character model label are all discarded.

Add identity tests proving a process name never decides identity by itself.
The current package family identifies the OpenAI provider, while an
allowlisted top-level UI anchor identifies the active ChatGPT or Codex
surface:

```rust
assert_eq!(
    preliminary_provider(&identity(
        "ChatGPT.exe",
        Some("OpenAI.Codex_2p2nqsd0c76g0"),
        "ChatGPT"
    )),
    Some(ProviderFamily::OpenAi)
);
assert_eq!(
    classify_surface(
        ProviderFamily::OpenAi,
        &[raw_control(ControlKind::Button, "Codex")],
        "ChatGPT",
    ),
    Some((
        ClientSurface::CodexDesktop,
        ClientSelectionConfidence::VisibleSelector,
    ))
);
assert_eq!(
    preliminary_provider(&identity("ChatGPT.exe", None, "ChatGPT")),
    None
);
```

This split is required because the current OpenAI desktop application can
contain both ChatGPT and Codex surfaces; package identity establishes the
provider, not the active surface. This follows OpenAI's current
[desktop migration guidance](https://help.openai.com/en/articles/20001276/),
which describes ChatGPT and Codex in one desktop application. Keep provider
fingerprints in one versioned table:

- OpenAI packaged apps require a package family whose publisher ID is
  `2p2nqsd0c76g0` and whose name begins `OpenAI.`.
- Claude preliminary identity requires either an `Anthropic.Claude` package
  family or all three unpackaged signals: canonical executable basename
  `Claude.exe`, an absolute non-temporary install path, and a Claude title
  hint. `confirm_provider` additionally requires a Claude header/accessibility
  anchor before any candidate is emitted.
- `ChatGPT.exe`, `Claude.exe`, a window title, or a model-looking label alone
  always returns `None`.

For an OpenAI provider, a header/toolbar `ChatGPT`, `Chat`, or `Work` anchor
maps to `ChatGpt`; a `Codex` anchor maps to `CodexDesktop`. Allowed controls
take precedence over the weaker title hint. When controls contain anchors
for both surfaces, return no surface rather than reading selection patterns
or guessing; use the title hint only when controls contain neither anchor.
Claude provider identity maps to `Claude`. Raw identity strings are
discarded after classification.

- [ ] **Step 2: Run parser tests and verify the module is absent**

Run:

```powershell
cargo test -p ability-radar --lib client_selection
```

Expected: compilation fails because the module and types do not exist.

- [ ] **Step 3: Define the serializable response types**

Create:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSelectionStatus {
    Detected,
    Multiple,
    NotRunning,
    NotExposed,
    Unsupported,
    TimedOut,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSurface {
    #[serde(rename = "chatgpt")]
    ChatGpt,
    CodexDesktop,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSelectionConfidence {
    VisibleSelector,
    BestEffort,
}
```

`ClientSelectionCandidate.source` is always
`ModelSource::WindowsAccessibility`.

- [ ] **Step 4: Implement restricted parsing**

Define internal observations:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlKind {
    Button,
    ComboBox,
    MenuItem,
    Document,
    Edit,
    Other,
}

struct RawControl {
    kind: ControlKind,
    name: String,
}

struct ObservedControl {
    surface: ClientSurface,
    kind: ControlKind,
    name: String,
}
```

Implement these exact filters:

```rust
fn allowed_selector(kind: ControlKind) -> bool {
    matches!(
        kind,
        ControlKind::Button | ControlKind::ComboBox | ControlKind::MenuItem
    )
}

fn normalized_effort(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let canonical = match trimmed.to_ascii_lowercase().as_str() {
        "无" | "none" => Some("none"),
        "最小" | "minimal" => Some("minimal"),
        "轻度" | "低" | "low" | "light" => Some("low"),
        "中" | "medium" => Some("medium"),
        "高" | "high" => Some("high"),
        "极高" | "extra high" | "xhigh" => Some("xhigh"),
        "最高" | "max" => Some("max"),
        "ultra" => Some("ultra"),
        _ => None,
    };
    if let Some(canonical) = canonical {
        return Some(canonical.to_owned());
    }
    let lower = trimmed.to_ascii_lowercase();
    let looks_like_custom_effort = [
        "reason", "thinking", "effort", "推理", "思考",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    (looks_like_custom_effort
        && !trimmed.is_empty()
        && trimmed.chars().count() <= 40
        && safe_display_text(trimmed))
    .then(|| trimmed.to_owned())
}
```

Model recognition must require a target-specific visible selector label:

```rust
fn looks_like_model(target: TargetKind, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    match target {
        TargetKind::ChatGptClient => {
            lower.contains("gpt")
                || lower.contains("codex")
                || (
                    lower.starts_with('o')
                        && lower[1..]
                            .chars()
                            .next()
                            .is_some_and(|value| value.is_ascii_digit())
                )
        }
        TargetKind::ClaudeClient => {
            lower.contains("claude")
                || lower.contains("sonnet")
                || lower.contains("opus")
                || lower.contains("haiku")
        }
        _ => false,
    }
}
```

Before accepting a label, trim it, enforce 1–120 visible characters, and call
the same forbidden-display-character predicate used for reported models.
For each surface, pair its unique model with its unique effort; if either
side has multiple distinct values, emit the distinct combinations as
candidates and return `Multiple` rather than selecting the first. A safe
custom effort is retained exactly and flows through the existing custom
effort validation. Deduplicate exact `(surface, model, effort)` candidates.

- [ ] **Step 5: Run parser and Tauri library tests**

Run:

```powershell
cargo test -p ability-radar --lib client_selection
cargo test -p ability-radar --lib
```

Expected: synthetic extraction, privacy exclusion, identity ambiguity, and
existing command-inventory tests pass.

- [ ] **Step 6: Commit the pure parser**

```powershell
git add apps/desktop/src-tauri/src/client_selection.rs apps/desktop/src-tauri/src/lib.rs
git commit -m "feat: parse visible client model selectors"
```

---

### Task 4: Add the Windows UI Automation Adapter and Narrow Command

**Files:**

- Create: `apps/desktop/src-tauri/src/client_selection/windows.rs`
- Modify: `apps/desktop/src-tauri/src/client_selection.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `Cargo.lock`
- Test: `apps/desktop/src-tauri/src/lib.rs`
- Test: `apps/desktop/src-tauri/src/dto.rs`

**Interfaces:**

- Consumes: Task 3 `ObservedControl`, identity classifier, and extraction.
- Produces:

```rust
pub async fn detect_client_selection(
    target: TargetKind,
) -> ClientSelectionDetection;
```

and Tauri command `detect_client_selection`.

- [ ] **Step 1: Write failing command inventory and DTO tests**

Add `detect_client_selection` to the exact expected invoke allowlist in
`lib.rs`.

Add DTO tests:

```rust
let input: DetectClientSelectionInput = serde_json::from_value(json!({
    "target": "chat_gpt_client"
}))
.unwrap();
assert_eq!(input.target, TargetKind::ChatGptClient);

assert!(serde_json::from_value::<DetectClientSelectionInput>(json!({
    "target": "codex_cli"
})).is_ok());
```

The command, not deserialization, rejects CLI target kinds with the stable
message `模型辅助识别仅支持客户端体检`.

Add native-boundary tests using synthetic traversal counters:

- node 513 is never visited;
- depth 25 is never descended into;
- a deadline-exhausted scan maps to `timed_out`;
- an empty allowlisted-window set maps to `not_running`;
- a non-Windows platform dispatch maps to `unsupported` when that target is
  built in cross-platform CI.

- [ ] **Step 2: Run the tests and verify command registration is absent**

Run:

```powershell
cargo test -p ability-radar --lib invoke_surface_is_the_exact_reviewed_allowlist
cargo test -p ability-radar --lib detect_client_selection_input
```

Expected: the inventory and DTO tests fail.

- [ ] **Step 3: Add reviewed Windows API features**

In the Windows dependency section of `apps/desktop/src-tauri/Cargo.toml`, add:

```toml
windows = { version = "=0.61.3", features = [
  "Win32_Foundation",
  "Win32_Storage_Packaging_Appx",
  "Win32_System_Com",
  "Win32_System_Threading",
  "Win32_UI_Accessibility",
  "Win32_UI_WindowsAndMessaging",
] }
```

Add Tokio `time` to the existing feature list:

```toml
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time"] }
```

Run:

```powershell
cargo check -p ability-radar --locked
```

If the lockfile changes because `windows` becomes a direct dependency, retain
only the dependency-edge update; version `0.61.3` is already present
transitively.

- [ ] **Step 4: Implement bounded Windows enumeration**

In `windows.rs`, define:

```rust
const MAX_WINDOWS: usize = 24;
const MAX_NODES: usize = 512;
const MAX_DEPTH: usize = 24;
const MAX_LABEL_CHARS: usize = 120;
const SCAN_BUDGET: Duration = Duration::from_millis(1_200);
```

Use `EnumWindows`, `IsWindowVisible`, `GetWindowThreadProcessId`,
`OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`,
`QueryFullProcessImageNameW`, and `GetPackageFamilyName` to build
`WindowIdentity`. Exclude `std::process::id()` and stop at `MAX_WINDOWS`.

Initialize COM and UI Automation on the blocking worker:

```rust
unsafe {
    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
}
let _com = ComGuard;
let automation: IUIAutomation = unsafe {
    CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
}.map_err(|_| ScanFailure::Unavailable)?;
```

`HRESULT::ok()` must accept both `S_OK` and `S_FALSE`; construct `ComGuard`
only after that success so `CoUninitialize` is balanced. Treat
`RPC_E_CHANGED_MODE` and every other COM initialization failure as
`ScanFailure::Unavailable` on this dedicated blocking worker.

Use `ElementFromHandle` and `RawViewWalker`. Traverse breadth-first, stopping
at the node, depth, or deadline limit. Call `CurrentName` only when:

1. `CurrentControlType` is Button, ComboBox, or MenuItem;
2. `CurrentBoundingRectangle` lies in the upper 28% of the provider window;
3. the label is at most `MAX_LABEL_CHARS`.

Do not call ValuePattern, TextPattern, `CurrentValue`, or any property on
Document/Edit controls. Run `preliminary_provider` before constructing UI
Automation so unrelated windows are never traversed. For a preliminary
provider window, collect bounded `RawControl` values, call
`confirm_provider`, classify the active surface, and then annotate only the
accepted values as `ObservedControl`; discard a window when either
classification is ambiguous. A ChatGPT/OpenAI target scans only OpenAI
provider candidates, and a Claude target scans only Claude candidates. Use
an RAII guard that calls `CoUninitialize`.

Keep traversal limits in a small pure `TraversalBudget` value. Unit tests
advance its node, depth, and deadline counters without constructing real UI
Automation objects. Keep the async timeout mapping in an internal
`detect_client_selection_with_budget` helper so its timeout test uses a
10-millisecond synthetic blocking scan rather than sleeping for 1.5 seconds.

Return `NotRunning` when no allowlisted provider window exists,
`NotExposed` when at least one provider window exists but parsing yields no
safe candidate, `TimedOut` when either bound expires, and `Failed` only for
initialization or unexpected native API failure.

- [ ] **Step 5: Add platform dispatch and timeout**

In `client_selection.rs`:

```rust
pub async fn detect_client_selection(
    target: TargetKind,
) -> ClientSelectionDetection {
    if !matches!(
        target,
        TargetKind::ChatGptClient | TargetKind::ClaudeClient
    ) {
        return ClientSelectionDetection::failed(
            ClientSelectionStatus::Failed,
        );
    }

    #[cfg(windows)]
    {
        return match tokio::time::timeout(
            Duration::from_millis(1_500),
            tokio::task::spawn_blocking(move || windows::scan(target)),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => ClientSelectionDetection::failed(
                ClientSelectionStatus::Failed,
            ),
            Err(_) => ClientSelectionDetection::failed(
                ClientSelectionStatus::TimedOut,
            ),
        };
    }

    #[cfg(not(windows))]
    ClientSelectionDetection::failed(ClientSelectionStatus::Unsupported)
}
```

Do not log raw scan data.

- [ ] **Step 6: Register the strict Tauri command**

Add:

```rust
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectClientSelectionInput {
    pub target: TargetKind,
}
```

Add the command:

```rust
#[tauri::command]
pub async fn detect_client_selection(
    input: DetectClientSelectionInput,
) -> Result<ClientSelectionDetection, String> {
    if !matches!(
        input.target,
        TargetKind::ChatGptClient | TargetKind::ClaudeClient
    ) {
        return Err("模型辅助识别仅支持客户端体检".into());
    }
    Ok(client_selection::detect_client_selection(input.target).await)
}
```

Register it through `command_inventory!`; do not add filesystem, shell, or
window-control permissions.

- [ ] **Step 7: Run Windows and cross-platform-safe tests**

Run:

```powershell
cargo fmt --all --check
cargo clippy -p ability-radar --all-targets --locked -- -D warnings
cargo test -p ability-radar --lib
```

Expected: all pass, and command inventory contains only the prior allowlist
plus `detect_client_selection`.

- [ ] **Step 8: Commit the native adapter**

```powershell
git add apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/src/client_selection/windows.rs apps/desktop/src-tauri/src/client_selection.rs apps/desktop/src-tauri/src/dto.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src-tauri/src/lib.rs Cargo.lock
git commit -m "feat: read client selectors through Windows accessibility"
```

---

### Task 5: Add the React Identification Panel and Manual Fallback

**Files:**

- Modify: `apps/desktop/src/api/backend.ts`
- Modify: `apps/desktop/src/api/runtimeValidation.ts`
- Modify: `apps/desktop/src/api/runtimeValidation.test.ts`
- Modify: `apps/desktop/src/api/tauriBackend.ts`
- Modify: `apps/desktop/src/api/tauriBackend.test.ts`
- Create: `apps/desktop/src/domain/modelProvenance.ts`
- Create: `apps/desktop/src/components/ClientSelectionPanel.tsx`
- Create: `apps/desktop/src/components/ClientSelectionPanel.test.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.test.tsx`
- Modify: `apps/desktop/src/app/App.test.tsx`
- Modify: `apps/desktop/src/pages/CliRunPage.test.tsx`
- Modify: `apps/desktop/src/pages/HistoryPage.ui.test.tsx`
- Modify: `apps/desktop/src/pages/HomePage.test.tsx`
- Modify: `apps/desktop/src/pages/ResultPage.test.tsx`
- Modify: `apps/desktop/src/test/accessibility.test.tsx`

**Interfaces:**

- Consumes: Task 4 Tauri response and Task 1 provenance fields.
- Produces: confirmed manual start requests and a reusable identification
  panel.

- [ ] **Step 1: Write failing backend bridge tests**

In `tauriBackend.test.ts`:

```ts
await tauriBackend.detectClientSelection("chat_gpt_client");
expect(invoke).toHaveBeenCalledWith("detect_client_selection", {
  input: { target: "chat_gpt_client" },
});
```

Return a response containing an extra `windowTitle`, `processPath`, or
`rawControls` field and assert the bridge rejects it with the generic local
protocol error. Also reject an unknown status, an unsafe model string, more
than 24 candidates, `detected` with anything other than one candidate, and
`multiple` with fewer than two. Put the exact shape cases in
`runtimeValidation.test.ts` and keep the invoke/error propagation cases in
`tauriBackend.test.ts`.

- [ ] **Step 2: Write failing panel behavior tests**

In `ClientSelectionPanel.test.tsx`, cover:

```ts
test("auto-runs once and applies one detected candidate", async () => {
  const detect = vi.fn(async () => ({
    status: "detected" as const,
    candidates: [{
      model: "GPT-5.6",
      reasoningEffort: "max",
      surface: "codex_desktop" as const,
      source: "windows_accessibility" as const,
      confidence: "visible_selector" as const,
    }],
  }));
  const onApply = vi.fn();

  render(
    <ClientSelectionPanel
      edited={false}
      enabled
      formDirty={false}
      onApply={onApply}
      target="chat_gpt_client"
      detect={detect}
    />,
  );

  expect(await screen.findByText("已从 Codex 客户端界面读取，待确认"))
    .toBeInTheDocument();
  expect(onApply).toHaveBeenCalledWith({
    model: "GPT-5.6",
    reasoningEffort: "max",
  });
});
```

Also test:

- `multiple` renders radio choices and does not call `onApply` before choice;
- a single result arriving after the user typed does not overwrite the form
  and instead offers an explicit “应用识别结果” action;
- detected and multiple-candidate rows show “Windows 可访问性” plus
  “可见选择器” or “最佳努力”, never a window title or process path;
- `not_running`, `not_exposed`, `timed_out`, and rejected Promise preserve
  “可手动填写”;
- a stale first request cannot overwrite a newer refresh;
- unmount ignores completion;
- disabling automatic detection writes the local setting and does not call
  the backend;
- denied or throwing `localStorage` access falls back to enabled for the
  current page without crashing.

- [ ] **Step 3: Run bridge and panel tests**

Run:

```powershell
npm test --workspace apps/desktop -- src/api/runtimeValidation.test.ts src/api/tauriBackend.test.ts src/components/ClientSelectionPanel.test.tsx
```

Expected: tests fail because the bridge and panel are absent.

- [ ] **Step 4: Add frontend response types and bridge**

In `backend.ts`, add exact unions matching Rust and:

```ts
export interface ClientSelectionCandidate {
  model?: string | null;
  reasoningEffort?: string | null;
  surface: "chatgpt" | "codex_desktop" | "claude";
  source: "windows_accessibility";
  confidence: "visible_selector" | "best_effort";
}

export interface ClientSelectionDetection {
  status:
    | "detected"
    | "multiple"
    | "not_running"
    | "not_exposed"
    | "unsupported"
    | "timed_out"
    | "failed";
  candidates: ClientSelectionCandidate[];
}
```

In `runtimeValidation.ts`, add an exact runtime validator. It must:

1. accept only the top-level keys `status` and `candidates`;
2. accept no more than 24 candidates;
3. accept only candidate keys `model`, `reasoningEffort`, `surface`,
   `source`, and `confidence`;
4. enforce the exact unions above, display-safe model text up to 120
   characters, display-safe effort text up to 40 characters, and at least one
   non-empty model or effort value per candidate;
5. require one candidate for `detected`, at least two for `multiple`, and
   zero for all failure/fallback statuses.

Add to `Backend`:

```ts
detectClientSelection(
  target: "chat_gpt_client" | "claude_client",
): Promise<ClientSelectionDetection>;
```

Add to `tauriBackend`, invoking as `unknown` and validating before return:

```ts
detectClientSelection: async (target) => {
  const value = await invoke<unknown>("detect_client_selection", {
    input: { target },
  });
  if (!isSafeClientSelectionDetection(value)) {
    throw new Error("本地模型识别返回了无效数据");
  }
  return value;
},
```

Update each fake backend with a deterministic default:

```ts
detectClientSelection: async () => ({
  status: "not_running",
  candidates: [],
}),
```

- [ ] **Step 5: Implement the panel state machine**

Use:

```ts
export const CLIENT_AUTO_DETECT_KEY =
  "ai-ability-radar.client-selection-auto-detect";
```

The `enabled` prop means the surrounding route permits scanning.
`formDirty` means the user has edited either field since the last applied
candidate, and `edited` means an automatically applied selection was later
changed. The panel also owns `autoDetectionEnabled`, the persisted user
preference. The panel:

1. defaults `autoDetectionEnabled` to true unless stored value is exactly
   `"false"`;
2. starts one request on mount when both flags are enabled;
3. uses a monotonically increasing request ID;
4. renders one `role="status"` region for loading/success/fallback;
5. renders radio buttons for distinct multiple candidates;
6. calls `onApply` automatically for one result only when `formDirty` is
   false; otherwise it stores the result and requires “应用识别结果”;
7. uses a ref for the latest `formDirty` value so a slow request cannot
   overwrite typing that happened after the request began;
8. exposes “重新识别”;
9. renders “用户已修改，请确认当前填写值” when `edited` is true;
10. catches synchronous and asynchronous backend failure;
11. never disables the surrounding manual fields.

Render the setting as a normal checkbox, not an icon-only control:

```tsx
<label className="selection-setting">
  <input
    checked={autoDetectionEnabled}
    onChange={(event) => setAutoDetection(event.target.checked)}
    type="checkbox"
  />
  <span>进入设置页时自动读取客户端可见选择器</span>
</label>
```

Wrap `localStorage.getItem` and `setItem` in `try/catch`; failure changes no
run data and never blocks manual entry.

Use this stable surface copy:

```ts
const surfaceLabels = {
  chatgpt: "ChatGPT",
  codex_desktop: "Codex",
  claude: "Claude",
} as const;
```

Use these source/confidence labels next to each candidate:

```ts
const detectionSourceLabel = "Windows 可访问性";
const confidenceLabels = {
  visible_selector: "可见选择器",
  best_effort: "最佳努力",
} as const;
```

- [ ] **Step 6: Integrate with ManualRunPage**

Track:

```ts
const [modelSource, setModelSource] =
  useState<ModelSource>("manual");
const [modelVerification] =
  useState<ModelVerification>("user_confirmed");
const [formDirty, setFormDirty] = useState(false);
const [selectionWasApplied, setSelectionWasApplied] = useState(false);
```

When the panel applies a candidate:

```ts
if (candidate.model) {
  setModel(candidate.model);
  setModelTouched(false);
  setModelSource("windows_accessibility");
}
if (candidate.reasoningEffort) {
  setReasoningEffort(candidate.reasoningEffort);
}
setFormDirty(false);
setSelectionWasApplied(true);
```

A partial candidate never clears an existing field. Effort-only assistance
does not claim the model came from Windows accessibility; entering or
retaining a manual model keeps `modelSource: "manual"`.

When the user changes either model or effort:

```ts
setFormDirty(true);
setModelSource("manual");
```

A change after an applied candidate passes
`edited={selectionWasApplied && formDirty}` to the panel. Pass `formDirty`
separately so an in-flight first result cannot overwrite early manual input.

Build the start target with:

```ts
modelSource,
modelVerification: "user_confirmed",
```

Extend `sameTarget` to compare provenance. Auto-detection must not run on a
resume review because the stored target is authoritative.

- [ ] **Step 7: Run manual page and accessibility tests**

Run:

```powershell
npm test --workspace apps/desktop -- src/api/runtimeValidation.test.ts src/api/tauriBackend.test.ts src/components/ClientSelectionPanel.test.tsx src/pages/ManualRunPage.test.tsx src/test/accessibility.test.tsx
npm run build --workspace apps/desktop
```

Expected: all pass; detection failure leaves model input usable and start
behavior unchanged after valid manual entry.

- [ ] **Step 8: Commit client interaction**

```powershell
git add apps/desktop/src/api/backend.ts apps/desktop/src/api/runtimeValidation.ts apps/desktop/src/api/runtimeValidation.test.ts apps/desktop/src/api/tauriBackend.ts apps/desktop/src/api/tauriBackend.test.ts apps/desktop/src/domain/modelProvenance.ts apps/desktop/src/components/ClientSelectionPanel.tsx apps/desktop/src/components/ClientSelectionPanel.test.tsx apps/desktop/src/pages/ManualRunPage.tsx apps/desktop/src/pages/ManualRunPage.test.tsx apps/desktop/src/test/accessibility.test.tsx apps/desktop/src/app/App.test.tsx apps/desktop/src/pages/HomePage.test.tsx apps/desktop/src/pages/CliRunPage.test.tsx apps/desktop/src/pages/HistoryPage.ui.test.tsx apps/desktop/src/pages/ResultPage.test.tsx
git commit -m "feat: assist client model identification"
```

---

### Task 6: Record CLI Provenance and Show It Everywhere

**Files:**

- Modify: `apps/desktop/src/pages/CliRunPage.tsx`
- Modify: `apps/desktop/src/pages/CliRunPage.test.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.tsx`
- Modify: `apps/desktop/src/pages/ManualRunPage.test.tsx`
- Modify: `apps/desktop/src/domain/modelProvenance.ts`
- Modify: `apps/desktop/src/pages/HistoryPage.tsx`
- Modify: `apps/desktop/src/pages/HistoryPage.ui.test.tsx`
- Modify: `apps/desktop/src/pages/ResultPage.tsx`
- Modify: `apps/desktop/src/pages/ResultPage.test.tsx`
- Modify: `crates/ability-core/src/report.rs`
- Modify: `crates/ability-core/tests/report.rs`
- Modify: `crates/ability-core/tests/report_schema.rs`
- Modify: `schemas/public-report.schema.json`
- Modify: `apps/desktop/src-tauri/src/data_management_tests.rs`

**Interfaces:**

- Consumes: persisted provenance.
- Produces: consistent local and public provenance labels; public report schema
  version 2.

- [ ] **Step 1: Write failing CLI and recovery display tests**

Task 1 already proves the exact start request. Here, render an empty CLI
model and assert:

```ts
expect(screen.getByText("模型来源：CLI 默认路由 · 未核验"))
  .toBeInTheDocument();
```

After entering `gpt-5.6-codex`, assert:

```ts
expect(screen.getByText("模型来源：CLI 本次明确指定 · 用户已确认"))
  .toBeInTheDocument();
```

Add manual and CLI resume-review assertions for the persisted provenance.
No resume page may rerun client identification or replace the stored source.

- [ ] **Step 2: Write failing history/result/report tests**

Add local display assertions:

```ts
expect(screen.getByText("模型来源：Windows 客户端界面 · 用户已确认"))
  .toBeInTheDocument();
expect(screen.getByText("模型来源：CLI 默认路由 · 未核验"))
  .toBeInTheDocument();
```

In Rust report tests, assert public target JSON contains:

```json
{
  "modelSource": "windows_accessibility",
  "modelVerification": "user_confirmed"
}
```

and that schema version is `2`.

- [ ] **Step 3: Run targeted tests and verify missing output**

Run:

```powershell
npm test --workspace apps/desktop -- src/pages/CliRunPage.test.tsx src/pages/ManualRunPage.test.tsx src/pages/HistoryPage.ui.test.tsx src/pages/ResultPage.test.tsx
cargo test -p ability-core --test report
cargo test -p ability-core --test report_schema
```

Expected: provenance request/display/schema assertions fail.

- [ ] **Step 4: Render CLI setup and recovery provenance**

Keep the exact target builder established in Task 1:

```ts
const reportedModel = model.trim() || "default";
const defaultRoute = reportedModel === "default";
const target = {
  kind,
  reportedModel,
  reasoningEffort:
    normalizeReasoningEffortForTarget(kind, reasoningEffort) || null,
  modelSource: defaultRoute ? "default_route" : "cli_requested",
  modelVerification: defaultRoute ? "unverified" : "user_confirmed",
} satisfies TargetSelection;
```

Do not set `cli_reported` unless a reviewed structured provider event contains
an explicit model field. Current event fixtures do not provide one, so no
reported model is fabricated in this release. Use
`formatModelProvenance(target)` in CLI setup preview and in both manual and
CLI resume-review metadata.

- [ ] **Step 5: Centralize Chinese provenance copy**

Implement:

```ts
export function formatModelProvenance(target: TargetSelection): string {
  const source = {
    manual: "用户填写",
    windows_accessibility: "Windows 客户端界面",
    cli_requested: "CLI 本次明确指定",
    cli_reported: "CLI 已报告",
    default_route: "CLI 默认路由",
    legacy_unknown: "历史记录，来源未知",
  }[target.modelSource];
  const verification = {
    user_confirmed: "用户已确认",
    provider_reported: "提供方已报告",
    unverified: "未核验",
    legacy_unknown: "可信状态未知",
  }[target.modelVerification];
  return `模型来源：${source} · ${verification}`;
}
```

Use this function in History and Result pages.

- [ ] **Step 6: Upgrade the public report to schema v2**

Set:

```rust
pub const PUBLIC_REPORT_SCHEMA_VERSION: u32 = 2;
```

Extend `PublicTarget` with `model_source` and `model_verification`, copy them
from the run, validate combinations, include their Chinese labels in HTML,
and include both values in the sensitive-source scan.

The report validator and JSON Schema `oneOf` accept only these pairs:

| `modelSource` | `modelVerification` |
| --- | --- |
| `manual` | `user_confirmed` |
| `windows_accessibility` | `user_confirmed` |
| `cli_requested` | `user_confirmed` |
| `cli_reported` | `provider_reported` |
| `default_route` | `unverified` |
| `legacy_unknown` | `legacy_unknown` |

Update `schemas/public-report.schema.json`:

```json
"title": "AI Ability Radar public report v2",
"schemaVersion": { "const": 2 }
```

Require target fields:

```json
"modelSource",
"modelVerification"
```

with exact enums matching the Rust wire values. Update full-backup and schema
fixtures from version 1 to 2 where they represent the public report.

- [ ] **Step 7: Run provenance and report regression**

Run:

```powershell
npm test --workspace apps/desktop -- src/pages/CliRunPage.test.tsx src/pages/ManualRunPage.test.tsx src/pages/HistoryPage.ui.test.tsx src/pages/ResultPage.test.tsx
cargo test -p ability-core --test storage
cargo test -p ability-core --test recovery
cargo test -p ability-core --test report
cargo test -p ability-core --test report_schema
cargo test -p ability-radar --lib
```

Expected: all pass; old run JSON is readable, and new public reports use
schema v2.

- [ ] **Step 8: Commit provenance display and report schema**

```powershell
git add apps/desktop/src/pages/CliRunPage.tsx apps/desktop/src/pages/CliRunPage.test.tsx apps/desktop/src/pages/ManualRunPage.tsx apps/desktop/src/pages/ManualRunPage.test.tsx apps/desktop/src/domain/modelProvenance.ts apps/desktop/src/pages/HistoryPage.tsx apps/desktop/src/pages/HistoryPage.ui.test.tsx apps/desktop/src/pages/ResultPage.tsx apps/desktop/src/pages/ResultPage.test.tsx crates/ability-core/src/report.rs crates/ability-core/tests/report.rs crates/ability-core/tests/report_schema.rs schemas/public-report.schema.json apps/desktop/src-tauri/src/data_management_tests.rs
git commit -m "feat: display trustworthy model provenance"
```

---

### Task 7: Full Verification and Real-Client Fallback

**Files:**

- Modify if verified behavior changes user guidance:
  `docs/privacy.md`
- Modify if verified behavior changes troubleshooting:
  `docs/troubleshooting.md`

**Interfaces:**

- Consumes: completed native and frontend identification paths.
- Produces: verified client behavior ready for visual refinement.

- [ ] **Step 1: Run complete automated verification**

Run:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm test
npm run validate:repository
```

Expected: all pass; no automated test starts a real AI provider.

- [ ] **Step 2: Launch the current source build**

Run:

```powershell
npm start
```

Expected: the desktop application opens with the new Tauri command.

- [ ] **Step 3: Verify current OpenAI/Codex behavior**

Open the ChatGPT/OpenAI manual setup route and confirm:

1. detection starts once;
2. if the current Codex WebView exposes no model selector, the UI says
   “当前客户端未公开选择器，可手填”;
3. model and reasoning controls remain editable;
4. “重新识别” works;
5. no click, keystroke, message, or model call occurs;
6. a manually entered run saves `manual`/`user_confirmed`.

The read-only feasibility probe on this machine currently exposes window
chrome but not the Codex model selector. `not_exposed` is therefore an
acceptable verified result; it must not be misreported as successful
automatic identification.

- [ ] **Step 4: Verify synthetic Claude coverage**

Because Claude Desktop is not installed on this machine, rerun:

```powershell
cargo test -p ability-radar --lib client_selection
```

Expected: Claude synthetic selector, multiple-candidate, unsafe-label, and
no-window tests all pass. Record Claude real-client validation as a release
checklist item, not as a passing real-machine claim.

- [ ] **Step 5: Update privacy and troubleshooting with verified language**

Add:

```text
模型辅助识别只读取受支持客户端窗口中可访问的选择器名称。
客户端未公开选择器时会回退手动填写；应用不会截图、读取对话正文、
点击客户端或额外调用模型。
```

State clearly that the current Codex client may return `not_exposed`.

- [ ] **Step 6: Commit verified guidance**

```powershell
git add docs/privacy.md docs/troubleshooting.md
git commit -m "docs: explain client model identification limits"
```
