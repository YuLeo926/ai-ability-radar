# AI Ability Radar Desktop MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Windows-first v0.1–v0.2 desktop application that runs eight assisted ChatGPT/Claude tasks, two automatic Codex CLI/Claude Code tasks, stores results locally, and exports a reviewed redacted report without collecting subscription credentials.

**Architecture:** A Tauri desktop shell hosts a React/TypeScript UI. A Rust workspace contains pure domain, pack-loading, grading, persistence, orchestration, and report code in `ability-core`; CLI-specific command construction and process parsing live in `ability-adapters`. The Tauri command layer is the only bridge between the unprivileged webview and privileged Rust code.

**Tech Stack:** Tauri 2, React, TypeScript, Vite, Vitest, React Testing Library, Rust stable MSVC, Tokio, Serde, SQLite through `rusqlite`, and GitHub Actions.

## Global Constraints

- Implement only v0.1 and v0.2; v0.5 and later remain outside this plan.
- Runtime target is Windows 10/11 x64; the app itself must not require Node.js,
  but the v0.2 CLI quick pack requires a supported Node.js 22 or 24 LTS runtime.
- ChatGPT, Claude, Codex CLI, and Claude Code are separate result series and never share one total score.
- v0.2 uses exactly eight client tasks: three instruction-following, three logic, and two code-review tasks.
- v0.2 uses exactly two CLI micro-repository tasks.
- Quick client target duration is 10–15 minutes; quick CLI target duration is 30–60 minutes, presented as an estimate rather than a guarantee.
- The UI never receives generic shell or unrestricted filesystem permissions.
- Codex uses ephemeral JSONL execution with workspace-write only; Claude Code never uses `--dangerously-skip-permissions`.
- Invalid infrastructure runs do not count as ability failures; exhausting a fixed agent turn/time budget without infrastructure failure does.
- v0.2 records objective scores and strictly comparable history only; it does not emit a degradation verdict, personal baseline, or calibrated confidence.
- Raw answers and logs remain local; exported reports use a field allowlist and must reject detected credentials, usernames, and absolute user paths.
- v0.2 has no default telemetry, automatic upload, GitHub authentication, public leaderboard, model judge, or desktop UI automation.
- Application source uses Apache-2.0; third-party task content has a separate license inventory.
- Use TDD, keep modules focused, and commit after every task.

---

## File and Responsibility Map

```text
Cargo.toml
rust-toolchain.toml
package.json
LICENSE
apps/desktop/
  package.json
  vite.config.ts
  vitest.config.ts
  src/
    app/App.tsx
    app/routes.tsx
    api/backend.ts
    api/tauriBackend.ts
    components/
    pages/
    test/
    styles/
  src-tauri/
    Cargo.toml
    tauri.conf.json
    capabilities/default.json
    src/app_state.rs
    src/commands.rs
    src/dto.rs
    src/lib.rs
    src/main.rs
crates/ability-core/
  Cargo.toml
  migrations/0001_init.sql
  migrations/0002_settings.sql
  src/domain.rs
  src/grading.rs
  src/orchestration.rs
  src/packs.rs
  src/report.rs
  src/storage.rs
  src/lib.rs
crates/ability-adapters/
  Cargo.toml
  src/process.rs
  src/classify.rs
  src/codex.rs
  src/claude.rs
  src/verifier.rs
  src/lib.rs
benchmark-packs/
  registry.json
  client-quick-v1/
  cli-quick-v1/
schemas/
  pack.schema.json
  public-report.schema.json
site/
  index.html
docs/
  privacy.md
  security.md
  troubleshooting.md
.github/workflows/
  ci.yml
  release.yml
  pages.yml
```

`ability-core` must not depend on Tauri or React. `ability-adapters` may depend on `ability-core`, but `ability-core` must not depend on `ability-adapters`. The Tauri crate composes both crates and owns OS paths, process lifetime, and frontend event emission.

---

### Task 1: Scaffold the Workspace and Prove the Desktop Shell

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `package.json`
- Create: `LICENSE`
- Create: `apps/desktop/**` from the official Tauri React/TypeScript template
- Create: `crates/ability-core/Cargo.toml`
- Create: `crates/ability-core/src/lib.rs`
- Create: `crates/ability-adapters/Cargo.toml`
- Create: `crates/ability-adapters/src/lib.rs`
- Create: `apps/desktop/src/app/App.test.tsx`
- Create: `apps/desktop/src/test/setup.ts`
- Create: `apps/desktop/vitest.config.ts`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/main.tsx`

**Interfaces:**
- Consumes: none.
- Produces: a Cargo workspace, an npm workspace, a launchable Tauri app, and the `ability_core` and `ability_adapters` crates.

- [ ] **Step 1: Scaffold Tauri and the two Rust library crates**

Run from the repository root:

```powershell
npm create tauri-app@latest apps/desktop
```

Choose these exact prompt values:

```text
Project name: ability-radar
Identifier: com.aiability.radar
Frontend language: TypeScript / JavaScript
Package manager: npm
UI template: React
UI flavor: TypeScript
```

Then run:

```powershell
cargo new --lib crates/ability-core
cargo new --lib crates/ability-adapters
```

Expected: Tauri template files exist under `apps/desktop` and both Rust crates
exist.

- [ ] **Step 2: Add root workspace configuration**

Create `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
  "apps/desktop/src-tauri",
  "crates/ability-core",
  "crates/ability-adapters",
]
```

Create `rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["clippy", "rustfmt"]
profile = "minimal"
```

Create `package.json`:

```json
{
  "name": "ai-ability-radar",
  "private": true,
  "workspaces": [
    "apps/desktop"
  ],
  "scripts": {
    "build": "npm run build --workspace apps/desktop",
    "test": "npm run test --workspace apps/desktop",
    "tauri": "npm run tauri --workspace apps/desktop"
  }
}
```

Only after the root workspace file exists, run:

```powershell
Remove-Item -LiteralPath apps/desktop/src-tauri/Cargo.lock -ErrorAction SilentlyContinue
Remove-Item -LiteralPath apps/desktop/package-lock.json -ErrorAction SilentlyContinue
npm install --workspace apps/desktop
npm install --workspace apps/desktop --save-dev vitest jsdom @testing-library/react @testing-library/jest-dom
```

Expected: npm creates the single root `package-lock.json` and exits with code 0.

Copy the full Apache License 2.0 text from <https://www.apache.org/licenses/LICENSE-2.0.txt> into `LICENSE`.

- [ ] **Step 3: Write the failing frontend smoke test**

Create `apps/desktop/src/test/setup.ts`:

```ts
import "@testing-library/jest-dom/vitest";
```

Create `apps/desktop/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    globals: true,
  },
});
```

Add this script to `apps/desktop/package.json`:

```json
"test": "vitest run"
```

Create `apps/desktop/src/app/App.test.tsx`:

```tsx
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
```

- [ ] **Step 4: Run the test and verify the generated app fails it**

Run:

```powershell
npm test
```

Expected: FAIL because `apps/desktop/src/app/App.tsx` does not exist or the generated page does not contain the required heading and button.

- [ ] **Step 5: Implement the minimal product shell**

Create `apps/desktop/src/app/App.tsx`:

```tsx
export function App() {
  return (
    <main>
      <h1>AI 能力雷达</h1>
      <p>本地优先的 AI 表现与降智检测工具</p>
      <button type="button">开始 AI 体检</button>
    </main>
  );
}
```

Replace `apps/desktop/src/App.tsx` with:

```tsx
export { App as default } from "./app/App";
```

Keep `apps/desktop/src/main.tsx` as the Tauri template entry point and ensure it imports `App` from `./App`.

- [ ] **Step 6: Verify frontend, Rust, and desktop builds**

Run:

```powershell
npm test
cargo test --workspace
npm run build
```

Expected: all commands exit 0; Vitest reports one passing test.

- [ ] **Step 7: Commit**

```powershell
git add Cargo.toml Cargo.lock rust-toolchain.toml package.json package-lock.json LICENSE apps crates
git commit -m "chore: scaffold desktop workspace"
```

---

### Task 2: Define Stable Domain Types and JSON Contracts

**Files:**
- Create: `crates/ability-core/src/domain.rs`
- Create: `crates/ability-core/tests/domain_contracts.rs`
- Modify: `crates/ability-core/src/lib.rs`
- Modify: `crates/ability-core/Cargo.toml`

**Interfaces:**
- Consumes: the `ability-core` crate from Task 1.
- Produces: `TargetKind`, `RunMode`, `RunStatus`, `TaskOutcome`, `FailureKind`, `Category`, `TargetSelection`, `EnvironmentFingerprint`, `RunRecord`, `TaskResult`, and `ScoreSummary`.

- [ ] **Step 1: Add domain dependencies**

Add to `crates/ability-core/Cargo.toml`:

```toml
[dependencies]
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["serde", "v4"] }
```

- [ ] **Step 2: Write failing serialization and constructor tests**

Create `crates/ability-core/tests/domain_contracts.rs`:

```rust
use ability_core::{
    Category, EnvironmentFingerprint, RunMode, RunRecord, RunStatus, TargetKind,
    TargetSelection,
};

#[test]
fn target_kind_serializes_as_stable_snake_case() {
    let json = serde_json::to_string(&TargetKind::ClaudeCode).unwrap();
    assert_eq!(json, "\"claude_code\"");
}

#[test]
fn a_new_run_starts_created_with_zero_progress() {
    let run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::CodexCli,
            reported_model: "current CLI selection".into(),
            reasoning_effort: Some("high".into()),
        },
        RunMode::Quick,
        "cli-quick".into(),
        "1.0.0".into(),
        2,
        EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "11".into(),
            app_version: "0.2.0".into(),
            cli_version: Some("codex 1.2.3".into()),
            verifier_runtime_version: Some("node v22.0.0".into()),
            suite_id: "cli-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "a".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    );

    assert_eq!(run.status, RunStatus::Created);
    assert_eq!(run.completed_tasks, 0);
    assert_eq!(run.total_tasks, 2);
    assert!(run.score.is_none());
    assert_eq!(Category::CliCoding.to_string(), "cli_coding");
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run:

```powershell
cargo test -p ability-core --test domain_contracts
```

Expected: FAIL with unresolved imports from `ability_core`.

- [ ] **Step 4: Implement the complete domain model**

Create `crates/ability-core/src/domain.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    ChatGptClient,
    ClaudeClient,
    CodexCli,
    ClaudeCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Quick,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Running,
    Completed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Passed,
    Failed,
    Invalid,
    Cancelled,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    CliMissing,
    RuntimeMissing,
    AuthExpired,
    QuotaExhausted,
    Network,
    UserCancelled,
    AppInterrupted,
    InfrastructureTimeout,
    AgentBudgetExceeded,
    VerifierError,
    WrongAnswer,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    InstructionFollowing,
    Logic,
    CodeReview,
    CliCoding,
}

impl Display for Category {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::InstructionFollowing => "instruction_following",
            Self::Logic => "logic",
            Self::CodeReview => "code_review",
            Self::CliCoding => "cli_coding",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSelection {
    pub kind: TargetKind,
    pub reported_model: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentFingerprint {
    pub os_family: String,
    pub os_version: String,
    pub app_version: String,
    pub cli_version: Option<String>,
    pub verifier_runtime_version: Option<String>,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_content_sha256: String,
    pub scoring_rule_version: String,
    pub resumed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSummary {
    pub ability_score: f64,
    pub passed_tasks: u32,
    pub valid_tasks: u32,
    pub total_tasks: u32,
    pub category_scores: BTreeMap<Category, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: Uuid,
    pub target: TargetSelection,
    pub mode: RunMode,
    pub suite_id: String,
    pub suite_version: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub total_tasks: u32,
    pub completed_tasks: u32,
    pub environment: EnvironmentFingerprint,
    pub score: Option<ScoreSummary>,
}

impl RunRecord {
    pub fn new(
        target: TargetSelection,
        mode: RunMode,
        suite_id: String,
        suite_version: String,
        total_tasks: u32,
        environment: EnvironmentFingerprint,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            target,
            mode,
            suite_id,
            suite_version,
            status: RunStatus::Created,
            started_at: Utc::now(),
            finished_at: None,
            total_tasks,
            completed_tasks: 0,
            environment,
            score: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub run_id: Uuid,
    pub task_id: String,
    pub category: Category,
    pub outcome: TaskOutcome,
    pub score: Option<f64>,
    pub failure_kind: Option<FailureKind>,
    pub duration_ms: u64,
    pub answer_rel_path: Option<String>,
    pub detail: String,
}
```

Replace `crates/ability-core/src/lib.rs` with:

```rust
mod domain;

pub use domain::*;
```

- [ ] **Step 5: Run domain tests and formatting**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-core --test domain_contracts
```

Expected: formatting exits 0 and both tests pass.

- [ ] **Step 6: Commit**

```powershell
git add Cargo.lock crates/ability-core
git commit -m "feat: define stable run domain contracts"
```

---

### Task 3: Load Versioned Task Packs Safely

**Files:**
- Create: `crates/ability-core/src/packs.rs`
- Create: `crates/ability-core/tests/pack_loading.rs`
- Create: `schemas/pack.schema.json`
- Modify: `crates/ability-core/src/lib.rs`
- Modify: `crates/ability-core/Cargo.toml`

**Interfaces:**
- Consumes: `TargetKind` and `Category` from Task 2.
- Produces: `PackManifest`, `TaskDefinition`, `GraderSpec`, `LoadedPack`, `LoadedTask`, `PackLoader::load(&Path)`, and `PackError`.

- [ ] **Step 1: Add loader dependencies**

Add to `crates/ability-core/Cargo.toml`:

```toml
regex = "1"
sha2 = "0.10"
thiserror = "2"
```

- [ ] **Step 2: Write failing safety tests**

Create `crates/ability-core/tests/pack_loading.rs`:

```rust
use ability_core::{PackError, PackLoader, PackRegistry};
use std::fs;
use tempfile::tempdir;

#[test]
fn loads_a_minimal_pack_and_computes_a_hash() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version": 1,
          "id": "smoke-pack",
          "version": "1.0.0",
          "title": "Smoke Pack",
          "target_kinds": ["chat_gpt_client"],
          "tasks": [{
            "id": "smoke-1",
            "category": "logic",
            "prompt_file": "prompt.txt",
            "starter_dir": null,
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"exact_text","expected":"4"}
          }]
        }"#,
    )
    .unwrap();

    let pack = PackLoader::load(dir.path()).unwrap();
    assert_eq!(pack.manifest.id, "smoke-pack");
    assert_eq!(pack.tasks[0].prompt, "Only answer 4.");
    assert_eq!(pack.content_sha256.len(), 64);
}

#[test]
fn hash_covers_starter_and_verifier_files_not_only_prompts() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("starter/src")).unwrap();
    fs::write(dir.path().join("prompt.txt"), "Fix the function.").unwrap();
    fs::write(dir.path().join("starter/src/index.mjs"), "export const value = 1;").unwrap();
    fs::write(dir.path().join("verify.mjs"), "console.log('TASK_PASSED');").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version": 1,
          "id": "hash-pack",
          "version": "1.0.0",
          "title": "Hash Pack",
          "target_kinds": ["codex_cli"],
          "tasks": [{
            "id": "hash-1",
            "category": "cli_coding",
            "prompt_file": "prompt.txt",
            "starter_dir": "starter",
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"external_verifier","verifier_id":"hash-v1"}
          }]
        }"#,
    )
    .unwrap();

    let before = PackLoader::load(dir.path()).unwrap().content_sha256;
    fs::write(dir.path().join("starter/src/index.mjs"), "export const value = 2;").unwrap();
    let after = PackLoader::load(dir.path()).unwrap().content_sha256;
    assert_ne!(before, after);
}

#[test]
fn embedded_registry_rejects_a_modified_bundled_pack() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("prompt.txt"), "Only answer 4.").unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version":1,"id":"sealed-pack","version":"1.0.0",
          "title":"Sealed","target_kinds":["chat_gpt_client"],
          "tasks":[{"id":"one","category":"logic","prompt_file":"prompt.txt",
            "starter_dir":null,"time_budget_secs":60,"max_turns":1,
            "grader":{"type":"exact_text","expected":"4"}}]
        }"#,
    )
    .unwrap();
    let pack = PackLoader::load(dir.path()).unwrap();
    let registry = PackRegistry::parse(
        r#"{"schema_version":1,"packs":[{
          "id":"sealed-pack","version":"1.0.0","path":"sealed-pack",
          "license":"Apache-2.0","bundled":true,
          "content_sha256":"0000000000000000000000000000000000000000000000000000000000000000"
        }]}"#,
    )
    .unwrap();
    assert!(matches!(
        registry.verify_bundled(&pack),
        Err(PackError::HashMismatch { .. })
    ));
}

#[test]
fn rejects_prompt_path_traversal() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{
          "schema_version": 1,
          "id": "bad-pack",
          "version": "1.0.0",
          "title": "Bad Pack",
          "target_kinds": ["chat_gpt_client"],
          "tasks": [{
            "id": "bad-1",
            "category": "logic",
            "prompt_file": "../secret.txt",
            "starter_dir": null,
            "time_budget_secs": 60,
            "max_turns": 1,
            "grader": {"type":"exact_text","expected":"4"}
          }]
        }"#,
    )
    .unwrap();

    assert!(matches!(
        PackLoader::load(dir.path()),
        Err(PackError::UnsafePath(_))
    ));
}
```

Add `tempfile = "3"` under `[dev-dependencies]`.

- [ ] **Step 3: Run tests to verify failure**

Run:

```powershell
cargo test -p ability-core --test pack_loading
```

Expected: FAIL with unresolved `PackLoader` and `PackError`.

- [ ] **Step 4: Implement manifest parsing, path checks, and hashing**

Create `crates/ability-core/src/packs.rs`:

```rust
use crate::{Category, TargetKind};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_PROMPT_BYTES: u64 = 256 * 1024;
const MAX_PACK_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PACK_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub title: String,
    pub target_kinds: Vec<TargetKind>,
    pub tasks: Vec<TaskDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    pub id: String,
    pub category: Category,
    pub prompt_file: String,
    pub starter_dir: Option<String>,
    pub time_budget_secs: u64,
    pub max_turns: u32,
    pub grader: GraderSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GraderSpec {
    ExactText { expected: String },
    ExactJson { expected: Value },
    JsonStringSet { expected: Vec<String> },
    ExternalVerifier { verifier_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRegistry {
    pub schema_version: u32,
    pub packs: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub id: String,
    pub version: String,
    pub path: String,
    pub license: String,
    pub bundled: bool,
    pub content_sha256: String,
}

impl PackRegistry {
    pub fn parse(json: &str) -> Result<Self, PackError> {
        let registry: Self = serde_json::from_str(json)?;
        if registry.schema_version != 1 {
            return Err(PackError::InvalidManifest(
                "unsupported registry schema".into(),
            ));
        }
        Ok(registry)
    }

    pub fn verify_bundled(&self, pack: &LoadedPack) -> Result<(), PackError> {
        let entry = self
            .packs
            .iter()
            .find(|entry| {
                entry.id == pack.manifest.id
                    && entry.version == pack.manifest.version
                    && entry.bundled
            })
            .ok_or_else(|| {
                PackError::InvalidManifest(format!(
                    "untrusted bundled pack {} {}",
                    pack.manifest.id, pack.manifest.version
                ))
            })?;
        if entry.content_sha256 != pack.content_sha256 {
            return Err(PackError::HashMismatch {
                expected: entry.content_sha256.clone(),
                actual: pack.content_sha256.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LoadedTask {
    pub definition: TaskDefinition,
    pub prompt: String,
    pub pack_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LoadedPack {
    pub manifest: PackManifest,
    pub tasks: Vec<LoadedTask>,
    pub content_sha256: String,
}

#[derive(Debug, Error)]
pub enum PackError {
    #[error("pack file is missing: {0}")]
    Missing(String),
    #[error("pack path is unsafe: {0}")]
    UnsafePath(String),
    #[error("pack file exceeds size limit: {0}")]
    TooLarge(String),
    #[error("pack id is invalid: {0}")]
    InvalidId(String),
    #[error("pack manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("pack hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("pack text is not UTF-8: {0}")]
    InvalidUtf8(String),
    #[error("pack JSON is invalid: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("pack I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub struct PackLoader;

impl PackLoader {
    pub fn load(root: &Path) -> Result<LoadedPack, PackError> {
        let root = root.canonicalize()?;
        let manifest_path = root.join("manifest.json");
        let metadata = fs::metadata(&manifest_path)
            .map_err(|_| PackError::Missing("manifest.json".into()))?;
        if metadata.len() > MAX_MANIFEST_BYTES {
            return Err(PackError::TooLarge("manifest.json".into()));
        }

        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest: PackManifest = serde_json::from_slice(&manifest_bytes)?;
        let id_re = Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").unwrap();
        let version_re = Regex::new(r"^\d+\.\d+\.\d+$").unwrap();
        if !id_re.is_match(&manifest.id) {
            return Err(PackError::InvalidId(manifest.id));
        }
        if manifest.schema_version != 1
            || !version_re.is_match(&manifest.version)
            || manifest.title.trim().is_empty()
            || manifest.target_kinds.is_empty()
            || manifest.tasks.is_empty()
        {
            return Err(PackError::InvalidManifest(manifest.id));
        }

        let content_sha256 = hash_directory(&root)?;
        let mut tasks = Vec::with_capacity(manifest.tasks.len());
        let mut task_ids = std::collections::BTreeSet::new();

        for definition in &manifest.tasks {
            if !id_re.is_match(&definition.id)
                || !task_ids.insert(definition.id.clone())
            {
                return Err(PackError::InvalidId(definition.id.clone()));
            }
            if definition.time_budget_secs == 0
                || definition.time_budget_secs > 7_200
                || definition.max_turns == 0
                || definition.max_turns > 100
            {
                return Err(PackError::InvalidManifest(
                    definition.id.clone(),
                ));
            }
            let prompt_path = safe_child(&root, &definition.prompt_file)?;
            let prompt_metadata = fs::metadata(&prompt_path)
                .map_err(|_| PackError::Missing(definition.prompt_file.clone()))?;
            if prompt_metadata.len() > MAX_PROMPT_BYTES {
                return Err(PackError::TooLarge(definition.prompt_file.clone()));
            }
            let prompt_bytes = fs::read(&prompt_path)?;
            let prompt = String::from_utf8(prompt_bytes)
                .map_err(|_| PackError::InvalidUtf8(definition.prompt_file.clone()))?;

            if let Some(starter_dir) = &definition.starter_dir {
                let starter_path = safe_child(&root, starter_dir)?;
                if !starter_path.is_dir() {
                    return Err(PackError::Missing(starter_dir.clone()));
                }
            }

            tasks.push(LoadedTask {
                definition: definition.clone(),
                prompt,
                pack_root: root.clone(),
            });
        }

        Ok(LoadedPack {
            manifest,
            tasks,
            content_sha256,
        })
    }
}

fn hash_directory(root: &Path) -> Result<String, PackError> {
    let mut files = Vec::<(String, PathBuf, u64)>::new();
    collect_pack_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let total_bytes = files.iter().map(|(_, _, size)| *size).sum::<u64>();
    if total_bytes > MAX_PACK_BYTES {
        return Err(PackError::TooLarge("entire pack".into()));
    }

    let mut digest = Sha256::new();
    for (relative, path, size) in files {
        let relative = relative.as_bytes();
        digest.update((relative.len() as u64).to_le_bytes());
        digest.update(relative);
        digest.update(size.to_le_bytes());
        digest.update(fs::read(path)?);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_pack_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf, u64)>,
) -> Result<(), PackError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| PackError::UnsafePath(path.display().to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        if metadata.file_type().is_symlink() {
            return Err(PackError::UnsafePath(relative));
        }
        if metadata.is_dir() {
            collect_pack_files(root, &path, files)?;
        } else if metadata.is_file() {
            if metadata.len() > MAX_PACK_FILE_BYTES {
                return Err(PackError::TooLarge(relative));
            }
            files.push((relative, path, metadata.len()));
        }
    }
    Ok(())
}

fn safe_child(root: &Path, relative: &str) -> Result<PathBuf, PackError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(PackError::UnsafePath(relative.into()));
    }
    let joined = root.join(relative_path);
    let canonical = joined
        .canonicalize()
        .map_err(|_| PackError::Missing(relative.into()))?;
    if !canonical.starts_with(root) {
        return Err(PackError::UnsafePath(relative.into()));
    }
    Ok(canonical)
}
```

Update `crates/ability-core/src/lib.rs`:

```rust
mod domain;
mod packs;

pub use domain::*;
pub use packs::*;
```

- [ ] **Step 5: Add the public JSON schema**

Create `schemas/pack.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema_version", "id", "version", "title", "target_kinds", "tasks"],
  "properties": {
    "schema_version": {"const": 1},
    "id": {"type": "string", "pattern": "^[a-z0-9]+(?:-[a-z0-9]+)*$"},
    "version": {"type": "string", "pattern": "^\\d+\\.\\d+\\.\\d+$"},
    "title": {"type": "string", "minLength": 1},
    "target_kinds": {
      "type": "array",
      "minItems": 1,
      "items": {
        "enum": ["chat_gpt_client", "claude_client", "codex_cli", "claude_code"]
      }
    },
    "tasks": {
      "type": "array",
      "minItems": 1,
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": [
          "id",
          "category",
          "prompt_file",
          "starter_dir",
          "time_budget_secs",
          "max_turns",
          "grader"
        ],
        "properties": {
          "id": {"type": "string", "pattern": "^[a-z0-9]+(?:-[a-z0-9]+)*$"},
          "category": {
            "enum": ["instruction_following", "logic", "code_review", "cli_coding"]
          },
          "prompt_file": {"type": "string", "minLength": 1},
          "starter_dir": {"type": ["string", "null"]},
          "time_budget_secs": {"type": "integer", "minimum": 1, "maximum": 7200},
          "max_turns": {"type": "integer", "minimum": 1, "maximum": 100},
          "grader": {
            "oneOf": [
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["type", "expected"],
                "properties": {
                  "type": {"const": "exact_text"},
                  "expected": {"type": "string"}
                }
              },
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["type", "expected"],
                "properties": {
                  "type": {"const": "exact_json"},
                  "expected": {}
                }
              },
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["type", "expected"],
                "properties": {
                  "type": {"const": "json_string_set"},
                  "expected": {
                    "type": "array",
                    "items": {"type": "string"},
                    "uniqueItems": true
                  }
                }
              },
              {
                "type": "object",
                "additionalProperties": false,
                "required": ["type", "verifier_id"],
                "properties": {
                  "type": {"const": "external_verifier"},
                  "verifier_id": {"type": "string", "pattern": "^[a-z0-9-]+$"}
                }
              }
            ]
          }
        }
      }
    }
  }
}
```

- [ ] **Step 6: Run pack tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-core --test pack_loading
```

Expected: all four pack-loading tests pass.

- [ ] **Step 7: Commit**

```powershell
git add Cargo.lock crates/ability-core schemas/pack.schema.json
git commit -m "feat: load versioned task packs safely"
```

---

### Task 4: Implement Deterministic Grading and Equal-Category Scoring

**Files:**
- Create: `crates/ability-core/src/grading.rs`
- Create: `crates/ability-core/tests/grading.rs`
- Modify: `crates/ability-core/src/lib.rs`

**Interfaces:**
- Consumes: `GraderSpec`, `Category`, `TaskOutcome`, `ScoreSummary`, and `TaskResult`.
- Produces: `TaskGrade`, `grade_submission(&GraderSpec, &str)`, and
  `summarize_scores(&[TaskResult], u32) -> Option<ScoreSummary>`.

- [ ] **Step 1: Write failing grader tests**

Create `crates/ability-core/tests/grading.rs`:

```rust
use ability_core::{
    grade_submission, summarize_scores, Category, FailureKind, GraderSpec, TaskOutcome,
    TaskResult,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn exact_json_rejects_extra_markdown_but_ignores_key_order() {
    let grader = GraderSpec::ExactJson {
        expected: json!({"count": 2, "names": ["A", "B"]}),
    };
    assert!(grade_submission(&grader, r#"{"names":["A","B"],"count":2}"#).passed);
    assert!(!grade_submission(
        &grader,
        "答案如下：\n```json\n{\"count\":2,\"names\":[\"A\",\"B\"]}\n```"
    )
    .passed);
}

#[test]
fn category_scores_are_equal_weighted() {
    let run_id = Uuid::new_v4();
    let result = |task_id: &str, category: Category, score: f64| TaskResult {
        run_id,
        task_id: task_id.into(),
        category,
        outcome: if score == 100.0 {
            TaskOutcome::Passed
        } else {
            TaskOutcome::Failed
        },
        score: Some(score),
        failure_kind: if score == 100.0 {
            None
        } else {
            Some(FailureKind::WrongAnswer)
        },
        duration_ms: 1,
        answer_rel_path: None,
        detail: String::new(),
    };
    let results = vec![
        result("i1", Category::InstructionFollowing, 100.0),
        result("i2", Category::InstructionFollowing, 100.0),
        result("i3", Category::InstructionFollowing, 100.0),
        result("l1", Category::Logic, 0.0),
    ];

    let summary = summarize_scores(&results, 4).unwrap();
    assert_eq!(summary.ability_score, 50.0);
    assert_eq!(summary.category_scores[&Category::InstructionFollowing], 100.0);
    assert_eq!(summary.category_scores[&Category::Logic], 0.0);
}

#[test]
fn invalid_tasks_do_not_enter_the_denominator() {
    let run_id = Uuid::new_v4();
    let results = vec![TaskResult {
        run_id,
        task_id: "network".into(),
        category: Category::Logic,
        outcome: TaskOutcome::Invalid,
        score: None,
        failure_kind: Some(FailureKind::Network),
        duration_ms: 1,
        answer_rel_path: None,
        detail: "network unavailable".into(),
    }];
    assert!(summarize_scores(&results, 1).is_none());
}
```

- [ ] **Step 2: Run the tests to verify failure**

Run:

```powershell
cargo test -p ability-core --test grading
```

Expected: FAIL with unresolved grading functions and `TaskGrade`.

- [ ] **Step 3: Implement all v0.2 deterministic graders**

Create `crates/ability-core/src/grading.rs`:

```rust
use crate::{Category, GraderSpec, ScoreSummary, TaskOutcome, TaskResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskGrade {
    pub score: f64,
    pub passed: bool,
    pub detail: String,
}

pub fn grade_submission(grader: &GraderSpec, submission: &str) -> TaskGrade {
    match grader {
        GraderSpec::ExactText { expected } => {
            binary_grade(submission.trim() == expected, "exact_text")
        }
        GraderSpec::ExactJson { expected } => match serde_json::from_str::<Value>(submission) {
            Ok(actual) => binary_grade(actual == *expected, "exact_json"),
            Err(error) => TaskGrade {
                score: 0.0,
                passed: false,
                detail: format!("invalid_json:{error}"),
            },
        },
        GraderSpec::JsonStringSet { expected } => {
            match serde_json::from_str::<Vec<String>>(submission) {
                Ok(actual) => {
                    let actual: BTreeSet<String> = actual.into_iter().collect();
                    let expected: BTreeSet<String> = expected.iter().cloned().collect();
                    binary_grade(actual == expected, "json_string_set")
                }
                Err(error) => TaskGrade {
                    score: 0.0,
                    passed: false,
                    detail: format!("invalid_string_array:{error}"),
                },
            }
        }
        GraderSpec::ExternalVerifier { verifier_id } => TaskGrade {
            score: 0.0,
            passed: false,
            detail: format!("requires_external_verifier:{verifier_id}"),
        },
    }
}

pub fn summarize_scores(
    results: &[TaskResult],
    total_tasks: u32,
) -> Option<ScoreSummary> {
    let mut grouped: BTreeMap<Category, Vec<f64>> = BTreeMap::new();
    let mut passed_tasks = 0_u32;
    let mut valid_tasks = 0_u32;

    for result in results {
        if let Some(score) = result.score {
            valid_tasks += 1;
            if result.outcome == TaskOutcome::Passed {
                passed_tasks += 1;
            }
            grouped.entry(result.category).or_default().push(score);
        }
    }

    if grouped.is_empty() {
        return None;
    }
    let category_scores: BTreeMap<Category, f64> = grouped
        .into_iter()
        .map(|(category, scores)| {
            let average = scores.iter().sum::<f64>() / scores.len() as f64;
            (category, round_one(average))
        })
        .collect();
    let ability_score = round_one(
        category_scores.values().sum::<f64>() / category_scores.len() as f64,
    );

    Some(ScoreSummary {
        ability_score,
        passed_tasks,
        valid_tasks,
        total_tasks,
        category_scores,
    })
}

fn binary_grade(passed: bool, label: &str) -> TaskGrade {
    TaskGrade {
        score: if passed { 100.0 } else { 0.0 },
        passed,
        detail: if passed {
            format!("{label}:pass")
        } else {
            format!("{label}:mismatch")
        },
    }
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
```

Update `crates/ability-core/src/lib.rs`:

```rust
mod domain;
mod grading;
mod packs;

pub use domain::*;
pub use grading::*;
pub use packs::*;
```

- [ ] **Step 4: Run grading tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-core --test grading
```

Expected: all three tests pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ability-core
git commit -m "feat: add deterministic grading"
```

---

### Task 5: Persist Runs, Checkpoints, and Raw Artifacts Locally

**Files:**
- Create: `crates/ability-core/migrations/0001_init.sql`
- Create: `crates/ability-core/src/storage.rs`
- Create: `crates/ability-core/tests/storage.rs`
- Modify: `crates/ability-core/src/lib.rs`
- Modify: `crates/ability-core/Cargo.toml`

**Interfaces:**
- Consumes: all run domain types from Task 2.
- Produces: `RunRepository::open`, `insert_run`, `save_task_result`, `complete_run`, `get_run`, `get_task_results`, `list_runs`, and `mark_running_as_interrupted`.

- [ ] **Step 1: Add SQLite dependencies**

Add to `crates/ability-core/Cargo.toml`:

```toml
rusqlite = { version = "0.40", features = ["backup", "bundled"] }
parking_lot = "0.12"
```

- [ ] **Step 2: Write failing repository tests**

Create `crates/ability-core/tests/storage.rs`:

```rust
use ability_core::{
    Category, EnvironmentFingerprint, RunMode, RunRecord, RunRepository, RunStatus,
    TargetKind, TargetSelection, TaskOutcome, TaskResult,
};
use tempfile::tempdir;

fn sample_run() -> RunRecord {
    RunRecord::new(
        TargetSelection {
            kind: TargetKind::ChatGptClient,
            reported_model: "user-selected".into(),
            reasoning_effort: None,
        },
        RunMode::Quick,
        "client-quick".into(),
        "1.0.0".into(),
        8,
        EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "11".into(),
            app_version: "0.2.0".into(),
            cli_version: None,
            verifier_runtime_version: None,
            suite_id: "client-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "b".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    )
}

#[test]
fn checkpoints_survive_repository_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ability.db");
    let run = sample_run();
    {
        let repo = RunRepository::open(&db_path).unwrap();
        repo.insert_run(&run).unwrap();
        repo.save_task_result(&TaskResult {
            run_id: run.id,
            task_id: "instruction-1".into(),
            category: Category::InstructionFollowing,
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            duration_ms: 250,
            answer_rel_path: Some("runs/a/answer.txt".into()),
            detail: "exact_json:pass".into(),
        })
        .unwrap();
    }
    let reopened = RunRepository::open(&db_path).unwrap();
    assert_eq!(reopened.get_task_results(run.id).unwrap().len(), 1);
}

#[test]
fn startup_marks_abandoned_running_runs_interrupted() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    repo.insert_run(&run).unwrap();
    assert_eq!(repo.mark_running_as_interrupted().unwrap(), 1);
    assert_eq!(repo.get_run(run.id).unwrap().unwrap().status, RunStatus::Interrupted);
}
```

- [ ] **Step 3: Run tests to verify failure**

Run:

```powershell
cargo test -p ability-core --test storage
```

Expected: FAIL with unresolved `RunRepository`.

- [ ] **Step 4: Create the migration**

Create `crates/ability-core/migrations/0001_init.sql`:

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA secure_delete = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS targets (
  target_json TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS suite_versions (
  suite_id TEXT NOT NULL,
  suite_version TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  scoring_rule_version TEXT NOT NULL,
  PRIMARY KEY (suite_id, suite_version)
);

CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY,
  target_json TEXT NOT NULL,
  mode_json TEXT NOT NULL,
  suite_id TEXT NOT NULL,
  suite_version TEXT NOT NULL,
  status_json TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  total_tasks INTEGER NOT NULL,
  completed_tasks INTEGER NOT NULL,
  environment_json TEXT NOT NULL,
  score_json TEXT,
  FOREIGN KEY (target_json) REFERENCES targets(target_json),
  FOREIGN KEY (suite_id, suite_version)
    REFERENCES suite_versions(suite_id, suite_version)
);

CREATE TABLE IF NOT EXISTS task_results (
  run_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  category_json TEXT NOT NULL,
  outcome_json TEXT NOT NULL,
  score REAL,
  failure_kind_json TEXT,
  duration_ms INTEGER NOT NULL,
  answer_rel_path TEXT,
  detail TEXT NOT NULL,
  PRIMARY KEY (run_id, task_id),
  FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS runs_started_at_idx ON runs(started_at DESC);
CREATE INDEX IF NOT EXISTS task_results_run_idx ON task_results(run_id);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
```

- [ ] **Step 5: Implement the repository**

Create `crates/ability-core/src/storage.rs` with:

```rust
use crate::{RunRecord, RunStatus, ScoreSummary, TaskResult};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stored timestamp is invalid: {0}")]
    Time(#[from] chrono::ParseError),
    #[error("stored enum is invalid: {0}")]
    Enum(String),
}

pub struct RunRepository {
    connection: Mutex<Connection>,
}

impl RunRepository {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        connection.execute_batch(include_str!("../migrations/0001_init.sql"))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn insert_run(&self, run: &RunRecord) -> Result<(), StorageError> {
        let target_json = serde_json::to_string(&run.target)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO targets(target_json) VALUES (?1)",
            [&target_json],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO suite_versions(
               suite_id,suite_version,content_sha256,scoring_rule_version
             ) VALUES (?1,?2,?3,?4)",
            params![
                &run.suite_id,
                &run.suite_version,
                &run.environment.suite_content_sha256,
                &run.environment.scoring_rule_version,
            ],
        )?;
        transaction.execute(
            "INSERT INTO runs(
              id,target_json,mode_json,suite_id,suite_version,status_json,started_at,
              finished_at,total_tasks,completed_tasks,environment_json,score_json
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                run.id.to_string(),
                target_json,
                serde_json::to_string(&run.mode)?,
                &run.suite_id,
                &run.suite_version,
                serde_json::to_string(&run.status)?,
                run.started_at.to_rfc3339(),
                run.finished_at.as_ref().map(|value| value.to_rfc3339()),
                run.total_tasks,
                run.completed_tasks,
                serde_json::to_string(&run.environment)?,
                run.score.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_task_result(&self, result: &TaskResult) -> Result<(), StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO task_results(
              run_id,task_id,category_json,outcome_json,score,failure_kind_json,
              duration_ms,answer_rel_path,detail
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
            ON CONFLICT(run_id,task_id) DO UPDATE SET
              category_json=excluded.category_json,
              outcome_json=excluded.outcome_json,
              score=excluded.score,
              failure_kind_json=excluded.failure_kind_json,
              duration_ms=excluded.duration_ms,
              answer_rel_path=excluded.answer_rel_path,
              detail=excluded.detail",
            params![
                result.run_id.to_string(),
                &result.task_id,
                serde_json::to_string(&result.category)?,
                serde_json::to_string(&result.outcome)?,
                result.score,
                result
                    .failure_kind
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                result.duration_ms,
                &result.answer_rel_path,
                &result.detail,
            ],
        )?;
        transaction.execute(
            "UPDATE runs SET completed_tasks=(
              SELECT COUNT(*) FROM task_results WHERE run_id=?1
            ) WHERE id=?1",
            [result.run_id.to_string()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn complete_run(
        &self,
        run_id: Uuid,
        score: Option<&ScoreSummary>,
    ) -> Result<(), StorageError> {
        self.connection.lock().execute(
            "UPDATE runs SET status_json=?2, finished_at=?3, score_json=?4 WHERE id=?1",
            params![
                run_id.to_string(),
                serde_json::to_string(&RunStatus::Completed)?,
                Utc::now().to_rfc3339(),
                score.map(serde_json::to_string).transpose()?,
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, run_id: Uuid) -> Result<Option<RunRecord>, StorageError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id,target_json,mode_json,suite_id,suite_version,status_json,
             started_at,finished_at,total_tasks,completed_tasks,environment_json,score_json
             FROM runs WHERE id=?1",
        )?;
        statement
            .query_row([run_id.to_string()], row_to_run)
            .optional()
            .map_err(StorageError::from)
    }

    pub fn get_task_results(&self, run_id: Uuid) -> Result<Vec<TaskResult>, StorageError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT run_id,task_id,category_json,outcome_json,score,failure_kind_json,
             duration_ms,answer_rel_path,detail
             FROM task_results WHERE run_id=?1 ORDER BY rowid",
        )?;
        let rows = statement.query_map([run_id.to_string()], |row| {
            let run_id: String = row.get(0)?;
            let category: String = row.get(2)?;
            let outcome: String = row.get(3)?;
            let failure: Option<String> = row.get(5)?;
            Ok(TaskResult {
                run_id: Uuid::parse_str(&run_id).map_err(to_sql_error)?,
                task_id: row.get(1)?,
                category: serde_json::from_str(&category).map_err(to_sql_error)?,
                outcome: serde_json::from_str(&outcome).map_err(to_sql_error)?,
                score: row.get(4)?,
                failure_kind: failure
                    .map(|value| serde_json::from_str(&value).map_err(to_sql_error))
                    .transpose()?,
                duration_ms: row.get(6)?,
                answer_rel_path: row.get(7)?,
                detail: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    pub fn list_runs(&self) -> Result<Vec<RunRecord>, StorageError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id,target_json,mode_json,suite_id,suite_version,status_json,
             started_at,finished_at,total_tasks,completed_tasks,environment_json,score_json
             FROM runs ORDER BY started_at DESC",
        )?;
        let rows = statement.query_map([], row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(StorageError::from)
    }

    pub fn mark_running_as_interrupted(&self) -> Result<usize, StorageError> {
        self.connection
            .lock()
            .execute(
                "UPDATE runs SET status_json=?1 WHERE status_json=?2",
                params![
                    serde_json::to_string(&RunStatus::Interrupted)?,
                    serde_json::to_string(&RunStatus::Running)?,
                ],
            )
            .map_err(StorageError::from)
    }
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    let id: String = row.get(0)?;
    let target: String = row.get(1)?;
    let mode: String = row.get(2)?;
    let status: String = row.get(5)?;
    let started_at: String = row.get(6)?;
    let finished_at: Option<String> = row.get(7)?;
    let environment: String = row.get(10)?;
    let score: Option<String> = row.get(11)?;
    Ok(RunRecord {
        id: Uuid::parse_str(&id).map_err(to_sql_error)?,
        target: serde_json::from_str(&target).map_err(to_sql_error)?,
        mode: serde_json::from_str(&mode).map_err(to_sql_error)?,
        suite_id: row.get(3)?,
        suite_version: row.get(4)?,
        status: serde_json::from_str(&status).map_err(to_sql_error)?,
        started_at: DateTime::parse_from_rfc3339(&started_at)
            .map_err(to_sql_error)?
            .with_timezone(&Utc),
        finished_at: finished_at
            .map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .map(|date| date.with_timezone(&Utc))
                    .map_err(to_sql_error)
            })
            .transpose()?,
        total_tasks: row.get(8)?,
        completed_tasks: row.get(9)?,
        environment: serde_json::from_str(&environment).map_err(to_sql_error)?,
        score: score
            .map(|value| serde_json::from_str(&value).map_err(to_sql_error))
            .transpose()?,
    })
}

fn to_sql_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(error),
    )
}
```

Update `crates/ability-core/src/lib.rs` to add:

```rust
mod storage;
pub use storage::*;
```

- [ ] **Step 6: Run storage and full core tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-core
```

Expected: all core tests pass, including repository reopen and interruption recovery.

- [ ] **Step 7: Commit**

```powershell
git add Cargo.lock crates/ability-core
git commit -m "feat: persist local run checkpoints"
```

---

### Task 6: Implement the Assisted Client Run Service

**Files:**
- Create: `crates/ability-core/src/orchestration.rs`
- Create: `crates/ability-core/tests/manual_run.rs`
- Modify: `crates/ability-core/src/storage.rs`
- Modify: `crates/ability-core/src/lib.rs`

**Interfaces:**
- Consumes: `LoadedPack`, `RunRepository`, `RunRecord`, `TaskResult`, `grade_submission`, and `summarize_scores`.
- Produces: `ManualRunService::start`, `ManualRunService::next_step`, `ManualRunService::submit_answer`, `ManualStep`, and `RunServiceError`.

- [ ] **Step 1: Write the failing end-to-end manual-run test**

Create `crates/ability-core/tests/manual_run.rs`:

```rust
use ability_core::{
    EnvironmentFingerprint, ManualRunService, PackLoader, RunMode, RunRepository,
    RunServiceError, RunStatus, TargetKind, TargetSelection,
};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn manual_answers_checkpoint_and_complete_the_run() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    fs::create_dir_all(&pack_dir).unwrap();
    fs::write(pack_dir.join("one.txt"), "只输出数字 4").unwrap();
    fs::write(
        pack_dir.join("manifest.json"),
        r#"{
          "schema_version":1,
          "id":"manual-smoke",
          "version":"1.0.0",
          "title":"Manual Smoke",
          "target_kinds":["chat_gpt_client"],
          "tasks":[{
            "id":"one",
            "category":"logic",
            "prompt_file":"one.txt",
            "starter_dir":null,
            "time_budget_secs":60,
            "max_turns":1,
            "grader":{"type":"exact_text","expected":"4"}
          }]
        }"#,
    )
    .unwrap();
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            TargetSelection {
                kind: TargetKind::ChatGptClient,
                reported_model: "user-selected".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            EnvironmentFingerprint {
                os_family: "windows".into(),
                os_version: "11".into(),
                app_version: "0.2.0".into(),
                cli_version: None,
                verifier_runtime_version: None,
                suite_id: "manual-smoke".into(),
                suite_version: "1.0.0".into(),
                suite_content_sha256: pack.content_sha256.clone(),
                scoring_rule_version: "ability-v1".into(),
                resumed: false,
            },
        )
        .unwrap();

    let step = service.next_step(run.id).unwrap().unwrap();
    assert_eq!(step.task_id, "one");
    assert!(matches!(
        service.submit_answer(run.id, "one", &"x".repeat(256 * 1024 + 1)),
        Err(RunServiceError::AnswerTooLarge)
    ));
    assert!(repo.get_task_results(run.id).unwrap().is_empty());
    service.submit_answer(run.id, "one", "4").unwrap();

    assert!(service.next_step(run.id).unwrap().is_none());
    let completed = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(completed.status, RunStatus::Completed);
    assert_eq!(completed.score.unwrap().ability_score, 100.0);
    let answer_path = dir
        .path()
        .join("artifacts")
        .join("runs")
        .join(run.id.to_string())
        .join("one.txt");
    assert_eq!(fs::read_to_string(answer_path).unwrap(), "4");
}
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
cargo test -p ability-core --test manual_run
```

Expected: FAIL with unresolved `ManualRunService`.

- [ ] **Step 3: Add a status update method to the repository**

Add to `impl RunRepository` in `crates/ability-core/src/storage.rs`:

```rust
pub fn set_run_status(
    &self,
    run_id: Uuid,
    status: RunStatus,
) -> Result<(), StorageError> {
    self.connection.lock().execute(
        "UPDATE runs SET status_json=?2 WHERE id=?1",
        params![
            run_id.to_string(),
            serde_json::to_string(&status)?,
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 4: Implement manual orchestration and artifact writes**

Create `crates/ability-core/src/orchestration.rs`:

```rust
use crate::{
    grade_submission, summarize_scores, EnvironmentFingerprint, FailureKind, LoadedPack,
    RunMode, RunRecord, RunRepository, RunStatus, StorageError, TargetKind,
    TargetSelection, TaskOutcome, TaskResult,
};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualStep {
    pub run_id: Uuid,
    pub task_id: String,
    pub task_number: u32,
    pub total_tasks: u32,
    pub prompt: String,
}

#[derive(Debug, Error)]
pub enum RunServiceError {
    #[error("run not found: {0}")]
    RunNotFound(Uuid),
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("target is not a client target")]
    WrongTarget,
    #[error("answer was submitted out of order")]
    OutOfOrder,
    #[error("answer exceeds the 256 KiB local limit")]
    AnswerTooLarge,
    #[error("storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("artifact I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("service state lock is poisoned")]
    Poisoned,
}

struct ActiveManualRun {
    pack: Arc<LoadedPack>,
    task_started: Instant,
}

pub struct ManualRunService {
    repository: Arc<RunRepository>,
    artifact_root: PathBuf,
    active: Mutex<HashMap<Uuid, ActiveManualRun>>,
}

impl ManualRunService {
    pub fn new(repository: Arc<RunRepository>, artifact_root: PathBuf) -> Self {
        Self {
            repository,
            artifact_root,
            active: Mutex::new(HashMap::new()),
        }
    }

    pub fn start(
        &self,
        pack: Arc<LoadedPack>,
        target: TargetSelection,
        mode: RunMode,
        environment: EnvironmentFingerprint,
    ) -> Result<RunRecord, RunServiceError> {
        if !matches!(
            target.kind,
            TargetKind::ChatGptClient | TargetKind::ClaudeClient
        ) {
            return Err(RunServiceError::WrongTarget);
        }
        let mut run = RunRecord::new(
            target,
            mode,
            pack.manifest.id.clone(),
            pack.manifest.version.clone(),
            pack.tasks.len() as u32,
            environment,
        );
        run.status = RunStatus::Running;
        self.repository.insert_run(&run)?;
        self.active
            .lock()
            .map_err(|_| RunServiceError::Poisoned)?
            .insert(
                run.id,
                ActiveManualRun {
                    pack,
                    task_started: Instant::now(),
                },
            );
        Ok(run)
    }

    pub fn next_step(&self, run_id: Uuid) -> Result<Option<ManualStep>, RunServiceError> {
        let active = self
            .active
            .lock()
            .map_err(|_| RunServiceError::Poisoned)?;
        let state = match active.get(&run_id) {
            Some(state) => state,
            None => {
                return match self.repository.get_run(run_id)? {
                    Some(run) if run.status == RunStatus::Completed => Ok(None),
                    _ => Err(RunServiceError::RunNotFound(run_id)),
                }
            }
        };
        let completed = self.repository.get_task_results(run_id)?.len();
        Ok(state.pack.tasks.get(completed).map(|task| ManualStep {
            run_id,
            task_id: task.definition.id.clone(),
            task_number: completed as u32 + 1,
            total_tasks: state.pack.tasks.len() as u32,
            prompt: task.prompt.clone(),
        }))
    }

    pub fn submit_answer(
        &self,
        run_id: Uuid,
        task_id: &str,
        answer: &str,
    ) -> Result<TaskResult, RunServiceError> {
        const MAX_ANSWER_BYTES: usize = 256 * 1024;
        if answer.len() > MAX_ANSWER_BYTES {
            return Err(RunServiceError::AnswerTooLarge);
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| RunServiceError::Poisoned)?;
        let state = active
            .get_mut(&run_id)
            .ok_or(RunServiceError::RunNotFound(run_id))?;
        let completed = self.repository.get_task_results(run_id)?.len();
        let task = state
            .pack
            .tasks
            .get(completed)
            .ok_or_else(|| RunServiceError::TaskNotFound(task_id.into()))?;
        if task.definition.id != task_id {
            return Err(RunServiceError::OutOfOrder);
        }

        let run_dir = self
            .artifact_root
            .join("runs")
            .join(run_id.to_string());
        fs::create_dir_all(&run_dir)?;
        let answer_path = run_dir.join(format!("{task_id}.txt"));
        fs::write(&answer_path, answer)?;
        let grade = grade_submission(&task.definition.grader, answer);
        let result = TaskResult {
            run_id,
            task_id: task_id.into(),
            category: task.definition.category,
            outcome: if grade.passed {
                TaskOutcome::Passed
            } else {
                TaskOutcome::Failed
            },
            score: Some(grade.score),
            failure_kind: if grade.passed {
                None
            } else {
                Some(FailureKind::WrongAnswer)
            },
            duration_ms: state.task_started.elapsed().as_millis() as u64,
            answer_rel_path: Some(format!("runs/{run_id}/{task_id}.txt")),
            detail: grade.detail,
        };
        self.repository.save_task_result(&result)?;

        let results = self.repository.get_task_results(run_id)?;
        let total_tasks = state.pack.tasks.len();
        let finished = results.len() == total_tasks;
        if finished {
            let summary = summarize_scores(&results, total_tasks as u32);
            self.repository.complete_run(run_id, summary.as_ref())?;
        } else {
            state.task_started = Instant::now();
        }
        if finished {
            active.remove(&run_id);
        }
        Ok(result)
    }
}
```

Update `crates/ability-core/src/lib.rs`:

```rust
mod orchestration;
pub use orchestration::*;
```

- [ ] **Step 5: Run manual orchestration and regression tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-core
```

Expected: the manual run test passes and all previous core tests remain green.

- [ ] **Step 6: Commit**

```powershell
git add crates/ability-core
git commit -m "feat: orchestrate assisted client runs"
```

---

### Task 7: Add a Cancellable, Time-Bounded Process Runner

**Files:**
- Create: `crates/ability-adapters/src/process.rs`
- Create: `crates/ability-adapters/tests/process_contract.rs`
- Modify: `crates/ability-adapters/src/lib.rs`
- Modify: `crates/ability-adapters/Cargo.toml`

**Interfaces:**
- Consumes: no domain behavior beyond shared error types.
- Produces: `ProcessSpec`, `ProcessOutput`, `ProcessError`, `ProcessRunner`, and `TokioProcessRunner`.

- [ ] **Step 1: Add asynchronous process dependencies**

Add to `crates/ability-adapters/Cargo.toml`:

```toml
[dependencies]
ability-core = { path = "../ability-core" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tokio = { version = "1", features = ["macros", "process", "rt-multi-thread", "sync", "time"] }
tokio-util = "0.7"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write failing process-contract tests**

Create `crates/ability-adapters/tests/process_contract.rs`:

```rust
use ability_adapters::{ProcessError, ProcessRunner, ProcessSpec, TokioProcessRunner};
use std::collections::BTreeMap;
use std::time::Duration;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn captures_stdout_and_exit_code() {
    let dir = tempdir().unwrap();
    let spec = ProcessSpec {
        program: "powershell".into(),
        args: vec![
            "-NoProfile".into(),
            "-Command".into(),
            "Write-Output ready; exit 0".into(),
        ],
        current_dir: dir.path().to_path_buf(),
        env: BTreeMap::new(),
        timeout: Duration::from_secs(5),
    };
    let output = TokioProcessRunner.run(spec, CancellationToken::new()).await.unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(output.stdout.contains("ready"));
}

#[tokio::test]
async fn cancellation_is_distinct_from_timeout() {
    let dir = tempdir().unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let spec = ProcessSpec {
        program: "powershell".into(),
        args: vec![
            "-NoProfile".into(),
            "-Command".into(),
            "Start-Sleep -Seconds 30".into(),
        ],
        current_dir: dir.path().to_path_buf(),
        env: BTreeMap::new(),
        timeout: Duration::from_secs(60),
    };
    assert!(matches!(
        TokioProcessRunner.run(spec, token).await,
        Err(ProcessError::Cancelled)
    ));
}
```

- [ ] **Step 3: Run tests to verify failure**

Run:

```powershell
cargo test -p ability-adapters --test process_contract
```

Expected: FAIL with unresolved process types.

- [ ] **Step 4: Implement the Windows process runner**

Create `crates/ability-adapters/src/process.rs`:

```rust
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub current_dir: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process could not start: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("process was cancelled")]
    Cancelled,
    #[error("process exceeded the agent budget")]
    TimedOut,
}

#[async_trait]
pub trait ProcessRunner: Send + Sync {
    async fn run(
        &self,
        spec: ProcessSpec,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError>;
}

pub struct TokioProcessRunner;

#[async_trait]
impl ProcessRunner for TokioProcessRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        if cancellation.is_cancelled() {
            return Err(ProcessError::Cancelled);
        }
        let started = Instant::now();
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.current_dir)
            .envs(&spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn()?;
        let pid = child.id();
        let wait = child.wait_with_output();
        tokio::pin!(wait);

        let output = tokio::select! {
            result = &mut wait => result?,
            _ = cancellation.cancelled() => {
                if let Some(pid) = pid {
                    terminate_process_tree(pid).await;
                }
                return Err(ProcessError::Cancelled);
            }
            _ = tokio::time::sleep(spec.timeout) => {
                if let Some(pid) = pid {
                    terminate_process_tree(pid).await;
                }
                return Err(ProcessError::TimedOut);
            }
        };

        Ok(ProcessOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(target_os = "windows")]
async fn terminate_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

#[cfg(not(target_os = "windows"))]
async fn terminate_process_tree(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}
```

Replace `crates/ability-adapters/src/lib.rs` with:

```rust
mod process;

pub use process::*;
```

- [ ] **Step 5: Run process tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-adapters --test process_contract
```

Expected: both async tests pass.

- [ ] **Step 6: Commit**

```powershell
git add Cargo.lock crates/ability-adapters
git commit -m "feat: add bounded process runner"
```

---

### Task 8: Build the Codex CLI Adapter

**Files:**
- Create: `crates/ability-adapters/src/classify.rs`
- Create: `crates/ability-adapters/src/codex.rs`
- Create: `crates/ability-adapters/tests/codex_adapter.rs`
- Modify: `crates/ability-adapters/src/lib.rs`

**Interfaces:**
- Consumes: `ProcessRunner`, `ProcessSpec`, `FailureKind`, and `TargetKind`.
- Produces: `AgentAdapter`, `TargetAvailability`, `ExecutionRequest`, `AdapterCompletion`, `AdapterError`, and `CodexAdapter`.

- [ ] **Step 1: Write the failing Codex command and JSONL tests**

Create `crates/ability-adapters/tests/codex_adapter.rs`:

```rust
use ability_adapters::{
    AdapterCompletion, AgentAdapter, AuthState, CodexAdapter, ExecutionRequest,
    ProcessError, ProcessOutput, ProcessRunner, ProcessSpec,
};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct FakeRunner {
    seen: Arc<Mutex<Vec<ProcessSpec>>>,
}

#[async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.seen.lock().unwrap().push(spec);
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"abc\"}\n",
                "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":4}}\n"
            )
            .into(),
            stderr: String::new(),
            duration_ms: 250,
        })
    }
}

#[tokio::test]
async fn codex_uses_ephemeral_json_workspace_write() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter = CodexAdapter::new(Arc::new(FakeRunner { seen: seen.clone() }));
    let result = adapter
        .execute(
            ExecutionRequest {
                prompt: "Fix the repository and run its visible tests.".into(),
                workspace: PathBuf::from("C:/temp/task"),
                time_budget_secs: 600,
                max_turns: 20,
                model: Some("gpt-test".into()),
                reasoning_effort: Some("high".into()),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(matches!(result, AdapterCompletion::Completed { .. }));
    let specs = seen.lock().unwrap();
    assert_eq!(specs[0].program, "codex");
    assert_eq!(
        specs[0].args,
        vec![
            "exec",
            "--ephemeral",
            "--json",
            "--sandbox",
            "workspace-write",
            "--ignore-user-config",
            "--ignore-rules",
            "--model",
            "gpt-test",
            "--config",
            "model_reasoning_effort=\"high\"",
            "Fix the repository and run its visible tests."
        ]
    );
}

struct ReadyDetectionRunner;

#[async_trait]
impl ProcessRunner for ReadyDetectionRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        let stdout = if spec.args.as_slice() == ["--version"] {
            "codex-cli 0.134.0"
        } else if spec.args.as_slice() == ["login", "status"] {
            "Logged in using ChatGPT"
        } else {
            panic!("unexpected detection command: {:?}", spec.args);
        };
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            duration_ms: 10,
        })
    }
}

#[tokio::test]
async fn codex_detection_uses_the_cli_status_without_reading_auth_files() {
    let availability = CodexAdapter::new(Arc::new(ReadyDetectionRunner))
        .detect()
        .await;
    assert!(availability.installed);
    assert_eq!(availability.version.as_deref(), Some("codex-cli 0.134.0"));
    assert_eq!(availability.auth_state, AuthState::Ready);
}
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
cargo test -p ability-adapters --test codex_adapter
```

Expected: FAIL with unresolved adapter contracts.

- [ ] **Step 3: Implement shared adapter contracts and error classification**

Create `crates/ability-adapters/src/classify.rs`:

```rust
use ability_core::FailureKind;

pub fn classify_cli_failure(text: &str) -> FailureKind {
    let lower = text.to_lowercase();
    if lower.contains("not logged in")
        || lower.contains("authentication")
        || lower.contains("unauthorized")
    {
        FailureKind::AuthExpired
    } else if lower.contains("quota")
        || lower.contains("usage limit")
        || lower.contains("rate limit")
    {
        FailureKind::QuotaExhausted
    } else if lower.contains("network")
        || lower.contains("connection")
        || lower.contains("dns")
    {
        FailureKind::Network
    } else {
        FailureKind::AppInterrupted
    }
}

pub fn is_agent_budget_exhaustion(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "max_turns",
        "max turns",
        "maximum number of turns",
        "turn limit reached",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}
```

Add these contracts to `crates/ability-adapters/src/lib.rs`:

```rust
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetAvailability {
    pub kind: TargetKind,
    pub installed: bool,
    pub version: Option<String>,
    pub auth_state: AuthState,
    pub prerequisites: Vec<PrerequisiteStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Unknown,
    Ready,
    NeedsLogin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrerequisiteStatus {
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub prompt: String,
    pub workspace: PathBuf,
    pub time_budget_secs: u64,
    pub max_turns: u32,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterCompletion {
    Completed {
        duration_ms: u64,
        stdout: String,
        stderr: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("target is unavailable")]
    Unavailable,
    #[error("task failed before verification: {kind:?}: {detail}")]
    Infrastructure {
        kind: FailureKind,
        detail: String,
    },
    #[error("agent budget was exhausted")]
    AgentBudgetExceeded,
    #[error("user cancelled the task")]
    Cancelled,
}

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn kind(&self) -> TargetKind;
    async fn detect(&self) -> TargetAvailability;
    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError>;
}
```

- [ ] **Step 4: Implement Codex detection and execution**

Create `crates/ability-adapters/src/codex.rs`:

```rust
use crate::{
    classify_cli_failure, AdapterCompletion, AdapterError, AgentAdapter, AuthState,
    ExecutionRequest, ProcessError, ProcessRunner, ProcessSpec, TargetAvailability,
};
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct CodexAdapter {
    runner: Arc<dyn ProcessRunner>,
}

impl CodexAdapter {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    fn kind(&self) -> TargetKind {
        TargetKind::CodexCli
    }

    async fn detect(&self) -> TargetAvailability {
        let version_spec = ProcessSpec {
            program: "codex".into(),
            args: vec!["--version".into()],
            current_dir: std::env::temp_dir(),
            env: BTreeMap::new(),
            timeout: Duration::from_secs(10),
        };
        let version = match self
            .runner
            .run(version_spec, CancellationToken::new())
            .await
        {
            Ok(output) if output.exit_code == Some(0) => {
                Some(output.stdout.trim().to_owned())
            }
            _ => {
                return TargetAvailability {
                    kind: self.kind(),
                    installed: false,
                    version: None,
                    auth_state: AuthState::Unknown,
                    prerequisites: Vec::new(),
                }
            }
        };
        let status_spec = ProcessSpec {
            program: "codex".into(),
            args: vec!["login".into(), "status".into()],
            current_dir: std::env::temp_dir(),
            env: BTreeMap::new(),
            timeout: Duration::from_secs(10),
        };
        let auth_state = match self
            .runner
            .run(status_spec, CancellationToken::new())
            .await
        {
            Ok(output) if output.stdout.to_lowercase().contains("not logged in") => {
                AuthState::NeedsLogin
            }
            Ok(output)
                if output.exit_code == Some(0)
                    && output.stdout.to_lowercase().contains("logged in") =>
            {
                AuthState::Ready
            }
            _ => AuthState::Unknown,
        };
        TargetAvailability {
                kind: self.kind(),
                installed: true,
                version,
                auth_state,
                prerequisites: Vec::new(),
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        let mut args = vec![
            "exec".into(),
            "--ephemeral".into(),
            "--json".into(),
            "--sandbox".into(),
            "workspace-write".into(),
            "--ignore-user-config".into(),
            "--ignore-rules".into(),
        ];
        if let Some(model) = request.model {
            args.extend(["--model".into(), model]);
        }
        if let Some(effort) = request.reasoning_effort {
            args.extend([
                "--config".into(),
                format!("model_reasoning_effort=\"{effort}\""),
            ]);
        }
        args.push(request.prompt);
        let spec = ProcessSpec {
            program: "codex".into(),
            args,
            current_dir: request.workspace,
            env: BTreeMap::new(),
            timeout: Duration::from_secs(request.time_budget_secs),
        };
        match self.runner.run(spec, cancellation).await {
            Ok(output) if output.exit_code == Some(0) && has_completed_turn(&output.stdout) => {
                Ok(AdapterCompletion::Completed {
                    duration_ms: output.duration_ms,
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
            Ok(output) => {
                let detail = format!("{}\n{}", output.stderr, output.stdout);
                Err(AdapterError::Infrastructure {
                    kind: classify_cli_failure(&detail),
                    detail,
                })
            }
            Err(ProcessError::TimedOut) => Err(AdapterError::AgentBudgetExceeded),
            Err(ProcessError::Cancelled) => Err(AdapterError::Cancelled),
            Err(ProcessError::Spawn(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(AdapterError::Unavailable)
            }
            Err(ProcessError::Spawn(error)) => Err(AdapterError::Infrastructure {
                kind: FailureKind::AppInterrupted,
                detail: error.to_string(),
            }),
        }
    }
}

fn has_completed_turn(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| value["type"].as_str().map(str::to_owned))
            .as_deref()
            == Some("turn.completed")
    })
}
```

Update the module declarations and exports in `crates/ability-adapters/src/lib.rs`:

```rust
mod classify;
mod codex;
mod process;

pub use classify::*;
pub use codex::*;
pub use process::*;
```

Keep the shared adapter contracts in the same `lib.rs` below the exports.

- [ ] **Step 5: Run Codex adapter and full adapter tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-adapters
```

Expected: the Codex command contract test and process tests pass.

- [ ] **Step 6: Commit**

```powershell
git add crates/ability-adapters
git commit -m "feat: automate Codex CLI tasks"
```

---

### Task 9: Build the Claude Code Adapter

**Files:**
- Create: `crates/ability-adapters/src/claude.rs`
- Create: `crates/ability-adapters/tests/claude_adapter.rs`
- Modify: `crates/ability-adapters/src/lib.rs`

**Interfaces:**
- Consumes: shared adapter and process contracts from Tasks 7–8.
- Produces: `ClaudeCodeAdapter`.

- [ ] **Step 1: Write the failing Claude command test**

Create `crates/ability-adapters/tests/claude_adapter.rs`:

```rust
use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState,
    ClaudeCodeAdapter, ExecutionRequest, ProcessError, ProcessOutput,
    ProcessRunner, ProcessSpec,
};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

struct FakeRunner {
    seen: Arc<Mutex<Vec<ProcessSpec>>>,
}

#[async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.seen.lock().unwrap().push(spec);
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: "{\"type\":\"result\",\"subtype\":\"success\"}\n".into(),
            stderr: String::new(),
            duration_ms: 300,
        })
    }
}

#[tokio::test]
async fn claude_uses_print_json_and_never_skips_permissions() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter =
        ClaudeCodeAdapter::new(Arc::new(FakeRunner { seen: seen.clone() }));
    let result = adapter
        .execute(
            ExecutionRequest {
                prompt: "Fix the repository.".into(),
                workspace: PathBuf::from("C:/temp/task"),
                time_budget_secs: 600,
                max_turns: 20,
                model: Some("sonnet".into()),
                reasoning_effort: Some("high".into()),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(matches!(result, AdapterCompletion::Completed { .. }));
    let specs = seen.lock().unwrap();
    assert!(specs[0].args.contains(&"--output-format".into()));
    assert!(specs[0].args.contains(&"stream-json".into()));
    assert!(specs[0].args.contains(&"--max-turns".into()));
    assert!(specs[0].args.windows(2).any(|window| {
        window == ["--model", "sonnet"]
    }));
    assert!(specs[0].args.windows(2).any(|window| {
        window == ["--effort", "high"]
    }));
    assert!(specs[0].args.contains(&"--bare".into()));
    assert!(specs[0].args.contains(&"--no-session-persistence".into()));
    assert!(specs[0].args.contains(&"--tools".into()));
    assert!(specs[0].args.contains(&"Read,Edit,Write".into()));
    assert!(specs[0].args.windows(4).any(|window| {
        window == ["--allowedTools", "Read", "Edit", "Write"]
    }));
    assert!(specs[0].args.contains(&"--permission-mode".into()));
    assert!(specs[0].args.contains(&"dontAsk".into()));
    assert!(!specs[0]
        .args
        .contains(&"--dangerously-skip-permissions".into()));
}

struct MaxTurnsRunner;

#[async_trait]
impl ProcessRunner for MaxTurnsRunner {
    async fn run(
        &self,
        _spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        Ok(ProcessOutput {
            exit_code: Some(1),
            stdout: r#"{"type":"result","subtype":"error_max_turns"}"#.into(),
            stderr: "Maximum number of turns reached".into(),
            duration_ms: 600,
        })
    }
}

#[tokio::test]
async fn claude_max_turns_is_a_scored_agent_budget_failure() {
    let adapter = ClaudeCodeAdapter::new(Arc::new(MaxTurnsRunner));
    let result = adapter
        .execute(
            ExecutionRequest {
                prompt: "Fix it.".into(),
                workspace: PathBuf::from("C:/temp/task"),
                time_budget_secs: 600,
                max_turns: 2,
                model: None,
                reasoning_effort: None,
            },
            CancellationToken::new(),
        )
        .await;
    assert!(matches!(result, Err(AdapterError::AgentBudgetExceeded)));
}

struct ReadyClaudeDetectionRunner;

#[async_trait]
impl ProcessRunner for ReadyClaudeDetectionRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        let (exit_code, stdout) = if spec.args.as_slice() == ["--version"] {
            (Some(0), "2.1.211")
        } else if spec.args.as_slice() == ["auth", "status"] {
            (Some(0), r#"{"loggedIn":true}"#)
        } else {
            panic!("unexpected detection command: {:?}", spec.args);
        };
        Ok(ProcessOutput {
            exit_code,
            stdout: stdout.into(),
            stderr: String::new(),
            duration_ms: 10,
        })
    }
}

#[tokio::test]
async fn claude_detection_uses_auth_status_without_persisting_its_json() {
    let availability =
        ClaudeCodeAdapter::new(Arc::new(ReadyClaudeDetectionRunner))
            .detect()
            .await;
    assert!(availability.installed);
    assert_eq!(availability.version.as_deref(), Some("2.1.211"));
    assert_eq!(availability.auth_state, AuthState::Ready);
}
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
cargo test -p ability-adapters --test claude_adapter
```

Expected: FAIL with unresolved `ClaudeCodeAdapter`.

- [ ] **Step 3: Implement Claude detection and constrained execution**

Create `crates/ability-adapters/src/claude.rs`:

```rust
use crate::{
    classify_cli_failure, is_agent_budget_exhaustion, AdapterCompletion,
    AdapterError, AgentAdapter, AuthState, ExecutionRequest, ProcessError,
    ProcessRunner, ProcessSpec, TargetAvailability,
};
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct ClaudeCodeAdapter {
    runner: Arc<dyn ProcessRunner>,
}

impl ClaudeCodeAdapter {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }
}

#[async_trait]
impl AgentAdapter for ClaudeCodeAdapter {
    fn kind(&self) -> TargetKind {
        TargetKind::ClaudeCode
    }

    async fn detect(&self) -> TargetAvailability {
        let version_spec = ProcessSpec {
            program: "claude".into(),
            args: vec!["--version".into()],
            current_dir: std::env::temp_dir(),
            env: BTreeMap::new(),
            timeout: Duration::from_secs(10),
        };
        let version = match self
            .runner
            .run(version_spec, CancellationToken::new())
            .await
        {
            Ok(output) if output.exit_code == Some(0) => {
                Some(output.stdout.trim().to_owned())
            }
            _ => {
                return TargetAvailability {
                    kind: self.kind(),
                    installed: false,
                    version: None,
                    auth_state: AuthState::Unknown,
                    prerequisites: Vec::new(),
                }
            }
        };
        let status_spec = ProcessSpec {
            program: "claude".into(),
            args: vec!["auth".into(), "status".into()],
            current_dir: std::env::temp_dir(),
            env: BTreeMap::new(),
            timeout: Duration::from_secs(10),
        };
        let auth_state = match self
            .runner
            .run(status_spec, CancellationToken::new())
            .await
        {
            Ok(output) if output.exit_code == Some(0) => AuthState::Ready,
            Ok(output) if output.exit_code == Some(1) => AuthState::NeedsLogin,
            _ => AuthState::Unknown,
        };
        TargetAvailability {
                kind: self.kind(),
                installed: true,
                version,
                auth_state,
                prerequisites: Vec::new(),
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        let mut args = vec![
            "-p".into(),
            request.prompt,
            "--bare".into(),
            "--no-session-persistence".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--max-turns".into(),
            request.max_turns.to_string(),
            "--tools".into(),
            "Read,Edit,Write".into(),
            "--allowedTools".into(),
            "Read".into(),
            "Edit".into(),
            "Write".into(),
            "--permission-mode".into(),
            "dontAsk".into(),
        ];
        if let Some(model) = request.model {
            args.extend(["--model".into(), model]);
        }
        if let Some(effort) = request.reasoning_effort {
            args.extend(["--effort".into(), effort]);
        }
        let spec = ProcessSpec {
            program: "claude".into(),
            args,
            current_dir: request.workspace,
            env: BTreeMap::new(),
            timeout: Duration::from_secs(request.time_budget_secs),
        };
        match self.runner.run(spec, cancellation).await {
            Ok(output)
                if output.exit_code == Some(0)
                    && output.stdout.contains("\"subtype\":\"success\"") =>
            {
                Ok(AdapterCompletion::Completed {
                    duration_ms: output.duration_ms,
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
            Ok(output) => {
                let detail = format!("{}\n{}", output.stderr, output.stdout);
                if is_agent_budget_exhaustion(&detail) {
                    Err(AdapterError::AgentBudgetExceeded)
                } else {
                    Err(AdapterError::Infrastructure {
                        kind: classify_cli_failure(&detail),
                        detail,
                    })
                }
            }
            Err(ProcessError::TimedOut) => Err(AdapterError::AgentBudgetExceeded),
            Err(ProcessError::Cancelled) => Err(AdapterError::Cancelled),
            Err(ProcessError::Spawn(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(AdapterError::Unavailable)
            }
            Err(ProcessError::Spawn(error)) => Err(AdapterError::Infrastructure {
                kind: FailureKind::AppInterrupted,
                detail: error.to_string(),
            }),
        }
    }
}
```

Update `crates/ability-adapters/src/lib.rs`:

```rust
mod claude;
pub use claude::*;
```

- [ ] **Step 4: Run adapter tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-adapters
```

Expected: Claude, Codex, and process tests all pass.

- [ ] **Step 5: Commit**

```powershell
git add crates/ability-adapters
git commit -m "feat: automate Claude Code tasks"
```

---

### Task 10: Add the Eight Objective Client Tasks

**Files:**
- Create: `benchmark-packs/registry.json`
- Create: `benchmark-packs/client-quick-v1/manifest.json`
- Create: `benchmark-packs/client-quick-v1/prompts/instruction-filter.json.txt`
- Create: `benchmark-packs/client-quick-v1/prompts/instruction-csv.txt`
- Create: `benchmark-packs/client-quick-v1/prompts/instruction-inventory.json.txt`
- Create: `benchmark-packs/client-quick-v1/prompts/logic-schedule.txt`
- Create: `benchmark-packs/client-quick-v1/prompts/logic-truth.txt`
- Create: `benchmark-packs/client-quick-v1/prompts/logic-capacity.txt`
- Create: `benchmark-packs/client-quick-v1/prompts/review-python.txt`
- Create: `benchmark-packs/client-quick-v1/prompts/review-typescript.txt`
- Create: `crates/ability-core/tests/official_client_pack.rs`

**Interfaces:**
- Consumes: the pack schema and deterministic graders.
- Produces: the built-in `client-quick` pack version `1.0.0`.

- [ ] **Step 1: Write the failing official-pack test**

Create `crates/ability-core/tests/official_client_pack.rs`:

```rust
use ability_core::{grade_submission, Category, PackLoader};
use std::path::PathBuf;

#[test]
fn client_quick_pack_has_the_approved_shape_and_gold_answers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmark-packs/client-quick-v1");
    let pack = PackLoader::load(&root).unwrap();
    assert_eq!(pack.tasks.len(), 8);
    assert_eq!(
        pack.tasks
            .iter()
            .filter(|task| task.definition.category == Category::InstructionFollowing)
            .count(),
        3
    );
    assert_eq!(
        pack.tasks
            .iter()
            .filter(|task| task.definition.category == Category::Logic)
            .count(),
        3
    );
    assert_eq!(
        pack.tasks
            .iter()
            .filter(|task| task.definition.category == Category::CodeReview)
            .count(),
        2
    );

    let gold = [
        r#"{"count":3,"names":["Mira","An","Bo"]}"#,
        "sku,total\nB2,42\nC3,35",
        r#"[{"sku":"C","net":90},{"sku":"A","net":72}]"#,
        r#"{"09:00":"D","10:00":"B","11:00":"A","12:00":"C"}"#,
        r#"{"liar":"B","box":3}"#,
        r#"{"trips":4,"unused":6}"#,
        r#"["A","D"]"#,
        r#"["A","C"]"#,
    ];
    for (task, answer) in pack.tasks.iter().zip(gold) {
        assert!(
            grade_submission(&task.definition.grader, answer).passed,
            "{}",
            task.definition.id
        );
    }
}
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
cargo test -p ability-core --test official_client_pack
```

Expected: FAIL because the built-in pack does not exist.

- [ ] **Step 3: Create the eight prompts exactly**

Create `benchmark-packs/client-quick-v1/prompts/instruction-filter.json.txt`:

```text
严格遵守以下要求：只输出一个 JSON 对象，不得使用 Markdown，不得添加解释。保留 active=true 且 score>=70 的人员，按 score 从高到低排序；分数相同时按 name 的 Unicode 升序排序。对象只能有 count 和 names 两个键。

数据：
[{"name":"Bo","score":88,"active":true},{"name":"Li","score":99,"active":false},{"name":"An","score":88,"active":true},{"name":"Mira","score":91,"active":true},{"name":"Xu","score":69,"active":true}]
```

Create `benchmark-packs/client-quick-v1/prompts/instruction-csv.txt`:

```text
只输出 CSV，不得使用代码块或解释。第一行必须是 sku,total。合并相同 sku 的数量，只保留 total>=30 的项目，按 total 从高到低排序；若相同则按 sku 升序。

记录：
B2 12
C3 35
B2 30
A1 29
D4 4
```

Create `benchmark-packs/client-quick-v1/prompts/instruction-inventory.json.txt`:

```text
只输出 JSON 数组，不得添加其他文字。计算 net=price*(100-discount)/100，只保留 stock>0 且 net>=70 的商品。每项只能包含 sku 和 net；按 net 从高到低排序。

商品：
{"sku":"A","price":80,"discount":10,"stock":3}
{"sku":"B","price":120,"discount":50,"stock":9}
{"sku":"C","price":100,"discount":10,"stock":2}
{"sku":"D","price":90,"discount":0,"stock":0}
```

Create `benchmark-packs/client-quick-v1/prompts/logic-schedule.txt`:

```text
四人 A、B、C、D 分别占用 09:00、10:00、11:00、12:00，每人一个时段。
条件：
1. A 晚于 B。
2. C 是最后一个。
3. D 不在 10:00 或 11:00。
4. B 不在 09:00。
只输出一个 JSON 对象，以时段为键、人员为值，不得解释。
```

Create `benchmark-packs/client-quick-v1/prompts/logic-truth.txt`:

```text
A、B、C 中恰有一人说谎。三个盒子中恰有一个装有钥匙。
A 说：“钥匙不在 1 号盒。”
B 说：“钥匙在 2 号盒。”
C 说：“A 说的是真话。”
只输出 {"liar":"姓名","box":盒号}，不得解释。
```

Create `benchmark-packs/client-quick-v1/prompts/logic-capacity.txt`:

```text
一辆车每次最多运 18 箱。仓库有 66 箱，必须全部运走，且每次只能装整数箱。问最少需要几次，最后一次之后所有车次合计还有多少未使用容量？只输出 {"trips":整数,"unused":整数}，不得解释。
```

Create `benchmark-packs/client-quick-v1/prompts/review-python.txt`:

```text
下面 Python 代码中只有两个标记行存在独立缺陷。只输出缺陷标签组成的 JSON 数组，不得解释。

def append_item(item, bucket=[]):             # [A]
    bucket.append(item)                       # [B]
    return bucket[:]                          # [C]

def load_user(name):
    query = f"SELECT * FROM users WHERE name='{name}'"  # [D]
    return db.execute(query).fetchone()        # [E]

候选缺陷定义：
- 可变默认参数导致不同调用共享状态。
- SQL 字符串拼接导致注入风险。
- 返回浅拷贝本身不构成这里要求识别的独立缺陷。
```

Create `benchmark-packs/client-quick-v1/prompts/review-typescript.txt`:

```text
下面 TypeScript 代码中只有两个标记行存在独立缺陷。只输出缺陷标签组成的 JSON 数组，不得解释。

async function load(ids: string[]) {
  const rows = await ids.map(async id =>
    (await fetch(`/api/${id}`)).json());                  // [A]
  return rows;                                            // [B]
}

function isEnabled(value: string | undefined) {
  return Boolean(value);                                  // [C]
}

候选缺陷定义：
- 返回的是 Promise 数组而不是已经解析的数据。
- 字符串 "false" 会被 Boolean 转成 true。
- 使用模板字符串构造该固定路径不单独计为缺陷。
```

- [ ] **Step 4: Create the manifest and registry**

Create `benchmark-packs/client-quick-v1/manifest.json`:

```json
{
  "schema_version": 1,
  "id": "client-quick",
  "version": "1.0.0",
  "title": "客户端快速体检",
  "target_kinds": ["chat_gpt_client", "claude_client"],
  "tasks": [
    {
      "id": "instruction-filter",
      "category": "instruction_following",
      "prompt_file": "prompts/instruction-filter.json.txt",
      "starter_dir": null,
      "time_budget_secs": 120,
      "max_turns": 1,
      "grader": {
        "type": "exact_json",
        "expected": {"count": 3, "names": ["Mira", "An", "Bo"]}
      }
    },
    {
      "id": "instruction-csv",
      "category": "instruction_following",
      "prompt_file": "prompts/instruction-csv.txt",
      "starter_dir": null,
      "time_budget_secs": 120,
      "max_turns": 1,
      "grader": {"type": "exact_text", "expected": "sku,total\nB2,42\nC3,35"}
    },
    {
      "id": "instruction-inventory",
      "category": "instruction_following",
      "prompt_file": "prompts/instruction-inventory.json.txt",
      "starter_dir": null,
      "time_budget_secs": 120,
      "max_turns": 1,
      "grader": {
        "type": "exact_json",
        "expected": [{"sku": "C", "net": 90}, {"sku": "A", "net": 72}]
      }
    },
    {
      "id": "logic-schedule",
      "category": "logic",
      "prompt_file": "prompts/logic-schedule.txt",
      "starter_dir": null,
      "time_budget_secs": 120,
      "max_turns": 1,
      "grader": {
        "type": "exact_json",
        "expected": {"09:00": "D", "10:00": "B", "11:00": "A", "12:00": "C"}
      }
    },
    {
      "id": "logic-truth",
      "category": "logic",
      "prompt_file": "prompts/logic-truth.txt",
      "starter_dir": null,
      "time_budget_secs": 120,
      "max_turns": 1,
      "grader": {"type": "exact_json", "expected": {"liar": "B", "box": 3}}
    },
    {
      "id": "logic-capacity",
      "category": "logic",
      "prompt_file": "prompts/logic-capacity.txt",
      "starter_dir": null,
      "time_budget_secs": 120,
      "max_turns": 1,
      "grader": {"type": "exact_json", "expected": {"trips": 4, "unused": 6}}
    },
    {
      "id": "review-python",
      "category": "code_review",
      "prompt_file": "prompts/review-python.txt",
      "starter_dir": null,
      "time_budget_secs": 180,
      "max_turns": 1,
      "grader": {"type": "json_string_set", "expected": ["A", "D"]}
    },
    {
      "id": "review-typescript",
      "category": "code_review",
      "prompt_file": "prompts/review-typescript.txt",
      "starter_dir": null,
      "time_budget_secs": 180,
      "max_turns": 1,
      "grader": {"type": "json_string_set", "expected": ["A", "C"]}
    }
  ]
}
```

Create `benchmark-packs/registry.json`:

```json
{
  "schema_version": 1,
  "packs": [
    {
      "id": "client-quick",
      "version": "1.0.0",
      "path": "client-quick-v1",
      "license": "Apache-2.0",
      "bundled": true
    },
    {
      "id": "cli-quick",
      "version": "1.0.0",
      "path": "cli-quick-v1",
      "license": "Apache-2.0",
      "bundled": true
    }
  ]
}
```

- [ ] **Step 5: Run the official-pack test**

Run:

```powershell
cargo test -p ability-core --test official_client_pack
```

Expected: PASS with exactly eight valid gold answers.

- [ ] **Step 6: Commit**

```powershell
git add benchmark-packs crates/ability-core/tests/official_client_pack.rs
git commit -m "feat: add objective client quick pack"
```

---

### Task 11: Add Two CLI Micro-Repositories and Hidden Verifiers

**Files:**
- Create: `benchmark-packs/cli-quick-v1/manifest.json`
- Create: `benchmark-packs/cli-quick-v1/tasks/dedupe-events/prompt.md`
- Create: `benchmark-packs/cli-quick-v1/tasks/dedupe-events/starter/src/dedupeEvents.mjs`
- Create: `benchmark-packs/cli-quick-v1/tasks/dedupe-events/verify.mjs`
- Create: `benchmark-packs/cli-quick-v1/tasks/retry-schedule/prompt.md`
- Create: `benchmark-packs/cli-quick-v1/tasks/retry-schedule/starter/src/retrySchedule.mjs`
- Create: `benchmark-packs/cli-quick-v1/tasks/retry-schedule/verify.mjs`
- Create: `crates/ability-adapters/src/verifier.rs`
- Create: `crates/ability-adapters/tests/verifier.rs`
- Create: `crates/ability-core/examples/pack_hashes.rs`
- Create: `crates/ability-core/tests/bundled_registry.rs`
- Modify: `crates/ability-adapters/src/lib.rs`

**Interfaces:**
- Consumes: `ProcessRunner`, `ProcessSpec`, `FailureKind`, and loaded external-verifier IDs.
- Produces: the `cli-quick` pack and a runtime-resource-backed
  `NodeVerifier::verify(verifier_id, workspace, cancellation)`.

- [ ] **Step 1: Write the failing verifier contract test**

Create `crates/ability-adapters/tests/verifier.rs`:

```rust
use ability_adapters::{NodeVerifier, ProcessError, ProcessOutput, ProcessRunner, ProcessSpec};
use ability_core::{FailureKind, TaskOutcome};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct PassingRunner;

#[async_trait]
impl ProcessRunner for PassingRunner {
    async fn run(
        &self,
        _spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: "TASK_PASSED".into(),
            stderr: String::new(),
            duration_ms: 10,
        })
    }
}

#[tokio::test]
async fn a_zero_exit_hidden_verifier_passes() {
    let verifier = NodeVerifier::new(
        Arc::new(PassingRunner),
        PathBuf::from("C:/bundled/benchmark-packs/cli-quick-v1"),
    );
    let grade = verifier
        .verify(
            "dedupe-events-v1",
            std::path::Path::new("C:/temp/workspace"),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(grade.outcome, TaskOutcome::Passed);
    assert_eq!(grade.score, Some(100.0));
    assert_eq!(grade.failure_kind, None);
}

#[tokio::test]
async fn an_unknown_verifier_is_not_executed() {
    let verifier = NodeVerifier::new(
        Arc::new(PassingRunner),
        PathBuf::from("C:/bundled/benchmark-packs/cli-quick-v1"),
    );
    let grade = verifier
        .verify(
            "untrusted-command",
            std::path::Path::new("C:/temp/workspace"),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(grade.outcome, TaskOutcome::Invalid);
    assert_eq!(grade.failure_kind, Some(FailureKind::VerifierError));
}
```

- [ ] **Step 2: Run the verifier test to confirm failure**

Run:

```powershell
cargo test -p ability-adapters --test verifier
```

Expected: FAIL with unresolved `NodeVerifier`.

- [ ] **Step 3: Create the first micro-repository**

Create `benchmark-packs/cli-quick-v1/tasks/dedupe-events/prompt.md`:

```markdown
修复 `src/dedupeEvents.mjs` 中的 `dedupeEvents(events)`。

要求：
1. 忽略不是对象、缺少非空字符串 `id`、或 `occurredAt` 无法被 `Date.parse` 解析的条目。
2. 每个 `id` 只保留时间最新的事件；时间相同则保留输入中靠后的事件。
3. 结果按 `occurredAt` 升序排列；时间相同按 `id` 升序。
4. 不得修改输入数组或输入对象。
5. 保持导出函数签名不变。
```

Create `benchmark-packs/cli-quick-v1/tasks/dedupe-events/starter/src/dedupeEvents.mjs`:

```js
export function dedupeEvents(events) {
  const seen = new Set();
  return events.filter((event) => {
    if (seen.has(event.id)) {
      return false;
    }
    seen.add(event.id);
    return true;
  });
}
```

Create `benchmark-packs/cli-quick-v1/tasks/dedupe-events/verify.mjs`:

```js
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
import path from "node:path";

const workspace = process.argv[2];
if (!workspace) {
  console.error("VERIFIER_ERROR missing workspace");
  process.exit(2);
}

try {
  const moduleUrl = pathToFileURL(
    path.join(workspace, "src", "dedupeEvents.mjs"),
  );
  moduleUrl.searchParams.set("run", String(Date.now()));
  const { dedupeEvents } = await import(moduleUrl.href);
  const original = [
    { id: "b", occurredAt: "2026-01-03T00:00:00Z", payload: { n: 1 } },
    { id: "a", occurredAt: "2026-01-02T00:00:00Z", payload: { n: 2 } },
    { id: "b", occurredAt: "2026-01-04T00:00:00Z", payload: { n: 3 } },
    { id: "a", occurredAt: "2026-01-02T00:00:00Z", payload: { n: 4 } },
    { id: "", occurredAt: "2026-01-01T00:00:00Z" },
    { id: "x", occurredAt: "invalid" },
    null,
  ];
  const snapshot = structuredClone(original);
  const result = dedupeEvents(original);
  assert.deepEqual(
    result.map((item) => [item.id, item.payload.n]),
    [["a", 4], ["b", 3]],
  );
  assert.deepEqual(original, snapshot);
  console.log("TASK_PASSED");
} catch (error) {
  if (error instanceof assert.AssertionError) {
    console.error(`TASK_FAILED ${error.message}`);
    process.exit(1);
  }
  console.error(`VERIFIER_ERROR ${error.stack ?? error}`);
  process.exit(2);
}
```

- [ ] **Step 4: Create the second micro-repository**

Create `benchmark-packs/cli-quick-v1/tasks/retry-schedule/prompt.md`:

```markdown
修复 `src/retrySchedule.mjs` 中的 `buildRetrySchedule(options)`。

要求：
1. `maxAttempts` 包含第一次立即执行，因此结果第一项始终是 `0`。
2. 后续基础延迟为 `baseDelayMs * 2^(retryIndex-1)`，并限制在 `maxDelayMs`。
3. `retryAfterMs` 可为每次重试提供最小延迟；实际延迟取基础延迟和对应值的较大者。
4. 返回累计时间点，而不是单次延迟。
5. 所有输入必须是非负整数，且 `maxAttempts>=1`、`baseDelayMs>=1`、`maxDelayMs>=baseDelayMs`；无效时抛出 `TypeError`。
6. 不得修改 `retryAfterMs`。
```

Create `benchmark-packs/cli-quick-v1/tasks/retry-schedule/starter/src/retrySchedule.mjs`:

```js
export function buildRetrySchedule({
  maxAttempts,
  baseDelayMs,
  maxDelayMs,
  retryAfterMs = [],
}) {
  const result = [];
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    result.push(baseDelayMs * 2 ** attempt);
  }
  return result;
}
```

Create `benchmark-packs/cli-quick-v1/tasks/retry-schedule/verify.mjs`:

```js
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
import path from "node:path";

const workspace = process.argv[2];
if (!workspace) {
  console.error("VERIFIER_ERROR missing workspace");
  process.exit(2);
}

try {
  const moduleUrl = pathToFileURL(
    path.join(workspace, "src", "retrySchedule.mjs"),
  );
  moduleUrl.searchParams.set("run", String(Date.now()));
  const { buildRetrySchedule } = await import(moduleUrl.href);
  const retryAfterMs = [0, 2500, 0, 9000];
  const snapshot = [...retryAfterMs];
  assert.deepEqual(
    buildRetrySchedule({
      maxAttempts: 5,
      baseDelayMs: 1000,
      maxDelayMs: 5000,
      retryAfterMs,
    }),
    [0, 1000, 3500, 7500, 16500],
  );
  assert.deepEqual(retryAfterMs, snapshot);
  assert.deepEqual(
    buildRetrySchedule({
      maxAttempts: 1,
      baseDelayMs: 10,
      maxDelayMs: 10,
    }),
    [0],
  );
  for (const options of [
    { maxAttempts: 0, baseDelayMs: 1, maxDelayMs: 1 },
    { maxAttempts: 2, baseDelayMs: 0, maxDelayMs: 1 },
    { maxAttempts: 2, baseDelayMs: 2, maxDelayMs: 1 },
    { maxAttempts: 2.5, baseDelayMs: 1, maxDelayMs: 2 },
  ]) {
    assert.throws(() => buildRetrySchedule(options), TypeError);
  }
  console.log("TASK_PASSED");
} catch (error) {
  if (error instanceof assert.AssertionError) {
    console.error(`TASK_FAILED ${error.message}`);
    process.exit(1);
  }
  console.error(`VERIFIER_ERROR ${error.stack ?? error}`);
  process.exit(2);
}
```

- [ ] **Step 5: Create the CLI manifest**

Create `benchmark-packs/cli-quick-v1/manifest.json`:

```json
{
  "schema_version": 1,
  "id": "cli-quick",
  "version": "1.0.0",
  "title": "CLI 快速体检",
  "target_kinds": ["codex_cli", "claude_code"],
  "tasks": [
    {
      "id": "dedupe-events",
      "category": "cli_coding",
      "prompt_file": "tasks/dedupe-events/prompt.md",
      "starter_dir": "tasks/dedupe-events/starter",
      "time_budget_secs": 1800,
      "max_turns": 20,
      "grader": {
        "type": "external_verifier",
        "verifier_id": "dedupe-events-v1"
      }
    },
    {
      "id": "retry-schedule",
      "category": "cli_coding",
      "prompt_file": "tasks/retry-schedule/prompt.md",
      "starter_dir": "tasks/retry-schedule/starter",
      "time_budget_secs": 1800,
      "max_turns": 20,
      "grader": {
        "type": "external_verifier",
        "verifier_id": "retry-schedule-v1"
      }
    }
  ]
}
```

- [ ] **Step 6: Implement the allowlisted Node verifier**

Create `crates/ability-adapters/src/verifier.rs`:

```rust
use crate::{ProcessError, ProcessRunner, ProcessSpec};
use ability_core::{FailureKind, TaskOutcome};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationGrade {
    pub outcome: TaskOutcome,
    pub score: Option<f64>,
    pub failure_kind: Option<FailureKind>,
    pub detail: String,
    pub duration_ms: u64,
}

#[async_trait]
pub trait WorkspaceVerifier: Send + Sync {
    async fn verify(
        &self,
        verifier_id: &str,
        workspace: &Path,
        cancellation: CancellationToken,
    ) -> VerificationGrade;
}

pub struct NodeVerifier {
    runner: Arc<dyn ProcessRunner>,
    pack_root: PathBuf,
}

impl NodeVerifier {
    pub fn new(runner: Arc<dyn ProcessRunner>, pack_root: PathBuf) -> Self {
        Self { runner, pack_root }
    }

    pub async fn verify(
        &self,
        verifier_id: &str,
        workspace: &Path,
        cancellation: CancellationToken,
    ) -> VerificationGrade {
        let script = match verifier_script(&self.pack_root, verifier_id) {
            Some(path) => path,
            None => {
                return VerificationGrade {
                    outcome: TaskOutcome::Invalid,
                    score: None,
                    failure_kind: Some(FailureKind::VerifierError),
                    detail: format!("unknown_verifier:{verifier_id}"),
                    duration_ms: 0,
                }
            }
        };
        let spec = ProcessSpec {
            program: "node".into(),
            args: vec![
                script.to_string_lossy().into_owned(),
                workspace.to_string_lossy().into_owned(),
            ],
            current_dir: workspace.to_path_buf(),
            env: BTreeMap::new(),
            timeout: Duration::from_secs(120),
        };
        match self.runner.run(spec, cancellation).await {
            Ok(output) if output.exit_code == Some(0) && output.stdout.contains("TASK_PASSED") => {
                VerificationGrade {
                    outcome: TaskOutcome::Passed,
                    score: Some(100.0),
                    failure_kind: None,
                    detail: "hidden_tests:pass".into(),
                    duration_ms: output.duration_ms,
                }
            }
            Ok(output) if output.stderr.contains("TASK_FAILED") => VerificationGrade {
                outcome: TaskOutcome::Failed,
                score: Some(0.0),
                failure_kind: Some(FailureKind::WrongAnswer),
                detail: output.stderr,
                duration_ms: output.duration_ms,
            },
            Ok(output) => VerificationGrade {
                outcome: TaskOutcome::Invalid,
                score: None,
                failure_kind: Some(FailureKind::VerifierError),
                detail: format!("{}\n{}", output.stdout, output.stderr),
                duration_ms: output.duration_ms,
            },
            Err(ProcessError::Spawn(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                VerificationGrade {
                    outcome: TaskOutcome::Invalid,
                    score: None,
                    failure_kind: Some(FailureKind::RuntimeMissing),
                    detail: "node_runtime_missing".into(),
                    duration_ms: 0,
                }
            }
            Err(ProcessError::TimedOut) => VerificationGrade {
                outcome: TaskOutcome::Invalid,
                score: None,
                failure_kind: Some(FailureKind::VerifierError),
                detail: "verifier_timeout".into(),
                duration_ms: 120_000,
            },
            Err(ProcessError::Cancelled) => VerificationGrade {
                outcome: TaskOutcome::Cancelled,
                score: None,
                failure_kind: Some(FailureKind::UserCancelled),
                detail: "verifier_cancelled".into(),
                duration_ms: 0,
            },
            Err(ProcessError::Spawn(error)) => VerificationGrade {
                outcome: TaskOutcome::Invalid,
                score: None,
                failure_kind: Some(FailureKind::VerifierError),
                detail: error.to_string(),
                duration_ms: 0,
            },
        }
    }
}

#[async_trait]
impl WorkspaceVerifier for NodeVerifier {
    async fn verify(
        &self,
        verifier_id: &str,
        workspace: &Path,
        cancellation: CancellationToken,
    ) -> VerificationGrade {
        NodeVerifier::verify(self, verifier_id, workspace, cancellation).await
    }
}

fn verifier_script(pack_root: &Path, verifier_id: &str) -> Option<PathBuf> {
    let root = pack_root.join("tasks");
    match verifier_id {
        "dedupe-events-v1" => Some(root.join("dedupe-events/verify.mjs")),
        "retry-schedule-v1" => Some(root.join("retry-schedule/verify.mjs")),
        _ => None,
    }
}
```

Update `crates/ability-adapters/src/lib.rs`:

```rust
mod verifier;
pub use verifier::*;
```

- [ ] **Step 7: Seal both bundled packs into the embedded registry**

Create `crates/ability-core/examples/pack_hashes.rs`:

```rust
use ability_core::PackLoader;
use serde_json::Value;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmark-packs");
    let registry_path = root.join("registry.json");
    let mut registry: Value = serde_json::from_str(
        &std::fs::read_to_string(&registry_path).unwrap(),
    )
    .unwrap();
    for directory in ["client-quick-v1", "cli-quick-v1"] {
        let pack = PackLoader::load(&root.join(directory)).unwrap();
        let entry = registry["packs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["id"] == pack.manifest.id)
            .unwrap();
        entry["content_sha256"] =
            Value::String(pack.content_sha256.clone());
        println!(
            "{} {} {}",
            pack.manifest.id, pack.manifest.version, pack.content_sha256
        );
    }
    std::fs::write(
        registry_path,
        format!("{}\n", serde_json::to_string_pretty(&registry).unwrap()),
    )
    .unwrap();
}
```

Run:

```powershell
cargo run -p ability-core --example pack_hashes
```

Expected: two lines ending in distinct 64-character lowercase SHA-256 values,
and `registry.json` is mechanically rewritten with those exact values. This
generator is the only permitted mechanical write to the registry; review its
diff before committing.

Create `crates/ability-core/tests/bundled_registry.rs`:

```rust
use ability_core::{PackLoader, PackRegistry};
use std::path::PathBuf;

#[test]
fn every_bundled_pack_matches_the_committed_registry_hash() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmark-packs");
    let registry = PackRegistry::parse(
        &std::fs::read_to_string(root.join("registry.json")).unwrap(),
    )
    .unwrap();
    for directory in ["client-quick-v1", "cli-quick-v1"] {
        let pack = PackLoader::load(&root.join(directory)).unwrap();
        registry.verify_bundled(&pack).unwrap();
    }
}
```

- [ ] **Step 8: Run verifier and sealed-pack tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-adapters --test verifier
cargo test -p ability-core --test official_client_pack
cargo test -p ability-core --test bundled_registry
```

Expected: verifier contract tests and the client pack regression test pass.

- [ ] **Step 9: Commit**

```powershell
git add benchmark-packs crates/ability-adapters crates/ability-core
git commit -m "feat: add CLI quick pack and verifiers"
```

---

### Task 12: Orchestrate Automatic CLI Runs with Per-Task Workspaces

**Files:**
- Create: `crates/ability-adapters/src/cli_run.rs`
- Create: `crates/ability-adapters/tests/cli_run.rs`
- Modify: `crates/ability-adapters/src/lib.rs`
- Modify: `crates/ability-core/src/storage.rs`

**Interfaces:**
- Consumes: `AgentAdapter`, `WorkspaceVerifier`, `LoadedPack`, `RunRepository`,
  `summarize_scores`, and `CancellationToken`.
- Produces: `CliRunService::prepare`, `CliRunService::execute`, `RunEvent`, and a
  safe per-task workspace copier.

- [ ] **Step 1: Write the failing orchestration test**

Create `crates/ability-adapters/tests/cli_run.rs`:

```rust
use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, CliRunService,
    ExecutionRequest, PrerequisiteStatus, RunEventKind, TargetAvailability,
    VerificationGrade, WorkspaceVerifier,
};
use ability_core::{
    EnvironmentFingerprint, FailureKind, PackLoader, RunMode, RunRepository,
    RunStatus, TargetKind, TargetSelection, TaskOutcome,
};
use async_trait::async_trait;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct FakeAdapter;

#[async_trait]
impl AgentAdapter for FakeAdapter {
    fn kind(&self) -> TargetKind {
        TargetKind::CodexCli
    }

    async fn detect(&self) -> TargetAvailability {
        TargetAvailability {
            kind: self.kind(),
            installed: true,
            version: Some("codex-test".into()),
            auth_state: AuthState::Unknown,
            prerequisites: vec![PrerequisiteStatus {
                name: "node".into(),
                available: true,
                version: Some("v22.0.0".into()),
            }],
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        _cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        assert!(request.workspace.join("src/index.mjs").is_file());
        fs::write(
            request.workspace.join("src/index.mjs"),
            "export const fixed = true;",
        )
        .unwrap();
        Ok(AdapterCompletion::Completed {
            duration_ms: 50,
            stdout: "agent completed".into(),
            stderr: String::new(),
        })
    }
}

struct PassingVerifier;

#[async_trait]
impl WorkspaceVerifier for PassingVerifier {
    async fn verify(
        &self,
        _verifier_id: &str,
        workspace: &Path,
        _cancellation: CancellationToken,
    ) -> VerificationGrade {
        assert!(workspace.join("src/index.mjs").is_file());
        VerificationGrade {
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            detail: "hidden_tests:pass".into(),
            duration_ms: 10,
        }
    }
}

fn environment() -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os_family: "windows".into(),
        os_version: "11".into(),
        app_version: "0.2.0".into(),
        cli_version: Some("codex-test".into()),
        verifier_runtime_version: Some("node v22.0.0".into()),
        suite_id: "cli-smoke".into(),
        suite_version: "1.0.0".into(),
        suite_content_sha256: "c".repeat(64),
        scoring_rule_version: "ability-v1".into(),
        resumed: false,
    }
}

#[tokio::test]
async fn copies_starter_runs_agent_verifies_and_checkpoints() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    fs::create_dir_all(pack_dir.join("task/starter/src")).unwrap();
    fs::write(pack_dir.join("task/prompt.md"), "Fix it.").unwrap();
    fs::write(
        pack_dir.join("task/starter/src/index.mjs"),
        "export const fixed = false;",
    )
    .unwrap();
    fs::write(
        pack_dir.join("manifest.json"),
        r#"{
          "schema_version":1,
          "id":"cli-smoke",
          "version":"1.0.0",
          "title":"CLI Smoke",
          "target_kinds":["codex_cli"],
          "tasks":[{
            "id":"fix-one",
            "category":"cli_coding",
            "prompt_file":"task/prompt.md",
            "starter_dir":"task/starter",
            "time_budget_secs":60,
            "max_turns":2,
            "grader":{"type":"external_verifier","verifier_id":"smoke-v1"}
          }]
        }"#,
    )
    .unwrap();

    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repository =
        Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = CliRunService::new(
        repository.clone(),
        dir.path().join("artifacts"),
    );
    let run = service
        .prepare(
            pack.clone(),
            TargetSelection {
                kind: TargetKind::CodexCli,
                reported_model: "CLI current selection".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            environment(),
        )
        .unwrap();
    let (events, mut receiver) = mpsc::unbounded_channel();

    service
        .execute(
            run.id,
            pack,
            Arc::new(FakeAdapter),
            Arc::new(PassingVerifier),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();

    let stored = repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Completed);
    assert_eq!(stored.score.unwrap().ability_score, 100.0);
    assert_eq!(repository.get_task_results(run.id).unwrap().len(), 1);
    let mut saw_finished = false;
    while let Ok(event) = receiver.try_recv() {
        saw_finished |= event.kind == RunEventKind::RunFinished;
    }
    assert!(saw_finished);
}

#[tokio::test]
async fn agent_budget_exhaustion_is_a_scored_failure() {
    struct Exhausted;
    #[async_trait]
    impl AgentAdapter for Exhausted {
        fn kind(&self) -> TargetKind { TargetKind::CodexCli }
        async fn detect(&self) -> TargetAvailability {
            TargetAvailability {
                kind: self.kind(),
                installed: true,
                version: None,
                auth_state: AuthState::Unknown,
                prerequisites: Vec::new(),
            }
        }
        async fn execute(
            &self,
            _request: ExecutionRequest,
            _cancellation: CancellationToken,
        ) -> Result<AdapterCompletion, AdapterError> {
            Err(AdapterError::AgentBudgetExceeded)
        }
    }

    let grade = ability_adapters::adapter_error_grade(
        AdapterError::AgentBudgetExceeded,
        60_000,
    );
    assert_eq!(grade.outcome, TaskOutcome::Failed);
    assert_eq!(grade.score, Some(0.0));
    assert_eq!(
        grade.failure_kind,
        Some(FailureKind::AgentBudgetExceeded)
    );
}
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
cargo test -p ability-adapters --test cli_run
```

Expected: FAIL with unresolved `CliRunService` and event types.

- [ ] **Step 3: Add the only extra repository method needed by cancellation**

Add to `impl RunRepository` in `crates/ability-core/src/storage.rs`:

```rust
pub fn finish_without_score(
    &self,
    run_id: Uuid,
    status: RunStatus,
) -> Result<(), StorageError> {
    debug_assert!(matches!(
        status,
        RunStatus::Cancelled | RunStatus::Interrupted
    ));
    self.connection.lock().execute(
        "UPDATE runs SET status_json=?2, finished_at=?3 WHERE id=?1",
        params![
            run_id.to_string(),
            serde_json::to_string(&status)?,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 4: Implement the coordinator**

Create `crates/ability-adapters/src/cli_run.rs`:

```rust
use crate::{
    AdapterCompletion, AdapterError, AgentAdapter, ExecutionRequest,
    VerificationGrade, WorkspaceVerifier,
};
use ability_core::{
    summarize_scores, EnvironmentFingerprint, FailureKind, GraderSpec, LoadedPack,
    LoadedTask, RunMode, RunRecord, RunRepository, RunStatus, StorageError,
    TargetKind, TargetSelection, TaskOutcome, TaskResult,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventKind {
    TaskStarted,
    TaskFinished,
    RunFinished,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: Uuid,
    pub kind: RunEventKind,
    pub task_id: Option<String>,
    pub completed_tasks: u32,
    pub total_tasks: u32,
}

#[derive(Debug, Error)]
pub enum CliRunError {
    #[error("target is not a CLI target")]
    WrongTarget,
    #[error("adapter target does not match the run")]
    AdapterMismatch,
    #[error("starter directory is missing for {0}")]
    MissingStarter(String),
    #[error("task does not use an external verifier: {0}")]
    UnsupportedGrader(String),
    #[error("workspace contains a symbolic link: {0}")]
    SymbolicLink(String),
    #[error("storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("workspace I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub struct CliRunService {
    repository: Arc<RunRepository>,
    artifact_root: PathBuf,
}

impl CliRunService {
    pub fn new(repository: Arc<RunRepository>, artifact_root: PathBuf) -> Self {
        Self {
            repository,
            artifact_root,
        }
    }

    pub fn prepare(
        &self,
        pack: Arc<LoadedPack>,
        target: TargetSelection,
        mode: RunMode,
        environment: EnvironmentFingerprint,
    ) -> Result<RunRecord, CliRunError> {
        if !matches!(target.kind, TargetKind::CodexCli | TargetKind::ClaudeCode) {
            return Err(CliRunError::WrongTarget);
        }
        if !pack.manifest.target_kinds.contains(&target.kind) {
            return Err(CliRunError::WrongTarget);
        }
        let mut run = RunRecord::new(
            target,
            mode,
            pack.manifest.id.clone(),
            pack.manifest.version.clone(),
            pack.tasks.len() as u32,
            environment,
        );
        run.status = RunStatus::Running;
        self.repository.insert_run(&run)?;
        Ok(run)
    }

    pub async fn execute(
        &self,
        run_id: Uuid,
        pack: Arc<LoadedPack>,
        adapter: Arc<dyn AgentAdapter>,
        verifier: Arc<dyn WorkspaceVerifier>,
        cancellation: CancellationToken,
        events: UnboundedSender<RunEvent>,
    ) -> Result<(), CliRunError> {
        let run = self
            .repository
            .get_run(run_id)?
            .ok_or_else(|| StorageError::Enum(format!("missing run {run_id}")))?;
        if run.target.kind != adapter.kind() {
            return Err(CliRunError::AdapterMismatch);
        }

        for (index, task) in pack.tasks.iter().enumerate() {
            if cancellation.is_cancelled() {
                self.repository
                    .finish_without_score(run_id, RunStatus::Cancelled)?;
                send_event(
                    &events,
                    run_id,
                    RunEventKind::RunFinished,
                    None,
                    index as u32,
                    pack.tasks.len() as u32,
                );
                return Ok(());
            }

            let workspace = self.create_workspace(run_id, task)?;
            send_event(
                &events,
                run_id,
                RunEventKind::TaskStarted,
                Some(task.definition.id.clone()),
                index as u32,
                pack.tasks.len() as u32,
            );
            let request = ExecutionRequest {
                prompt: task.prompt.clone(),
                workspace: workspace.clone(),
                time_budget_secs: task.definition.time_budget_secs,
                max_turns: task.definition.max_turns,
                model: (run.target.reported_model != "default")
                    .then(|| run.target.reported_model.clone()),
                reasoning_effort: run.target.reasoning_effort.clone(),
            };

            let result = match adapter
                .execute(request, cancellation.child_token())
                .await
            {
                Ok(AdapterCompletion::Completed {
                    duration_ms,
                    stdout,
                    stderr,
                }) => {
                    let log_rel = self.write_agent_log(
                        run_id,
                        &task.definition.id,
                        &stdout,
                        &stderr,
                    )?;
                    let verifier_id = match &task.definition.grader {
                        GraderSpec::ExternalVerifier { verifier_id } => verifier_id,
                        _ => {
                            return Err(CliRunError::UnsupportedGrader(
                                task.definition.id.clone(),
                            ))
                        }
                    };
                    let grade = verifier
                        .verify(
                            verifier_id,
                            &workspace,
                            cancellation.child_token(),
                        )
                        .await;
                    task_result(run_id, task, grade, Some(log_rel), duration_ms)
                }
                Err(error) => {
                    let grade = adapter_error_grade(
                        error,
                        task.definition.time_budget_secs * 1_000,
                    );
                    task_result(run_id, task, grade, None, 0)
                }
            };

            let stop_run = matches!(
                result.failure_kind,
                Some(
                    FailureKind::CliMissing
                        | FailureKind::AuthExpired
                        | FailureKind::QuotaExhausted
                        | FailureKind::Network
                        | FailureKind::RuntimeMissing
                        | FailureKind::AppInterrupted
                        | FailureKind::InfrastructureTimeout
                        | FailureKind::VerifierError
                        | FailureKind::UserCancelled
                )
            );
            self.repository.save_task_result(&result)?;
            send_event(
                &events,
                run_id,
                RunEventKind::TaskFinished,
                Some(result.task_id.clone()),
                index as u32 + 1,
                pack.tasks.len() as u32,
            );

            if result.outcome == TaskOutcome::Cancelled {
                self.repository
                    .finish_without_score(run_id, RunStatus::Cancelled)?;
                send_event(
                    &events,
                    run_id,
                    RunEventKind::RunFinished,
                    None,
                    index as u32 + 1,
                    pack.tasks.len() as u32,
                );
                return Ok(());
            }
            if stop_run {
                break;
            }
        }

        let results = self.repository.get_task_results(run_id)?;
        let summary = summarize_scores(&results, pack.tasks.len() as u32);
        self.repository.complete_run(run_id, summary.as_ref())?;
        send_event(
            &events,
            run_id,
            RunEventKind::RunFinished,
            None,
            results.len() as u32,
            pack.tasks.len() as u32,
        );
        Ok(())
    }

    fn create_workspace(
        &self,
        run_id: Uuid,
        task: &LoadedTask,
    ) -> Result<PathBuf, CliRunError> {
        let starter = task
            .definition
            .starter_dir
            .as_ref()
            .ok_or_else(|| CliRunError::MissingStarter(task.definition.id.clone()))?;
        let source = task.pack_root.join(starter);
        if !source.is_dir() {
            return Err(CliRunError::MissingStarter(task.definition.id.clone()));
        }
        let destination = self
            .artifact_root
            .join("runs")
            .join(run_id.to_string())
            .join("workspaces")
            .join(&task.definition.id);
        if destination.exists() {
            let metadata = fs::symlink_metadata(&destination)?;
            if metadata.file_type().is_symlink() {
                return Err(CliRunError::SymbolicLink(
                    destination.display().to_string(),
                ));
            }
            fs::remove_dir_all(&destination)?;
        }
        fs::create_dir_all(&destination)?;
        copy_tree(&source, &destination)?;
        Ok(destination)
    }

    fn write_agent_log(
        &self,
        run_id: Uuid,
        task_id: &str,
        stdout: &str,
        stderr: &str,
    ) -> Result<String, CliRunError> {
        let relative = format!("runs/{run_id}/logs/{task_id}.log");
        let path = self.artifact_root.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("STDOUT\n{stdout}\nSTDERR\n{stderr}"))?;
        Ok(relative)
    }
}

pub fn adapter_error_grade(error: AdapterError, budget_ms: u64) -> VerificationGrade {
    match error {
        AdapterError::AgentBudgetExceeded => VerificationGrade {
            outcome: TaskOutcome::Failed,
            score: Some(0.0),
            failure_kind: Some(FailureKind::AgentBudgetExceeded),
            detail: "agent_budget_exceeded".into(),
            duration_ms: budget_ms,
        },
        AdapterError::Cancelled => VerificationGrade {
            outcome: TaskOutcome::Cancelled,
            score: None,
            failure_kind: Some(FailureKind::UserCancelled),
            detail: "user_cancelled".into(),
            duration_ms: 0,
        },
        AdapterError::Unavailable => VerificationGrade {
            outcome: TaskOutcome::Invalid,
            score: None,
            failure_kind: Some(FailureKind::CliMissing),
            detail: "cli_unavailable".into(),
            duration_ms: 0,
        },
        AdapterError::Infrastructure { kind, detail } => VerificationGrade {
            outcome: TaskOutcome::Invalid,
            score: None,
            failure_kind: Some(kind),
            detail,
            duration_ms: 0,
        },
    }
}

fn task_result(
    run_id: Uuid,
    task: &LoadedTask,
    grade: VerificationGrade,
    answer_rel_path: Option<String>,
    agent_duration_ms: u64,
) -> TaskResult {
    TaskResult {
        run_id,
        task_id: task.definition.id.clone(),
        category: task.definition.category,
        outcome: grade.outcome,
        score: grade.score,
        failure_kind: grade.failure_kind,
        duration_ms: agent_duration_ms + grade.duration_ms,
        answer_rel_path,
        detail: grade.detail,
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), CliRunError> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let metadata = fs::symlink_metadata(&from)?;
        if metadata.file_type().is_symlink() {
            return Err(CliRunError::SymbolicLink(from.display().to_string()));
        }
        let to = destination.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir_all(&to)?;
            copy_tree(&from, &to)?;
        } else if metadata.is_file() {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn send_event(
    sender: &UnboundedSender<RunEvent>,
    run_id: Uuid,
    kind: RunEventKind,
    task_id: Option<String>,
    completed_tasks: u32,
    total_tasks: u32,
) {
    let _ = sender.send(RunEvent {
        run_id,
        kind,
        task_id,
        completed_tasks,
        total_tasks,
    });
}
```

Update `crates/ability-adapters/src/lib.rs`:

```rust
mod cli_run;
pub use cli_run::*;
```

- [ ] **Step 5: Run coordinator and regression tests**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-adapters --test cli_run
cargo test --workspace
```

Expected: both coordinator tests pass; no real subscription-backed CLI is started.

- [ ] **Step 6: Commit**

```powershell
git add crates/ability-core crates/ability-adapters
git commit -m "feat: orchestrate automatic CLI runs"
```

---

### Task 13: Compose the Secure Tauri Command Layer

**Files:**
- Create: `apps/desktop/src-tauri/src/app_state.rs`
- Create: `apps/desktop/src-tauri/src/commands.rs`
- Create: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/main.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Modify: `apps/desktop/src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: the core services, adapters, bundled pack directories, and Tauri app
  data/resource paths.
- Produces: a narrow invoke API for discovery, manual runs, CLI runs,
  cancellation, history, and run details.

- [ ] **Step 1: Add backend composition dependencies**

Merge these entries into `apps/desktop/src-tauri/Cargo.toml`:

```toml
[dependencies]
ability-core = { path = "../../../crates/ability-core" }
ability-adapters = { path = "../../../crates/ability-adapters" }
os_info = "3"
parking_lot = "0.12"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tauri = { version = "2", features = [] }
tokio = { version = "1", features = ["rt-multi-thread", "sync"] }
tokio-util = "0.7"
uuid = { version = "1", features = ["serde", "v4"] }
```

- [ ] **Step 2: Write the failing resource-layout unit test**

At the bottom of the not-yet-created `apps/desktop/src-tauri/src/app_state.rs`,
the implementation must include this test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pack_paths_do_not_depend_on_the_source_checkout() {
        let layout = ResourceLayout::from_resource_dir(Path::new("D:/app/resources"));
        assert_eq!(
            layout.client_pack,
            PathBuf::from("D:/app/resources/benchmark-packs/client-quick-v1")
        );
        assert_eq!(
            layout.cli_pack,
            PathBuf::from("D:/app/resources/benchmark-packs/cli-quick-v1")
        );
    }

    #[test]
    fn only_the_v02_tested_node_lts_lines_are_accepted() {
        assert!(supported_node_lts("v22.23.1"));
        assert!(supported_node_lts("v24.18.0"));
        assert!(!supported_node_lts("v20.20.0"));
        assert!(!supported_node_lts("v26.5.0"));
        assert!(!supported_node_lts("not-node"));
    }
}
```

Run:

```powershell
cargo test -p ability-radar --lib app_state
```

Expected: FAIL because `ResourceLayout` and `app_state` do not exist. If the
template generated a different Rust package name, use that exact package name in
this and all later `cargo -p` commands.

- [ ] **Step 3: Define frontend-safe DTOs**

Create `apps/desktop/src-tauri/src/dto.rs`:

```rust
use ability_adapters::{RunEvent, TargetAvailability};
use ability_core::{
    RunMode, RunRecord, TargetSelection, TaskResult,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackSummaryDto {
    pub id: String,
    pub version: String,
    pub title: String,
    pub task_count: u32,
    pub estimated_minutes: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapDto {
    pub targets: Vec<TargetAvailability>,
    pub client_pack: PackSummaryDto,
    pub cli_pack: PackSummaryDto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunInput {
    pub target: TargetSelection,
    pub mode: RunMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitAnswerInput {
    pub run_id: String,
    pub task_id: String,
    pub answer: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetailDto {
    pub run: RunRecord,
    pub task_results: Vec<TaskResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunErrorEvent {
    pub run_id: String,
    pub message: String,
}

pub type CliRunEventDto = RunEvent;
```

- [ ] **Step 4: Build application state from runtime paths**

Create `apps/desktop/src-tauri/src/app_state.rs`:

```rust
use ability_adapters::{
    AgentAdapter, AuthState, ClaudeCodeAdapter, CliRunService, CodexAdapter,
    NodeVerifier, PrerequisiteStatus, ProcessRunner, ProcessSpec,
    TargetAvailability, TokioProcessRunner, WorkspaceVerifier,
};
use ability_core::{
    LoadedPack, ManualRunService, PackLoader, PackRegistry, RunRepository,
    TargetKind,
};
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub struct ResourceLayout {
    pub client_pack: PathBuf,
    pub cli_pack: PathBuf,
}

impl ResourceLayout {
    pub fn from_resource_dir(resource_dir: &Path) -> Self {
        let packs = resource_dir.join("benchmark-packs");
        Self {
            client_pack: packs.join("client-quick-v1"),
            cli_pack: packs.join("cli-quick-v1"),
        }
    }
}

pub struct AppState {
    pub(crate) repository: Arc<RunRepository>,
    pub(crate) manual_runs: Arc<ManualRunService>,
    pub(crate) cli_runs: Arc<CliRunService>,
    pub(crate) client_pack: Arc<LoadedPack>,
    pub(crate) cli_pack: Arc<LoadedPack>,
    pub(crate) adapters: BTreeMap<TargetKind, Arc<dyn AgentAdapter>>,
    pub(crate) verifier: Arc<dyn WorkspaceVerifier>,
    pub(crate) runner: Arc<dyn ProcessRunner>,
    pub(crate) cancellations: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
}

impl AppState {
    pub fn build(app: &tauri::App) -> Result<Self, String> {
        let app_data = app.path().app_data_dir().map_err(|error| error.to_string())?;
        let resource_dir = app.path().resource_dir().map_err(|error| error.to_string())?;
        fs::create_dir_all(&app_data).map_err(|error| error.to_string())?;
        let layout = ResourceLayout::from_resource_dir(&resource_dir);
        let client_pack =
            Arc::new(PackLoader::load(&layout.client_pack).map_err(|error| error.to_string())?);
        let cli_pack =
            Arc::new(PackLoader::load(&layout.cli_pack).map_err(|error| error.to_string())?);
        let trusted_registry = PackRegistry::parse(include_str!(
            "../../../../benchmark-packs/registry.json"
        ))
        .map_err(|error| error.to_string())?;
        trusted_registry
            .verify_bundled(&client_pack)
            .map_err(|error| error.to_string())?;
        trusted_registry
            .verify_bundled(&cli_pack)
            .map_err(|error| error.to_string())?;
        let repository = Arc::new(
            RunRepository::open(&app_data.join("ability-radar.db"))
                .map_err(|error| error.to_string())?,
        );
        repository
            .mark_running_as_interrupted()
            .map_err(|error| error.to_string())?;

        let artifact_root = app_data.join("artifacts");
        fs::create_dir_all(&artifact_root).map_err(|error| error.to_string())?;
        let runner: Arc<dyn ProcessRunner> = Arc::new(TokioProcessRunner);
        let mut adapters: BTreeMap<TargetKind, Arc<dyn AgentAdapter>> =
            BTreeMap::new();
        adapters.insert(
            TargetKind::CodexCli,
            Arc::new(CodexAdapter::new(runner.clone())),
        );
        adapters.insert(
            TargetKind::ClaudeCode,
            Arc::new(ClaudeCodeAdapter::new(runner.clone())),
        );
        let verifier: Arc<dyn WorkspaceVerifier> = Arc::new(NodeVerifier::new(
            runner.clone(),
            layout.cli_pack,
        ));

        Ok(Self {
            manual_runs: Arc::new(ManualRunService::new(
                repository.clone(),
                artifact_root.clone(),
            )),
            cli_runs: Arc::new(CliRunService::new(
                repository.clone(),
                artifact_root,
            )),
            repository,
            client_pack,
            cli_pack,
            adapters,
            verifier,
            runner,
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn target_availability(&self) -> Vec<TargetAvailability> {
        let node = probe_node(self.runner.clone()).await;
        let mut targets = vec![
            TargetAvailability {
                kind: TargetKind::ChatGptClient,
                installed: true,
                version: None,
                auth_state: AuthState::Unknown,
                prerequisites: Vec::new(),
            },
            TargetAvailability {
                kind: TargetKind::ClaudeClient,
                installed: true,
                version: None,
                auth_state: AuthState::Unknown,
                prerequisites: Vec::new(),
            },
        ];
        for adapter in self.adapters.values() {
            let mut availability = adapter.detect().await;
            availability.prerequisites.push(node.clone());
            targets.push(availability);
        }
        targets
    }
}

pub async fn probe_node(runner: Arc<dyn ProcessRunner>) -> PrerequisiteStatus {
    let result = runner
        .run(
            ProcessSpec {
                program: "node".into(),
                args: vec!["--version".into()],
                current_dir: std::env::temp_dir(),
                env: BTreeMap::new(),
                timeout: Duration::from_secs(10),
            },
            CancellationToken::new(),
        )
        .await;
    match result {
        Ok(output) if output.exit_code == Some(0) => {
            let version = output.stdout.trim().to_owned();
            PrerequisiteStatus {
                name: "Node.js 22/24 LTS".into(),
                available: supported_node_lts(&version),
                version: Some(version),
            }
        }
        _ => PrerequisiteStatus {
            name: "Node.js 22/24 LTS".into(),
            available: false,
            version: None,
        },
    }
}

fn supported_node_lts(version: &str) -> bool {
    version
        .trim()
        .strip_prefix('v')
        .and_then(|value| value.split('.').next())
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|major| matches!(major, 22 | 24))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pack_paths_do_not_depend_on_the_source_checkout() {
        let layout = ResourceLayout::from_resource_dir(Path::new("D:/app/resources"));
        assert_eq!(
            layout.client_pack,
            PathBuf::from("D:/app/resources/benchmark-packs/client-quick-v1")
        );
        assert_eq!(
            layout.cli_pack,
            PathBuf::from("D:/app/resources/benchmark-packs/cli-quick-v1")
        );
    }

    #[test]
    fn only_the_v02_tested_node_lts_lines_are_accepted() {
        assert!(supported_node_lts("v22.23.1"));
        assert!(supported_node_lts("v24.18.0"));
        assert!(!supported_node_lts("v20.20.0"));
        assert!(!supported_node_lts("v26.5.0"));
        assert!(!supported_node_lts("not-node"));
    }
}
```

- [ ] **Step 5: Implement the command allowlist**

Create `apps/desktop/src-tauri/src/commands.rs`:

```rust
use crate::app_state::{probe_node, AppState};
use crate::dto::{
    BootstrapDto, PackSummaryDto, RunDetailDto, RunErrorEvent, StartRunInput,
    SubmitAnswerInput,
};
use ability_adapters::{AuthState, RunEvent};
use ability_core::{
    EnvironmentFingerprint, ManualStep, RunRecord, TargetKind, TargetSelection,
    TaskResult,
};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn environment(
    pack: &ability_core::LoadedPack,
    cli_version: Option<String>,
    verifier_runtime_version: Option<String>,
) -> EnvironmentFingerprint {
    let os = os_info::get();
    EnvironmentFingerprint {
        os_family: os.os_type().to_string(),
        os_version: os.version().to_string(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        cli_version,
        verifier_runtime_version,
        suite_id: pack.manifest.id.clone(),
        suite_version: pack.manifest.version.clone(),
        suite_content_sha256: pack.content_sha256.clone(),
        scoring_rule_version: "ability-v1".into(),
        resumed: false,
    }
}

fn parse_run_id(value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| "无效的测试编号".into())
}

fn normalize_target(mut target: TargetSelection) -> Result<TargetSelection, String> {
    target.reported_model = target.reported_model.trim().to_owned();
    if target.reported_model.is_empty()
        || target.reported_model.chars().count() > 120
        || contains_forbidden_display_character(&target.reported_model)
    {
        return Err("模型名称必须是 1–120 个可见字符".into());
    }
    if matches!(target.kind, TargetKind::CodexCli | TargetKind::ClaudeCode)
        && target.reported_model != "default"
        && (target.reported_model.starts_with('-')
            || target.reported_model.chars().any(char::is_whitespace))
    {
        return Err("CLI 模型名不能以连字符开头或包含空白".into());
    }
    target.reasoning_effort = target
        .reasoning_effort
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if target.reasoning_effort.as_ref().is_some_and(|value| {
        !matches!(value.as_str(), "low" | "medium" | "high")
    }) {
        return Err("首版推理档位只能是 low、medium 或 high".into());
    }
    Ok(target)
}

#[tauri::command]
pub async fn get_bootstrap(state: State<'_, AppState>) -> Result<BootstrapDto, String> {
    Ok(BootstrapDto {
        targets: state.target_availability().await,
        client_pack: PackSummaryDto {
            id: state.client_pack.manifest.id.clone(),
            version: state.client_pack.manifest.version.clone(),
            title: state.client_pack.manifest.title.clone(),
            task_count: state.client_pack.tasks.len() as u32,
            estimated_minutes: "10–15".into(),
        },
        cli_pack: PackSummaryDto {
            id: state.cli_pack.manifest.id.clone(),
            version: state.cli_pack.manifest.version.clone(),
            title: state.cli_pack.manifest.title.clone(),
            task_count: state.cli_pack.tasks.len() as u32,
            estimated_minutes: "30–60".into(),
        },
    })
}

#[tauri::command]
pub fn start_manual_run(
    state: State<'_, AppState>,
    input: StartRunInput,
) -> Result<RunRecord, String> {
    state
        .manual_runs
        .start(
            state.client_pack.clone(),
            normalize_target(input.target)?,
            input.mode,
            environment(&state.client_pack, None, None),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn next_manual_step(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<ManualStep>, String> {
    state
        .manual_runs
        .next_step(parse_run_id(&run_id)?)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn submit_manual_answer(
    state: State<'_, AppState>,
    input: SubmitAnswerInput,
) -> Result<TaskResult, String> {
    state
        .manual_runs
        .submit_answer(
            parse_run_id(&input.run_id)?,
            &input.task_id,
            &input.answer,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn start_cli_run(
    app: AppHandle,
    state: State<'_, AppState>,
    input: StartRunInput,
) -> Result<RunRecord, String> {
    if !matches!(input.target.kind, TargetKind::CodexCli | TargetKind::ClaudeCode) {
        return Err("请选择 Codex CLI 或 Claude Code".into());
    }
    let target = normalize_target(input.target)?;
    let adapter = state
        .adapters
        .get(&target.kind)
        .cloned()
        .ok_or_else(|| "该 CLI 暂不支持".to_string())?;
    let availability = adapter.detect().await;
    if !availability.installed {
        return Err("未找到所选 CLI，请先安装并完成登录".into());
    }
    if availability.auth_state == AuthState::NeedsLogin {
        return Err("所选 CLI 尚未登录，请先在终端完成登录".into());
    }
    let node = probe_node(state.runner.clone()).await;
    if !node.available {
        return Err("CLI 快速体检需要 Node.js 22 或 24 LTS 来运行本地验证器".into());
    }

    let run = state
        .cli_runs
        .prepare(
            state.cli_pack.clone(),
            target,
            input.mode,
            environment(&state.cli_pack, availability.version, node.version),
        )
        .map_err(|error| error.to_string())?;
    let run_id = run.id;
    let cancellation = CancellationToken::new();
    state
        .cancellations
        .lock()
        .insert(run_id, cancellation.clone());

    let service = state.cli_runs.clone();
    let pack = state.cli_pack.clone();
    let verifier = state.verifier.clone();
    let repository = state.repository.clone();
    let cancellations = state.cancellations.clone();
    let (sender, mut receiver) = mpsc::unbounded_channel::<RunEvent>();
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let _ = event_app.emit("run://event", event);
        }
    });
    tauri::async_runtime::spawn(async move {
        if let Err(error) = service
            .execute(
                run_id,
                pack,
                adapter,
                verifier,
                cancellation,
                sender,
            )
            .await
        {
            let _ = repository.finish_without_score(
                run_id,
                ability_core::RunStatus::Interrupted,
            );
            let _ = app.emit(
                "run://error",
                RunErrorEvent {
                    run_id: run_id.to_string(),
                    message: error.to_string(),
                },
            );
        }
        cancellations.lock().remove(&run_id);
    });
    Ok(run)
}

#[tauri::command]
pub fn cancel_run(state: State<'_, AppState>, run_id: String) -> Result<bool, String> {
    let run_id = parse_run_id(&run_id)?;
    Ok(match state.cancellations.lock().get(&run_id) {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    })
}

#[tauri::command]
pub fn list_runs(state: State<'_, AppState>) -> Result<Vec<RunRecord>, String> {
    state.repository.list_runs().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_run_detail(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<RunDetailDto>, String> {
    let run_id = parse_run_id(&run_id)?;
    let Some(run) = state
        .repository
        .get_run(run_id)
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let task_results = state
        .repository
        .get_task_results(run_id)
        .map_err(|error| error.to_string())?;
    Ok(Some(RunDetailDto { run, task_results }))
}
```

For v0.2.1, `contains_forbidden_display_character` is the shared core
predicate used by backend start/resume and public-report validation. It rejects
Unicode `Cc`, `Cf`, `Default_Ignorable_Code_Point`, and bidi formatting
characters. Frontend manual-start validation mirrors this policy; invalid
legacy stored labels are displayed through a stable localized placeholder
without rewriting the stored record.

- [ ] **Step 6: Register state and commands**

Replace `apps/desktop/src-tauri/src/lib.rs` with:

```rust
mod app_state;
mod commands;
mod dto;

use app_state::AppState;
use commands::{
    cancel_run, get_bootstrap, get_run_detail, list_runs, next_manual_step,
    start_cli_run, start_manual_run, submit_manual_answer,
};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let state = AppState::build(app)
                .map_err(std::io::Error::other)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            start_manual_run,
            next_manual_step,
            submit_manual_answer,
            start_cli_run,
            cancel_run,
            list_runs,
            get_run_detail,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AI Ability Radar");
}
```

Keep the generated `main.rs` as a one-line call to the library crate’s `run()`
function.

- [ ] **Step 7: Bundle packs at a stable runtime path and minimize capabilities**

Merge this resource mapping into `bundle` in
`apps/desktop/src-tauri/tauri.conf.json`:

```json
"resources": {
  "../../../benchmark-packs/": "benchmark-packs/"
}
```

Set the Windows bundle target to NSIS and MSI:

```json
"targets": ["nsis", "msi"]
```

Replace `apps/desktop/src-tauri/capabilities/default.json` with:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Only core window functionality; privileged work uses reviewed Rust commands.",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

Do not add shell, process, filesystem, HTTP, clipboard, or SQL plugin permissions.
The webview can only invoke the registered Rust commands.

- [ ] **Step 8: Verify runtime composition**

Run:

```powershell
cargo fmt --all --check
cargo test --workspace
npm run tauri -- build --debug
```

Expected: all Rust tests pass; the debug bundle contains
`resources/benchmark-packs/client-quick-v1/manifest.json` and
`resources/benchmark-packs/cli-quick-v1/manifest.json`; launching the executable
does not report a missing pack.

- [ ] **Step 9: Commit**

```powershell
git add Cargo.lock apps/desktop/src-tauri
git commit -m "feat: expose secure desktop command layer"
```

---

### Task 14: Add a Typed Frontend Boundary and Navigable Shell

**Files:**
- Create: `apps/desktop/src/api/backend.ts`
- Create: `apps/desktop/src/api/tauriBackend.ts`
- Create: `apps/desktop/src/api/BackendContext.tsx`
- Create: `apps/desktop/src/app/routes.tsx`
- Create: `apps/desktop/src/components/AppShell.tsx`
- Create: `apps/desktop/src/pages/HomePage.tsx`
- Create: `apps/desktop/src/pages/HomePage.test.tsx`
- Create: `apps/desktop/src/pages/PlaceholderPages.tsx`
- Modify: `apps/desktop/src/app/App.tsx`
- Modify: `apps/desktop/package.json`

**Interfaces:**
- Consumes: the Tauri commands and events from Task 13.
- Produces: one mockable `Backend` interface and routes for home, manual run,
  CLI run, history, and result details.

- [ ] **Step 1: Add routing and test interaction dependencies**

Run:

```powershell
npm install --workspace apps/desktop react-router-dom
npm install --workspace apps/desktop --save-dev @testing-library/user-event
```

- [ ] **Step 2: Define the exact TypeScript contract**

Create `apps/desktop/src/api/backend.ts`:

```ts
export type TargetKind =
  | "chat_gpt_client"
  | "claude_client"
  | "codex_cli"
  | "claude_code";
export type RunMode = "quick" | "deep";
export type RunStatus =
  | "created"
  | "running"
  | "completed"
  | "cancelled"
  | "interrupted";
export type TaskOutcome = "passed" | "failed" | "invalid" | "cancelled";
export type FailureKind =
  | "cli_missing"
  | "runtime_missing"
  | "auth_expired"
  | "quota_exhausted"
  | "network"
  | "user_cancelled"
  | "app_interrupted"
  | "infrastructure_timeout"
  | "agent_budget_exceeded"
  | "verifier_error"
  | "wrong_answer";
export type Category =
  | "instruction_following"
  | "logic"
  | "code_review"
  | "cli_coding";

export interface TargetSelection {
  kind: TargetKind;
  reportedModel: string;
  reasoningEffort?: string | null;
}

export interface TargetAvailability {
  kind: TargetKind;
  installed: boolean;
  version?: string | null;
  authState: "unknown" | "ready" | "needs_login";
  prerequisites: Array<{
    name: string;
    available: boolean;
    version?: string | null;
  }>;
}

export interface PackSummary {
  id: string;
  version: string;
  title: string;
  taskCount: number;
  estimatedMinutes: string;
}

export interface Bootstrap {
  targets: TargetAvailability[];
  clientPack: PackSummary;
  cliPack: PackSummary;
}

export interface ScoreSummary {
  abilityScore: number;
  passedTasks: number;
  validTasks: number;
  totalTasks: number;
  categoryScores: Partial<Record<Category, number>>;
}

export interface RunRecord {
  id: string;
  target: TargetSelection;
  mode: RunMode;
  suiteId: string;
  suiteVersion: string;
  status: RunStatus;
  startedAt: string;
  finishedAt?: string | null;
  totalTasks: number;
  completedTasks: number;
  environment: {
    osFamily: string;
    osVersion: string;
    appVersion: string;
    cliVersion?: string | null;
    verifierRuntimeVersion?: string | null;
    suiteId: string;
    suiteVersion: string;
    suiteContentSha256: string;
    scoringRuleVersion: string;
    resumed: boolean;
  };
  score?: ScoreSummary | null;
}

export interface TaskResult {
  runId: string;
  taskId: string;
  category: Category;
  outcome: TaskOutcome;
  score?: number | null;
  failureKind?: FailureKind | null;
  durationMs: number;
  answerRelPath?: string | null;
  detail: string;
}

export interface ManualStep {
  runId: string;
  taskId: string;
  taskNumber: number;
  totalTasks: number;
  prompt: string;
}

export interface RunDetail {
  run: RunRecord;
  taskResults: TaskResult[];
}

export interface StartRunInput {
  target: TargetSelection;
  mode: RunMode;
}

export interface RunEvent {
  runId: string;
  kind: "task_started" | "task_finished" | "run_finished";
  taskId?: string | null;
  completedTasks: number;
  totalTasks: number;
}

export interface RunErrorEvent {
  runId: string;
  message: string;
}

export type Unlisten = () => void;

export interface Backend {
  getBootstrap(): Promise<Bootstrap>;
  startManualRun(input: StartRunInput): Promise<RunRecord>;
  nextManualStep(runId: string): Promise<ManualStep | null>;
  submitManualAnswer(input: {
    runId: string;
    taskId: string;
    answer: string;
  }): Promise<TaskResult>;
  startCliRun(input: StartRunInput): Promise<RunRecord>;
  cancelRun(runId: string): Promise<boolean>;
  listRuns(): Promise<RunRecord[]>;
  getRunDetail(runId: string): Promise<RunDetail | null>;
  onRunEvent(listener: (event: RunEvent) => void): Promise<Unlisten>;
  onRunError(listener: (event: RunErrorEvent) => void): Promise<Unlisten>;
}
```

- [ ] **Step 3: Implement only the reviewed Tauri calls**

Create `apps/desktop/src/api/tauriBackend.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Backend,
  Bootstrap,
  ManualStep,
  RunDetail,
  RunErrorEvent,
  RunEvent,
  RunRecord,
  StartRunInput,
  TaskResult,
} from "./backend";

export const tauriBackend: Backend = {
  getBootstrap: () => invoke<Bootstrap>("get_bootstrap"),
  startManualRun: (input) =>
    invoke<RunRecord>("start_manual_run", { input }),
  nextManualStep: (runId) =>
    invoke<ManualStep | null>("next_manual_step", { runId }),
  submitManualAnswer: (input) =>
    invoke<TaskResult>("submit_manual_answer", { input }),
  startCliRun: (input) => invoke<RunRecord>("start_cli_run", { input }),
  cancelRun: (runId) => invoke<boolean>("cancel_run", { runId }),
  listRuns: () => invoke<RunRecord[]>("list_runs"),
  getRunDetail: (runId) =>
    invoke<RunDetail | null>("get_run_detail", { runId }),
  onRunEvent: async (listener) =>
    listen<RunEvent>("run://event", ({ payload }) => listener(payload)),
  onRunError: async (listener) =>
    listen<RunErrorEvent>("run://error", ({ payload }) => listener(payload)),
};
```

Create `apps/desktop/src/api/BackendContext.tsx`:

```tsx
import { createContext, useContext, type ReactNode } from "react";
import type { Backend } from "./backend";
import { tauriBackend } from "./tauriBackend";

const BackendContext = createContext<Backend>(tauriBackend);

export function BackendProvider({
  backend,
  children,
}: {
  backend: Backend;
  children: ReactNode;
}) {
  return (
    <BackendContext.Provider value={backend}>
      {children}
    </BackendContext.Provider>
  );
}

export function useBackend(): Backend {
  return useContext(BackendContext);
}
```

- [ ] **Step 4: Write the failing home-page test**

Create `apps/desktop/src/pages/HomePage.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type { Backend, Bootstrap } from "../api/backend";
import { HomePage } from "./HomePage";

const bootstrap: Bootstrap = {
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
    taskCount: 2,
    estimatedMinutes: "30–60",
  },
  targets: [
    {
      kind: "chat_gpt_client",
      installed: true,
      authState: "unknown",
      prerequisites: [],
    },
    {
      kind: "codex_cli",
      installed: true,
      version: "codex 1.2.3",
      authState: "unknown",
      prerequisites: [
        { name: "Node.js 22/24 LTS", available: false },
      ],
    },
  ],
};

const backend = {
  getBootstrap: async () => bootstrap,
} as Backend;

test("shows separate client and CLI choices with honest prerequisites", async () => {
  render(
    <MemoryRouter>
      <BackendProvider backend={backend}>
        <HomePage />
      </BackendProvider>
    </MemoryRouter>,
  );

  expect(
    await screen.findByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
  expect(screen.getByText("ChatGPT 客户端")).toBeInTheDocument();
  expect(screen.getByText("Codex CLI")).toBeInTheDocument();
  expect(screen.getByText("缺少 Node.js 22/24 LTS")).toBeInTheDocument();
  expect(screen.getByText("约 10–15 分钟")).toBeInTheDocument();
});
```

Run:

```powershell
npm test -- --run src/pages/HomePage.test.tsx
```

Expected: FAIL because `HomePage` does not exist.

- [ ] **Step 5: Implement the shell and home page**

Create `apps/desktop/src/components/AppShell.tsx`:

```tsx
import { NavLink, Outlet } from "react-router-dom";

export function AppShell() {
  return (
    <div className="app-shell">
      <header className="topbar">
        <NavLink className="brand" to="/">AI 能力雷达</NavLink>
        <nav aria-label="主导航">
          <NavLink to="/">开始体检</NavLink>
          <NavLink to="/history">历史记录</NavLink>
        </nav>
      </header>
      <Outlet />
    </div>
  );
}
```

Create `apps/desktop/src/pages/HomePage.tsx`:

```tsx
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type { Bootstrap, TargetAvailability, TargetKind } from "../api/backend";

const labels: Record<TargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

function isCli(kind: TargetKind) {
  return kind === "codex_cli" || kind === "claude_code";
}

function blocker(target: TargetAvailability): string | null {
  if (!target.installed) return "未检测到安装";
  if (target.authState === "needs_login") return "CLI 需要登录";
  const missing = target.prerequisites.find((item) => !item.available);
  return missing ? `缺少 ${missing.name}` : null;
}

export function HomePage() {
  const backend = useBackend();
  const [data, setData] = useState<Bootstrap | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    void backend.getBootstrap().then(setData).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [backend]);

  if (error) return <main><h1>无法读取本机环境</h1><p>{error}</p></main>;
  if (!data) return <main aria-busy="true"><p>正在检查本机环境…</p></main>;

  return (
    <main>
      <section className="hero">
        <p className="eyebrow">本地优先 · 不读取账号密码</p>
        <h1>选择要体检的 AI</h1>
        <p>客户端采用复制粘贴，CLI 在隔离的临时任务目录中自动运行。</p>
      </section>
      <section className="target-grid" aria-label="可测试目标">
        {data.targets.map((target) => {
          const pack = isCli(target.kind) ? data.cliPack : data.clientPack;
          const reason = blocker(target);
          return (
            <article className="target-card" key={target.kind}>
              <h2>{labels[target.kind]}</h2>
              <p>{pack.taskCount} 道任务 · 约 {pack.estimatedMinutes} 分钟</p>
              <p className={reason ? "status status-warn" : "status status-ok"}>
                {reason ?? (isCli(target.kind) ? "本机环境可用" : "手动复制粘贴")}
              </p>
              {reason ? (
                <button type="button" disabled>暂时无法开始</button>
              ) : (
                <Link
                  className="button"
                  to={`/${isCli(target.kind) ? "cli" : "manual"}/${target.kind}`}
                >
                  选择
                </Link>
              )}
            </article>
          );
        })}
      </section>
      <aside className="notice">
        运行 CLI 任务会消耗你自己的订阅额度；本项目维护者不会代付，也不会接触你的凭据。
      </aside>
    </main>
  );
}
```

Create `apps/desktop/src/pages/PlaceholderPages.tsx`:

```tsx
export const ManualRunPage = () => <main><h1>客户端体检</h1></main>;
export const CliRunPage = () => <main><h1>CLI 体检</h1></main>;
export const HistoryPage = () => <main><h1>历史记录</h1></main>;
export const ResultPage = () => <main><h1>体检结果</h1></main>;
```

Create `apps/desktop/src/app/routes.tsx`:

```tsx
import { Route, Routes } from "react-router-dom";
import { AppShell } from "../components/AppShell";
import { HomePage } from "../pages/HomePage";
import {
  CliRunPage,
  HistoryPage,
  ManualRunPage,
  ResultPage,
} from "../pages/PlaceholderPages";

export function AppRoutes() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<HomePage />} />
        <Route path="manual/:target" element={<ManualRunPage />} />
        <Route path="cli/:target" element={<CliRunPage />} />
        <Route path="history" element={<HistoryPage />} />
        <Route path="results/:runId" element={<ResultPage />} />
      </Route>
    </Routes>
  );
}
```

Replace `apps/desktop/src/app/App.tsx` with:

```tsx
import { BrowserRouter } from "react-router-dom";
import { AppRoutes } from "./routes";

export function App() {
  return (
    <BrowserRouter>
      <AppRoutes />
    </BrowserRouter>
  );
}
```

- [ ] **Step 6: Update the original smoke test and run the frontend suite**

Replace `apps/desktop/src/app/App.test.tsx` with:

```tsx
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type { Backend } from "../api/backend";
import { AppRoutes } from "./routes";

const backend = {
  getBootstrap: async () => ({
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
      taskCount: 2,
      estimatedMinutes: "30–60",
    },
    targets: [],
  }),
} as Backend;

test("renders the product entry point", async () => {
  render(
    <MemoryRouter>
      <BackendProvider backend={backend}>
        <AppRoutes />
      </BackendProvider>
    </MemoryRouter>,
  );
  expect(screen.getByText("AI 能力雷达")).toBeInTheDocument();
  expect(
    await screen.findByRole("heading", { name: "选择要体检的 AI" }),
  ).toBeInTheDocument();
});
```

Then run:

```powershell
npm test
npm run build
```

Expected: all frontend tests pass and the production TypeScript build has no
type errors.

- [ ] **Step 7: Commit**

```powershell
git add apps/desktop package-lock.json
git commit -m "feat: add typed desktop navigation"
```

---

### Task 15: Implement the Assisted ChatGPT and Claude Wizard

**Files:**
- Create: `apps/desktop/src/pages/ManualRunPage.tsx`
- Create: `apps/desktop/src/pages/ManualRunPage.test.tsx`
- Modify: `apps/desktop/src/app/routes.tsx`
- Modify: `apps/desktop/src/pages/PlaceholderPages.tsx`

**Interfaces:**
- Consumes: `startManualRun`, `nextManualStep`, and `submitManualAnswer`.
- Produces: the complete eight-task copy/paste flow with explicit fresh-chat and
  self-reported-model controls.

- [ ] **Step 1: Write the failing wizard test**

Create `apps/desktop/src/pages/ManualRunPage.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type { Backend, RunRecord } from "../api/backend";
import { ManualRunPage } from "./ManualRunPage";

const run: RunRecord = {
  id: "6c8cce50-bbf3-4bc5-890d-1f3316222a46",
  target: {
    kind: "chat_gpt_client",
    reportedModel: "GPT-5",
    reasoningEffort: null,
  },
  mode: "quick",
  suiteId: "client-quick",
  suiteVersion: "1.0.0",
  status: "running",
  startedAt: "2026-07-17T00:00:00Z",
  finishedAt: null,
  totalTasks: 8,
  completedTasks: 0,
  environment: {
    osFamily: "Windows",
    osVersion: "11",
    appVersion: "0.2.0",
    suiteId: "client-quick",
    suiteVersion: "1.0.0",
    suiteContentSha256: "b".repeat(64),
    scoringRuleVersion: "ability-v1",
    resumed: false,
  },
};

test("starts only after fresh-chat confirmation and submits a pasted answer", async () => {
  const user = userEvent.setup();
  let nextCalls = 0;
  const backend = {
    startManualRun: async () => run,
    nextManualStep: async () => {
      nextCalls += 1;
      return nextCalls === 1
        ? {
            runId: run.id,
            taskId: "instruction-filter",
            taskNumber: 1,
            totalTasks: 8,
            prompt: "只输出 JSON",
          }
        : null;
    },
    submitManualAnswer: async () => ({
      runId: run.id,
      taskId: "instruction-filter",
      category: "instruction_following" as const,
      outcome: "passed" as const,
      score: 100,
      durationMs: 100,
      detail: "exact_json:pass",
    }),
  } as Backend;

  render(
    <MemoryRouter initialEntries={["/manual/chat_gpt_client"]}>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/manual/:target" element={<ManualRunPage />} />
          <Route path="/results/:runId" element={<h1>体检结果</h1>} />
        </Routes>
      </BackendProvider>
    </MemoryRouter>,
  );

  const start = screen.getByRole("button", { name: "开始 8 题体检" });
  expect(start).toBeDisabled();
  await user.type(screen.getByLabelText("当前显示的模型"), "GPT-5");
  await user.click(screen.getByLabelText("我会为每道题新建空白对话"));
  expect(start).toBeEnabled();
  await user.click(start);
  expect(await screen.findByText("只输出 JSON")).toBeInTheDocument();
  await user.type(screen.getByLabelText("粘贴 AI 的完整回答"), '{"ok":true}');
  await user.click(screen.getByRole("button", { name: "提交并进入下一题" }));
  expect(
    await screen.findByRole("heading", { name: "体检结果" }),
  ).toBeInTheDocument();
});
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
npm test -- --run src/pages/ManualRunPage.test.tsx
```

Expected: FAIL because the real wizard does not exist.

- [ ] **Step 3: Implement setup, prompt, answer, and completion states**

Create `apps/desktop/src/pages/ManualRunPage.tsx`:

```tsx
import { useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type {
  ManualStep,
  RunRecord,
  TargetKind,
} from "../api/backend";

const clientLabels: Partial<Record<TargetKind, string>> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
};

export function ManualRunPage() {
  const { target = "" } = useParams();
  const kind = target as TargetKind;
  const label = clientLabels[kind];
  const backend = useBackend();
  const navigate = useNavigate();
  const [model, setModel] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState("");
  const [freshChat, setFreshChat] = useState(false);
  const [run, setRun] = useState<RunRecord | null>(null);
  const [step, setStep] = useState<ManualStep | null>(null);
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState("");

  if (!label) {
    return <main><h1>不支持的客户端</h1><p>请返回首页重新选择。</p></main>;
  }

  async function start() {
    setBusy(true);
    setError("");
    try {
      const created = await backend.startManualRun({
        target: {
          kind,
          reportedModel: model.trim(),
          reasoningEffort: reasoningEffort || null,
        },
        mode: "quick",
      });
      setRun(created);
      setStep(await backend.nextManualStep(created.id));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function copyPrompt() {
    if (!step) return;
    try {
      await navigator.clipboard.writeText(step.prompt);
      setCopied(true);
    } catch {
      setError("自动复制失败，请选中题目文字后手动复制。");
    }
  }

  async function submit() {
    if (!run || !step || !answer.trim()) return;
    setBusy(true);
    setError("");
    try {
      await backend.submitManualAnswer({
        runId: run.id,
        taskId: step.taskId,
        answer,
      });
      const next = await backend.nextManualStep(run.id);
      if (!next) {
        navigate(`/results/${run.id}`);
        return;
      }
      setStep(next);
      setAnswer("");
      setCopied(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  if (!run || !step) {
    return (
      <main className="run-page">
        <p className="eyebrow">客户端 · 约 10–15 分钟</p>
        <h1>{label}快速体检</h1>
        <p>
          这里不会读取客户端或账号。你负责复制题目和粘贴完整回答，
          工具只在本机评分。
        </p>
        <label>
          当前显示的模型
          <input
            autoComplete="off"
            value={model}
            onChange={(event) => setModel(event.target.value)}
            placeholder="例如 GPT-5、Claude Sonnet"
          />
        </label>
        <label>
          推理档位（没有显示可留空）
          <select
            value={reasoningEffort}
            onChange={(event) => setReasoningEffort(event.target.value)}
          >
            <option value="">未显示 / 不适用</option>
            <option value="low">低</option>
            <option value="medium">中</option>
            <option value="high">高</option>
          </select>
        </label>
        <label className="check-row">
          <input
            type="checkbox"
            checked={freshChat}
            onChange={(event) => setFreshChat(event.target.checked)}
          />
          我会为每道题新建空白对话
        </label>
        <p className="hint">
          不使用旧聊天，关闭联网搜索、画布和连接器，不追加解释性提示，
          也不要把评分结果发回给 AI。
        </p>
        {error && <p role="alert">{error}</p>}
        <button
          type="button"
          disabled={busy || !freshChat || !model.trim()}
          onClick={() => void start()}
        >
          {busy ? "正在创建…" : "开始 8 题体检"}
        </button>
      </main>
    );
  }

  return (
    <main className="run-page">
      <div className="progress-copy">
        <p>第 {step.taskNumber} / {step.totalTasks} 题</p>
        <progress value={step.taskNumber - 1} max={step.totalTasks}>
          {step.taskNumber - 1}/{step.totalTasks}
        </progress>
      </div>
      <h1>在新对话中完成这道题</h1>
      <pre className="prompt-box">{step.prompt}</pre>
      <button type="button" className="secondary" onClick={() => void copyPrompt()}>
        {copied ? "已复制" : "复制题目"}
      </button>
      <label>
        粘贴 AI 的完整回答
        <textarea
          rows={10}
          maxLength={262_144}
          value={answer}
          onChange={(event) => setAnswer(event.target.value)}
          placeholder="不要修改、删减或补充回答"
        />
        <small className="hint">最多保存 256 KiB；普通回答远小于此限制。</small>
      </label>
      {error && <p role="alert">{error}</p>}
      <button
        type="button"
        disabled={busy || !answer.trim()}
        onClick={() => void submit()}
      >
        {busy ? "正在保存…" : "提交并进入下一题"}
      </button>
    </main>
  );
}
```

- [ ] **Step 4: Wire the real page into routing**

In `apps/desktop/src/app/routes.tsx`, import `ManualRunPage` from
`../pages/ManualRunPage` and remove it from the placeholder import. Delete the
`ManualRunPage` export from `PlaceholderPages.tsx`.

- [ ] **Step 5: Run the wizard and full frontend tests**

Run:

```powershell
npm test
npm run build
```

Expected: the assisted-flow test passes, a failed submission preserves the
textarea, and all earlier tests remain green.

- [ ] **Step 6: Commit**

```powershell
git add apps/desktop
git commit -m "feat: add assisted client benchmark flow"
```

---

### Task 16: Implement Automatic CLI Progress and Cancellation

**Files:**
- Create: `apps/desktop/src/pages/CliRunPage.tsx`
- Create: `apps/desktop/src/pages/CliRunPage.test.tsx`
- Modify: `apps/desktop/src/app/routes.tsx`
- Modify: `apps/desktop/src/pages/PlaceholderPages.tsx`

**Interfaces:**
- Consumes: CLI discovery, `startCliRun`, `cancelRun`, run events, run errors,
  and `getRunDetail`.
- Produces: a cost-aware setup screen, live two-task progress, a two-step cancel
  action, and an event-loss-safe polling fallback.

- [ ] **Step 1: Write the failing CLI-flow test**

Create `apps/desktop/src/pages/CliRunPage.test.tsx`:

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type {
  Backend,
  RunEvent,
  RunRecord,
} from "../api/backend";
import { CliRunPage } from "./CliRunPage";

const run: RunRecord = {
  id: "abcf6868-c431-49d9-9fd8-b89dba28969b",
  target: {
    kind: "codex_cli",
    reportedModel: "default",
    reasoningEffort: null,
  },
  mode: "quick",
  suiteId: "cli-quick",
  suiteVersion: "1.0.0",
  status: "running",
  startedAt: "2026-07-17T00:00:00Z",
  totalTasks: 2,
  completedTasks: 0,
  environment: {
    osFamily: "Windows",
    osVersion: "11",
    appVersion: "0.2.0",
    cliVersion: "codex 1.2.3",
    verifierRuntimeVersion: "v22.0.0",
    suiteId: "cli-quick",
    suiteVersion: "1.0.0",
    suiteContentSha256: "c".repeat(64),
    scoringRuleVersion: "ability-v1",
    resumed: false,
  },
};

test("requires a cost acknowledgement and follows run events", async () => {
  const user = userEvent.setup();
  let listener: ((event: RunEvent) => void) | undefined;
  const backend = {
    getBootstrap: async () => ({
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
        taskCount: 2,
        estimatedMinutes: "30–60",
      },
      targets: [{
        kind: "codex_cli" as const,
        installed: true,
        version: "codex 1.2.3",
        authState: "unknown" as const,
        prerequisites: [{
          name: "Node.js 22/24 LTS",
          available: true,
          version: "v22.0.0",
        }],
      }],
    }),
    onRunEvent: async (next: (event: RunEvent) => void) => {
      listener = next;
      return () => undefined;
    },
    onRunError: async () => () => undefined,
    startCliRun: async () => run,
    getRunDetail: async () => ({
      run,
      taskResults: [],
    }),
  } as Backend;

  render(
    <MemoryRouter initialEntries={["/cli/codex_cli"]}>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/cli/:target" element={<CliRunPage />} />
          <Route path="/results/:runId" element={<h1>CLI 结果</h1>} />
        </Routes>
      </BackendProvider>
    </MemoryRouter>,
  );

  const start = await screen.findByRole("button", { name: "开始 2 个任务" });
  await waitFor(() => expect(listener).toBeDefined());
  expect(start).toBeDisabled();
  await user.click(screen.getByLabelText("我了解这会消耗自己的订阅额度"));
  await user.click(start);
  expect(await screen.findByText("0 / 2 已完成")).toBeInTheDocument();
  listener?.({
    runId: run.id,
    kind: "run_finished",
    taskId: null,
    completedTasks: 2,
    totalTasks: 2,
  });
  expect(
    await screen.findByRole("heading", { name: "CLI 结果" }),
  ).toBeInTheDocument();
});
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
npm test -- --run src/pages/CliRunPage.test.tsx
```

Expected: FAIL because `CliRunPage` does not exist.

- [ ] **Step 3: Implement preflight, live progress, polling, and cancellation**

Create `apps/desktop/src/pages/CliRunPage.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type {
  Bootstrap,
  RunEvent,
  RunRecord,
  TargetKind,
  Unlisten,
} from "../api/backend";

const labels: Partial<Record<TargetKind, string>> = {
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

export function CliRunPage() {
  const { target = "" } = useParams();
  const kind = target as TargetKind;
  const label = labels[kind];
  const backend = useBackend();
  const navigate = useNavigate();
  const activeRunId = useRef<string | null>(null);
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null);
  const [run, setRun] = useState<RunRecord | null>(null);
  const [acceptedCost, setAcceptedCost] = useState(false);
  const [model, setModel] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState("");
  const [progress, setProgress] = useState({ completed: 0, total: 2 });
  const [currentTask, setCurrentTask] = useState("");
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    void backend.getBootstrap().then(setBootstrap).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [backend]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Unlisten[] = [];
    void backend.onRunEvent((event: RunEvent) => {
      if (event.runId !== activeRunId.current) return;
      setProgress({
        completed: event.completedTasks,
        total: event.totalTasks,
      });
      if (event.kind === "task_started") {
        setCurrentTask(event.taskId ?? "");
      }
      if (event.kind === "run_finished") {
        navigate(`/results/${event.runId}`);
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    });
    void backend.onRunError((event) => {
      if (event.runId === activeRunId.current) {
        setError(`运行中断：${event.message}。该次不会按能力失败计分。`);
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    });
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [backend, navigate]);

  useEffect(() => {
    if (!run) return;
    const timer = window.setInterval(() => {
      void backend.getRunDetail(run.id).then((detail) => {
        if (!detail) return;
        setProgress({
          completed: detail.run.completedTasks,
          total: detail.run.totalTasks,
        });
        if (
          detail.run.status === "completed" ||
          detail.run.status === "cancelled" ||
          detail.run.status === "interrupted"
        ) {
          navigate(`/results/${run.id}`);
        }
      }).catch(() => undefined);
    }, 2_000);
    return () => window.clearInterval(timer);
  }, [backend, navigate, run]);

  if (!label) {
    return <main><h1>不支持的 CLI</h1><p>请返回首页重新选择。</p></main>;
  }
  const availability = bootstrap?.targets.find((item) => item.kind === kind);
  const missing = availability?.prerequisites.find((item) => !item.available);
  const needsLogin = availability?.authState === "needs_login";

  async function start() {
    setBusy(true);
    setError("");
    try {
      const created = await backend.startCliRun({
        target: {
          kind,
          reportedModel: model.trim() || "default",
          reasoningEffort: reasoningEffort || null,
        },
        mode: "quick",
      });
      activeRunId.current = created.id;
      setRun(created);
      setProgress({ completed: 0, total: created.totalTasks });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    if (!run) return;
    setBusy(true);
    try {
      await backend.cancelRun(run.id);
      setConfirmCancel(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  if (run) {
    return (
      <main className="run-page">
        <p className="eyebrow">{label} · 自动运行</p>
        <h1>正在完成本地微型项目</h1>
        <p>{progress.completed} / {progress.total} 已完成</p>
        <progress value={progress.completed} max={progress.total}>
          {progress.completed}/{progress.total}
        </progress>
        {currentTask && <p className="hint">当前任务：{currentTask}</p>}
        <p>可以最小化窗口；请不要关闭应用或所选 CLI 的登录会话。</p>
        {error && <p role="alert">{error}</p>}
        {confirmCancel ? (
          <div role="group" aria-label="确认取消">
            <p>取消会终止当前 CLI 进程树，本次记录为“已取消”，不计能力分。</p>
            <button type="button" disabled={busy} onClick={() => void cancel()}>
              确认取消
            </button>
            <button type="button" className="secondary" onClick={() => setConfirmCancel(false)}>
              继续运行
            </button>
          </div>
        ) : (
          <button type="button" className="danger" onClick={() => setConfirmCancel(true)}>
            取消运行
          </button>
        )}
      </main>
    );
  }

  return (
    <main className="run-page">
      <p className="eyebrow">CLI · 预计 30–60 分钟</p>
      <h1>{label}快速体检</h1>
      {!bootstrap ? (
        <p aria-busy="true">正在检查 CLI 与 Node.js…</p>
      ) : (
        <>
          <dl className="environment-list">
            <div><dt>CLI</dt><dd>{availability?.version ?? "未检测到"}</dd></div>
            <div>
              <dt>登录</dt>
              <dd>
                {needsLogin
                  ? "需要先登录"
                  : availability?.authState === "ready"
                    ? "CLI 已确认登录"
                    : "将在启动时复核"}
              </dd>
            </div>
            <div>
              <dt>本地验证器</dt>
              <dd>{missing ? `缺少 ${missing.name}` : "Node.js 可用"}</dd>
            </div>
            <div><dt>任务</dt><dd>2 个隔离的 JavaScript 微型项目</dd></div>
          </dl>
          <p>
            工具不会读取或保存登录凭据，也不会改写你的真实项目。
            CLI 调用产生的额度费用由你的订阅承担。
          </p>
          <label>
            固定模型（可选）
            <input
              autoComplete="off"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder={kind === "codex_cli" ? "例如 gpt-5.4" : "例如 sonnet"}
            />
            <small className="hint">
              留空会测试 CLI 的默认路由，并在记录中明确标为 default。
            </small>
          </label>
          <label>
            推理档位（可选）
            <select
              value={reasoningEffort}
              onChange={(event) => setReasoningEffort(event.target.value)}
            >
              <option value="">CLI 默认</option>
              <option value="low">低</option>
              <option value="medium">中</option>
              <option value="high">高</option>
            </select>
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={acceptedCost}
              onChange={(event) => setAcceptedCost(event.target.checked)}
            />
            我了解这会消耗自己的订阅额度
          </label>
          {error && <p role="alert">{error}</p>}
          <button
            type="button"
            disabled={
              busy ||
              !acceptedCost ||
              !availability?.installed ||
              needsLogin ||
              Boolean(missing)
            }
            onClick={() => void start()}
          >
            {busy ? "正在启动…" : "开始 2 个任务"}
          </button>
        </>
      )}
    </main>
  );
}
```

The polling fallback is deliberate: a desktop webview may subscribe a few
milliseconds after the first event. Events provide responsiveness; persisted
state remains authoritative.

- [ ] **Step 4: Wire the real page into routing**

In `routes.tsx`, import `CliRunPage` from `../pages/CliRunPage` and remove it from
the placeholder import. Delete its placeholder export.

- [ ] **Step 5: Run frontend tests and a fake-process desktop test**

Run:

```powershell
npm test
npm run build
cargo test --workspace
```

Expected: CLI UI tests pass; Rust tests prove cancellation and process-tree
handling without starting Codex or Claude.

- [ ] **Step 6: Commit**

```powershell
git add apps/desktop
git commit -m "feat: add automatic CLI run experience"
```

---
### Task 17: Show Honest Results and Keep History Series Separate

**Files:**
- Create: `apps/desktop/src/pages/ResultPage.tsx`
- Create: `apps/desktop/src/pages/ResultPage.test.tsx`
- Create: `apps/desktop/src/pages/HistoryPage.tsx`
- Create: `apps/desktop/src/pages/HistoryPage.test.ts`
- Create: `apps/desktop/src/components/CategoryBars.tsx`
- Modify: `apps/desktop/src/app/routes.tsx`
- Modify: `apps/desktop/src/pages/PlaceholderPages.tsx`

**Interfaces:**
- Consumes: `getRunDetail` and `listRuns`.
- Produces: objective result cards, invalid-sample explanations, category
  visualization, and history grouped by an exact comparability key.

This task intentionally does not implement a degradation verdict. Frozen
personal baselines, paired comparison, MAD thresholds, and confidence
calibration belong to the separately specified v0.5 phase.

- [ ] **Step 1: Write the failing result-semantics test**

Create `apps/desktop/src/pages/ResultPage.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type { Backend, RunDetail } from "../api/backend";
import { ResultPage } from "./ResultPage";

const detail: RunDetail = {
  run: {
    id: "a8ecbc64-9160-448d-9426-e21c6839d219",
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
      suiteId: "client-quick",
      suiteVersion: "1.0.0",
      suiteContentSha256: "e".repeat(64),
      scoringRuleVersion: "ability-v1",
      resumed: false,
    },
    score: {
      abilityScore: 62.5,
      passedTasks: 5,
      validTasks: 8,
      totalTasks: 8,
      categoryScores: {
        instruction_following: 66.7,
        logic: 66.7,
        code_review: 50,
      },
    },
  },
  taskResults: [],
};

test("shows objective evidence without an IQ or premature degradation verdict", async () => {
  const backend = {
    getRunDetail: async () => detail,
  } as Backend;
  render(
    <MemoryRouter initialEntries={[`/results/${detail.run.id}`]}>
      <BackendProvider backend={backend}>
        <Routes>
          <Route path="/results/:runId" element={<ResultPage />} />
        </Routes>
      </BackendProvider>
    </MemoryRouter>,
  );

  expect(
    await screen.findByRole("heading", { name: "本次体检结果" }),
  ).toBeInTheDocument();
  expect(screen.getByText("62.5")).toBeInTheDocument();
  expect(screen.getByText("有效题目 8 / 8")).toBeInTheDocument();
  expect(screen.getByText(/v0.2 不生成“降智”结论/)).toBeInTheDocument();
  expect(screen.queryByText(/检测到明显下降|发现疑似下降/)).not.toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: /IQ/i })).not.toBeInTheDocument();
});
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
npm test -- --run src/pages/ResultPage.test.tsx
```

Expected: FAIL because the real result page does not exist.

- [ ] **Step 3: Implement the category visualization**

Create `apps/desktop/src/components/CategoryBars.tsx`:

```tsx
import type { Category } from "../api/backend";

const labels: Record<Category, string> = {
  instruction_following: "指令遵循",
  logic: "逻辑推理",
  code_review: "代码审查",
  cli_coding: "CLI 编码",
};

export function CategoryBars({
  scores,
}: {
  scores: Partial<Record<Category, number>>;
}) {
  return (
    <div className="category-bars" aria-label="各能力分类得分">
      {(Object.entries(scores) as Array<[Category, number]>).map(
        ([category, score]) => (
          <div className="category-row" key={category}>
            <span>{labels[category]}</span>
            <div className="bar-track" aria-hidden="true">
              <span style={{ width: `${Math.max(0, Math.min(100, score))}%` }} />
            </div>
            <strong>{score.toFixed(1)}</strong>
          </div>
        ),
      )}
    </div>
  );
}
```

- [ ] **Step 4: Implement result interpretation**

Create `apps/desktop/src/pages/ResultPage.tsx`:

```tsx
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type {
  FailureKind,
  RunDetail,
  TargetKind,
} from "../api/backend";
import { CategoryBars } from "../components/CategoryBars";

const targetLabels: Record<TargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

const failureLabels: Partial<Record<FailureKind, string>> = {
  cli_missing: "CLI 未安装",
  runtime_missing: "本地验证器环境缺失",
  auth_expired: "登录失效",
  quota_exhausted: "额度不足",
  network: "网络异常",
  user_cancelled: "用户取消",
  app_interrupted: "应用中断",
  infrastructure_timeout: "基础设施超时",
  agent_budget_exceeded: "代理在固定预算内未完成",
  verifier_error: "验证器异常",
  wrong_answer: "答案或代码未通过",
};

export function ResultPage() {
  const { runId = "" } = useParams();
  const backend = useBackend();
  const [detail, setDetail] = useState<RunDetail | null | undefined>();
  const [error, setError] = useState("");

  useEffect(() => {
    void backend.getRunDetail(runId).then(setDetail).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [backend, runId]);

  if (error) return <main><h1>无法读取结果</h1><p role="alert">{error}</p></main>;
  if (detail === undefined) return <main aria-busy="true"><p>正在整理结果…</p></main>;
  if (detail === null) return <main><h1>没有找到这次体检</h1></main>;

  const { run, taskResults } = detail;
  const score = run.score;
  const modelLabel = run.target.reportedModel === "default"
    ? "默认路由（未固定模型）"
    : run.target.reportedModel;
  const invalidCount = taskResults.filter(
    (result) => result.outcome === "invalid",
  ).length;
  const heading = run.status === "cancelled"
    ? "本次体检已取消"
    : run.status === "interrupted"
      ? "本次体检被中断"
      : "本次体检结果";

  return (
    <main className="result-page">
      <p className="eyebrow">
        {targetLabels[run.target.kind]} · {modelLabel}
      </p>
      <h1>{heading}</h1>
      <aside className="notice">
        v0.2 不生成“降智”结论。它保存可复核的分数、环境和历史；
        个人基线、配对比较与可信度判断在 v0.5 经真实试运行校准后加入。
      </aside>

      {score ? (
        <>
          <section className="score-grid" aria-label="本次得分摘要">
            <article>
              <span>能力表现分</span>
              <strong>{score.abilityScore.toFixed(1)}</strong>
              <small>0–100，本题包内的客观通过表现，不是 IQ</small>
            </article>
            <article>
              <span>原始通过</span>
              <strong>{score.passedTasks} / {score.totalTasks}</strong>
              <small>有效题目 {score.validTasks} / {score.totalTasks}</small>
            </article>
            <article>
              <span>运行质量</span>
              <strong>{invalidCount === 0 ? "完整" : `${invalidCount} 题无效`}</strong>
              <small>无效基础设施样本不按能力失败计分</small>
            </article>
          </section>
          <section>
            <h2>能力分类</h2>
            <CategoryBars scores={score.categoryScores} />
          </section>
        </>
      ) : (
        <section className="notice">
          本次没有可计算的能力分。取消、网络、登录、额度和验证器问题不会被伪装成能力失败。
        </section>
      )}

      {taskResults.length > 0 && (
        <section>
          <h2>逐题状态</h2>
          <ul className="task-results">
            {taskResults.map((result) => (
              <li key={result.taskId}>
                <span>{result.taskId}</span>
                <strong>{result.outcome}</strong>
                {result.failureKind && (
                  <small>{failureLabels[result.failureKind] ?? result.failureKind}</small>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      <div className="button-row">
        <Link className="button" to="/">再测一次</Link>
        <Link className="button secondary" to="/history">查看历史</Link>
      </div>
    </main>
  );
}
```

- [ ] **Step 5: Implement strictly separated history series**

Create `apps/desktop/src/pages/HistoryPage.tsx`:

```tsx
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useBackend } from "../api/BackendContext";
import type { RunRecord, TargetKind } from "../api/backend";

const labels: Record<TargetKind, string> = {
  chat_gpt_client: "ChatGPT 客户端",
  claude_client: "Claude 客户端",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
};

export function comparableSeriesKey(run: RunRecord) {
  return [
    run.target.kind,
    run.target.reportedModel.trim(),
    run.target.reasoningEffort ?? "",
    run.mode,
    run.suiteId,
    run.suiteVersion,
    run.environment.suiteContentSha256,
    run.environment.scoringRuleVersion,
    run.environment.osFamily,
    run.environment.osVersion,
    run.environment.appVersion,
    run.environment.cliVersion ?? "",
    run.environment.verifierRuntimeVersion ?? "",
    run.environment.resumed ? "resumed" : "clean",
  ].join("\u001f");
}

export function HistoryPage() {
  const backend = useBackend();
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    void backend.listRuns().then(setRuns).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [backend]);

  const groups = useMemo(() => {
    const grouped = new Map<string, RunRecord[]>();
    runs.forEach((run) => {
      const key = comparableSeriesKey(run);
      grouped.set(key, [...(grouped.get(key) ?? []), run]);
    });
    return [...grouped.values()];
  }, [runs]);

  return (
    <main>
      <p className="eyebrow">同对象、同模型、同设置、同题包分别展示</p>
      <h1>历史记录</h1>
      <p>v0.2 不跨系列求平均，也不根据少量历史自动下“降智”结论。</p>
      {error && <p role="alert">{error}</p>}
      {groups.length === 0 && !error && <p>还没有体检记录。</p>}
      <div className="history-groups">
        {groups.map((group) => {
          const first = group[0];
          const modelLabel = first.target.reportedModel === "default"
            ? "默认路由（未固定模型）"
            : first.target.reportedModel;
          return (
            <section key={comparableSeriesKey(first)} className="history-group">
              <h2>{labels[first.target.kind]} · {modelLabel}</h2>
              <p>
                题包 {first.suiteVersion} · {group.length} 次记录
                {first.environment.resumed ? " · 恢复运行" : ""}
              </p>
              <details>
                <summary>本组比较条件</summary>
                <ul>
                  <li>推理档位：{first.target.reasoningEffort || "未记录"}</li>
                  <li>
                    系统：{first.environment.osFamily} {first.environment.osVersion}
                  </li>
                  <li>应用：{first.environment.appVersion}</li>
                  {first.environment.cliVersion && (
                    <li>CLI：{first.environment.cliVersion}</li>
                  )}
                  {first.environment.verifierRuntimeVersion && (
                    <li>验证器：{first.environment.verifierRuntimeVersion}</li>
                  )}
                  <li>
                    题包哈希：{first.environment.suiteContentSha256.slice(0, 12)}
                  </li>
                  <li>评分规则：{first.environment.scoringRuleVersion}</li>
                </ul>
              </details>
              <ol>
                {group.map((run) => (
                  <li key={run.id}>
                    <time dateTime={run.startedAt}>
                      {new Date(run.startedAt).toLocaleString()}
                    </time>
                    <span>
                      {run.score
                        ? `${run.score.abilityScore.toFixed(1)} 分`
                        : run.status}
                    </span>
                    <Link to={`/results/${run.id}`}>查看</Link>
                  </li>
                ))}
              </ol>
            </section>
          );
        })}
      </div>
    </main>
  );
}
```

Create `apps/desktop/src/pages/HistoryPage.test.ts`:

```ts
import { describe, expect, test } from "vitest";
import type { RunRecord } from "../api/backend";
import { comparableSeriesKey } from "./HistoryPage";

const clientRun: RunRecord = {
  id: "10000000-0000-4000-8000-000000000001",
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
    verifierRuntimeVersion: null,
    suiteId: "client-quick",
    suiteVersion: "1.0.0",
    suiteContentSha256: "a".repeat(64),
    scoringRuleVersion: "ability-v1",
    resumed: false,
  },
  score: null,
};

const cliRun: RunRecord = {
  ...clientRun,
  id: "20000000-0000-4000-8000-000000000001",
  target: {
    kind: "codex_cli",
    reportedModel: "codex-current",
    reasoningEffort: "high",
  },
  suiteId: "cli-quick",
  totalTasks: 2,
  completedTasks: 2,
  environment: {
    ...clientRun.environment,
    cliVersion: "codex 1.0.0",
    verifierRuntimeVersion: "node v22.0.0",
    suiteId: "cli-quick",
    suiteContentSha256: "b".repeat(64),
  },
};

describe("comparableSeriesKey", () => {
  test("keeps ChatGPT and Claude client history separate", () => {
    const claude: RunRecord = {
      ...clientRun,
      target: { ...clientRun.target, kind: "claude_client" },
    };
    expect(comparableSeriesKey(clientRun)).not.toBe(comparableSeriesKey(claude));
  });

  test("separates every reproducibility-affecting CLI field", () => {
    const variants: RunRecord[] = [
      cliRun,
      {
        ...cliRun,
        target: { ...cliRun.target, reportedModel: "codex-next" },
      },
      {
        ...cliRun,
        target: { ...cliRun.target, reasoningEffort: "medium" },
      },
      {
        ...cliRun,
        mode: "deep",
      },
      {
        ...cliRun,
        suiteId: "cli-quick-next",
      },
      {
        ...cliRun,
        suiteVersion: "1.0.1",
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          cliVersion: "codex 1.1.0",
        },
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          verifierRuntimeVersion: "node v24.0.0",
        },
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          suiteContentSha256: "c".repeat(64),
        },
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          scoringRuleVersion: "ability-v2",
        },
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          osFamily: "Linux",
        },
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          osVersion: "10",
        },
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          appVersion: "0.2.1",
        },
      },
      {
        ...cliRun,
        environment: {
          ...cliRun.environment,
          resumed: true,
        },
      },
    ];
    expect(new Set(variants.map(comparableSeriesKey)).size).toBe(variants.length);
  });
});
```

In `routes.tsx`, import `HistoryPage` and `ResultPage` from their real files,
remove them from the placeholder import, then delete
`PlaceholderPages.tsx` after its final exports are gone.

- [ ] **Step 6: Run frontend regression tests**

Run:

```powershell
npm test
npm run build
```

Expected: result semantics pass, no UI presents the number as IQ, and
incomparable targets/configurations never share a history group.

- [ ] **Step 7: Commit**

```powershell
git add apps/desktop
git commit -m "feat: explain objective results and separate history"
```

---
### Task 18: Export an Allowlisted, Redaction-Checked Static Report

**Files:**
- Create: `crates/ability-core/src/report.rs`
- Create: `crates/ability-core/tests/report.rs`
- Create: `crates/ability-core/tests/report_schema.rs`
- Create: `schemas/public-report.schema.json`
- Modify: `crates/ability-core/Cargo.toml`
- Modify: `crates/ability-core/src/lib.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/api/backend.ts`
- Modify: `apps/desktop/src/api/tauriBackend.ts`
- Modify: `apps/desktop/src/pages/ResultPage.tsx`

**Interfaces:**
- Consumes: a run and its task results.
- Produces: a new anonymous report ID, strictly allowlisted JSON, and a
  self-contained static HTML report selected by the user.

- [ ] **Step 1: Write failing privacy tests**

Create `crates/ability-core/tests/report.rs`:

```rust
use ability_core::{
    build_public_report, render_public_report_html, Category,
    EnvironmentFingerprint, FailureKind, ReportError, RunMode, RunRecord,
    RunStatus, ScoreSummary, TargetKind, TargetSelection, TaskOutcome,
    TaskResult,
};
use std::collections::BTreeMap;

fn sample_run(model: &str) -> RunRecord {
    let mut run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::CodexCli,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
        },
        RunMode::Quick,
        "cli-quick".into(),
        "1.0.0".into(),
        2,
        EnvironmentFingerprint {
            os_family: "Windows".into(),
            os_version: "11 Pro 22631".into(),
            app_version: "0.2.0".into(),
            cli_version: Some("codex 1.2.3".into()),
            verifier_runtime_version: Some("node v22.0.0".into()),
            suite_id: "cli-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "f".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    );
    run.status = RunStatus::Completed;
    run.completed_tasks = 2;
    run.score = Some(ScoreSummary {
        ability_score: 50.0,
        passed_tasks: 1,
        valid_tasks: 2,
        total_tasks: 2,
        category_scores: BTreeMap::from([(Category::CliCoding, 50.0)]),
    });
    run
}

#[test]
fn public_report_never_serializes_raw_detail_or_artifact_paths() {
    let run = sample_run("CLI current");
    let tasks = vec![TaskResult {
        run_id: run.id,
        task_id: "one".into(),
        category: Category::CliCoding,
        outcome: TaskOutcome::Invalid,
        score: None,
        failure_kind: Some(FailureKind::Network),
        duration_ms: 1_000,
        answer_rel_path: Some("runs/id/logs/secret.log".into()),
        detail: "secret raw answer from C:\\Users\\Alice".into(),
    }];
    let report = build_public_report(&run, &tasks).unwrap();
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("secret raw answer"));
    assert!(!json.contains("runs/id/logs"));
    assert!(!json.contains("Alice"));
    assert!(!json.contains("11 Pro 22631"));
    assert!(json.contains("\"osFamily\":\"Windows\""));
    assert!(json.contains("\"interpretationStatus\":\"not_evaluated\""));
}

#[test]
fn suspicious_free_text_blocks_export_instead_of_guessing_redaction() {
    for model in [
        "sk-ant-api03-not-a-real-token",
        r#"C:\Users\Alice\model"#,
        r#"D:\work\private\model"#,
        r#"\\DESKTOP\share\model"#,
        "Bearer abcdefghijklmnopqrstuvwxyz",
        "alice@example.com",
    ] {
        assert!(matches!(
            build_public_report(&sample_run(model), &[]),
            Err(ReportError::SensitiveText(_))
        ));
    }
}

#[test]
fn html_is_self_contained_and_escapes_free_text() {
    let report = build_public_report(&sample_run("<Model & Test>"), &[]).unwrap();
    let html = render_public_report_html(&report).unwrap();
    assert!(html.contains("&lt;Model &amp; Test&gt;"));
    assert!(html.contains("v0.2 不生成降智结论"));
    assert!(!html.contains("src=\"http"));
    assert!(!html.contains("href=\"http"));
}
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```powershell
cargo test -p ability-core --test report
```

Expected: FAIL with unresolved report functions and types.

- [ ] **Step 3: Implement the allowlist and scanner**

Create `crates/ability-core/src/report.rs`:

```rust
use crate::{
    Category, FailureKind, RunRecord, RunStatus, TargetKind, TaskOutcome,
    TaskResult,
};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicReport {
    pub schema_version: u32,
    pub report_id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub target: PublicTarget,
    pub environment: PublicEnvironment,
    pub result: PublicResult,
    pub methodology: PublicMethodology,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicTarget {
    pub kind: TargetKind,
    pub reported_model: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicEnvironment {
    pub os_family: String,
    pub app_version: String,
    pub cli_version: Option<String>,
    pub verifier_runtime_version: Option<String>,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_content_sha256: String,
    pub scoring_rule_version: String,
    pub resumed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicResult {
    pub run_status: RunStatus,
    pub ability_score: Option<f64>,
    pub passed_tasks: u32,
    pub valid_tasks: u32,
    pub total_tasks: u32,
    pub category_scores: BTreeMap<Category, f64>,
    pub outcome_counts: BTreeMap<String, u32>,
    pub failure_counts: BTreeMap<FailureKind, u32>,
    pub total_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicMethodology {
    pub interpretation_status: String,
    pub statement: String,
}

#[derive(Debug, Error)]
pub enum ReportError {
    #[error("report contains sensitive-looking text in {0}")]
    SensitiveText(String),
    #[error("report JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn build_public_report(
    run: &RunRecord,
    tasks: &[TaskResult],
) -> Result<PublicReport, ReportError> {
    let score = run.score.as_ref();
    let mut outcome_counts = BTreeMap::new();
    let mut failure_counts = BTreeMap::new();
    for task in tasks {
        *outcome_counts
            .entry(outcome_name(task.outcome).to_owned())
            .or_insert(0) += 1;
        if let Some(failure) = task.failure_kind {
            *failure_counts.entry(failure).or_insert(0) += 1;
        }
    }
    let report = PublicReport {
        schema_version: 1,
        report_id: Uuid::new_v4(),
        generated_at: Utc::now(),
        target: PublicTarget {
            kind: run.target.kind,
            reported_model: run.target.reported_model.trim().to_owned(),
            reasoning_effort: run.target.reasoning_effort.clone(),
        },
        environment: PublicEnvironment {
            os_family: run.environment.os_family.clone(),
            app_version: run.environment.app_version.clone(),
            cli_version: run.environment.cli_version.clone(),
            verifier_runtime_version: run
                .environment
                .verifier_runtime_version
                .clone(),
            suite_id: run.suite_id.clone(),
            suite_version: run.suite_version.clone(),
            suite_content_sha256: run.environment.suite_content_sha256.clone(),
            scoring_rule_version: run.environment.scoring_rule_version.clone(),
            resumed: run.environment.resumed,
        },
        result: PublicResult {
            run_status: run.status,
            ability_score: score.map(|value| value.ability_score),
            passed_tasks: score.map_or(0, |value| value.passed_tasks),
            valid_tasks: score.map_or(0, |value| value.valid_tasks),
            total_tasks: run.total_tasks,
            category_scores: score
                .map(|value| value.category_scores.clone())
                .unwrap_or_default(),
            outcome_counts,
            failure_counts,
            total_duration_ms: tasks.iter().map(|task| task.duration_ms).sum(),
        },
        methodology: PublicMethodology {
            interpretation_status: "not_evaluated".into(),
            statement: "v0.2 不生成降智结论；仅展示本题包的客观结果。".into(),
        },
    };
    validate_public_report(&report)?;
    Ok(report)
}

pub fn validate_public_report(report: &PublicReport) -> Result<(), ReportError> {
    let candidates = [
        ("reportedModel", report.target.reported_model.as_str()),
        (
            "reasoningEffort",
            report.target.reasoning_effort.as_deref().unwrap_or(""),
        ),
        ("osFamily", report.environment.os_family.as_str()),
        ("appVersion", report.environment.app_version.as_str()),
        (
            "cliVersion",
            report.environment.cli_version.as_deref().unwrap_or(""),
        ),
        (
            "verifierRuntimeVersion",
            report
                .environment
                .verifier_runtime_version
                .as_deref()
                .unwrap_or(""),
        ),
        ("suiteId", report.environment.suite_id.as_str()),
        ("suiteVersion", report.environment.suite_version.as_str()),
        ("statement", report.methodology.statement.as_str()),
    ];
    let sensitive = Regex::new(
        r"(?ix)
        (?:sk-(?:ant-)?[a-z0-9_-]{12,}) |
        (?:github_pat_[a-z0-9_]{12,}) |
        (?:gh[pousr]_[a-z0-9]{12,}) |
        (?:bearer\s+[a-z0-9._~+/-]{12,}) |
        (?:akia[a-z0-9]{16}) |
        (?:[a-z]:[\\/][^\r\n]+) |
        (?:\\\\[a-z0-9._-]+\\[^\r\n]+) |
        (?:/(?:home|users|tmp|var|opt)/[^\s]+) |
        (?:[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,})
        ",
    )
    .expect("static report scanner regex");
    for (field, value) in candidates {
        if sensitive.is_match(value) {
            return Err(ReportError::SensitiveText(field.into()));
        }
    }
    serde_json::to_vec(report)?;
    Ok(())
}

pub fn render_public_report_html(report: &PublicReport) -> Result<String, ReportError> {
    validate_public_report(report)?;
    let json = serde_json::to_string(report)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let score = report.result.ability_score
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "无有效分".into());
    Ok(format!(
        r#"<!doctype html><html lang="zh-CN"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>AI 能力雷达公开报告</title>
<style>body{{margin:0;background:#0b1220;color:#edf4ff;font:16px/1.6 system-ui,sans-serif}}
main{{max-width:850px;margin:auto;padding:48px 24px}}.card{{background:#111b2d;border:1px solid #33445f;border-radius:18px;padding:24px;margin:18px 0}}
.score{{font-size:64px;font-weight:800;color:#69d3c6}}small{{color:#aab8cb}}</style>
</head><body><main><p>AI 能力雷达 · 匿名公开报告</p>
<h1>{target} · {model}</h1><section class="card"><small>本题包能力表现分（不是 IQ）</small>
<div class="score">{score}</div><p>{passed}/{total} 题通过，{valid}/{total} 题有效</p></section>
<section class="card"><h2>解释边界</h2><p>{statement}</p></section>
<p><small>报告编号 {report_id} · 评分规则 {rule}</small></p>
<script type="application/json" id="ability-radar-report">{json}</script>
</main></body></html>"#,
        target = html_escape(&format!("{:?}", report.target.kind)),
        model = html_escape(&report.target.reported_model),
        score = score,
        passed = report.result.passed_tasks,
        valid = report.result.valid_tasks,
        total = report.result.total_tasks,
        statement = html_escape(&report.methodology.statement),
        report_id = report.report_id,
        rule = html_escape(&report.environment.scoring_rule_version),
        json = json,
    ))
}

pub fn public_report_sha256(contents: &str) -> String {
    format!("{:x}", Sha256::digest(contents.as_bytes()))
}

fn outcome_name(outcome: TaskOutcome) -> &'static str {
    match outcome {
        TaskOutcome::Passed => "passed",
        TaskOutcome::Failed => "failed",
        TaskOutcome::Invalid => "invalid",
        TaskOutcome::Cancelled => "cancelled",
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
```

Export the module from `ability-core/src/lib.rs`.

- [ ] **Step 4: Add the public JSON schema**

Merge the local-only validator into the existing `[dev-dependencies]` table in
`crates/ability-core/Cargo.toml`:

```toml
jsonschema = { version = "0.48", default-features = false }
```

Create `schemas/public-report.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "AI Ability Radar public report v1",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schemaVersion",
    "reportId",
    "generatedAt",
    "target",
    "environment",
    "result",
    "methodology"
  ],
  "properties": {
    "schemaVersion": {
      "const": 1
    },
    "reportId": {
      "type": "string",
      "format": "uuid"
    },
    "generatedAt": {
      "type": "string",
      "format": "date-time"
    },
    "target": {
      "type": "object",
      "additionalProperties": false,
      "required": ["kind", "reportedModel", "reasoningEffort"],
      "properties": {
        "kind": {
          "enum": [
            "chat_gpt_client",
            "claude_client",
            "codex_cli",
            "claude_code"
          ]
        },
        "reportedModel": {
          "type": "string",
          "minLength": 1
        },
        "reasoningEffort": {
          "type": ["string", "null"]
        }
      }
    },
    "environment": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "osFamily",
        "appVersion",
        "cliVersion",
        "verifierRuntimeVersion",
        "suiteId",
        "suiteVersion",
        "suiteContentSha256",
        "scoringRuleVersion",
        "resumed"
      ],
      "properties": {
        "osFamily": {
          "type": "string",
          "minLength": 1
        },
        "appVersion": {
          "type": "string",
          "minLength": 1
        },
        "cliVersion": {
          "type": ["string", "null"]
        },
        "verifierRuntimeVersion": {
          "type": ["string", "null"]
        },
        "suiteId": {
          "type": "string",
          "minLength": 1
        },
        "suiteVersion": {
          "type": "string",
          "minLength": 1
        },
        "suiteContentSha256": {
          "type": "string",
          "pattern": "^[0-9a-f]{64}$"
        },
        "scoringRuleVersion": {
          "type": "string",
          "minLength": 1
        },
        "resumed": {
          "type": "boolean"
        }
      }
    },
    "result": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "runStatus",
        "abilityScore",
        "passedTasks",
        "validTasks",
        "totalTasks",
        "categoryScores",
        "outcomeCounts",
        "failureCounts",
        "totalDurationMs"
      ],
      "properties": {
        "runStatus": {
          "enum": [
            "created",
            "running",
            "completed",
            "cancelled",
            "interrupted"
          ]
        },
        "abilityScore": {
          "type": ["number", "null"],
          "minimum": 0,
          "maximum": 100
        },
        "passedTasks": {
          "type": "integer",
          "minimum": 0
        },
        "validTasks": {
          "type": "integer",
          "minimum": 0
        },
        "totalTasks": {
          "type": "integer",
          "minimum": 0
        },
        "categoryScores": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "instruction_following": {
              "type": "number",
              "minimum": 0,
              "maximum": 100
            },
            "logic": {
              "type": "number",
              "minimum": 0,
              "maximum": 100
            },
            "code_review": {
              "type": "number",
              "minimum": 0,
              "maximum": 100
            },
            "cli_coding": {
              "type": "number",
              "minimum": 0,
              "maximum": 100
            }
          }
        },
        "outcomeCounts": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "passed": {
              "type": "integer",
              "minimum": 0
            },
            "failed": {
              "type": "integer",
              "minimum": 0
            },
            "invalid": {
              "type": "integer",
              "minimum": 0
            },
            "cancelled": {
              "type": "integer",
              "minimum": 0
            }
          }
        },
        "failureCounts": {
          "type": "object",
          "additionalProperties": false,
          "properties": {
            "cli_missing": {
              "type": "integer",
              "minimum": 0
            },
            "runtime_missing": {
              "type": "integer",
              "minimum": 0
            },
            "auth_expired": {
              "type": "integer",
              "minimum": 0
            },
            "quota_exhausted": {
              "type": "integer",
              "minimum": 0
            },
            "network": {
              "type": "integer",
              "minimum": 0
            },
            "user_cancelled": {
              "type": "integer",
              "minimum": 0
            },
            "app_interrupted": {
              "type": "integer",
              "minimum": 0
            },
            "infrastructure_timeout": {
              "type": "integer",
              "minimum": 0
            },
            "agent_budget_exceeded": {
              "type": "integer",
              "minimum": 0
            },
            "verifier_error": {
              "type": "integer",
              "minimum": 0
            },
            "wrong_answer": {
              "type": "integer",
              "minimum": 0
            }
          }
        },
        "totalDurationMs": {
          "type": "integer",
          "minimum": 0
        }
      }
    },
    "methodology": {
      "type": "object",
      "additionalProperties": false,
      "required": ["interpretationStatus", "statement"],
      "properties": {
        "interpretationStatus": {
          "const": "not_evaluated"
        },
        "statement": {
          "type": "string",
          "minLength": 1
        }
      }
    }
  }
}
```

Create `crates/ability-core/tests/report_schema.rs`:

```rust
use ability_core::{build_public_report, EnvironmentFingerprint, RunMode,
    RunRecord, TargetKind, TargetSelection};
use serde_json::{json, Value};

fn fixture_report() -> Value {
    let run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::ChatGptClient,
            reported_model: "GPT-X".into(),
            reasoning_effort: None,
        },
        RunMode::Quick,
        "client-quick".into(),
        "1.0.0".into(),
        8,
        EnvironmentFingerprint {
            os_family: "Windows".into(),
            os_version: "11".into(),
            app_version: "0.2.0".into(),
            cli_version: None,
            verifier_runtime_version: None,
            suite_id: "client-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "a".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    );
    serde_json::to_value(build_public_report(&run, &[]).unwrap()).unwrap()
}

fn schema() -> Value {
    serde_json::from_str(include_str!(
        "../../../schemas/public-report.schema.json"
    ))
    .unwrap()
}

#[test]
fn public_report_matches_the_committed_schema() {
    let schema = schema();
    jsonschema::meta::validate(&schema).unwrap();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    assert!(validator.is_valid(&fixture_report()));
}

#[test]
fn schema_rejects_unknown_sensitive_fields_and_invalid_formats() {
    let schema = schema();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();

    let mut with_raw_answer = fixture_report();
    with_raw_answer["result"]["rawAnswer"] = json!("private answer");
    assert!(!validator.is_valid(&with_raw_answer));

    let mut invalid_identity = fixture_report();
    invalid_identity["reportId"] = json!("not-a-uuid");
    invalid_identity["generatedAt"] = json!("17 July");
    assert!(!validator.is_valid(&invalid_identity));
}
```

- [ ] **Step 5: Add one dedicated export command**

Add `tauri-plugin-dialog = "2"` to the Tauri crate. Add
`ExportReportInput { run_id }` with camelCase serde fields. The command itself
must open the native save dialog so an IPC caller can never supply an arbitrary
filesystem destination. Implement:

```rust
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn export_public_report(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ExportReportInput,
) -> Result<Option<String>, String> {
    let run_id = parse_run_id(&input.run_id)?;
    let run = state.repository.get_run(run_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "没有找到这次体检".to_string())?;
    let tasks = state.repository.get_task_results(run_id)
        .map_err(|error| error.to_string())?;

    let run_key = run_id.simple().to_string();
    let selected = app
        .dialog()
        .file()
        .set_title("导出可分享报告")
        .add_filter("HTML report", &["html"])
        .set_file_name(format!("ability-radar-{}.html", &run_key[..8]))
        .blocking_save_file();
    let Some(destination) = selected else {
        return Ok(None);
    };
    let destination = destination
        .into_path()
        .map_err(|_| "仅支持保存到本地文件路径".to_string())?;
    let has_html_extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("html"));
    if !destination.is_absolute() || !has_html_extension {
        return Err("报告必须保存为用户选择的 .html 文件".into());
    }
    if destination
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("拒绝写入符号链接目标".into());
    }

    let report = ability_core::build_public_report(&run, &tasks)
        .map_err(|error| error.to_string())?;
    let html = ability_core::render_public_report_html(&report)
        .map_err(|error| error.to_string())?;
    std::fs::write(&destination, &html).map_err(|error| error.to_string())?;
    Ok(Some(report.report_id.to_string()))
}
```

Register the command and `.plugin(tauri_plugin_dialog::init())`. Keep the
webview capability at `["core:default"]`: neither the JavaScript dialog
permission nor the filesystem plugin is needed because the Rust command owns
both path selection and the write.

- [ ] **Step 6: Wire the save picker and export button**

Add to `Backend` and `tauriBackend`:

```ts
exportPublicReport(runId: string): Promise<string | null>;
```

The backend command opens the native picker with an HTML filter and default name
`ability-radar-<first-eight-run-id-characters>.html`; do not import a JavaScript
save API. After export, show the anonymous report ID in an
`aria-live="polite"` status. Treat a `null` response as a normal user
cancellation. Make the initial button text “检查并导出可分享报告”.

The initial button must reveal an inline `role="dialog"` review panel before it
opens the save picker. The panel shows the exact allowlist grouped as:

- report metadata: schema version, plus a new anonymous report ID and generation
  timestamp that will be created only after confirmation;
- test target: target kind, trimmed reported model, and optional reasoning
  effort;
- reproducibility: OS family only, app/CLI/Node versions, suite ID/version/hash,
  scoring-rule version, and resumed state;
- objective result: run status, ability/category scores when available,
  passed/valid/total counts, outcome/failure counts, and total duration;
- methodology: `not_evaluated` and the v0.2 no-verdict statement.

Beside it, show an explicit exclusion list: raw answers, prompts, CLI logs,
task-detail free text, username, hostname, OS build, absolute paths, credentials,
and destination path. Require a checkbox reading “我已检查以上公开字段” before
enabling “选择位置并导出”. Cancel closes the panel and does not call
`exportPublicReport`. Only the confirmed second button invokes the backend,
which then opens the native picker.

v0.2 performs no automatic upload or GitHub publication.

- [ ] **Step 7: Verify privacy and export**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-core --test report
cargo test --workspace
npm test
npm run build
```

Manually export a fixture whose local raw log contains a fake token and a
generic Windows user path. Expected: neither appears in the HTML; putting either
in the user-entered model field blocks export with the field name. Also verify
that opening and cancelling the field-review panel creates no file and opens no
save picker, while cancelling the native picker returns `null` and creates no
publication record.

- [ ] **Step 8: Commit**

```powershell
git add Cargo.lock crates schemas apps/desktop
git commit -m "feat: export privacy-checked static reports"
```

---

### Task 19: Recover Interrupted Runs and Add Destructive Data Controls

**Files:**
- Create: `crates/ability-core/tests/recovery.rs`
- Modify: `crates/ability-core/src/storage.rs`
- Modify: `crates/ability-core/src/orchestration.rs`
- Modify: `crates/ability-adapters/src/cli_run.rs`
- Modify: `apps/desktop/src-tauri/src/app_state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/api/backend.ts`
- Modify: `apps/desktop/src/api/tauriBackend.ts`
- Modify: `apps/desktop/src/pages/ManualRunPage.tsx`
- Modify: `apps/desktop/src/pages/CliRunPage.tsx`
- Modify: `apps/desktop/src/pages/HistoryPage.tsx`
- Modify: `apps/desktop/src/pages/ResultPage.tsx`

**Interfaces:**
- Consumes: interrupted checkpoints and user-confirmed deletion actions.
- Produces: explicit resumed runs (kept in a separate history series), safe remaining-task
  execution, delete-raw-only, delete-one-run, and delete-target-history commands.

- [ ] **Step 1: Write the failing recovery test**

Create `crates/ability-core/tests/recovery.rs`:

```rust
use ability_core::{
    EnvironmentFingerprint, ManualRunService, PackLoader, RunMode,
    RunRepository, RunStatus, TargetKind, TargetSelection,
};
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

#[test]
fn manual_run_resumes_at_the_next_checkpoint_and_is_marked_resumed() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    fs::create_dir_all(&pack_dir).unwrap();
    fs::write(pack_dir.join("one.txt"), "one").unwrap();
    fs::write(pack_dir.join("two.txt"), "two").unwrap();
    fs::write(
        pack_dir.join("manifest.json"),
        r#"{
          "schema_version":1,
          "id":"resume-pack",
          "version":"1.0.0",
          "title":"Resume",
          "target_kinds":["chat_gpt_client"],
          "tasks":[
            {"id":"one","category":"logic","prompt_file":"one.txt",
             "starter_dir":null,"time_budget_secs":60,"max_turns":1,
             "grader":{"type":"exact_text","expected":"one"}},
            {"id":"two","category":"logic","prompt_file":"two.txt",
             "starter_dir":null,"time_budget_secs":60,"max_turns":1,
             "grader":{"type":"exact_text","expected":"two"}}
          ]
        }"#,
    )
    .unwrap();
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repository =
        Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let first_service =
        ManualRunService::new(repository.clone(), dir.path().join("artifacts"));
    let run = first_service
        .start(
            pack.clone(),
            TargetSelection {
                kind: TargetKind::ChatGptClient,
                reported_model: "Model-X".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            EnvironmentFingerprint {
                os_family: "Windows".into(),
                os_version: "11".into(),
                app_version: "0.2.0".into(),
                cli_version: None,
                verifier_runtime_version: None,
                suite_id: "resume-pack".into(),
                suite_version: "1.0.0".into(),
                suite_content_sha256: pack.content_sha256.clone(),
                scoring_rule_version: "ability-v1".into(),
                resumed: false,
            },
        )
        .unwrap();
    first_service.submit_answer(run.id, "one", "one").unwrap();

    repository.mark_running_as_interrupted().unwrap();
    let restarted =
        ManualRunService::new(repository.clone(), dir.path().join("artifacts"));
    let resumed = restarted.resume(run.id, pack).unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    assert!(resumed.environment.resumed);
    assert_eq!(
        restarted.next_step(run.id).unwrap().unwrap().task_id,
        "two"
    );
    restarted.submit_answer(run.id, "two", "two").unwrap();
    assert_eq!(
        repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Completed
    );
}
```

- [ ] **Step 2: Run the test to verify failure**

Run:

```powershell
cargo test -p ability-core --test recovery
```

Expected: FAIL because `ManualRunService::resume` does not exist.

- [ ] **Step 3: Add repository recovery and deletion transactions**

Add to `RunRepository`:

```rust
pub fn resume_run(&self, run_id: Uuid) -> Result<RunRecord, StorageError> {
    let mut run = self
        .get_run(run_id)?
        .ok_or_else(|| StorageError::Enum(format!("missing run {run_id}")))?;
    if run.status != RunStatus::Interrupted {
        return Err(StorageError::Enum("run is not interrupted".into()));
    }
    run.status = RunStatus::Running;
    run.finished_at = None;
    run.environment.resumed = true;
    let mut connection = self.connection.lock();
    let transaction = connection.transaction()?;
    transaction.execute(
        "UPDATE runs SET status_json=?2,finished_at=NULL,environment_json=?3
         WHERE id=?1",
        params![
            run_id.to_string(),
            serde_json::to_string(&run.status)?,
            serde_json::to_string(&run.environment)?,
        ],
    )?;
    transaction.commit()?;
    Ok(run)
}

pub fn clear_artifact_references(&self, run_id: Uuid) -> Result<(), StorageError> {
    self.connection.lock().execute(
        "UPDATE task_results SET answer_rel_path=NULL WHERE run_id=?1",
        [run_id.to_string()],
    )?;
    Ok(())
}

pub fn delete_run(&self, run_id: Uuid) -> Result<bool, StorageError> {
    let mut connection = self.connection.lock();
    let transaction = connection.transaction()?;
    let changed =
        transaction.execute("DELETE FROM runs WHERE id=?1", [run_id.to_string()])?;
    transaction.execute(
        "DELETE FROM targets
         WHERE NOT EXISTS (
           SELECT 1 FROM runs WHERE runs.target_json=targets.target_json
         )",
        [],
    )?;
    transaction.execute(
        "DELETE FROM suite_versions
         WHERE NOT EXISTS (
           SELECT 1 FROM runs
           WHERE runs.suite_id=suite_versions.suite_id
             AND runs.suite_version=suite_versions.suite_version
         )",
        [],
    )?;
    transaction.commit()?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(changed == 1)
}
```

- [ ] **Step 4: Resume the manual service**

Add a `NotResumable(String)` variant to `RunServiceError`, then add:

```rust
pub fn resume(
    &self,
    run_id: Uuid,
    pack: Arc<LoadedPack>,
) -> Result<RunRecord, RunServiceError> {
    let stored = self
        .repository
        .get_run(run_id)?
        .ok_or(RunServiceError::RunNotFound(run_id))?;
    if !matches!(
        stored.target.kind,
        TargetKind::ChatGptClient | TargetKind::ClaudeClient
    ) || stored.suite_id != pack.manifest.id
        || stored.suite_version != pack.manifest.version
        || stored.environment.suite_content_sha256 != pack.content_sha256
    {
        return Err(RunServiceError::NotResumable(
            "target or task pack changed".into(),
        ));
    }
    let run = self.repository.resume_run(run_id)?;
    self.active
        .lock()
        .map_err(|_| RunServiceError::Poisoned)?
        .insert(
            run_id,
            ActiveManualRun {
                pack,
                task_started: Instant::now(),
            },
        );
    Ok(run)
}
```

- [ ] **Step 5: Make CLI resume skip durable checkpoints**

Add to `CliRunService`:

```rust
pub fn resume(
    &self,
    run_id: Uuid,
    pack: &LoadedPack,
) -> Result<RunRecord, CliRunError> {
    let stored = self
        .repository
        .get_run(run_id)?
        .ok_or_else(|| StorageError::Enum(format!("missing run {run_id}")))?;
    if !matches!(stored.target.kind, TargetKind::CodexCli | TargetKind::ClaudeCode)
        || stored.suite_id != pack.manifest.id
        || stored.suite_version != pack.manifest.version
        || stored.environment.suite_content_sha256 != pack.content_sha256
    {
        return Err(CliRunError::AdapterMismatch);
    }
    self.repository.resume_run(run_id).map_err(CliRunError::from)
}
```

At the start of `execute`, load checkpoint IDs:

```rust
let completed_ids = self
    .repository
    .get_task_results(run_id)?
    .into_iter()
    .map(|result| result.task_id)
    .collect::<std::collections::BTreeSet<_>>();
let mut completed_count = completed_ids.len() as u32;
```

At the top of the task loop:

```rust
if completed_ids.contains(&task.definition.id) {
    continue;
}
```

Use `completed_count` rather than `index` in progress events and increment it
after each newly saved result. A resumed CLI run therefore starts the first
uncheckpointed task in a fresh workspace; it never reruns a completed,
subscription-consuming task.

- [ ] **Step 6: Add narrow resume and delete commands**

Store the artifact root in `AppState`:

```rust
pub(crate) artifact_root: PathBuf,
```

Add DTO:

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunIdInput {
    pub run_id: String,
}
```

Add these commands:

```rust
#[tauri::command]
pub fn resume_manual_run(
    state: State<'_, AppState>,
    input: RunIdInput,
) -> Result<RunRecord, String> {
    let run_id = parse_run_id(&input.run_id)?;
    let stored = state
        .repository
        .get_run(run_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "没有找到这次体检".to_string())?;
    let current = environment(&state.client_pack, None, None);
    if stored.environment.os_family != current.os_family
        || stored.environment.os_version != current.os_version
        || stored.environment.app_version != current.app_version
    {
        return Err("系统或应用版本已变化；请开始一次新体检".into());
    }
    state
        .manual_runs
        .resume(run_id, state.client_pack.clone())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_raw_artifacts(
    state: State<'_, AppState>,
    input: RunIdInput,
) -> Result<(), String> {
    let run_id = parse_run_id(&input.run_id)?;
    remove_artifact_dir(&state.artifact_root, run_id)?;
    state
        .repository
        .clear_artifact_references(run_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_run(
    state: State<'_, AppState>,
    input: RunIdInput,
) -> Result<bool, String> {
    let run_id = parse_run_id(&input.run_id)?;
    remove_artifact_dir(&state.artifact_root, run_id)?;
    state
        .repository
        .delete_run(run_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_target_history(
    state: State<'_, AppState>,
    target: TargetKind,
) -> Result<u32, String> {
    let ids = state
        .repository
        .list_runs()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|run| run.target.kind == target)
        .map(|run| run.id)
        .collect::<Vec<_>>();
    for run_id in &ids {
        remove_artifact_dir(&state.artifact_root, *run_id)?;
        state
            .repository
            .delete_run(*run_id)
            .map_err(|error| error.to_string())?;
    }
    Ok(ids.len() as u32)
}

fn remove_artifact_dir(root: &std::path::Path, run_id: Uuid) -> Result<(), String> {
    let runs_root = root.join("runs");
    let destination = runs_root.join(run_id.to_string());
    if destination.parent() != Some(runs_root.as_path()) {
        return Err("拒绝删除应用数据目录之外的路径".into());
    }
    if destination.exists() {
        let metadata =
            std::fs::symlink_metadata(&destination).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("拒绝删除符号链接目录".into());
        }
        std::fs::remove_dir_all(destination).map_err(|error| error.to_string())?;
    }
    Ok(())
}
```

For CLI resume, move the already-reviewed event and execution spawn body out of
`start_cli_run` into this private helper:

```rust
fn spawn_cli_execution(
    app: AppHandle,
    state: &AppState,
    run_id: Uuid,
    adapter: std::sync::Arc<dyn ability_adapters::AgentAdapter>,
) {
    let cancellation = CancellationToken::new();
    state
        .cancellations
        .lock()
        .insert(run_id, cancellation.clone());
    let service = state.cli_runs.clone();
    let pack = state.cli_pack.clone();
    let verifier = state.verifier.clone();
    let repository = state.repository.clone();
    let cancellations = state.cancellations.clone();
    let (sender, mut receiver) = mpsc::unbounded_channel::<RunEvent>();
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let _ = event_app.emit("run://event", event);
        }
    });
    tauri::async_runtime::spawn(async move {
        if let Err(error) = service
            .execute(
                run_id,
                pack,
                adapter,
                verifier,
                cancellation,
                sender,
            )
            .await
        {
            let _ = repository.finish_without_score(
                run_id,
                ability_core::RunStatus::Interrupted,
            );
            let _ = app.emit(
                "run://error",
                RunErrorEvent {
                    run_id: run_id.to_string(),
                    message: error.to_string(),
                },
            );
        }
        cancellations.lock().remove(&run_id);
    });
}
```

Replace everything in `start_cli_run` after `prepare(...)` with:

```rust
spawn_cli_execution(app, state.inner(), run.id, adapter);
Ok(run)
```

Add the exact resume command:

```rust
#[tauri::command]
pub async fn resume_cli_run(
    app: AppHandle,
    state: State<'_, AppState>,
    input: RunIdInput,
) -> Result<RunRecord, String> {
    let run_id = parse_run_id(&input.run_id)?;
    let stored = state
        .repository
        .get_run(run_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "没有找到这次体检".to_string())?;
    let adapter = state
        .adapters
        .get(&stored.target.kind)
        .cloned()
        .ok_or_else(|| "该 CLI 暂不支持".to_string())?;
    let availability = adapter.detect().await;
    if !availability.installed {
        return Err("未找到原 CLI，请先恢复安装和登录".into());
    }
    if availability.auth_state == AuthState::NeedsLogin {
        return Err("原 CLI 已退出登录，请先重新登录".into());
    }
    let node = probe_node(state.runner.clone()).await;
    if !node.available {
        return Err("恢复运行需要 Node.js 22 或 24 LTS".into());
    }
    if availability.version != stored.environment.cli_version
        || node.version != stored.environment.verifier_runtime_version
    {
        return Err(
            "CLI 或 Node.js 版本已变化；为保证结果可复现，请开始一次新体检"
                .into(),
        );
    }
    let run = state
        .cli_runs
        .resume(run_id, &state.cli_pack)
        .map_err(|error| error.to_string())?;
    spawn_cli_execution(app, state.inner(), run.id, adapter);
    Ok(run)
}
```

Register all five commands. They still require no general filesystem or shell
capability.

- [ ] **Step 7: Extend the frontend boundary and recovery routes**

Add to `Backend` and `tauriBackend`:

```ts
resumeManualRun(runId: string): Promise<RunRecord>;
resumeCliRun(runId: string): Promise<RunRecord>;
deleteRawArtifacts(runId: string): Promise<void>;
deleteRun(runId: string): Promise<boolean>;
deleteTargetHistory(target: TargetKind): Promise<number>;
```

Use the same `{ input: { runId } }` shape for the first four commands and
`{ target }` for target-history deletion.

In `HistoryPage`:

- show “继续” only for `interrupted` runs;
- route client resumes to `/manual/<kind>?resume=<runId>`;
- route CLI resumes to `/cli/<kind>?resume=<runId>`;
- put “删除该测试对象全部历史” behind a two-step inline confirmation that names
  the exact target and record count.

In `ManualRunPage`, read `resume` with `useSearchParams`; on first load call
`resumeManualRun`, then `nextManualStep`, rather than creating a new run.

In `CliRunPage`, read `resume`; keep the cost acknowledgement and environment
preflight visible, change the button text to “继续剩余任务”, and call
`resumeCliRun`.

In `ResultPage`, add a “数据管理” disclosure with two separate confirmations:

- “只删除原始回答和 CLI 日志，保留分数”;
- “从应用中删除本次记录和原始数据”.

After deleting a whole run, navigate to `/history`. Never combine these into one
ambiguous delete button.

- [ ] **Step 8: Verify recovery and destructive boundaries**

Run:

```powershell
cargo fmt --all --check
cargo test -p ability-core --test recovery
cargo test --workspace
npm test
npm run build
```

Manual Windows check:

1. complete one client task, force-close the app, reopen, and resume at task 2;
2. complete one CLI task, force-close, reopen, and verify task 1 is not rerun;
3. confirm resumed runs display the resumed flag and remain a separate history series;
4. delete raw artifacts and confirm the score remains;
5. delete the run and confirm both DB rows and the UUID-scoped directory are gone.

- [ ] **Step 9: Commit**

```powershell
git add crates apps/desktop
git commit -m "feat: recover runs and manage local data"
```

---

### Task 20: Add Raw-Data Retention and Full Local Backup

**Files:**
- Create: `crates/ability-core/migrations/0002_settings.sql`
- Create: `apps/desktop/src-tauri/src/data_management.rs`
- Create: `apps/desktop/src-tauri/src/data_management_tests.rs`
- Modify: `crates/ability-core/src/storage.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src-tauri/src/app_state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/dto.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/api/backend.ts`
- Modify: `apps/desktop/src/api/tauriBackend.ts`
- Modify: `apps/desktop/src/pages/HistoryPage.tsx`

**Interfaces:**
- Consumes: the live SQLite connection and UUID-scoped artifact tree.
- Produces: a default-forever raw retention policy, immediate pruning after an
  explicit policy change, and a user-selected unencrypted ZIP backup.

- [ ] **Step 1: Add settings migration and repository methods**

Create `crates/ability-core/migrations/0002_settings.sql`:

```sql
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL
);

INSERT OR IGNORE INTO settings(key,value_json)
VALUES ('raw_retention_days', 'null');

CREATE TABLE IF NOT EXISTS publications (
  report_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  exported_at TEXT NOT NULL,
  report_sha256 TEXT NOT NULL,
  destination_kind TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (2);
```

Execute it second in `RunRepository::open`, immediately after
`0001_init.sql`.

Add:

```rust
pub fn raw_retention_days(&self) -> Result<Option<u32>, StorageError> {
    let value: String = self.connection.lock().query_row(
        "SELECT value_json FROM settings WHERE key='raw_retention_days'",
        [],
        |row| row.get(0),
    )?;
    serde_json::from_str(&value).map_err(StorageError::from)
}

pub fn set_raw_retention_days(
    &self,
    days: Option<u32>,
) -> Result<(), StorageError> {
    self.connection.lock().execute(
        "INSERT INTO settings(key,value_json)
         VALUES ('raw_retention_days',?1)
         ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
        [serde_json::to_string(&days)?],
    )?;
    Ok(())
}

pub fn backup_to(&self, destination: &Path) -> Result<(), StorageError> {
    let source = self.connection.lock();
    let mut target = Connection::open(destination)?;
    let backup = rusqlite::backup::Backup::new(&source, &mut target)?;
    backup.run_to_completion(
        128,
        std::time::Duration::from_millis(10),
        None,
    )?;
    Ok(())
}

pub fn record_publication(
    &self,
    report_id: Uuid,
    run_id: Uuid,
    report_sha256: &str,
    destination_kind: &str,
) -> Result<(), StorageError> {
    self.connection.lock().execute(
        "INSERT INTO publications(
           report_id,run_id,exported_at,report_sha256,destination_kind
         ) VALUES (?1,?2,?3,?4,?5)",
        params![
            report_id.to_string(),
            run_id.to_string(),
            Utc::now().to_rfc3339(),
            report_sha256,
            destination_kind,
        ],
    )?;
    Ok(())
}
```

- [ ] **Step 2: Add ZIP dependency and write failing management tests**

Add to `apps/desktop/src-tauri/Cargo.toml`:

```toml
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
zip = { version = "8.6", default-features = false, features = ["deflate"] }
```

Create `apps/desktop/src-tauri/src/data_management_tests.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::data_management::{collect_backup_files, retention_expired};
    use chrono::{Duration, TimeZone, Utc};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn retention_uses_finished_time_and_forever_never_expires() {
        let finished = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let now = finished + Duration::days(31);
        assert!(retention_expired(finished, now, Some(30)));
        assert!(!retention_expired(finished, now, None));
    }

    #[test]
    fn backup_collector_emits_only_uuid_scoped_relative_names() {
        let dir = tempdir().unwrap();
        let id = "10000000-0000-4000-8000-000000000001";
        fs::create_dir_all(dir.path().join("runs").join(id)).unwrap();
        fs::write(
            dir.path().join("runs").join(id).join("answer.txt"),
            "answer",
        )
        .unwrap();
        let files = collect_backup_files(dir.path()).unwrap();
        assert_eq!(
            files[0].0,
            format!("artifacts/runs/{id}/answer.txt"),
        );
    }

    #[test]
    fn backup_collector_rejects_non_uuid_run_directories() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("runs/not-a-run")).unwrap();
        assert!(collect_backup_files(dir.path()).is_err());
    }
}
```

Add `tempfile = "3"` to Tauri crate dev dependencies.

- [ ] **Step 3: Implement pruning and backup packaging**

Create `apps/desktop/src-tauri/src/data_management.rs`:

```rust
use ability_core::{RunRepository, StorageError};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZIP operation failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("manifest JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("artifact tree contains a symbolic link: {0}")]
    SymbolicLink(String),
    #[error("artifact tree contains an unexpected entry: {0}")]
    UnexpectedEntry(String),
}

pub fn retention_expired(
    finished: DateTime<Utc>,
    now: DateTime<Utc>,
    days: Option<u32>,
) -> bool {
    days.is_some_and(|days| finished + Duration::days(days.into()) < now)
}

pub fn prune_expired_artifacts(
    repository: &RunRepository,
    artifact_root: &Path,
    now: DateTime<Utc>,
) -> Result<u32, DataError> {
    let days = repository.raw_retention_days()?;
    let mut removed = 0;
    for run in repository.list_runs()? {
        let timestamp = run.finished_at.unwrap_or(run.started_at);
        if !retention_expired(timestamp, now, days) {
            continue;
        }
        let directory = artifact_root.join("runs").join(run.id.to_string());
        if directory.exists() {
            let metadata = fs::symlink_metadata(&directory)?;
            if metadata.file_type().is_symlink() {
                return Err(DataError::SymbolicLink(directory.display().to_string()));
            }
            fs::remove_dir_all(directory)?;
        }
        repository.clear_artifact_references(run.id)?;
        removed += 1;
    }
    Ok(removed)
}

pub fn collect_backup_files(
    artifact_root: &Path,
) -> Result<Vec<(String, PathBuf)>, DataError> {
    let mut files = Vec::new();
    let runs_root = artifact_root.join("runs");
    if !runs_root.exists() {
        return Ok(files);
    }
    let root_metadata = fs::symlink_metadata(&runs_root)?;
    if root_metadata.file_type().is_symlink() {
        return Err(DataError::SymbolicLink(runs_root.display().to_string()));
    }
    if !root_metadata.is_dir() {
        return Err(DataError::UnexpectedEntry(
            runs_root.display().to_string(),
        ));
    }
    for entry in fs::read_dir(&runs_root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(DataError::SymbolicLink(path.display().to_string()));
        }
        if !metadata.is_dir()
            || Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
        {
            return Err(DataError::UnexpectedEntry(path.display().to_string()));
        }
        collect(artifact_root, &path, &mut files)?;
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn collect(
    root: &Path,
    current: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), DataError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(DataError::SymbolicLink(path.display().to_string()));
        }
        if metadata.is_dir() {
            collect(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("collector stays under root")
                .to_string_lossy()
                .replace('\\', "/");
            files.push((format!("artifacts/{relative}"), path));
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest<'a> {
    schema_version: u32,
    created_at: DateTime<Utc>,
    app_version: &'a str,
    contains_raw_answers_and_logs: bool,
    encrypted: bool,
}

pub fn create_full_backup(
    repository: &RunRepository,
    artifact_root: &Path,
    temporary_dir: &Path,
    destination: &Path,
    app_version: &str,
) -> Result<(), DataError> {
    let nonce = Uuid::new_v4();
    let snapshot = temporary_dir.join(format!("{nonce}.sqlite"));
    let archive_path = temporary_dir.join(format!("{nonce}.zip"));
    let result = (|| -> Result<(), DataError> {
        repository.backup_to(&snapshot)?;

        let archive_file = fs::File::create(&archive_path)?;
        let mut archive = zip::ZipWriter::new(archive_file);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        archive.start_file("ability-radar.sqlite", options)?;
        let mut database = fs::File::open(&snapshot)?;
        std::io::copy(&mut database, &mut archive)?;

        let manifest = BackupManifest {
            schema_version: 1,
            created_at: Utc::now(),
            app_version,
            contains_raw_answers_and_logs: true,
            encrypted: false,
        };
        archive.start_file("backup-manifest.json", options)?;
        archive.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

        for (name, path) in collect_backup_files(artifact_root)? {
            archive.start_file(name, options)?;
            let mut source = fs::File::open(path)?;
            std::io::copy(&mut source, &mut archive)?;
        }
        archive.finish()?;
        fs::copy(&archive_path, destination)?;
        Ok(())
    })();
    let _ = fs::remove_file(snapshot);
    let _ = fs::remove_file(archive_path);
    result
}
```

- [ ] **Step 4: Run pruning at startup and expose settings**

Store `app_data: PathBuf` in `AppState`. After state construction and before
returning it, call:

```rust
data_management::prune_expired_artifacts(
    &repository,
    &artifact_root,
    chrono::Utc::now(),
)
.map_err(|error| error.to_string())?;
```

Add DTOs:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSettingsDto {
    pub raw_retention_days: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRetentionInput {
    pub raw_retention_days: Option<u32>,
}
```

Add commands:

```rust
#[tauri::command]
pub fn get_data_settings(
    state: State<'_, AppState>,
) -> Result<DataSettingsDto, String> {
    Ok(DataSettingsDto {
        raw_retention_days: state
            .repository
            .raw_retention_days()
            .map_err(|error| error.to_string())?,
    })
}

#[tauri::command]
pub fn set_raw_retention(
    state: State<'_, AppState>,
    input: SetRetentionInput,
) -> Result<u32, String> {
    if !matches!(input.raw_retention_days, None | Some(7 | 30 | 90)) {
        return Err("保留期限只能是永久、7、30 或 90 天".into());
    }
    state
        .repository
        .set_raw_retention_days(input.raw_retention_days)
        .map_err(|error| error.to_string())?;
    data_management::prune_expired_artifacts(
        &state.repository,
        &state.artifact_root,
        chrono::Utc::now(),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn export_full_backup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let selected = app
        .dialog()
        .file()
        .set_title("导出完整本地备份")
        .add_filter("ZIP backup", &["zip"])
        .set_file_name(format!(
            "ability-radar-full-backup-{}.zip",
            chrono::Utc::now().format("%Y%m%d")
        ))
        .blocking_save_file();
    let Some(destination) = selected else {
        return Ok(false);
    };
    let destination = destination
        .into_path()
        .map_err(|_| "仅支持保存到本地文件路径".to_string())?;
    let has_zip_extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("zip"));
    if !destination.is_absolute() || !has_zip_extension {
        return Err("备份必须保存为用户选择的 .zip 文件".into());
    }
    if destination
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("拒绝写入符号链接目标".into());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "备份路径没有父目录".to_string())?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let app_data = state
        .app_data
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if parent.starts_with(app_data) {
        return Err("请把备份保存到应用数据目录之外".into());
    }
    data_management::create_full_backup(
        &state.repository,
        &state.artifact_root,
        &state.app_data,
        &destination,
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|error| error.to_string())?;
    Ok(true)
}
```

Import `tauri_plugin_dialog::DialogExt`; the plugin is already registered by
Task 18. Register the module, tests, and commands.

Also extend the Task 18 `export_public_report` command immediately after the
successful file write:

```rust
let report_sha256 = ability_core::public_report_sha256(&html);
state
    .repository
    .record_publication(
        report.report_id,
        run_id,
        &report_sha256,
        "local_html",
    )
    .map_err(|error| error.to_string())?;
```

The database stores only the report hash and coarse destination kind
(`local_html`), never the absolute destination path.

- [ ] **Step 5: Add the history-page data settings**

Extend `Backend` with:

```ts
getDataSettings(): Promise<{ rawRetentionDays: number | null }>;
setRawRetention(rawRetentionDays: number | null): Promise<number>;
exportFullBackup(): Promise<boolean>;
```

Implement them with the matching commands. On `HistoryPage`, add a “本地数据”
section containing:

- a select with 永久（默认）, 90 天, 30 天, and 7 天;
- an explanation that expiry deletes only raw answers/logs and preserves scores;
- an “导出完整本地备份” button whose backend command opens the native save
  dialog with a `.zip` filter;
- an explicit checkbox reading “我知道此 ZIP 未加密，并包含原始回答和日志”
  before enabling backup.

Do not call the public-report scanner for a full backup: this is deliberately a
private, user-selected copy of all local data, and the UI must label it as such.
Do not pass a destination path through IPC. Treat a `false` return as a normal
user cancellation.

- [ ] **Step 6: Verify settings, backup, and startup pruning**

Run:

```powershell
cargo fmt --all --check
cargo test --workspace
npm test
npm run build
```

Open the generated ZIP and verify it contains exactly:

- `ability-radar.sqlite`;
- `backup-manifest.json`;
- zero or more paths below `artifacts/runs/<uuid>/`.

Expected: no absolute local path is used as a ZIP entry, symlinks abort backup,
changing retention to 7 days immediately prunes only expired raw artifacts, and
cancelling the native picker creates neither a ZIP nor a temporary snapshot.

- [ ] **Step 7: Commit**

```powershell
git add Cargo.lock crates apps/desktop
git commit -m "feat: add local retention and full backup"
```

---

### Task 21: Apply the Dashboard Visual System, Themes, i18n Foundation, and Accessibility

**Files:**
- Create: `apps/desktop/src/styles/tokens.css`
- Create: `apps/desktop/src/styles/app.css`
- Create: `apps/desktop/src/i18n/messages.ts`
- Create: `apps/desktop/src/i18n/I18nContext.tsx`
- Create: `apps/desktop/src/components/ThemeToggle.tsx`
- Create: `apps/desktop/src/test/accessibility.test.tsx`
- Modify: `apps/desktop/src/app/App.tsx`
- Modify: `apps/desktop/src/components/AppShell.tsx`
- Modify: all page components to use the shared messages and `id="page-content"`
- Modify: `apps/desktop/package.json`

**Interfaces:**
- Consumes: the completed functional UI.
- Produces: a calm “health dashboard” visual language, light/dark themes,
  keyboard operation, Chinese message indirection, and automated axe checks.

- [ ] **Step 1: Add automated accessibility checks**

Run:

```powershell
npm install --workspace apps/desktop --save-dev axe-core
```

Create `apps/desktop/src/test/accessibility.test.tsx`:

```tsx
import axe from "axe-core";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { BackendProvider } from "../api/BackendContext";
import type { Backend } from "../api/backend";
import { AppRoutes } from "../app/routes";

const backend = {
  getBootstrap: async () => ({
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
      taskCount: 2,
      estimatedMinutes: "30–60",
    },
    targets: [],
  }),
} as Backend;

test("home route has no serious axe violations", async () => {
  const { container } = render(
    <MemoryRouter>
      <BackendProvider backend={backend}>
        <AppRoutes />
      </BackendProvider>
    </MemoryRouter>,
  );
  await new Promise((resolve) => window.setTimeout(resolve, 0));
  const result = await axe.run(container, {
    runOnly: {
      type: "tag",
      values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"],
    },
  });
  expect(
    result.violations.filter((violation) =>
      ["serious", "critical"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
});
```

- [ ] **Step 2: Create semantic design tokens**

Create `apps/desktop/src/styles/tokens.css`:

```css
:root {
  color-scheme: light;
  --bg: #f4f7fb;
  --surface: #ffffff;
  --surface-raised: #ffffff;
  --surface-muted: #eaf0f7;
  --text: #152033;
  --text-muted: #5e6a7d;
  --border: #ccd6e4;
  --brand: #176b68;
  --brand-strong: #0f514f;
  --brand-soft: #d9f1ed;
  --ok: #117a53;
  --ok-soft: #daf2e6;
  --warn: #9a5b00;
  --warn-soft: #fff0cb;
  --danger: #ad3038;
  --danger-soft: #fde4e6;
  --focus: #316bd8;
  --shadow: 0 18px 50px rgb(31 49 76 / 10%);
  --radius-sm: 10px;
  --radius-md: 16px;
  --radius-lg: 24px;
  --space-1: 0.375rem;
  --space-2: 0.625rem;
  --space-3: 1rem;
  --space-4: 1.5rem;
  --space-5: 2rem;
  --space-6: 3rem;
}

:root[data-theme="dark"] {
  color-scheme: dark;
  --bg: #0b1220;
  --surface: #111b2d;
  --surface-raised: #17243a;
  --surface-muted: #1d2b43;
  --text: #edf4ff;
  --text-muted: #aab8cb;
  --border: #33445f;
  --brand: #69d3c6;
  --brand-strong: #9ae6dc;
  --brand-soft: #153c3b;
  --ok: #72d6aa;
  --ok-soft: #173b31;
  --warn: #f1bc5b;
  --warn-soft: #44351b;
  --danger: #ff9298;
  --danger-soft: #48252c;
  --focus: #8eb5ff;
  --shadow: 0 22px 60px rgb(0 0 0 / 28%);
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme]) {
    color-scheme: dark;
    --bg: #0b1220;
    --surface: #111b2d;
    --surface-raised: #17243a;
    --surface-muted: #1d2b43;
    --text: #edf4ff;
    --text-muted: #aab8cb;
    --border: #33445f;
    --brand: #69d3c6;
    --brand-strong: #9ae6dc;
    --brand-soft: #153c3b;
    --ok: #72d6aa;
    --ok-soft: #173b31;
    --warn: #f1bc5b;
    --warn-soft: #44351b;
    --danger: #ff9298;
    --danger-soft: #48252c;
    --focus: #8eb5ff;
    --shadow: 0 22px 60px rgb(0 0 0 / 28%);
  }
  :root:not([data-theme]) button,
  :root:not([data-theme]) .button { color: #071817; }
}
```

- [ ] **Step 3: Create the responsive dashboard stylesheet**

Create `apps/desktop/src/styles/app.css` with these complete base and component
rules, then add any page-specific selectors already used by Tasks 14–21:

```css
* { box-sizing: border-box; }
html { min-width: 320px; background: var(--bg); }
body {
  margin: 0;
  min-width: 320px;
  min-height: 100vh;
  background:
    radial-gradient(circle at 85% -10%, color-mix(in srgb, var(--brand) 17%, transparent), transparent 34rem),
    var(--bg);
  color: var(--text);
  font: 400 16px/1.55 "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
}
button, input, textarea, select { font: inherit; }
a { color: var(--brand-strong); }
button, .button {
  display: inline-flex;
  min-height: 44px;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  padding: 0.7rem 1rem;
  background: var(--brand);
  color: #fff;
  font-weight: 700;
  text-decoration: none;
  cursor: pointer;
}
:root[data-theme="dark"] button,
:root[data-theme="dark"] .button { color: #071817; }
button.secondary, .button.secondary {
  border-color: var(--border);
  background: var(--surface);
  color: var(--text);
}
button.danger { background: var(--danger); }
button:disabled { cursor: not-allowed; opacity: 0.52; }
:focus-visible { outline: 3px solid var(--focus); outline-offset: 3px; }
.skip-link {
  position: fixed; z-index: 100; top: 8px; left: 8px;
  transform: translateY(-150%);
}
.skip-link:focus { transform: none; }
.sr-only {
  position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0;
}
.topbar {
  position: sticky; z-index: 20; top: 0;
  display: flex; align-items: center; justify-content: space-between;
  min-height: 64px; padding: 0 var(--space-5);
  border-bottom: 1px solid var(--border);
  background: color-mix(in srgb, var(--bg) 88%, transparent);
  backdrop-filter: blur(18px);
}
.topbar nav, .button-row { display: flex; flex-wrap: wrap; gap: var(--space-2); }
.brand { color: var(--text); font-size: 1.05rem; font-weight: 800; text-decoration: none; }
main { width: min(1120px, calc(100% - 32px)); margin: 0 auto; padding: var(--space-6) 0; }
h1 { max-width: 18ch; margin: 0 0 var(--space-3); font-size: clamp(2rem, 5vw, 3.8rem); line-height: 1.06; letter-spacing: -0.035em; }
h2 { margin: var(--space-4) 0 var(--space-2); line-height: 1.2; }
.eyebrow { color: var(--brand-strong); font-weight: 800; letter-spacing: 0.08em; text-transform: uppercase; }
.hero { max-width: 760px; margin-bottom: var(--space-5); }
.target-grid, .score-grid {
  display: grid; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
  gap: var(--space-3);
}
.target-card, .score-grid article, .history-group, .result-banner, .notice {
  border: 1px solid var(--border); border-radius: var(--radius-md);
  background: var(--surface); box-shadow: var(--shadow); padding: var(--space-4);
}
.target-card { display: flex; min-height: 220px; flex-direction: column; }
.target-card .button, .target-card button { margin-top: auto; }
.status { display: flex; align-items: center; gap: 0.5rem; font-weight: 700; }
.status::before { content: "●"; }
.status-ok { color: var(--ok); }
.status-warn { color: var(--warn); }
.run-page { max-width: 780px; }
label { display: grid; gap: var(--space-1); margin: var(--space-3) 0; font-weight: 700; }
.check-row { display: flex; align-items: flex-start; }
input, textarea, select {
  width: 100%; border: 1px solid var(--border); border-radius: var(--radius-sm);
  padding: 0.75rem; background: var(--surface); color: var(--text);
}
textarea { resize: vertical; }
.prompt-box {
  max-height: 42vh; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere;
  border: 1px solid var(--border); border-radius: var(--radius-md);
  padding: var(--space-4); background: var(--surface-muted);
}
progress { width: 100%; height: 12px; accent-color: var(--brand); }
.score-grid strong { display: block; margin: 0.25rem 0; font-size: 2.5rem; }
.score-grid small, .hint { color: var(--text-muted); }
.category-row {
  display: grid; grid-template-columns: minmax(7rem, 1fr) minmax(8rem, 3fr) 4rem;
  gap: var(--space-3); align-items: center; margin: var(--space-2) 0;
}
.bar-track { height: 12px; overflow: hidden; border-radius: 999px; background: var(--surface-muted); }
.bar-track span { display: block; height: 100%; background: var(--brand); }
.result-ok { border-left: 7px solid var(--ok); }
.result-warn { border-left: 7px solid var(--warn); }
.result-danger { border-left: 7px solid var(--danger); }
.result-neutral { border-left: 7px solid var(--text-muted); }
.environment-list > div, .history-group li, .task-results li {
  display: flex; flex-wrap: wrap; justify-content: space-between;
  gap: var(--space-2); padding: var(--space-2) 0; border-bottom: 1px solid var(--border);
}
.environment-list dt { color: var(--text-muted); }
.environment-list dd { margin: 0; font-weight: 700; }

@media (max-width: 640px) {
  .topbar { align-items: flex-start; padding: var(--space-3); }
  .topbar nav { justify-content: flex-end; }
  main { width: min(100% - 24px, 1120px); padding: var(--space-5) 0; }
  .category-row { grid-template-columns: 1fr 3.5rem; }
  .category-row .bar-track { grid-column: 1 / -1; grid-row: 2; }
}

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    scroll-behavior: auto !important;
    transition-duration: 0.01ms !important;
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
  }
}
```

- [ ] **Step 4: Add explicit theme control**

Create `ThemeToggle.tsx`:

```tsx
import { useEffect, useState } from "react";

type Theme = "system" | "light" | "dark";

export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(
    () => (localStorage.getItem("theme") as Theme | null) ?? "system",
  );
  useEffect(() => {
    if (theme === "system") {
      document.documentElement.removeAttribute("data-theme");
      localStorage.removeItem("theme");
    } else {
      document.documentElement.dataset.theme = theme;
      localStorage.setItem("theme", theme);
    }
  }, [theme]);
  return (
    <label className="theme-control">
      <span className="sr-only">配色主题</span>
      <select
        aria-label="配色主题"
        value={theme}
        onChange={(event) => setTheme(event.target.value as Theme)}
      >
        <option value="system">跟随系统</option>
        <option value="light">浅色</option>
        <option value="dark">深色</option>
      </select>
    </label>
  );
}
```

Add a visually-hidden `.sr-only` utility to CSS. Put `ThemeToggle` in the
topbar and add a keyboard-visible skip link:

```tsx
<a className="skip-link button" href="#page-content">跳到主要内容</a>
```

Every routed `<main>` receives `id="page-content"` and `tabIndex={-1}`.

- [ ] **Step 5: Add Chinese message indirection**

Create `messages.ts`:

```ts
export const messages = {
  "app.name": "AI 能力雷达",
  "nav.start": "开始体检",
  "nav.history": "历史记录",
  "result.title": "本次体检结果",
  "result.boundary": "v0.2 只展示本题包的客观结果，不生成降智结论",
  "result.validTasks": "有效题目",
  "result.historyBoundary": "不同对象、模型、设置和题包分开记录",
  "common.loading": "正在读取本机数据…",
  "common.retry": "重试",
} as const;

export type MessageKey = keyof typeof messages;
```

Create `I18nContext.tsx`:

```tsx
import { createContext, useContext, type ReactNode } from "react";
import { messages, type MessageKey } from "./messages";

const I18nContext = createContext((key: MessageKey) => messages[key]);

export function I18nProvider({ children }: { children: ReactNode }) {
  return (
    <I18nContext.Provider value={(key) => messages[key]}>
      {children}
    </I18nContext.Provider>
  );
}

export const useT = () => useContext(I18nContext);
```

Wrap `AppRoutes` in `I18nProvider`. Move shared navigation, status, button, and
error labels to `messages.ts`; task prompt text remains pack content. This keeps
v0.2 Chinese-only while making a later English dictionary a data change rather
than a routing rewrite.

- [ ] **Step 6: Import styles and fix all automated findings**

At the top of `App.tsx`:

```ts
import "../styles/tokens.css";
import "../styles/app.css";
```

Add `aria-live="polite"` to run progress and export status, keep destructive
confirmations in the tab order, and ensure every status uses text plus the
colored border/dot. Run:

```powershell
npm test
npm run build
```

Expected: axe has no serious/critical WCAG A/AA findings; all controls work with
Tab, Shift+Tab, Enter, and Space; light/dark/system modes persist correctly.

- [ ] **Step 7: Check Windows scale and reduced motion**

Manually inspect at 100%, 125%, 150%, and 200% Windows display scaling with
window sizes 1280×800 and 1024×720. Expected: no horizontal page scroll, prompt
and answer areas remain usable, focus rings are never clipped, and no conclusion
depends on green/yellow/red alone.

- [ ] **Step 8: Commit**

```powershell
git add apps/desktop package-lock.json
git commit -m "feat: polish accessible dashboard interface"
```

---

### Task 22: Add GitHub CI, Windows Releases, Pages, and Maintainer Documentation

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Create: `.github/workflows/pages.yml`
- Create: `.github/ISSUE_TEMPLATE/bug.yml`
- Create: `.github/pull_request_template.md`
- Create: `README.md`
- Create: `CONTRIBUTING.md`
- Create: `SECURITY.md`
- Create: `THIRD_PARTY_NOTICES.md`
- Create: `docs/privacy.md`
- Create: `docs/security.md`
- Create: `docs/troubleshooting.md`
- Create: `docs/methodology.md`
- Create: `site/index.html`
- Create: `site/.nojekyll`

**Interfaces:**
- Consumes: the complete tested Windows application and static documentation.
- Produces: secret-free pull-request checks, draft Windows installers with
  checksums, and a GitHub Pages information/download site.

- [ ] **Step 1: Add the Windows-first CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  test:
    runs-on: windows-latest
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-node@v6
        with:
          node-version: 22
          cache: npm
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy,rustfmt
      - name: Install frontend dependencies
        run: npm ci
      - name: Check Rust formatting
        run: cargo fmt --all --check
      - name: Lint Rust
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: Test Rust
        run: cargo test --workspace --locked
      - name: Test frontend
        run: npm test
      - name: Build frontend
        run: npm run build
      - name: Prove Windows desktop bundle
        run: npm run tauri -- build --debug --bundles nsis
      - name: Upload debug installer
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: windows-debug-installer
          path: target/debug/bundle/nsis/*.exe
          if-no-files-found: ignore
          retention-days: 7
```

The workflow must not install or run Codex, Claude Code, ChatGPT, or Claude and
must not define provider API keys. All automatic adapter tests use fakes.

- [ ] **Step 2: Add a draft, unsigned-alpha Windows release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: windows-release

on:
  push:
    tags:
      - "v*"

permissions:
  contents: write

jobs:
  release:
    runs-on: windows-latest
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@v6
        with:
          fetch-depth: 0
      - uses: actions/setup-node@v6
        with:
          node-version: 22
          cache: npm
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy,rustfmt
      - run: npm ci
      - name: Verify tag matches application version
        shell: pwsh
        run: |
          $config = Get-Content apps/desktop/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
          if ("v$($config.version)" -ne "${{ github.ref_name }}") {
            throw "Tag does not match tauri.conf.json version"
          }
      - run: cargo fmt --all --check
      - run: cargo test --workspace --locked
      - run: npm test
      - name: Build and create draft release
        id: tauri
        uses: tauri-apps/tauri-action@v1
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          projectPath: apps/desktop
          tauriScript: npm run tauri --
          tagName: ${{ github.ref_name }}
          releaseName: "AI 能力雷达 ${{ github.ref_name }}"
          releaseBody: |
            Windows x64 预览版。

            - 安装包尚未进行商业代码签名，Windows SmartScreen 可能提示风险。
            - 核心数据默认只保存在本机。
            - CLI 测试消耗运行者自己的 Codex/Claude 订阅额度。
            - 请下载 SHA256SUMS.txt 校验安装包。
          releaseDraft: true
          prerelease: true
          uploadUpdaterSignatures: false
      - name: Generate SHA-256 checksums
        shell: pwsh
        run: |
          $files = Get-ChildItem target/release/bundle -Recurse -File |
            Where-Object { $_.Extension -in ".exe", ".msi" } |
            Sort-Object FullName
          $lines = foreach ($file in $files) {
            $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLower()
            "$hash  $($file.Name)"
          }
          Set-Content -LiteralPath SHA256SUMS.txt -Value $lines -Encoding utf8
      - name: Upload checksums
        shell: pwsh
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: gh release upload "${{ github.ref_name }}" SHA256SUMS.txt --clobber
```

Keep the Tauri updater plugin absent and updater configuration disabled. Add
signing and automatic updates only in the later stable-release phase after a
protected signing secret and Windows code-signing certificate exist.

- [ ] **Step 3: Create the GitHub Pages workflow**

Create `.github/workflows/pages.yml`:

```yaml
name: pages

on:
  push:
    branches: [main]
    paths:
      - "site/**"
      - "docs/**"
      - ".github/workflows/pages.yml"
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: true

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: actions/configure-pages@v5
      - name: Assemble static site
        run: |
          mkdir -p site/docs
          cp docs/privacy.md site/docs/privacy.md
          cp docs/security.md site/docs/security.md
          cp docs/methodology.md site/docs/methodology.md
          cp docs/troubleshooting.md site/docs/troubleshooting.md
      - uses: actions/upload-pages-artifact@v4
        with:
          path: site

  deploy:
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    runs-on: ubuntu-latest
    needs: build
    steps:
      - name: Deploy
        id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 4: Build a static, no-tracker information site**

Create `site/index.html` as a standalone page using the same semantic colors and
system fonts as the desktop UI. It must contain:

- a plain-language explanation of what “降智检测” can and cannot conclude;
- separate cards for client copy/paste and automatic CLI tests;
- “谁支付测试费用” stating the runner’s own subscription is charged;
- Windows 10/11 x64 scope and Node.js 22/24 LTS only for CLI tasks;
- privacy: no default upload, telemetry, or credential collection;
- links to methodology, privacy, security, source, and latest release;
- a warning that preview installers are unsigned;
- no analytics, cookies, external font, CDN script, or external image.

Use this complete initial `site/index.html`:

```html
<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <meta
    http-equiv="Content-Security-Policy"
    content="default-src 'self'; img-src 'none'; font-src 'none'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'"
  >
  <meta
    name="description"
    content="AI 能力雷达：在本机复测 ChatGPT、Claude、Codex CLI 和 Claude Code 的客观任务表现。"
  >
  <title>AI 能力雷达</title>
  <style>
    :root {
      color-scheme: light dark;
      --bg: #f4f7fb;
      --surface: #fff;
      --muted: #eaf0f7;
      --text: #152033;
      --subtle: #5e6a7d;
      --border: #ccd6e4;
      --brand: #176b68;
      --brand-strong: #0f514f;
      --warning: #8a5200;
      --shadow: 0 18px 50px rgb(31 49 76 / 10%);
    }
    * { box-sizing: border-box; }
    html { scroll-behavior: smooth; background: var(--bg); }
    body {
      margin: 0;
      color: var(--text);
      background:
        radial-gradient(circle at 88% 0, rgb(23 107 104 / 14%), transparent 30rem),
        var(--bg);
      font: 400 16px/1.65 "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
    }
    a { color: var(--brand-strong); }
    .skip {
      position: fixed;
      top: 8px;
      left: 8px;
      z-index: 5;
      padding: .6rem .9rem;
      transform: translateY(-160%);
      background: var(--surface);
    }
    .skip:focus { transform: none; }
    header, main, footer { width: min(1080px, calc(100% - 32px)); margin: auto; }
    header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 1rem;
      padding: 1.25rem 0;
    }
    nav { display: flex; flex-wrap: wrap; gap: 1rem; }
    .brand { color: var(--text); font-weight: 850; text-decoration: none; }
    main { padding: 4rem 0; }
    .hero { max-width: 800px; padding: 3rem 0 4rem; }
    .eyebrow {
      color: var(--brand-strong);
      font-weight: 800;
      letter-spacing: .08em;
      text-transform: uppercase;
    }
    h1 {
      margin: 0 0 1.25rem;
      font-size: clamp(2.6rem, 8vw, 5.6rem);
      line-height: 1;
      letter-spacing: -.055em;
    }
    h2 { margin-top: 0; line-height: 1.2; }
    .lead { max-width: 66ch; color: var(--subtle); font-size: 1.18rem; }
    .actions { display: flex; flex-wrap: wrap; gap: .75rem; margin-top: 1.75rem; }
    .button {
      display: inline-flex;
      min-height: 44px;
      align-items: center;
      padding: .7rem 1rem;
      border: 1px solid var(--brand);
      border-radius: 10px;
      background: var(--brand);
      color: #fff;
      font-weight: 750;
      text-decoration: none;
    }
    .button.secondary { background: transparent; color: var(--brand-strong); }
    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
      gap: 1rem;
      margin: 1.5rem 0 4rem;
    }
    .card, .notice {
      padding: 1.5rem;
      border: 1px solid var(--border);
      border-radius: 18px;
      background: var(--surface);
      box-shadow: var(--shadow);
    }
    .card p, .notice p { color: var(--subtle); }
    .notice { margin: 1.5rem 0 4rem; border-left: 7px solid var(--warning); }
    .facts { padding-left: 1.2rem; }
    .facts li { margin: .65rem 0; }
    footer { padding: 2rem 0 3rem; border-top: 1px solid var(--border); color: var(--subtle); }
    footer nav { margin-bottom: .75rem; }
    :focus-visible { outline: 3px solid #316bd8; outline-offset: 3px; }
    @media (prefers-color-scheme: dark) {
      :root {
        --bg: #0b1220;
        --surface: #111b2d;
        --muted: #17243a;
        --text: #edf4ff;
        --subtle: #aab8cb;
        --border: #33445f;
        --brand: #69d3c6;
        --brand-strong: #9ae6dc;
        --warning: #f1bc5b;
        --shadow: 0 22px 60px rgb(0 0 0 / 28%);
      }
      .button { color: #071817; }
    }
    @media (prefers-reduced-motion: reduce) {
      html { scroll-behavior: auto; }
    }
  </style>
</head>
<body>
  <a class="skip" href="#content">跳到主要内容</a>
  <header>
    <a class="brand" href="./">AI 能力雷达</a>
    <nav aria-label="主导航">
      <a href="#how">怎么测</a>
      <a href="#privacy">隐私</a>
      <a id="source-nav">源代码</a>
    </nav>
  </header>
  <main id="content">
    <section class="hero">
      <p class="eyebrow">Windows 本地工具 · v0.2 预览</p>
      <h1>把“感觉降智”变成可复核的测试记录</h1>
      <p class="lead">
        分别测试 ChatGPT、Claude、Codex CLI 和 Claude Code。
        首版提供客观得分和严格分组的历史，不把一次波动包装成 IQ，
        也不自动下“降智”结论。
      </p>
      <div class="actions">
        <a class="button" id="release-link">下载 Windows 预览版</a>
        <a class="button secondary" id="source-link">查看源代码</a>
      </div>
    </section>

    <section id="how" aria-labelledby="how-title">
      <p class="eyebrow">两条独立测试线</p>
      <h2 id="how-title">客户端和 CLI 不混分</h2>
      <div class="grid">
        <article class="card">
          <h3>ChatGPT / Claude 客户端</h3>
          <p>
            8 道快速题：3 道指令遵循、3 道逻辑、2 道代码审查。
            你在新对话中复制题目、粘贴原回答，预计 10–15 分钟。
          </p>
        </article>
        <article class="card">
          <h3>Codex CLI / Claude Code</h3>
          <p>
            自动完成 2 个临时 JavaScript 微型项目并由本地验证器评分，
            预计 30–60 分钟。需要 Node.js 22 或 24 LTS。
          </p>
        </article>
        <article class="card">
          <h3>结果怎么读</h3>
          <p>
            得分只表示当前题包内的客观通过表现。不同测试对象、模型、
            推理档位、题包、运行环境和恢复状态会分开保存。
          </p>
        </article>
      </div>
    </section>

    <section aria-labelledby="cost-title">
      <p class="eyebrow">费用边界</p>
      <h2 id="cost-title">谁运行，谁承担自己的订阅用量</h2>
      <div class="notice">
        <p>
          GitHub CI 只运行假 CLI，不会消耗 Codex 或 Claude 订阅。
          在个人电脑上启动真实 CLI 测试时，用量计入运行者自己的账号；
          项目维护者不提供共享密钥，也不会代付。
        </p>
      </div>
    </section>

    <section id="privacy" aria-labelledby="privacy-title">
      <p class="eyebrow">本地优先</p>
      <h2 id="privacy-title">凭据不进入应用，原文默认不离开电脑</h2>
      <ul class="facts">
        <li>不读取或保存 ChatGPT、Claude、Codex、GitHub 登录凭据。</li>
        <li>没有默认遥测、后台上传、公共排行榜、广告或分析脚本。</li>
        <li>测试题和临时代码仍会发送给你选择的 AI 服务，并受该服务自己的隐私与遥测规则约束。</li>
        <li>原始回答和 CLI 日志保存在本机；公开报告先展示字段白名单。</li>
        <li>可分享报告排除原始回答、日志、用户名、设备名和绝对路径。</li>
        <li>完整本地备份包含原文且不加密，导出前会再次明确提醒。</li>
      </ul>
    </section>

    <section aria-labelledby="support-title">
      <p class="eyebrow">首发范围</p>
      <h2 id="support-title">Windows 10/11 x64</h2>
      <p class="lead">
        首个公开预览版提供 Windows 安装包。安装包在预览阶段可能尚未进行
        商业代码签名，因此 SmartScreen 可能显示提示；请同时核对发布页的
        SHA-256 校验值。Windows 会在后续所有阶段继续受到支持。
      </p>
    </section>
  </main>
  <footer>
    <nav aria-label="文档">
      <a href="docs/methodology.md">方法说明</a>
      <a href="docs/privacy.md">隐私说明</a>
      <a href="docs/security.md">安全说明</a>
      <a href="docs/troubleshooting.md">故障排查</a>
    </nav>
    <small>无 Cookie · 无统计脚本 · 无外部字体或图片</small>
  </footer>
  <script>
    const owner = location.hostname.split(".")[0];
    const [projectPath] = location.pathname.split("/").filter(Boolean);
    const repository = projectPath || `${owner}.github.io`;
    const root = `https://github.com/${owner}/${repository}`;
    document.querySelector("#source-nav").href = root;
    document.querySelector("#source-link").href = root;
    document.querySelector("#release-link").href = `${root}/releases/latest`;
  </script>
</body>
</html>
```

When served from a custom domain later, replace this derivation with a generated
repository URL during the Pages build.

- [ ] **Step 5: Write maintainer and user documentation**

`README.md` must lead with:

- current status: v0.2 Windows preview;
- screenshots or GIFs only after the UI is stable;
- supported targets and exact task counts;
- a prominent subscription-cost explanation;
- cost separation: ordinary GitHub CI uses fake CLIs and consumes no AI
  subscription; GitHub-hosted runner billing follows the repository owner's
  GitHub plan; any optional real-CLI release check consumes only the volunteer
  tester's own subscription;
- install from Releases and checksum verification;
- development prerequisites and commands;
- architecture boundaries;
- link to the approved design and implementation plan.

`docs/methodology.md` must document:

- objective graders and category-equal weighting;
- that the v0.2 prompts and micro-repositories are original first-party quick
  checks, not copied Codex Radar or DeepSWE tasks, plus the limitation that
  open-source fixed tasks can eventually become contaminated;
- CLI model semantics: blank means the CLI default route and is stored as
  `default`; a supplied model and low/medium/high effort are passed explicitly
  to the selected CLI and become part of the history key;
- duration semantics: assisted-client time includes the human copy/paste
  interval, while CLI time is process plus verifier time, so duration is never
  compared across those tracks;
- the exact history-series key: target kind, trimmed reported model, reasoning
  effort, run mode, suite ID/version/hash, scoring-rule version, OS
  family/version, app version, CLI version, Node verifier version, and
  clean-versus-resumed state;
- the explicit v0.2 rule that no degradation verdict or confidence level is generated;
- the planned v0.5 baseline/calibration boundary without describing it as shipped;
- invalid infrastructure vs agent-budget failure;
- all rule version strings.

`docs/privacy.md` and `docs/security.md` must mirror the implemented behavior,
not aspirations. `docs/troubleshooting.md` must cover CLI missing/login/Node,
quota, network, SmartScreen, interrupted recovery, and where local data lives
without printing a real username.

The privacy/security docs must not call deletion a forensic wipe. State that the
app removes files normally, enables SQLite `secure_delete`, and truncates the WAL
after whole-run deletion, but SSD behavior, filesystem snapshots, antivirus
quarantine, and external backups can retain recoverable copies.

They must also distinguish application telemetry from provider traffic: the app
has no telemetry or upload endpoint, but prompts and temporary benchmark code
are necessarily sent to the selected AI provider, and the invoked CLI/provider
can apply its own logging, retention, and telemetry policy.

Document the real v0.2 isolation level: per-task directories, Codex
`workspace-write`, Claude’s allowlisted Read/Edit/Write tools with `dontAsk`,
fixed time/turn limits, and verifier allowlisting reduce accidental scope, but
this is not a container, VM, or malicious-code sandbox. Stronger WSL/container
isolation remains a v0.5 gate for real repositories and DeepSWE.

`THIRD_PARTY_NOTICES.md` lists:

- all Rust and npm dependencies through generated license reports;
- the two built-in first-party task packs as Apache-2.0;
- any future imported task content separately;
- no DeepSWE content until its redistribution review is complete.

`SECURITY.md` provides private vulnerability-report instructions without
inventing an email address: direct reporters to the repository’s GitHub
Security Advisory “Report a vulnerability” button.

- [ ] **Step 6: Add contribution templates and supply-chain checks**

The bug form asks for app version, Windows version, target type, task pack
version, and redacted error category; it explicitly warns against pasting raw
logs or tokens. The PR template requires:

- tests added/updated;
- no real subscription CLI executed in CI;
- privacy field review;
- capability diff review;
- task-license review when packs change;
- Windows manual check for process/cancellation changes.

Add to CI after dependency installation:

```yaml
- name: Audit committed dependency lockfiles
  run: |
    cargo install cargo-audit --locked
    cargo audit
    npm audit --audit-level=high
```

If installation time becomes excessive, pin a reviewed `cargo-audit` action or
run it in a scheduled `security.yml`; do not silently remove the audit.

- [ ] **Step 7: Validate GitHub artifacts without publishing a release**

Run locally:

```powershell
cargo test --workspace --locked
npm test
npm run build
npm run tauri -- build --debug --bundles nsis
```

Push a branch and open a pull request. Expected: CI produces a debug installer
artifact but performs no subscription test and creates no release. Run Pages
manually on a test fork before enabling GitHub Pages on the main repository.

- [ ] **Step 8: Commit**

```powershell
git add .github README.md CONTRIBUTING.md SECURITY.md THIRD_PARTY_NOTICES.md docs site
git commit -m "chore: add github build and release infrastructure"
```

---

### Task 23: Prove the Release with Fake CLIs and the Windows Acceptance Matrix

**Files:**
- Create: `tools/fake-cli/Cargo.toml`
- Create: `tools/fake-cli/src/main.rs`
- Create: `apps/desktop/src-tauri/tests/fake_cli_e2e.rs`
- Create: `docs/release-checklist.md`
- Create: `docs/test-matrix.md`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the release candidate and all public workflows.
- Produces: a no-subscription end-to-end CLI fixture, reproducible validation
  evidence, and the go/no-go checklist for v0.2.

- [ ] **Step 1: Add a deterministic fake CLI executable**

Add `"tools/fake-cli"` to the Cargo workspace. Create
`tools/fake-cli/Cargo.toml`:

```toml
[package]
name = "ability-radar-fake-cli"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
serde_json = "1"
```

Create `tools/fake-cli/src/main.rs`:

```rust
use std::env;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|value| value == "--version") {
        println!("ability-radar-fake-cli 0.1.0");
        return;
    }
    if args.as_slice() == ["login", "status"] {
        println!("Logged in using ChatGPT");
        return;
    }
    if args.as_slice() == ["auth", "status"] {
        println!(r#"{{"loggedIn":true}}"#);
        return;
    }
    let workspace = env::current_dir().expect("current directory");
    let delay = env::var("ABILITY_RADAR_FAKE_DELAY_MS").ok().or_else(|| {
        workspace.ancestors().find_map(|directory| {
            fs::read_to_string(directory.join(".ability-radar-fake-delay-ms"))
                .ok()
        })
    });
    if let Some(milliseconds) = delay {
        let milliseconds = milliseconds
            .trim()
            .parse::<u64>()
            .expect("fake delay must be an integer");
        thread::sleep(Duration::from_millis(milliseconds));
    }

    if workspace.ends_with("dedupe-events") {
        write(
            &workspace.join("src/dedupeEvents.mjs"),
            r#"export function dedupeEvents(events) {
  const latest = new Map();
  events.forEach((event, index) => {
    if (!event || typeof event !== "object" || typeof event.id !== "string" ||
        event.id.length === 0 || Number.isNaN(Date.parse(event.occurredAt))) return;
    const previous = latest.get(event.id);
    const time = Date.parse(event.occurredAt);
    if (!previous || time >= previous.time) {
      latest.set(event.id, { time, index, event: structuredClone(event) });
    }
  });
  return [...latest.values()]
    .sort((a, b) => a.time - b.time || a.event.id.localeCompare(b.event.id))
    .map(({ event }) => event);
}"#,
        );
    } else if workspace.ends_with("retry-schedule") {
        write(
            &workspace.join("src/retrySchedule.mjs"),
            r#"export function buildRetrySchedule({
  maxAttempts, baseDelayMs, maxDelayMs, retryAfterMs = [],
}) {
  const values = [maxAttempts, baseDelayMs, maxDelayMs, ...retryAfterMs];
  if (!values.every(Number.isInteger) || values.some((value) => value < 0) ||
      maxAttempts < 1 || baseDelayMs < 1 || maxDelayMs < baseDelayMs) {
    throw new TypeError("invalid options");
  }
  const result = [0];
  for (let retry = 1; retry < maxAttempts; retry += 1) {
    const base = Math.min(baseDelayMs * 2 ** (retry - 1), maxDelayMs);
    const delay = Math.max(base, retryAfterMs[retry - 1] ?? 0);
    result.push(result.at(-1) + delay);
  }
  return result;
}"#,
        );
    }

    let invoked_as = env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|value| value.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    if invoked_as.contains("claude") {
        println!(r#"{{"type":"result","subtype":"success"}}"#);
    } else {
        println!(r#"{{"type":"turn.completed","usage":{{"input_tokens":0,"output_tokens":0}}}}"#);
    }
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fixture solution");
}
```

The retry fixture must match the verifier’s exact indexing convention. Run the
hidden verifier against this fixture before accepting it; if the verifier and
fixture disagree, correct the implementation rather than weakening the test.

- [ ] **Step 2: Add a fixture-install script to CI**

After building `ability-radar-fake-cli`, use PowerShell to copy the executable
twice into a temporary directory:

```powershell
cargo build -p ability-radar-fake-cli
$fakeBin = Join-Path $env:RUNNER_TEMP "ability-radar-fake-bin"
New-Item -ItemType Directory -Force -Path $fakeBin | Out-Null
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "codex.exe")
Copy-Item target/debug/ability-radar-fake-cli.exe (Join-Path $fakeBin "claude.exe")
"$fakeBin" | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
```

Create an ignored developer script or document the same commands for local
Windows acceptance. Never bundle fake binaries into the release.

- [ ] **Step 3: Add a real-coordinator integration test using fake executables**

Use the `tempfile = "3"` dev dependency already added to the Tauri crate in
Task 20.

Create `apps/desktop/src-tauri/tests/fake_cli_e2e.rs`:

```rust
use ability_adapters::{
    AgentAdapter, AuthState, ClaudeCodeAdapter, CliRunService, CodexAdapter,
    NodeVerifier, ProcessRunner, TokioProcessRunner, WorkspaceVerifier,
};
use ability_core::{
    EnvironmentFingerprint, LoadedPack, PackLoader, RunMode, RunRepository,
    RunStatus, TargetKind, TargetSelection,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn require_opt_in() -> bool {
    if std::env::var("ABILITY_RADAR_FAKE_CLI_E2E").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("set ABILITY_RADAR_FAKE_CLI_E2E=1 to run this ignored test");
        false
    }
}

fn assert_program(program: &str) {
    let output = Command::new(program)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("{program} is missing from PATH: {error}"));
    assert!(
        output.status.success(),
        "{program} --version failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

fn source_pack_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../benchmark-packs/cli-quick-v1")
        .canonicalize()
        .expect("bundled CLI quick pack")
}

fn environment(
    pack: &LoadedPack,
    cli_version: String,
) -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os_family: "Windows".into(),
        os_version: "fake-cli-e2e".into(),
        app_version: "0.2.0-test".into(),
        cli_version: Some(cli_version),
        verifier_runtime_version: Some("node from PATH".into()),
        suite_id: pack.manifest.id.clone(),
        suite_version: pack.manifest.version.clone(),
        suite_content_sha256: pack.content_sha256.clone(),
        scoring_rule_version: "ability-v1".into(),
        resumed: false,
    }
}

async fn execute_complete_run(
    service: &CliRunService,
    repository: &RunRepository,
    pack: Arc<LoadedPack>,
    adapter: Arc<dyn AgentAdapter>,
    verifier: Arc<dyn WorkspaceVerifier>,
) {
    let availability = adapter.detect().await;
    assert!(availability.installed);
    assert_eq!(availability.auth_state, AuthState::Ready);
    let version = availability.version.expect("fake CLI version");
    let run = service
        .prepare(
            pack.clone(),
            TargetSelection {
                kind: adapter.kind(),
                reported_model: "deterministic fake".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            environment(&pack, version),
        )
        .unwrap();
    let (events, _receiver) = mpsc::unbounded_channel();
    service
        .execute(
            run.id,
            pack,
            adapter,
            verifier,
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();

    let stored = repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Completed);
    let score = stored.score.expect("completed score");
    assert_eq!(score.passed_tasks, 2);
    assert_eq!(score.valid_tasks, 2);
    assert_eq!(score.total_tasks, 2);
    assert_eq!(score.ability_score, 100.0);
}

async fn execute_cancelled_run(
    service: &CliRunService,
    repository: &RunRepository,
    artifact_root: &Path,
    pack: Arc<LoadedPack>,
    adapter: Arc<dyn AgentAdapter>,
    verifier: Arc<dyn WorkspaceVerifier>,
) {
    let run = service
        .prepare(
            pack.clone(),
            TargetSelection {
                kind: TargetKind::CodexCli,
                reported_model: "delayed deterministic fake".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            environment(&pack, "fake delayed".into()),
        )
        .unwrap();
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        trigger.cancel();
    });
    let (events, _receiver) = mpsc::unbounded_channel();
    let delay_marker = artifact_root.join(".ability-radar-fake-delay-ms");
    fs::write(&delay_marker, "10000").unwrap();
    let result = service
        .execute(
            run.id,
            pack,
            adapter,
            verifier,
            cancellation,
            events,
        )
        .await;
    fs::remove_file(delay_marker).unwrap();
    cancel_task.await.unwrap();
    result.unwrap();

    let stored = repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Cancelled);
}

fn assert_tree_is_contained(root: &Path) {
    let canonical_root = root.canonicalize().expect("artifact root");
    let mut pending = vec![canonical_root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let metadata = fs::symlink_metadata(entry.path()).unwrap();
            assert!(!metadata.file_type().is_symlink());
            let canonical = entry.path().canonicalize().unwrap();
            assert!(
                canonical.starts_with(&canonical_root),
                "{} escaped {}",
                canonical.display(),
                canonical_root.display(),
            );
            if metadata.is_dir() {
                pending.push(canonical);
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires fake codex.exe/claude.exe plus real node.exe on PATH"]
async fn bundled_pack_passes_both_adapters_and_cancellation_is_safe() {
    if !require_opt_in() {
        return;
    }
    assert_program("codex");
    assert_program("claude");
    assert_program("node");

    let temporary = tempdir().unwrap();
    let artifact_root = temporary.path().join("artifacts");
    fs::create_dir_all(&artifact_root).unwrap();
    let repository = Arc::new(
        RunRepository::open(&temporary.path().join("runs.sqlite")).unwrap(),
    );
    let pack = Arc::new(PackLoader::load(&source_pack_root()).unwrap());
    assert_eq!(pack.tasks.len(), 2);
    let runner: Arc<dyn ProcessRunner> = Arc::new(TokioProcessRunner);
    let verifier: Arc<dyn WorkspaceVerifier> = Arc::new(NodeVerifier::new(
        runner.clone(),
        source_pack_root(),
    ));
    let service = CliRunService::new(repository.clone(), artifact_root.clone());

    execute_complete_run(
        &service,
        &repository,
        pack.clone(),
        Arc::new(CodexAdapter::new(runner.clone())),
        verifier.clone(),
    )
    .await;
    execute_complete_run(
        &service,
        &repository,
        pack.clone(),
        Arc::new(ClaudeCodeAdapter::new(runner.clone())),
        verifier.clone(),
    )
    .await;
    execute_cancelled_run(
        &service,
        &repository,
        &artifact_root,
        pack,
        Arc::new(CodexAdapter::new(runner)),
        verifier,
    )
    .await;
    assert_tree_is_contained(&artifact_root);
}
```

This test requires `ABILITY_RADAR_FAKE_CLI_E2E=1`, locates `codex`,
`claude`, and `node` through `PATH`, uses the real bundled pack, real adapters,
real process runner, real Node verifier, and the production coordinator. The
fake process delay makes cancellation deterministic without contacting either
subscription service.

Run it in CI only after installing fake executables:

```powershell
$env:ABILITY_RADAR_FAKE_CLI_E2E="1"
cargo test -p ability-radar --test fake_cli_e2e -- --ignored
```

Expected: no network request and no subscription consumption.

- [ ] **Step 4: Create the formal test matrix**

Create `docs/test-matrix.md` with these required rows:

| Area | Windows 10 x64 | Windows 11 x64 | Automated | Release blocker |
|---|---:|---:|---:|---:|
| Install / uninstall NSIS | Yes | Yes | No | Yes |
| MSI installation | Yes | Yes | No | Yes |
| Client run without Node.js | Yes | Yes | Partial | Yes |
| Codex fake CLI 2/2 | Yes | Yes | Yes | Yes |
| Claude fake CLI 2/2 | Yes | Yes | Yes | Yes |
| Missing Node blocks before CLI call | Yes | Yes | Yes | Yes |
| Unsupported Node 20/26 blocks before CLI call | Yes | Yes | Yes | Yes |
| Cancel kills child process tree | Yes | Yes | Yes | Yes |
| Crash/restart resumes checkpoint | Yes | Yes | Yes | Yes |
| Clean and resumed history stay separate | Yes | Yes | Yes | Yes |
| Public report redaction | Yes | Yes | Yes | Yes |
| Full backup and retention | Yes | Yes | Yes | Yes |
| Light/dark, keyboard, 200% scale | Yes | Yes | Partial | Yes |
| GitHub Pages no tracker | N/A | N/A | Yes | Yes |
| macOS/Linux runtime | Deferred | Deferred | Compile optional | No for v0.2 |

Each manual cell records tester, date, app commit, OS build, and pass/fail link.
Do not mark a row passed based only on code inspection.

- [ ] **Step 5: Create the v0.2 release checklist**

Create `docs/release-checklist.md`:

```markdown
# v0.2 Windows Preview Release Checklist

## Scope and truthfulness
- [ ] Exactly 8 client tasks and 2 CLI tasks are bundled.
- [ ] ChatGPT, Claude, Codex CLI, and Claude Code never share a score.
- [ ] No screen says IQ or claims certainty from insufficient evidence.
- [ ] Time and subscription-cost estimates appear before starting.
- [ ] Infrastructure invalidity and agent-budget failure remain distinct.

## Reproducibility
- [ ] All pack hashes match the release resources.
- [ ] Same fixture history produces byte-equivalent analysis JSON.
- [ ] Clean and resumed runs never share one history series.
- [ ] v0.2 never emits a degradation verdict from historical scores.

## Privacy and security
- [ ] No API key, login token, CLI auth file, or environment dump is collected.
- [ ] Tauri capability file has no shell, HTTP, filesystem, or SQL permission.
- [ ] Public report contains no raw answer, log, username, hostname, or path.
- [ ] Full backup is explicitly labeled unencrypted/private.
- [ ] Cancellation kills the Windows child process tree.
- [ ] Only signed bundled verifier IDs can execute.

## Quality
- [ ] cargo fmt, clippy, tests, npm tests, build, and axe pass.
- [ ] Windows 10 and 11 acceptance matrix is complete.
- [ ] NSIS and MSI install, launch, and uninstall.
- [ ] 100–200% scaling and keyboard-only operation pass.
- [ ] Offline client-only use works without Node.js.

## GitHub release
- [ ] Version matches tag and documentation.
- [ ] THIRD_PARTY_NOTICES is current.
- [ ] SHA256SUMS.txt matches every installer.
- [ ] Draft notes state unsigned preview / SmartScreen behavior.
- [ ] Updater remains disabled.
- [ ] Pages links point to the correct repository and release.
```

- [ ] **Step 6: Run the complete verification sequence**

Use the `superpowers:verification-before-completion` skill at implementation
time. From a clean checkout:

```powershell
npm ci
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
npm test
npm run build
cargo build -p ability-radar-fake-cli
$env:ABILITY_RADAR_FAKE_CLI_E2E="1"
cargo test -p ability-radar --test fake_cli_e2e -- --ignored
npm run tauri -- build --bundles nsis,msi
```

Then run the Windows manual matrix on clean Windows 10 and 11 VMs. Record exact
evidence; do not infer a pass from a developer machine.

- [ ] **Step 7: Run adversarial release checks**

Run repository searches and inspect every result:

```powershell
rg -n --hidden --glob '!target/**' --glob '!node_modules/**' `
  'dangerously-skip-permissions|danger-full-access|api[_-]?key|BEGIN PRIVATE KEY'
rg -n --hidden --glob '!target/**' --glob '!node_modules/**' `
  'shell:|fs:|http:|sql:' apps/desktop/src-tauri/capabilities
rg -n --hidden --glob '!target/**' --glob '!node_modules/**' `
  'C:\\Users\\|/home/|/Users/' site docs benchmark-packs
```

Expected:

- dangerous execution flags: zero;
- generic Tauri capabilities: zero;
- credentials/private keys: zero, apart from clearly fake test literals;
- local path hits: only redaction tests and generic documentation examples.

- [ ] **Step 8: Commit the acceptance harness**

```powershell
git add Cargo.toml Cargo.lock tools apps/desktop/src-tauri/tests docs .github/workflows/ci.yml
git commit -m "test: add windows release acceptance harness"
```

---

## Implementation Exit Criteria

The v0.2 implementation is complete only when all 23 tasks are checked, the
clean-checkout verification sequence passes, and every release-blocking Windows
matrix row has recorded evidence. A green unit-test suite alone is not enough.

The approved full-C roadmap remains in
`docs/superpowers/specs/2026-07-17-ai-ability-radar-design.md`:

- v0.1: engineering foundation;
- v0.2: this lightweight Windows release;
- v0.5: calibrated deep detection, real repositories, DeepSWE compatibility,
  and stronger WSL/container isolation;
- v0.8: optional GitHub CLI publication and community reports;
- v1.0: signed stable Windows release and managed personal report repository;
- v2.0: Windows remains supported while macOS/Linux and broader community
  benchmarking mature.

Each later version gets its own incremental specification and implementation
plan after evidence from the preceding stage; none is silently pulled into this
v0.2 release.
