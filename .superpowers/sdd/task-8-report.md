# Task 8 Report: Codex CLI Adapter

## Delivered behavior

- Added shared `AgentAdapter` contracts, availability/auth status models, execution request,
  local completion, and adapter error models to `ability-adapters`.
- Added `CodexAdapter`, which detects availability only through `codex --version` and
  `codex login status`; it never reads authentication files.
- Executes the exact direct argv contract: `codex exec --ephemeral --json --sandbox
  workspace-write --ignore-user-config --ignore-rules`, followed by optional owned model and
  reasoning config values, then the owned prompt. It does not construct a shell command and it
  does not add an unapproved `max_turns` argument.
- Uses the request workspace and `time_budget_secs` as the process working directory and fixed
  timeout. Stdout is accepted only when every nonblank line is valid JSON, no `turn.failed` or
  `error` event appears, and the final event is `turn.completed` with exit code zero.
- Preserves runner outcomes truthfully: timeout is agent-budget exhaustion, cancellation remains
  cancellation, a missing executable is unavailable, and every other runner error is an
  infrastructure interruption. Agent-budget markers in returned CLI output are checked before
  generic CLI failure classification.
- Raw stdout and stderr are retained only in the returned local completion or error object; the
  adapter has no logging, serialization, upload, environment values, or model invocation.

## RED evidence

Before production adapter code was added, I created `tests/codex_adapter.rs` with fake runners
only and ran the required VS developer shell followed by:

```powershell
cargo test -p ability-adapters --test codex_adapter
```

Result: expected RED failure `E0432`, because `AdapterCompletion`, `AdapterError`,
`AgentAdapter`, `AuthState`, `CodexAdapter`, and `ExecutionRequest` did not yet exist in the
crate exports. No Codex CLI process or paid model invocation occurred.

## Verification

Every Rust build/test command below was preceded in its PowerShell process by:

```powershell
& 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1' -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
```

- `cargo test -p ability-adapters --test codex_adapter` — PASS, 10 tests.
- `cargo test -p ability-adapters --test process_contract` — PASS, 12 tests.
- `cargo test -p ability-adapters` — PASS, 22 integration tests plus unit/doc targets.
- `cargo test --workspace` — PASS, all workspace tests. The desktop library emitted its existing
  linker message warning; no test failed.
- `cargo fmt --all --check` — PASS (after applying `cargo fmt --all`).
- `git diff --check 9854146f39e58da89e4ff359b25ba08ff36a4b03` — PASS.

## Commit / handoff convention

- Message: `feat: automate Codex CLI tasks`.
- The final commit SHA is recorded in the task handoff after committing. A commit cannot embed its
  own final content hash, so this report intentionally uses that handoff convention.

## Self-review

- Confirmed only fake `ProcessRunner` implementations are used in adapter tests and no Codex
  executable is invoked by a test.
- Confirmed optional model, reasoning effort, and prompt occupy separate argv elements even when
  they contain spaces or shell metacharacters.
- Confirmed detection makes exactly CLI version/status requests and does not inspect auth files.
- Confirmed malformed nonblank JSONL, missing completion, nonterminal completion, `turn.failed`,
  `error`, and nonzero exits are all rejected.
- Confirmed every Task 7 `ProcessError` variant is explicitly mapped; only `TimedOut` becomes an
  agent-budget outcome.
- Confirmed the adapter neither emits environment values nor persists/logs raw process output.

## Concerns

None. The workspace suite's desktop linker message warning pre-existed and did not affect test
outcomes.

## Review follow-up: terminal JSONL and TOML-safe configuration

The review found two important correctness gaps and both are resolved in the amended commit.

- JSONL completion now uses an explicit `Open` / `Completed` / `Failed` state machine. A stream
  succeeds only if it reaches one `turn.completed` terminal event and every remaining line is
  blank. A second completion, a nonterminal event after completion, or any event after a failure
  terminal is invalid.
- Reasoning effort is now encoded with `serde_json::to_string` before it is placed after the
  `model_reasoning_effort=` TOML assignment. JSON string escaping is compatible with TOML basic
  strings, so quotes, backslashes, and newlines stay data in one `--config` argv value rather
  than becoming TOML/argv injection.

### RED/GREEN evidence

Before either production change, after initializing the required VS developer shell, I ran:

```powershell
cargo test -p ability-adapters --test codex_adapter
```

Result: expected RED failure, 11 passed / 2 failed. The duplicate
`turn.completed` stream incorrectly returned local completion, and a reasoning effort containing
a quote, backslash, newline, and `--config` text produced an unescaped configuration argument.
The focused suite then passed 13/13 after the state-machine and JSON string-encoding changes.
All tests continue to use fake runners only; no Codex CLI or paid model run occurred.

### Follow-up verification

Every Rust build/test command was run after the VS developer shell command recorded above.

- `cargo test -p ability-adapters --test codex_adapter` — PASS, 13 tests.
- `cargo test -p ability-adapters --test process_contract` — PASS, 12 tests.
- `cargo test -p ability-adapters` — PASS, 25 integration tests plus unit/doc targets.
- `cargo test --workspace` — PASS, all workspace tests; the existing desktop linker message
  warning remained non-failing.
- `cargo fmt --all --check` — PASS.
- `git diff --check 9854146f39e58da89e4ff359b25ba08ff36a4b03` — PASS.

### Follow-up self-review

- Confirmed duplicate completion, completion/nonterminal/completion, and trailing blank-lines
  regressions are covered and that only one terminal completion is accepted.
- Confirmed the delimiter test asserts the complete argv, one `--config` occurrence, and JSON
  round-tripping of the TOML value, preventing a second config/argv injection.
- Recorded reviewer minors without expanding production scope: detection-state/spec assertions,
  command working-directory/timeout/empty-environment assertions, and serde wire-format
  regressions remain useful isolated future coverage additions.
