# Task 7 Report: Bounded Process Runner

## Delivered behavior

- Added `ProcessSpec`, `ProcessOutput`, `ProcessError`, `ProcessRunner`, and
  `TokioProcessRunner` to `ability-adapters`.
- Executes a program directly with its exact argv vector, an owned working
  directory, and an environment overlay; it does not build a shell command or
  log environment values.
- Reads stdout and stderr concurrently, retaining at most 1 MiB per stream.
  A limit breach returns the stream-specific `OutputLimit` error without
  including captured content and cleans up the process tree.
- Returns exit status, both captured streams, and a checked `u128` to `u64`
  elapsed-millisecond conversion.
- A pre-cancelled token returns before spawning. On Windows, the runner creates
  a kill-on-close Job Object, assigns a suspended child before resuming it, and
  confirms the Job Object has no assigned process before reporting a terminal
  cleanup outcome. Non-Windows builds use a best-effort direct kill and reap
  fallback.

## Reconstructed RED evidence

The interrupted worktree contained an unimplemented runner and an untracked
`process_contract.rs`. I inspected the test contract before writing production
code, retained its valid cases, corrected its output-leak assertion to use the
actual emitted `Z` marker, and ran:

```powershell
& 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1' -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
cargo test -p ability-adapters --test process_contract --locked
```

Result: expected RED failure (`E0432`) for unresolved `OutputStream`,
`ProcessError`, `ProcessRunner`, `ProcessSpec`, `TokioProcessRunner`, and
`MAX_CAPTURE_BYTES_PER_STREAM` imports. No runner implementation existed at
that point.

## Verification

All Rust commands below were run after initializing the required VS developer
shell shown above.

- `cargo test -p ability-adapters --test process_contract --locked` — PASS,
  8 tests.
- `cargo test -p ability-adapters --locked` — PASS, 8 integration tests and
  unit/doc targets.
- `cargo test --workspace --locked` — PASS, all workspace tests. The desktop
  crate emitted its existing linker message warning; no test failed.
- `cargo fmt --all --check` — PASS.
- `git diff --check ee0acfa` — PASS.

## Second review follow-up: parent-exit proof

The Windows process-tree harness now publishes both direct-parent and helper
PIDs. Tests open each published process with `PROCESS_SYNCHRONIZE`, release a
test-controlled parent-exit gate, and require the direct-parent handle to be
signaled before triggering cancellation, allowing timeout, or opening the
descendant's overflow gate. The overflow bytes are now emitted by the
supervised descendant rather than the direct parent. Each cleanup case also
waits for the helper process handle to be signaled and checks that its delayed
sentinel was not written. The unmanaged positive control proves that the same
descendant writes its sentinel when no runner cleanup occurs.

### RED/GREEN evidence

The first focused run of the strengthened harness failed because it attempted
to read the parent PID before the parent had published it (`Os { code: 2,
NotFound }`). After adding an explicit PID-file readiness wait before opening
the synchronization handle, the following focused runs passed from the
required VS developer shell:

- `cargo test -p ability-adapters --test process_contract cancellation_kills_a_ready_grandchild_after_the_parent_exits --locked -- --nocapture` — PASS.
- `cargo test -p ability-adapters --test process_contract timeout_kills_a_ready_grandchild_after_the_parent_exits --locked -- --nocapture` — PASS.
- `cargo test -p ability-adapters --test process_contract output_limit_kills_a_ready_grandchild_after_the_parent_exits --locked -- --nocapture` — PASS.
- `cargo test -p ability-adapters --test process_contract --locked` — PASS,
  12 tests.

### Second follow-up self-review

- Confirmed every parent-exits-first cleanup test opens the live parent handle
  before releasing the parent-exit gate and waits for that exact handle to be
  signaled before cleanup can start.
- Confirmed cancellation, timeout, and overflow all use a supervised helper
  PID and verify it is signaled after cleanup, in addition to the sentinel
  assertion.
- Confirmed the overflow test's direct parent exits before the test releases
  the descendant gate; only the descendant writes over the bounded stderr
  stream.
- Recorded but did not expand the review's non-blocking cautions: capture task
  join behavior, NTSTATUS translation and undocumented `NtResumeProcess`, and
  fixed Job Object PID-list capacity. Windows 10/11 x64 remains the accepted
  release target.

### Second follow-up verification

- `cargo test -p ability-adapters --locked` — PASS.
- `cargo test --workspace --locked` — PASS (the existing desktop linker
  message warning remained; no test failed).
- `cargo fmt --all --check` — PASS.
- `git diff --check ee0acfa` — PASS.
- A final post-contract process query found no `delayed-sentinel`,
  `start-helper`, `orphan-sentinel`, or `parent-exit.gate` PowerShell helper
  process. A stale parent from the earlier intentionally failing PID-ordering
  test was terminated by its verified PID before this final clean rerun.

## Third review follow-up: timeout liveness proof

The timeout test now makes the runner's seven-second timeout strictly greater
than the finite six-second worst case of its two PID/readiness waits and its
direct-parent signal wait. Its helper's natural sentinel deadline is 8.2
seconds after helper readiness. Once the exact parent handle is signaled, the
test explicitly requires that the runner future is still unfinished and that
the helper handle is still `WAIT_TIMEOUT` before it awaits the natural
`TimedOut` result. This distinguishes a real elapsed timeout from Job Object
cleanup causing the parent signal.

### RED/GREEN evidence and final verification

Review of the prior timeout contract was the RED evidence: its four-second
timeout could expire before the bounded readiness and parent-exit waits
completed, permitting a false pass. The strengthened timeout test passed:

- `cargo test -p ability-adapters --test process_contract timeout_kills_a_ready_grandchild_after_the_parent_exits --locked -- --nocapture` — PASS.
- `cargo test -p ability-adapters --test process_contract --locked` — PASS,
  12 tests.
- `cargo test -p ability-adapters --locked` — PASS.
- `cargo test --workspace --locked` — PASS (existing desktop linker message
  warning only).
- `cargo fmt --all --check` — PASS.
- `git diff --check ee0acfa` — PASS.
- Final helper-process query — PASS; no test helper remained.

### Third follow-up self-review

- Confirmed the timeout cannot precede the stated finite synchronization
  budget, and the helper's natural sentinel deadline is later than that
  timeout.
- Confirmed the liveness checks use the already-opened helper process handle
  and do not consume or close it before final post-cleanup confirmation.
- Left the review's unchanged non-blocking cautions recorded: detached capture
  tasks, NTSTATUS/Win32 diagnostics, undocumented `NtResumeProcess`, and fixed
  Job Object PID-list capacity.

## Commit

- Initial SHA before review amendment: `df8f495ca5cc5db18e36ae8929e226102107863b`.
- The amended SHA is recorded in the task handoff (a commit cannot embed its
  own final content hash).
- Message: `feat: add bounded process runner`

## Self-review

- Confirmed direct `Command::new(program).args(args)` execution preserves argv
  boundaries; no shell command construction is present.
- Confirmed the cancellation check occurs before `Command::spawn`.
- Confirmed capture tasks drain both pipes concurrently and impose an exact
  per-stream bound; every capture terminal error routes through cleanup.
- Confirmed Windows cleanup uses a race-free suspended-child Job Object,
  verifies an empty assigned-process list, and reaps the child with a bounded
  wait; cleanup failure is explicit.
- Confirmed the Windows sentinel test waits beyond the grandchild's delayed
  write deadline and finds no orphaned sentinel.
- Confirmed no environment values are serialized, logged, or sent externally.

## Concerns

None for the Windows 10/11 x64 release gate. Non-Windows tree cleanup remains
the documented best-effort fallback.

## Review follow-up: Job Object containment

The original `taskkill` cleanup was replaced after review with Windows Job
Object supervision. The runner now creates a job with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, starts the direct child suspended,
assigns its process handle to the job, and resumes it only after assignment.
The job's assigned-process list, rather than the direct child's exit status,
controls successful completion. Every post-spawn terminal path (assignment or
resume failure, missing capture pipe, child-wait error, capture read error,
closed capture channel, cancellation, timeout, output limit, and job-query
error) calls the same terminate-and-confirm path. That path terminates the job,
waits until its assigned-process list is empty, and reaps the direct child; a
failure at any point returns `TerminationFailed`.

`ProcessSpec` now has an explicit `Debug` implementation which shows only
environment keys, never values.

### Additional RED/GREEN evidence

Before the Job Object implementation, the focused contract command below
failed as expected: the Debug test found its unique secret marker in
`ProcessSpec` formatting, and the parent-exits-first readiness tests could not
observe a supervised helper. The corrected contract suite is green with a
positive control, deterministic helper-readiness handshake, serialized
Windows process-tree cases, and delayed-sentinel assertions for cancellation,
timeout, and output-limit cleanup.

```powershell
& 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1' -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
cargo test -p ability-adapters --test process_contract --locked
```

Result: PASS, 12 tests. A post-suite `Win32_Process` command-line check found
no `delayed-sentinel`, `start-helper`, or `orphan-sentinel` helper process.

### Follow-up self-review

- Verified `NtResumeProcess` is invoked only after successful Job Object
  assignment, removing the spawn-before-assignment race.
- Verified completion cannot be inferred from direct-child reaping: it requires
  an empty Job Object assigned-process list.
- Verified the test helper would write its sentinel unmanaged, then proved each
  cleanup outcome prevents that write after the direct parent exits.
- Verified `ability-core`, `serde`, and `serde_json` remain as the approved
  Task 7 dependencies despite not yet being used directly.

### Follow-up verification

After the Job Object rewrite, all commands were run from the VS developer
shell specified by the brief:

- `cargo test -p ability-adapters --test process_contract --locked` — PASS,
  12 tests.
- `cargo test -p ability-adapters --locked` — PASS.
- `cargo test --workspace --locked` — PASS. The existing desktop linker
  message warning remained, with no test failures.
- `cargo fmt --all --check` — PASS.
- `git diff --check ee0acfa` — PASS.
