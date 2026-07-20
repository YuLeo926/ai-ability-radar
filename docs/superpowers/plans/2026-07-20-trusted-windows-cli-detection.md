# Trusted Windows CLI Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect the first usable trusted Codex CLI or Claude Code installation in PATH order, expose precise failure states, and make this machine report `codex-cli 0.142.5`.

**Architecture:** Replace the one-shot Windows command locator with ordered candidate discovery. A shared provider probe tries each candidate with the version command, retains the first successful launch, and returns a stable availability status and source label without exposing absolute paths.

**Tech Stack:** Rust 2024, Tokio, Tauri 2, React 19, TypeScript 5.8, Vitest.

## Global Constraints

- Windows 10/11 x64 remains the release platform.
- Never execute provider commands through `cmd.exe`, PowerShell, or another shell.
- Never execute arbitrary `.cmd`, `.bat`, `.ps1`, or extensionless npm shims.
- Only accept native EXEs or reviewed `@openai/codex` and `@anthropic-ai/claude-code` package layouts.
- Preserve PATH directory order; a later WindowsApps EXE must not shadow an earlier reviewed npm package.
- Version and login probes must not invoke a model or consume subscription quota.
- Do not expose absolute executable paths, raw PATH, authentication output, or raw process errors to the webview.
- Existing Node.js 22/24 LTS verifier prerequisite remains required for CLI benchmark execution.
- No real provider invocation is allowed in automated tests.

---

## File Structure

- `crates/ability-adapters/src/lib.rs` — public availability status and launch-source wire types.
- `crates/ability-adapters/src/command_locator.rs` — ordered, trusted candidate discovery only.
- `crates/ability-adapters/src/provider_detection.rs` — shared asynchronous version probing and failure aggregation.
- `crates/ability-adapters/src/codex.rs` — Codex auth probe and execution using the retained successful launch.
- `crates/ability-adapters/src/claude.rs` — Claude auth probe and execution using the retained successful launch.
- `crates/ability-adapters/tests/codex_adapter.rs` — Codex fallback and status behavior.
- `crates/ability-adapters/tests/claude_adapter.rs` — Claude fallback and status behavior.
- `apps/desktop/src-tauri/src/app_state.rs` — bootstrap availability for all four targets.
- `apps/desktop/src-tauri/src/commands.rs` — status-aware CLI readiness errors.
- `apps/desktop/src/api/backend.ts` — TypeScript availability contract.
- `apps/desktop/src/pages/HomePage.tsx` — stable Chinese state copy and source display.
- `apps/desktop/src/pages/HomePage.test.tsx` — frontend behavior for every state.

### Shared Interfaces

This plan produces:

```rust
pub enum AvailabilityStatus {
    Ready,
    NeedsLogin,
    NotFound,
    RuntimeMissing,
    EntryInaccessible,
    VersionProbeFailed,
}

pub enum LaunchSource {
    NativeExe,
    ReviewedNpm,
}

pub struct TargetAvailability {
    pub kind: TargetKind,
    pub installed: bool,
    pub version: Option<String>,
    pub auth_state: AuthState,
    pub status: AvailabilityStatus,
    pub source: Option<LaunchSource>,
    pub prerequisites: Vec<PrerequisiteStatus>,
}
```

Frontend wire values are snake case: `ready`, `needs_login`, `not_found`,
`runtime_missing`, `entry_inaccessible`, `version_probe_failed`,
`native_exe`, and `reviewed_npm`.

---

### Task 1: Add the Stable Availability Contract

**Files:**

- Modify: `crates/ability-adapters/src/lib.rs`
- Modify: `crates/ability-adapters/src/codex.rs`
- Modify: `crates/ability-adapters/src/claude.rs`
- Modify: `crates/ability-adapters/src/cli_run.rs`
- Modify: `crates/ability-adapters/tests/cli_run.rs`
- Modify: `apps/desktop/src-tauri/src/app_state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src/api/backend.ts`
- Modify: `apps/desktop/src/pages/HomePage.test.tsx`
- Modify: `apps/desktop/src/pages/CliRunPage.test.tsx`
- Modify: `apps/desktop/src/test/accessibility.test.tsx`
- Test: `crates/ability-adapters/src/lib.rs`

**Interfaces:**

- Consumes: existing `TargetKind`, `AuthState`, and `PrerequisiteStatus`.
- Produces: `AvailabilityStatus`, `LaunchSource`, and the extended
  `TargetAvailability` used by every later task.

- [ ] **Step 1: Write the failing Rust wire-shape test**

Add this test module to `crates/ability-adapters/src/lib.rs`:

```rust
#[cfg(test)]
mod availability_contract_tests {
    use super::*;
    use ability_core::TargetKind;
    use serde_json::json;

    #[test]
    fn availability_serializes_stable_status_and_source_values() {
        let value = serde_json::to_value(TargetAvailability {
            kind: TargetKind::CodexCli,
            installed: true,
            version: Some("codex-cli 0.142.5".into()),
            auth_state: AuthState::Ready,
            status: AvailabilityStatus::Ready,
            source: Some(LaunchSource::ReviewedNpm),
            prerequisites: Vec::new(),
        })
        .unwrap();

        assert_eq!(value["status"], json!("ready"));
        assert_eq!(value["source"], json!("reviewed_npm"));
    }
}
```

- [ ] **Step 2: Run the test and verify the contract is missing**

Run:

```powershell
cargo test -p ability-adapters availability_serializes_stable_status_and_source_values
```

Expected: compilation fails because `AvailabilityStatus`, `LaunchSource`,
`status`, and `source` do not exist.

- [ ] **Step 3: Add the Rust enums and fields**

Insert before `TargetAvailability` in `crates/ability-adapters/src/lib.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityStatus {
    Ready,
    NeedsLogin,
    NotFound,
    RuntimeMissing,
    EntryInaccessible,
    VersionProbeFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchSource {
    NativeExe,
    ReviewedNpm,
}
```

Extend `TargetAvailability` with:

```rust
pub status: AvailabilityStatus,
pub source: Option<LaunchSource>,
```

Add matching frontend types in `apps/desktop/src/api/backend.ts`:

```ts
export type AvailabilityStatus =
  | "ready"
  | "needs_login"
  | "not_found"
  | "runtime_missing"
  | "entry_inaccessible"
  | "version_probe_failed";

export type LaunchSource = "native_exe" | "reviewed_npm";
```

Extend `TargetAvailability`:

```ts
status: AvailabilityStatus;
source?: LaunchSource | null;
```

Update every initializer returned by:

```powershell
rg -n "TargetAvailability \{" crates apps -g "*.rs"
rg -n "installed:\s*(true|false)" apps/desktop/src -g "*.test.ts" -g "*.test.tsx"
```

Client targets use `Ready`/`None`, healthy CLI targets use `Ready` plus the
relevant source, and unavailable CLI fixtures use the status under test.
The expected touched files are exactly the files listed for this task; stop
and update this plan before staging if the search reveals another
initializer.

- [ ] **Step 4: Run contract and frontend type tests**

Run:

```powershell
cargo test -p ability-adapters availability_serializes_stable_status_and_source_values
npm run build --workspace apps/desktop
```

Expected: both commands pass.

- [ ] **Step 5: Commit the contract**

```powershell
git add crates/ability-adapters/src/lib.rs crates/ability-adapters/src/codex.rs crates/ability-adapters/src/claude.rs crates/ability-adapters/src/cli_run.rs crates/ability-adapters/tests/cli_run.rs apps/desktop/src-tauri/src/app_state.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src/api/backend.ts apps/desktop/src/pages/HomePage.test.tsx apps/desktop/src/pages/CliRunPage.test.tsx apps/desktop/src/test/accessibility.test.tsx
git commit -m "feat: add stable CLI availability states"
```

---

### Task 2: Discover Trusted Provider Candidates in PATH Order

**Files:**

- Modify: `crates/ability-adapters/src/command_locator.rs`
- Modify: `crates/ability-adapters/src/codex.rs`
- Modify: `crates/ability-adapters/src/claude.rs`
- Modify: `crates/ability-adapters/src/process.rs`
- Test: `crates/ability-adapters/src/command_locator.rs`

**Interfaces:**

- Consumes: `LaunchSource` from Task 1.
- Produces:

```rust
pub(crate) struct LaunchCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
    pub source: LaunchSource,
}

pub(crate) struct LaunchDiscovery {
    pub candidates: Vec<LaunchCommand>,
    pub reviewed_npm_without_node: bool,
}

pub(crate) fn discover_provider_commands(
    provider: &str,
    inherited_path: Option<&OsStr>,
) -> io::Result<LaunchDiscovery>;
```

- [ ] **Step 1: Replace the old precedence test with a failing PATH-order test**

Add a helper that writes an executable-looking file and then add:

```rust
#[test]
fn earlier_reviewed_npm_precedes_later_native_exe() {
    let temp = tempfile::tempdir().unwrap();
    let npm = temp.path().join("npm");
    let node_bin = temp.path().join("node");
    let later_native = temp.path().join("windows-app");
    std::fs::create_dir_all(npm.join("node_modules/@openai/codex/bin")).unwrap();
    std::fs::create_dir_all(&node_bin).unwrap();
    std::fs::create_dir_all(&later_native).unwrap();
    std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
    std::fs::write(
        npm.join("node_modules/@openai/codex/bin/codex.js"),
        "console.log('fake')",
    )
    .unwrap();
    write_package_metadata(
        &npm.join("node_modules/@openai/codex"),
        "@openai/codex",
        "codex",
        "bin/codex.js",
    );
    std::fs::write(node_bin.join("node.exe"), b"MZ").unwrap();
    std::fs::write(later_native.join("codex.exe"), b"MZ").unwrap();
    let path = std::env::join_paths([&npm, &node_bin, &later_native]).unwrap();

    let discovery = discover_provider_commands("codex", Some(&path)).unwrap();

    assert_eq!(discovery.candidates.len(), 2);
    assert_eq!(discovery.candidates[0].source, LaunchSource::ReviewedNpm);
    assert_eq!(discovery.candidates[1].source, LaunchSource::NativeExe);
}
```

Also add:

```rust
#[test]
fn reviewed_npm_without_node_is_reported_without_a_launch_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let npm = temp.path().join("npm");
    std::fs::create_dir_all(npm.join("node_modules/@openai/codex/bin")).unwrap();
    std::fs::write(npm.join("codex.cmd"), "@echo off").unwrap();
    std::fs::write(
        npm.join("node_modules/@openai/codex/bin/codex.js"),
        "console.log('fake')",
    )
    .unwrap();
    write_package_metadata(
        &npm.join("node_modules/@openai/codex"),
        "@openai/codex",
        "codex",
        "bin/codex.js",
    );
    let path = std::env::join_paths([&npm]).unwrap();

    let discovery = discover_provider_commands("codex", Some(&path)).unwrap();

    assert!(discovery.candidates.is_empty());
    assert!(discovery.reviewed_npm_without_node);
}
```

- [ ] **Step 2: Run locator tests and confirm the old locator fails**

Run:

```powershell
cargo test -p ability-adapters command_locator -- --nocapture
```

Expected: compilation fails because ordered discovery and source metadata do
not exist; the old `native_exe_wins_without_executing_any_shim` behavior is
incompatible with the new PATH-order assertion.

- [ ] **Step 3: Implement ordered discovery**

Change `LaunchCommand` to include `source`, derive `Clone` for both candidate
types, add `LaunchDiscovery`, and replace the Windows one-shot resolver with
this structure:

```rust
pub(crate) fn discover_provider_commands(
    provider: &str,
    inherited_path: Option<&OsStr>,
) -> io::Result<LaunchDiscovery> {
    let inherited = inherited_path
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is unavailable"))?;
    let directories = path_directories(inherited).collect::<Vec<_>>();
    let node = directories
        .iter()
        .map(|directory| directory.join("node.exe"))
        .find(|candidate| candidate.is_file());
    let mut candidates = Vec::new();
    let mut reviewed_npm_without_node = false;

    for directory in directories {
        let native = directory.join(format!("{provider}.exe"));
        if native.is_file() {
            candidates.push(LaunchCommand {
                program: native,
                prefix_args: Vec::new(),
                source: LaunchSource::NativeExe,
            });
        }

        if let Some(script) = reviewed_package_entry(&directory, provider) {
            if let Some(node) = node.as_ref() {
                candidates.push(LaunchCommand {
                    program: node.clone(),
                    prefix_args: vec![script.to_string_lossy().into_owned()],
                    source: LaunchSource::ReviewedNpm,
                });
            } else {
                reviewed_npm_without_node = true;
            }
        }
    }

    candidates.dedup_by(|left, right| {
        left.program == right.program && left.prefix_args == right.prefix_args
    });
    Ok(LaunchDiscovery {
        candidates,
        reviewed_npm_without_node,
    })
}
```

Keep `resolve_launch_command` for ordinary processes and absolute provider
launches. For a bare `codex` or `claude`, it must now return the first
candidate from `discover_provider_commands`; provider adapters will use the
full discovery API in Task 3.

In the existing `with_resolved_command` constructors in `codex.rs` and
`claude.rs`, set `source: LaunchSource::NativeExe`. This is a compatibility
edit required for the crate to compile after adding the field; Task 3 will
replace the adapters' one-shot discovery behavior.

Also set `source: LaunchSource::NativeExe` on the direct
`ProcessEnvironment::Clear` launch constructed in `process.rs`. That generic
branch resolves an executable path directly and therefore cannot be a reviewed
npm entry; this is the fourth required compatibility initializer.

- [ ] **Step 4: Run the full locator suite**

Run:

```powershell
cargo test -p ability-adapters command_locator -- --nocapture
```

Expected: all locator tests pass, including native, npm, package identity,
relative PATH, containment, ordering, and missing Node cases.

- [ ] **Step 5: Commit ordered discovery**

```powershell
git add crates/ability-adapters/src/command_locator.rs crates/ability-adapters/src/codex.rs crates/ability-adapters/src/claude.rs crates/ability-adapters/src/process.rs
git commit -m "fix: preserve PATH order for provider discovery"
```

---

### Task 3: Probe Candidates and Retain the First Working Launch

**Files:**

- Create: `crates/ability-adapters/src/provider_detection.rs`
- Modify: `crates/ability-adapters/src/lib.rs`
- Modify: `crates/ability-adapters/src/codex.rs`
- Modify: `crates/ability-adapters/src/claude.rs`
- Test: `crates/ability-adapters/tests/codex_adapter.rs`
- Test: `crates/ability-adapters/tests/claude_adapter.rs`

**Interfaces:**

- Consumes: `discover_provider_commands`, `LaunchCommand`,
  `AvailabilityStatus`, and `ProcessRunner`.
- Produces:

```rust
pub(crate) struct WorkingLaunch {
    pub launch: LaunchCommand,
    pub version: String,
}

pub(crate) async fn probe_launch_candidates(
    discovery: LaunchDiscovery,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus>;

pub(crate) async fn probe_provider_launches(
    provider: &str,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus>;
```

- [ ] **Step 1: Write failing adapter fallback tests**

Extend the Codex test runner so it records `ProcessSpec` and returns:

- `ProcessError::Spawn(PermissionDenied)` for a candidate whose program ends
  in `windows-app/codex.exe`;
- `codex-cli 0.142.5` for the reviewed npm Node launch;
- `Logged in` for the retained launch's `login status`.

Add this test:

```rust
#[tokio::test]
async fn codex_detection_skips_an_inaccessible_candidate_and_retains_npm() {
    let runner = Arc::new(OrderedCandidateRunner::default());
    let adapter = CodexAdapter::with_candidate_commands(
        runner.clone(),
        vec![
            (
                PathBuf::from("windows-app/codex.exe"),
                Vec::new(),
                LaunchSource::NativeExe,
            ),
            (
                PathBuf::from("node.exe"),
                vec!["npm/node_modules/@openai/codex/bin/codex.js".into()],
                LaunchSource::ReviewedNpm,
            ),
        ],
        false,
    );

    let availability = adapter.detect().await;

    assert!(availability.installed);
    assert_eq!(availability.status, AvailabilityStatus::Ready);
    assert_eq!(availability.source, Some(LaunchSource::ReviewedNpm));
    assert_eq!(availability.version.as_deref(), Some("codex-cli 0.142.5"));
    adapter
        .execute(sample_request(), CancellationToken::new())
        .await
        .unwrap();
    assert!(runner.execution_used_reviewed_npm());
}
```

Add unit tests inside `provider_detection.rs` asserting:

```rust
assert_eq!(availability.status, AvailabilityStatus::RuntimeMissing);
assert_eq!(availability.status, AvailabilityStatus::EntryInaccessible);
assert_eq!(availability.status, AvailabilityStatus::VersionProbeFailed);
```

Use a synthetic never-completing runner plus a 10-millisecond injected total
budget to assert timeout also becomes `VersionProbeFailed`; no test waits for
the production 25-second budget. Add the equivalent fallback test to
`claude_adapter.rs`.

- [ ] **Step 2: Run adapter tests and confirm they fail**

Run:

```powershell
cargo test -p ability-adapters --test codex_adapter -- --nocapture
cargo test -p ability-adapters --test claude_adapter -- --nocapture
```

Expected: compilation fails because candidate injection, shared probing, and
precise availability states are absent.

- [ ] **Step 3: Implement shared candidate probing**

Create `provider_detection.rs` with the two-layer probe. The public-to-crate
entry discovers the real PATH, while the lower layer accepts synthetic
discovery for tests:

```rust
use crate::command_locator::{
    LaunchCommand, LaunchDiscovery, discover_provider_commands,
};
use crate::{
    AvailabilityStatus, ProcessEnvironment, ProcessError, ProcessRunner, ProcessSpec,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(crate) struct WorkingLaunch {
    pub launch: LaunchCommand,
    pub version: String,
}

const SINGLE_PROBE_BUDGET: Duration = Duration::from_secs(8);
const TOTAL_PROBE_BUDGET: Duration = Duration::from_secs(25);

pub(crate) async fn probe_provider_launches(
    provider: &str,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    let discovery = discover_provider_commands(provider, std::env::var_os("PATH").as_deref())
        .map_err(|_| AvailabilityStatus::NotFound)?;
    probe_launch_candidates(discovery, runner).await
}

pub(crate) async fn probe_launch_candidates(
    discovery: LaunchDiscovery,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    probe_launch_candidates_with_budget(
        discovery,
        runner,
        TOTAL_PROBE_BUDGET,
    )
    .await
}

async fn probe_launch_candidates_with_budget(
    discovery: LaunchDiscovery,
    runner: Arc<dyn ProcessRunner>,
    total_budget: Duration,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    match tokio::time::timeout(
        total_budget,
        probe_launch_candidates_inner(discovery, runner),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(AvailabilityStatus::VersionProbeFailed),
    }
}

async fn probe_launch_candidates_inner(
    discovery: LaunchDiscovery,
    runner: Arc<dyn ProcessRunner>,
) -> Result<WorkingLaunch, AvailabilityStatus> {
    if discovery.candidates.is_empty() {
        return Err(if discovery.reviewed_npm_without_node {
            AvailabilityStatus::RuntimeMissing
        } else {
            AvailabilityStatus::NotFound
        });
    }

    let reviewed_npm_without_node = discovery.reviewed_npm_without_node;
    let mut inaccessible = false;
    let mut probe_failed = false;
    for launch in discovery.candidates {
        let mut args = launch.prefix_args.clone();
        args.push("--version".into());
        let output = runner
            .run(
                ProcessSpec {
                    program: launch.program.clone(),
                    args,
                    current_dir: std::env::temp_dir(),
                    env: BTreeMap::new(),
                    environment: ProcessEnvironment::Inherit,
                    timeout: SINGLE_PROBE_BUDGET,
                },
                CancellationToken::new(),
            )
            .await;
        match output {
            Ok(output)
                if output.exit_code == Some(0)
                    && valid_version_text(output.stdout.trim()) =>
            {
                return Ok(WorkingLaunch {
                    launch,
                    version: output.stdout.trim().to_owned(),
                });
            }
            Err(ProcessError::Spawn(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                ) =>
            {
                inaccessible = true;
            }
            Err(ProcessError::Supervision(_)) => inaccessible = true,
            _ => probe_failed = true,
        }
    }

    Err(if probe_failed {
        AvailabilityStatus::VersionProbeFailed
    } else if reviewed_npm_without_node {
        AvailabilityStatus::RuntimeMissing
    } else if inaccessible {
        AvailabilityStatus::EntryInaccessible
    } else {
        AvailabilityStatus::NotFound
    })
}

fn valid_version_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
}
```

Register the module in `lib.rs`:

```rust
mod provider_detection;
```

- [ ] **Step 4: Refactor Codex and Claude adapters**

For each adapter, use:

```rust
pub struct CodexAdapter {
    runner: Arc<dyn ProcessRunner>,
    launch: Mutex<Option<LaunchCommand>>,
    discovery_override: Option<LaunchDiscovery>,
}
```

`ClaudeCodeAdapter` uses the same fields. `new` sets no override and no
retained launch. `with_resolved_command` creates a one-candidate override and
also seeds `launch`, preserving tests that intentionally execute without
calling `detect` first. Add this public test constructor without exposing
the private candidate type:

```rust
pub fn with_candidate_commands(
    runner: Arc<dyn ProcessRunner>,
    candidates: Vec<(PathBuf, Vec<String>, LaunchSource)>,
    reviewed_npm_without_node: bool,
) -> Self
```

It converts tuples into a `LaunchDiscovery`, stores it in
`discovery_override`, and leaves `launch` empty. Then:

1. If an override exists, `detect()` calls `probe_launch_candidates`; otherwise
   it calls `probe_provider_launches`.
2. Store only the launch that passed the version probe.
3. Run auth status against that stored launch.
4. `execute()` uses only the retained successful launch; it never rediscovers
   or silently switches candidates. The resolved-command constructor remains
   the explicit test escape hatch.
5. Return `installed: true` for a successful version probe even when auth is
   unknown.
6. Set status to `needs_login` only when the official auth command says login
   is required.
7. Return the exact shared failure status when no launch succeeds.

The successful return shape is:

```rust
TargetAvailability {
    kind: self.kind(),
    installed: true,
    version: Some(working.version),
    auth_state,
    status: if auth_state == AuthState::NeedsLogin {
        AvailabilityStatus::NeedsLogin
    } else {
        AvailabilityStatus::Ready
    },
    source: Some(working.launch.source),
    prerequisites: Vec::new(),
}
```

The failure helper is:

```rust
fn unavailable(kind: TargetKind, status: AvailabilityStatus) -> TargetAvailability {
    TargetAvailability {
        kind,
        installed: false,
        version: None,
        auth_state: AuthState::Unknown,
        status,
        source: None,
        prerequisites: Vec::new(),
    }
}
```

The shared probe unit tests cover all failure aggregation, including the
25-second total budget. The Codex and Claude adapter tests cover retained
fallback execution so the second command cannot return to the inaccessible
first candidate.

- [ ] **Step 5: Run all adapter tests**

Run:

```powershell
cargo test -p ability-adapters --all-targets -- --nocapture
```

Expected: all ability-adapters unit and integration tests pass; no test runs a
real provider.

- [ ] **Step 6: Commit provider fallback**

```powershell
git add crates/ability-adapters/src/provider_detection.rs crates/ability-adapters/src/lib.rs crates/ability-adapters/src/codex.rs crates/ability-adapters/src/claude.rs crates/ability-adapters/tests/codex_adapter.rs crates/ability-adapters/tests/claude_adapter.rs
git commit -m "fix: fall back across trusted CLI candidates"
```

---

### Task 4: Surface Precise CLI States in Tauri and Home

**Files:**

- Modify: `apps/desktop/src-tauri/src/app_state.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src/pages/HomePage.tsx`
- Modify: `apps/desktop/src/pages/HomePage.test.tsx`
- Modify: `apps/desktop/src/test/accessibility.test.tsx`

**Interfaces:**

- Consumes: Task 1 availability wire contract and Task 3 adapter results.
- Produces: user-facing state copy and source labels without raw paths.

- [ ] **Step 1: Write frontend tests for every stable state**

Add table-driven tests to `HomePage.test.tsx`:

```ts
test.each([
  ["not_found", "未检测到受支持入口"],
  ["runtime_missing", "缺少 Node.js 运行时"],
  ["entry_inaccessible", "入口不可访问"],
  ["version_probe_failed", "版本检测失败"],
  ["needs_login", "需要先在终端登录"],
] as const)("renders %s as %s", async (status, copy) => {
  const bootstrap = readyBootstrap();
  bootstrap.targets = bootstrap.targets.map((target) =>
    target.kind === "codex_cli"
      ? {
          ...target,
          installed: status === "needs_login",
          status,
          authState: status === "needs_login" ? "needs_login" : "unknown",
          version: status === "needs_login" ? "codex-cli 0.142.5" : null,
          source: status === "needs_login" ? "reviewed_npm" : null,
        }
      : target,
  );

  renderHome(backendFor(async () => bootstrap));

  expect(
    await screen.findByRole("status", {
      name: `Codex CLI 状态：${copy}`,
    }),
  ).toBeInTheDocument();
});
```

Add an assertion for a ready npm installation:

```ts
expect(screen.getByText("npm 安装")).toBeInTheDocument();
```

- [ ] **Step 2: Run the Home tests and verify copy is missing**

Run:

```powershell
npm test --workspace apps/desktop -- src/pages/HomePage.test.tsx
```

Expected: the new state-copy and source assertions fail.

- [ ] **Step 3: Update backend readiness mapping**

In `app_state.rs`, create manual target availability as:

```rust
status: AvailabilityStatus::Ready,
source: None,
```

Keep the Node verifier prerequisite appended to both CLI targets.

In `commands.rs`, replace the generic `!installed` error with:

```rust
match availability.status {
    AvailabilityStatus::Ready => {}
    AvailabilityStatus::NeedsLogin => {
        return Err("所选 CLI 尚未登录，请先在终端完成登录".into());
    }
    AvailabilityStatus::NotFound => {
        return Err("未检测到受支持的 CLI 入口".into());
    }
    AvailabilityStatus::RuntimeMissing => {
        return Err("CLI 的 Node.js 运行时不可用".into());
    }
    AvailabilityStatus::EntryInaccessible => {
        return Err("检测到 CLI 入口，但当前应用无权启动".into());
    }
    AvailabilityStatus::VersionProbeFailed => {
        return Err("CLI 版本检测失败".into());
    }
}
```

Continue validating the public CLI version and Node 22/24 prerequisite after
the status match.

- [ ] **Step 4: Render status and source from the stable enum**

In `HomePage.tsx`, replace the generic installed blocker with:

```ts
const statusCopy: Record<AvailabilityStatus, string | null> = {
  ready: null,
  needs_login: "需要先在终端登录",
  not_found: "未检测到受支持入口",
  runtime_missing: "缺少 Node.js 运行时",
  entry_inaccessible: "入口不可访问",
  version_probe_failed: "版本检测失败",
};

const sourceCopy: Record<LaunchSource, string> = {
  native_exe: "原生安装",
  reviewed_npm: "npm 安装",
};
```

For CLI targets, use `statusCopy[target.status]` before prerequisite and
legacy `installed` fallbacks. This preserves an exact candidate failure such
as `entry_inaccessible`; when status is `ready`, a missing Node verifier
prerequisite still becomes the blocker. Display the source only when status
is ready or needs login:

```tsx
{target.source &&
target.status !== "not_found" &&
target.status !== "runtime_missing" &&
target.status !== "entry_inaccessible" &&
target.status !== "version_probe_failed" ? (
  <p className="target-source">
    入口来源：{sourceCopy[target.source]}
  </p>
) : null}
```

Do not render absolute paths or raw errors.

- [ ] **Step 5: Run Tauri and frontend tests**

Run:

```powershell
cargo test -p ability-radar --lib
npm test --workspace apps/desktop -- src/pages/HomePage.test.tsx src/test/accessibility.test.tsx
```

Expected: all selected tests pass.

- [ ] **Step 6: Commit the UI state integration**

```powershell
git add apps/desktop/src-tauri/src/app_state.rs apps/desktop/src-tauri/src/commands.rs apps/desktop/src/pages/HomePage.tsx apps/desktop/src/pages/HomePage.test.tsx apps/desktop/src/test/accessibility.test.tsx
git commit -m "feat: explain CLI detection failures"
```

---

### Task 5: Real-Machine Verification and Plan Gate

**Files:**

- Modify if evidence requires correction:
  `docs/troubleshooting.md`
- Test: existing Rust, frontend, and repository suites.

**Interfaces:**

- Consumes: the completed trusted detection path.
- Produces: a verified base for the client-model and visual plans.

- [ ] **Step 1: Run focused and full automated verification**

Run:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm test --workspace apps/desktop
npm run validate:repository
```

Expected: every command exits zero and no test invokes Codex or Claude.

- [ ] **Step 2: Launch the source build**

Run:

```powershell
npm start
```

Expected: Tauri opens the current source build.

- [ ] **Step 3: Verify the current machine**

On the home page:

1. Codex CLI shows `codex-cli 0.142.5`.
2. The source label is `npm 安装`.
3. Codex is no longer blocked by the later WindowsApps resource.
4. Claude Code remains unavailable if it is not installed.
5. Clicking “重新检测 CLI” refreshes state without starting a model request.

Also run this read-only shell confirmation:

```powershell
codex --version
```

Expected: `codex-cli 0.142.5`.

- [ ] **Step 4: Update troubleshooting only with verified copy**

If the UI and shell verification pass, add this exact distinction to
`docs/troubleshooting.md`:

```text
“未检测到受支持入口”表示 PATH 中没有通过安全审核的安装；
“入口不可访问”表示发现了候选但 Windows 拒绝启动；
“版本检测失败”表示候选可以启动但没有通过只读版本探测。
重新检测不会调用模型，也不会消耗订阅额度。
```

- [ ] **Step 5: Commit verified documentation**

```powershell
git add docs/troubleshooting.md
git commit -m "docs: explain trusted CLI detection states"
```

If `docs/troubleshooting.md` already contains the exact verified guidance,
skip the commit and record the clean status in the execution notes.

---

### Task 6: Keep the Reviewed npm Entry Node-Compatible on Windows

**Files:**

- Modify: `crates/ability-adapters/src/command_locator.rs`
- Modify: `crates/ability-adapters/tests/process_contract.rs`

**Interfaces:**

- Consumes: the canonical package-identity and containment checks from Task 2.
- Produces: a reviewed npm launch whose Node main-script argument remains the
  validated ordinary absolute path instead of Windows' `\\?\` verbatim
  canonical form.

- [ ] **Step 1: Write the failing Windows launch-path regression**

Update the reviewed npm locator assertion so it requires the fixed lexical
entry path built from the absolute PATH directory and the reviewed relative
entry. On Windows, also assert that the Node prefix argument does not start
with `\\?\`.

Update the child-scoped process-contract expectation to the same ordinary
absolute entry path. Run:

```powershell
cargo test -p ability-adapters command_locator -- --nocapture
cargo test -p ability-adapters --test process_contract -- --nocapture
```

Expected before the implementation: the locator regression fails because
`canonical_reviewed_file` returns the `std::fs::canonicalize` result as the
Node argument.

- [ ] **Step 2: Separate trust validation from the launch spelling**

Keep canonicalization for both the reviewed package root and candidate, and
keep the exact canonical containment/relative-path check. After the entry
passes that check, return the original absolute
`package_root.join(entry_relative)` spelling for Node to execute:

```rust
let entry = package_root.join(entry_relative);
canonical_reviewed_file(&package_root, &entry, entry_relative)?;
Some(entry)
```

Do not weaken package name, single exact `bin` mapping, shim evidence, or
canonical containment. Do not add a shell/npm-shim launch path.

- [ ] **Step 3: Run focused and strict regression gates**

Run:

```powershell
cargo test -p ability-adapters command_locator -- --nocapture
cargo test -p ability-adapters --test process_contract -- --nocapture
cargo test -p ability-adapters --all-targets -- --nocapture
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all commands pass and no test invokes a real provider or model.

- [ ] **Step 4: Commit the Windows path correction**

```powershell
git add crates/ability-adapters/src/command_locator.rs crates/ability-adapters/tests/process_contract.rs
git commit -m "fix: pass Node-compatible reviewed entry paths"
```

After independent review, repeat every Task 5 automated and real-machine gate
before adding troubleshooting guidance.
