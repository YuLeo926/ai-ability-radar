# Multi-Target Batch Scanning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans`. Implement one task at a time with TDD and a
> fresh fixed-base review package.

**Goal:** Run several explicitly identified targets inside a homogeneous
client or CLI comparison cohort against the same sealed pack, preserve
isolation evidence, estimate subscription exposure before starting, and detect
a likely regression only against a frozen compatible historical baseline.

**Architecture:** Keep `RunRecord` and `TaskResult` canonical. Add a batch
coordination layer that owns immutable targets, a deterministic member
schedule, execution authorizations, and atomically pre-owned ordinary runs.
Guided-client and automated-CLI cohorts are separate because their current
packs and graders are incompatible; a campaign may present both only as
non-comparable groups. Guided clients advance one member at a time with
per-task user attestation. CLI members run sequentially through reviewed
adapters. A pure analysis module freezes historical evidence before the
candidate batch and performs cluster-aware, versioned analysis.

**Tech stack:** Rust 2024, SQLite/rusqlite, Tauri 2, Tokio,
`CancellationToken`, React 19, TypeScript 5.8, Vitest.

**Design authority:**
`docs/superpowers/specs/2026-07-23-multi-target-batch-scan-design.md`.

## Dependencies

Do not execute this plan until all three v0.2.2 plans pass their final
whole-branch review:

- `2026-07-20-trusted-windows-cli-detection.md`
- `2026-07-20-client-model-identification-and-provenance.md`
- `2026-07-20-precision-radar-ui-and-v022-integration.md`

The v0.3/v0.4 tasks may ship with only `insufficient_data` and descriptive
comparisons. Task 7 regression labels cannot ship until a versioned calibration
policy has real-user acceptance evidence.

## Global constraints

- Never infer a hidden model from answer text or model self-report.
- Never start a provider request before displaying and acknowledging the
  immutable cost estimate.
- Never compare guided-client and CLI scores until both runners implement one
  reviewed cross-surface pack and grader contract.
- Normal automated tests use fake adapters and synthetic answers only.
- Default CLI batch concurrency is exactly one.
- No silent retry budget in the first implementation.
- Guided-client isolation is user-attested per task; CLI task session/workspace
  isolation is machine-enforced. Never present the former as verified.
- Infrastructure/auth/quota/network failures are invalid evidence, not wrong
  answers.
- `default_route` remains a route identity and never becomes a concrete model.
- Old databases and old single-run history remain readable.
- Database ownership changes are transactional. Filesystem deletion uses a
  recoverable two-phase lifecycle; backup import/restore is out of scope.
- Do not add action-capable Windows client automation in this plan.
- Do not push, publish, sign, package, install, or run real subscription tests
  unless a later acceptance task explicitly authorizes them.

## Planned file structure

### Core

- Create `crates/ability-core/src/batch.rs`
- Create `crates/ability-core/src/batch_schedule.rs`
- Create `crates/ability-core/src/batch_analysis.rs`
- Create `crates/ability-core/migrations/0003_scan_batches.sql`
- Create `crates/ability-core/tests/batch_contracts.rs`
- Create `crates/ability-core/tests/batch_storage.rs`
- Create `crates/ability-core/tests/batch_schedule.rs`
- Create `crates/ability-core/tests/batch_analysis.rs`
- Create `crates/ability-core/tests/batch_report_schema.rs`
- Create `schemas/public-batch-report.schema.json`
- Modify `crates/ability-core/src/lib.rs`
- Modify `crates/ability-core/src/storage.rs`
- Modify `crates/ability-core/src/orchestration.rs`
- Modify `crates/ability-core/src/domain.rs`
- Modify `crates/ability-core/tests/domain_contracts.rs`
- Modify `crates/ability-core/tests/recovery.rs`

### Adapters

- Modify `crates/ability-adapters/src/cli_run.rs`
- Modify `crates/ability-adapters/tests/cli_run.rs`

### Desktop backend

- Create `apps/desktop/src-tauri/src/batch_commands.rs`
- Create `apps/desktop/src-tauri/src/batch_runner.rs`
- Create `apps/desktop/src-tauri/src/batch_tests.rs`
- Modify `apps/desktop/src-tauri/src/app_state.rs`
- Modify `apps/desktop/src-tauri/src/dto.rs`
- Modify `apps/desktop/src-tauri/src/lib.rs`
- Modify `apps/desktop/src-tauri/src/data_management.rs`
- Modify `apps/desktop/src-tauri/src/data_management_tests.rs`

### Frontend

- Create `apps/desktop/src/domain/batch.ts`
- Create `apps/desktop/src/domain/batch.test.ts`
- Create `apps/desktop/src/pages/BatchSetupPage.tsx`
- Create `apps/desktop/src/pages/BatchSetupPage.test.tsx`
- Create `apps/desktop/src/pages/BatchRunPage.tsx`
- Create `apps/desktop/src/pages/BatchRunPage.test.tsx`
- Create `apps/desktop/src/pages/BatchResultPage.tsx`
- Create `apps/desktop/src/pages/BatchResultPage.test.tsx`
- Modify `apps/desktop/src/api/backend.ts`
- Modify `apps/desktop/src/api/runtimeValidation.ts`
- Modify `apps/desktop/src/api/runtimeValidation.test.ts`
- Modify `apps/desktop/src/api/tauriBackend.ts`
- Modify `apps/desktop/src/api/tauriBackend.test.ts`
- Modify `apps/desktop/src/app/routes.tsx`
- Modify `apps/desktop/src/components/AppShell.tsx`
- Modify history/accessibility tests where batch navigation and rows are added

---

### Task 1: Define Batch Contracts, Fingerprints, and Cost Estimates

**Files:**

- Create `crates/ability-core/src/batch.rs`
- Create `crates/ability-core/tests/batch_contracts.rs`
- Modify `crates/ability-core/src/lib.rs`
- Modify `crates/ability-core/src/domain.rs`
- Modify `crates/ability-core/tests/domain_contracts.rs`
- Modify `crates/ability-core/tests/recovery.rs`

**Step 1 — RED tests**

Add tests for:

- exact cost-policy-v1 rows:
  - guided quick `2..=4 × 1`, at most 4 members, 32 interactions/turns,
    4,320 task seconds, 4-hour execution window;
  - CLI quick `2..=4 × 1`, at most 4 members, 8 launches, 160 turns,
    14,400 task seconds, 8-hour window;
  - CLI standard `2..=4 × 3`, at most 12 members, 24 launches, 480 turns,
    43,200 task seconds, 24-hour window;
  - CLI full `2..=5 × 5`, at most 25 members, 50 launches, 1,000 turns,
    90,000 task seconds, 72-hour window;
- guided standard/full and any cohort above 25 members are rejected;
- initial estimate acknowledgement expires after 15 minutes; execution
  authorization expiry continues through pauses;
- checked overflow rejection;
- mixed guided/CLI cohort rejection;
- duplicate route-identity rejection;
- `default_route` route identity distinct from a concrete requested model;
- route-identity stability across map/order serialization while exact
  provenance remains separately preserved;
- accepted provenance-class policy tests;
- path-free `ExecutionAdapterIdentity` normalization and compatibility;
- no path, user name, package identity, raw label, or timestamp in identities;
- old environment JSON defaults adapter identity to absent without rewriting
  existing runs;
- new batch runs require adapter identity and resume rejects an incompatible
  adapter contract/version;
- exact task-launch/provider-turn/time/guided-interaction estimates from sealed
  task budgets;
- exact checked formulas from design §8.1, including sequential reviewed
  expected bands and `summed_task_budget + 300 seconds/member` execution
  ceiling;
- unknown token/quota amount is represented as unknown, never zero;
- any plan mutation changes the acknowledgement hash.

Run:

```powershell
cargo test -p ability-core --test batch_contracts
```

Expected: compile failure because batch contracts do not exist.

**Step 2 — GREEN implementation**

Implement and export:

```rust
pub enum BatchMode { QuickComparison, Standard, Full }
pub enum BatchStatus {
    Created, Running, Paused, Completed, Cancelled, Interrupted
}
pub enum BatchExecutionSurface { GuidedClient, AutomatedCli }
pub enum BatchFeatureLevel { GuidedQuickV1, CliStandardV1, ReliableFullV1 }
pub struct TargetRouteIdentity { /* model-or-route config, no provenance */ }
pub struct ExecutionAdapterIdentity { /* path-free reviewed execution facts */ }
pub struct ScanBatchTarget { /* target + route/provenance/adapter evidence */ }
pub struct BatchCostEstimate { /* launches, turns, time, expiry, unknown quota */ }
pub struct BatchCostPolicy { /* version 1 exact rows/formulas */ }
pub struct ScanBatchPlan { /* sealed pack, targets, mode, seed, estimate */ }
```

Use canonical length-prefixed hashing or canonical JSON over a dedicated hash
payload. Do not hash `HashMap` iteration order. Normalize model/reasoning with
the already-reviewed domain helpers; reject control characters and incoherent
provenance through the v0.2.2 validation policy. Keep adapter identity
core-owned so `ability-core` never depends on `ability-adapters`. Add an
optional, legacy-defaulted adapter identity to `EnvironmentFingerprint`; batch
runs require it while old single runs remain readable.

**Step 3 — verify and commit**

```powershell
cargo test -p ability-core --test batch_contracts
$tests = cargo test -p ability-core --test batch_contracts -- --list
foreach ($name in @('cost_policy_v1_exact_boundaries','rejects_mixed_surface_cohort','adapter_identity_is_path_free')) { if (-not ($tests -match [regex]::Escape($name))) { throw "missing test: $name" } }
cargo test -p ability-core --all-targets
cargo fmt --all -- --check
git diff --check
git add crates/ability-core/src/batch.rs crates/ability-core/src/domain.rs crates/ability-core/src/lib.rs crates/ability-core/tests/batch_contracts.rs crates/ability-core/tests/domain_contracts.rs crates/ability-core/tests/recovery.rs
git diff --cached --name-only
git commit -m "feat: define batch scan contracts"
```

The `--list` output must contain every named contract test. Confirm the staged
list contains only Task 1 files.

---

### Task 2: Persist Batches and Member Ownership Transactionally

**Files:**

- Create `crates/ability-core/migrations/0003_scan_batches.sql`
- Modify `crates/ability-core/src/storage.rs`
- Create `crates/ability-core/tests/batch_storage.rs`
- Modify `crates/ability-core/tests/storage.rs`

**Step 1 — RED tests**

Cover:

- migration of a real v2 fixture without rewriting old run JSON;
- atomic insertion of a batch, ordered targets, and planned members;
- duplicate ordinal/target/repetition rejection;
- route/adapter/provenance identities and suite hash must match the plan;
- reserving a member atomically preallocates and inserts its `RunRecord` plus
  ownership before any launch;
- one run id can belong to exactly one member and cannot be rebound;
- crash fixtures for: planned only, reserved run, launching run, running run,
  terminal run with stale member, and terminal member with inconsistent run;
- startup reconciliation never creates a replacement run or marks ambiguous
  provider work completed;
- invalid state transition leaves all rows unchanged;
- completed member cannot be rebound or rerun;
- earliest-runnable claiming deterministically skips deferred members while
  preserving their ordinals;
- target A deferred, target B completed, then A explicitly reauthorized;
- derived batch status stays `running` while any member is runnable/active,
  becomes `paused` only with deferred members and no runnable/active member,
  and becomes `completed` only when every member is terminal;
- startup with one deferred and one runnable member reconciles to `running`;
- cancelling marks only planned/reserved/deferred members cancelled;
- delete refuses active batches;
- foreign-key deletion cannot orphan an owned run;
- target/suite identity cleanup retains a created batch with zero launched runs
  while deleting unrelated run identities;
- `save_guided_task_result_with_isolation` validates exact
  batch/member/run/task ownership, active authorization, policy version, and
  positive attestation;
- injected failure after either isolation insert or task-result insert rolls
  back both rows; wrong ownership/authorization/attestation writes neither.

Run:

```powershell
cargo test -p ability-core --test batch_storage
```

Expected: failure before migration/repository methods exist.

**Step 2 — migration**

Create:

- `scan_batches`;
- `scan_batch_targets`;
- `scan_batch_members`;
- `scan_batch_task_isolation`;
- `scan_execution_authorizations`;
- `baseline_snapshots`;
- `scan_deletion_intents`;
- indexes for status/created time and batch/member order;
- foreign keys to `runs` and the persisted target JSON;
- `schema_migrations` version 3.

Persist the full immutable plan JSON plus indexed lifecycle columns. Validate
decoded JSON against the indexed values on every read.

**Step 3 — repository API**

Add narrow methods such as:

```rust
insert_batch_plan
get_batch
list_batches
reserve_next_runnable_member_and_run
mark_member_launching
mark_member_running
finish_batch_member
defer_batch_member
append_execution_authorization
save_guided_task_result_with_isolation
reconcile_batches_after_startup
derive_batch_status
pause_batch
resume_batch
cancel_batch
```

All claims and transitions use `TransactionBehavior::Immediate`. Reservation
accepts a preallocated run id and inserts both run and ownership in one
transaction. The scheduler claims the earliest runnable ordinal, may pass a
deferred member, and never passes an active member. `launching` is durable
before the provider boundary. Reconciliation treats `launching`/`running` as
potentially consumed and never auto-replays them.

Update `clean_orphan_identities` so target and suite rows referenced only by a
batch remain live. Validate suite content/scoring hashes, not only id/version.
`save_guided_task_result_with_isolation` performs both inserts in one immediate
transaction and exposes no public half-write method.

**Step 4 — verify and commit**

```powershell
cargo test -p ability-core --test batch_storage
$tests = cargo test -p ability-core --test batch_storage -- --list
foreach ($name in @('reserves_member_and_run_atomically','guided_result_and_attestation_are_atomic','reconciles_ambiguous_launch_without_replay','batch_status_is_derived_from_members')) { if (-not ($tests -match [regex]::Escape($name))) { throw "missing test: $name" } }
cargo test -p ability-core --all-targets
cargo fmt --all -- --check
git diff --check
git add crates/ability-core/migrations/0003_scan_batches.sql crates/ability-core/src/storage.rs crates/ability-core/tests/batch_storage.rs crates/ability-core/tests/storage.rs
git diff --cached --name-only
git commit -m "feat: persist batch scan ownership"
```

---

### Task 3: Build a Deterministic Balanced Member Schedule

**Files:**

- Create `crates/ability-core/src/batch_schedule.rs`
- Create `crates/ability-core/tests/batch_schedule.rs`
- Modify `crates/ability-core/src/lib.rs`
- Modify `crates/ability-core/src/batch.rs`

**Step 1 — RED tests**

Use golden schedules to prove:

- same seed and target set produce byte-identical schedules;
- every target appears exactly once per repetition;
- repetition starts rotate across targets;
- alternate rounds reverse order;
- removing/reordering a target changes the plan hash;
- no target is always first/last in standard or full mode;
- earliest-runnable selection skips deferred but not active members;
- reauthorized deferred ordinal re-enters without changing completed order;
- resume never returns an already terminal or ambiguous active member;
- task/session policy is bound to the sealed pack.

**Step 2 — GREEN implementation**

Implement a pure scheduler with no clock, filesystem, database, or random OS
source. Accept the stored seed explicitly. Return stable member ordinals and
target/repetition indexes. Do not create `RunRecord`s in the pure scheduler.

**Step 3 — verify and commit**

```powershell
cargo test -p ability-core --test batch_schedule
$tests = cargo test -p ability-core --test batch_schedule -- --list
foreach ($name in @('schedule_is_deterministic_and_balanced','earliest_runnable_skips_deferred_not_active','reauthorized_member_keeps_ordinal')) { if (-not ($tests -match [regex]::Escape($name))) { throw "missing test: $name" } }
cargo test -p ability-core --all-targets
cargo fmt --all -- --check
git diff --check
git add crates/ability-core/src/batch.rs crates/ability-core/src/batch_schedule.rs crates/ability-core/src/lib.rs crates/ability-core/tests/batch_schedule.rs
git diff --cached --name-only
git commit -m "feat: schedule balanced batch members"
```

---

### Task 4: Add Strict Tauri and TypeScript Batch Boundaries

**Files:**

- Create `apps/desktop/src-tauri/src/batch_commands.rs`
- Create `apps/desktop/src-tauri/src/batch_tests.rs`
- Modify `apps/desktop/src-tauri/src/dto.rs`
- Modify `apps/desktop/src-tauri/src/app_state.rs`
- Modify `apps/desktop/src-tauri/src/lib.rs`
- Create `apps/desktop/src/domain/batch.ts`
- Create `apps/desktop/src/domain/batch.test.ts`
- Modify `apps/desktop/src/api/backend.ts`
- Modify `apps/desktop/src/api/runtimeValidation.ts`
- Modify `apps/desktop/src/api/runtimeValidation.test.ts`
- Modify `apps/desktop/src/api/tauriBackend.ts`
- Modify `apps/desktop/src/api/tauriBackend.test.ts`

**Step 1 — RED boundary tests**

Test exact camelCase request/response shapes for:

- estimate batch;
- create acknowledged batch;
- get/list batch;
- authorize initial execution;
- estimate and authorize an explicit resumable attempt;
- start/resume/pause/cancel;
- get next guided member.

Reject:

- unknown enum strings and unknown fields;
- stale or mismatched acknowledgement hashes;
- Full estimate/create/start while `ReliableFullV1` is unavailable; assert no
  batch row, run row, member reservation, or provider event is produced;
- duplicate targets;
- mixed guided/CLI cohorts or mismatched pack/runner contracts;
- client target marked automated;
- CLI target marked guided only when policy forbids it;
- incoherent model provenance;
- path-bearing or unknown adapter identity;
- expired authorizations;
- a retry authorization for wrong-answer evidence or a failure class that does
  not match the durable marker;
- counts outside hard limits;
- non-finite analysis values;
- response arrays above hard bounds.

**Step 2 — GREEN DTO and command implementation**

Register only reviewed commands in the compile-time command inventory. The
estimate command is read-only. Creation recomputes the plan/estimate instead
of trusting frontend totals. Initial and subsequent execution authorizations
contain checked task-launch/turn/time/guided-interaction budgets, attempt
number, expiry, and acknowledgement hash. They never mutate the member
schedule. Mutating commands use the local-data gate.

During Tasks 4–6 the backend capability set contains `GuidedQuickV1` and
`CliStandardV1` only. Full is rejected at the command boundary even when a
caller bypasses the UI.

Frontend runtime validation must fail closed for nested targets, members,
estimates, and status values.

**Step 3 — verify and commit**

```powershell
cargo test -p ability-radar --lib
$tests = cargo test -p ability-radar --lib -- --list
foreach ($name in @('batch_tests::full_mode_is_gated_before_reliable_analysis','batch_tests::stale_acknowledgement_is_rejected','batch_tests::mixed_surface_cohort_is_rejected')) { if (-not ($tests -match [regex]::Escape($name))) { throw "missing test: $name" } }
npm test --workspace apps/desktop -- src/domain/batch.test.ts src/api/runtimeValidation.test.ts src/api/tauriBackend.test.ts
npm run build --workspace apps/desktop
cargo fmt --all -- --check
git diff --check
git add apps/desktop/src-tauri/src/app_state.rs apps/desktop/src-tauri/src/batch_commands.rs apps/desktop/src-tauri/src/batch_tests.rs apps/desktop/src-tauri/src/dto.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src/api/backend.ts apps/desktop/src/api/runtimeValidation.ts apps/desktop/src/api/runtimeValidation.test.ts apps/desktop/src/api/tauriBackend.ts apps/desktop/src/api/tauriBackend.test.ts apps/desktop/src/domain/batch.ts apps/desktop/src/domain/batch.test.ts
git diff --cached --name-only
git commit -m "feat: expose strict batch scan API"
```

---

### Task 5: Deliver Guided Multi-Target Client Scans

**Files:**

- Create `apps/desktop/src/pages/BatchSetupPage.tsx`
- Create `apps/desktop/src/pages/BatchSetupPage.test.tsx`
- Create `apps/desktop/src/pages/BatchRunPage.tsx`
- Create `apps/desktop/src/pages/BatchRunPage.test.tsx`
- Modify `apps/desktop/src/app/routes.tsx`
- Modify `apps/desktop/src/components/AppShell.tsx`
- Modify `apps/desktop/src-tauri/src/batch_commands.rs`
- Modify `apps/desktop/src-tauri/src/batch_tests.rs`
- Modify `crates/ability-core/src/orchestration.rs`
- Modify `crates/ability-core/tests/manual_run.rs`

**Step 1 — RED interaction tests**

Prove:

- the estimate updates when target/repetition/pack changes;
- a stale acknowledgement disables start;
- guided setup accepts quick-comparison mode only in Phase B;
- mixed surface selection is rejected with a non-comparable explanation;
- two guided targets advance in the persisted schedule;
- each task requires a fresh “new blank conversation” user attestation before
  answer submission;
- attestation policy/version/time are persisted with the task evidence;
- missing/declined attestation makes the task invalid for regression evidence;
- forced checkpoint failure leaves neither task result nor isolation row and
  removes the just-written answer artifact;
- forced checkpoint plus artifact-cleanup failure interrupts the same owned run
  through the existing recovery path while still leaving no half database
  evidence;
- UI labels guided isolation as user-attested, never machine-verified;
- selected target/model/reasoning/provenance remain visible;
- user typing is never overwritten by a late detection response;
- interrupted/ambiguous work is never automatically reopened or repeated;
- refresh/reopen resumes the exact next member;
- completion navigates to the batch result;
- all controls pass axe/accessibility tests.

**Step 2 — GREEN workflow**

Refactor `ManualRunService` with an owned-run entry point that validates and
executes the `RunRecord` atomically reserved by Task 2; it must not insert a
second run. Use the existing deterministic grading. Store isolation
attestation by calling Task 2's
`save_guided_task_result_with_isolation` checkpoint, so result and attestation
share one repository transaction. Preserve the existing answer-artifact
cleanup/interrupt behavior around a failed checkpoint. Do not
copy client text automatically and do not add clipboard authority in this
task. Phase B has no in-place retry; a failure is terminal or requires a later
explicit execution authorization supported by a future phase.

**Step 3 — verify and commit**

```powershell
npm test --workspace apps/desktop -- src/pages/BatchSetupPage.test.tsx src/pages/BatchRunPage.test.tsx src/test/accessibility.test.tsx
cargo test -p ability-radar --lib
cargo test -p ability-core --test manual_run
npm run build --workspace apps/desktop
git diff --check
git add apps/desktop/src/pages/BatchSetupPage.tsx apps/desktop/src/pages/BatchSetupPage.test.tsx apps/desktop/src/pages/BatchRunPage.tsx apps/desktop/src/pages/BatchRunPage.test.tsx apps/desktop/src/app/routes.tsx apps/desktop/src/components/AppShell.tsx apps/desktop/src-tauri/src/batch_commands.rs apps/desktop/src-tauri/src/batch_tests.rs crates/ability-core/src/orchestration.rs crates/ability-core/tests/manual_run.rs
git diff --cached --name-only
git commit -m "feat: guide multi-target client scans"
```

This completes Phase B/v0.3. It must display `insufficient_data`, not a
degradation conclusion.

---

### Task 6: Queue Automated CLI Members Sequentially

**Files:**

- Create `apps/desktop/src-tauri/src/batch_runner.rs`
- Modify `apps/desktop/src-tauri/src/batch_commands.rs`
- Modify `apps/desktop/src-tauri/src/app_state.rs`
- Modify `apps/desktop/src-tauri/src/batch_tests.rs`
- Modify `apps/desktop/src/pages/BatchRunPage.tsx`
- Modify `apps/desktop/src/pages/BatchRunPage.test.tsx`
- Modify `crates/ability-adapters/src/cli_run.rs`
- Modify `crates/ability-adapters/tests/cli_run.rs`

**Step 1 — RED fake-adapter tests**

Use fake process runners/adapters only. Prove:

- at most one CLI member executes at a time;
- Standard is the highest executable Phase C mode; a forged Full command is
  rejected before reservation/launch;
- runner executes the preallocated, already-owned `RunRecord` and never inserts
  or attaches a second run;
- `reserved` is committed before launch and `launching` before crossing the
  provider boundary;
- crash/reopen at reserved, launching, running, and terminal mismatch states
  reconciles to the same run id without automatic replay;
- next member starts only after terminal evidence and member commit;
- retained trusted launch identity is used for every member;
- path-free execution-adapter identity is stored in the run environment and
  checked on resume;
- targets receive the same sealed pack/hash/scoring version;
- each CLI task gets a fresh provider session and service-owned workspace;
- estimate totals exactly match adapter task launches, sum of task max-turn
  budgets, and sum of time-budget upper bounds;
- quota/auth/network/runtime/infrastructure failures pause without score loss;
- no automatic retry occurs;
- explicit resume requires a fresh bounded authorization for the exact durable
  failure marker and same run id;
- target A can defer, target B completes, then A resumes without schedule
  mutation;
- Tauri events/status remain `running` while B is runnable/active, change to
  `paused` only after no runnable/active member remains, and become terminal
  only after all members are terminal;
- cancelling stops the active token and prevents all future launches;
- resume does not repeat a completed member;
- one unavailable target does not corrupt or relabel another target.

**Step 2 — GREEN runner**

Refactor `CliRunService::prepare` so a batch entry point validates and executes
an already-reserved run rather than inserting it. Single-run behavior remains
backward compatible. The runner loops through transactional earliest-runnable
claims and delegates the owned run to `CliRunService`. Keep adapter detection
and retained-launch semantics unchanged. A restart never auto-runs a
`launching` or `running` member. Emit bounded batch events containing ids,
ordinals, counts, and safe status only.

**Step 3 — verify and commit**

```powershell
cargo test -p ability-radar --lib
$tests = cargo test -p ability-radar --lib -- --list
foreach ($name in @('batch_runner::executes_at_most_one_cli_member','batch_runner::ambiguous_launch_is_not_replayed','batch_runner::deferred_target_does_not_block_runnable_target')) { if (-not ($tests -match [regex]::Escape($name))) { throw "missing test: $name" } }
cargo test -p ability-adapters --all-targets
npm test --workspace apps/desktop -- src/pages/BatchRunPage.test.tsx
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
git add apps/desktop/src-tauri/src/batch_runner.rs apps/desktop/src-tauri/src/batch_commands.rs apps/desktop/src-tauri/src/app_state.rs apps/desktop/src-tauri/src/batch_tests.rs apps/desktop/src/pages/BatchRunPage.tsx apps/desktop/src/pages/BatchRunPage.test.tsx crates/ability-adapters/src/cli_run.rs crates/ability-adapters/tests/cli_run.rs
git diff --cached --name-only
git commit -m "feat: run sequential CLI scan batches"
```

This completes Phase C/v0.4 execution. Higher concurrency remains out of
scope.

---

### Task 7: Add Matched Analysis, Baselines, and Versioned Signals

**Files:**

- Create `crates/ability-core/src/batch_analysis.rs`
- Create `crates/ability-core/tests/batch_analysis.rs`
- Modify `crates/ability-core/src/batch.rs`
- Modify `crates/ability-core/src/lib.rs`
- Modify `crates/ability-core/src/storage.rs`
- Modify `apps/desktop/src-tauri/src/batch_commands.rs`
- Modify frontend batch types and runtime validation

**Step 1 — RED goldens**

Add table/golden tests for:

- category medians and median absolute deviation;
- `baseline_as_of` is obtained before candidate insertion in the same
  transaction, and eligible evidence requires `finished_at < baseline_as_of`;
- candidate/later/duplicate evidence ids are always excluded;
- 90-day window, latest completed compatible batch before the cutoff per UTC
  day, then the 12 most recent selected days, are selected deterministically;
- production evidence minimum is five compatible historical full batches over
  at least three UTC days plus five valid candidate member runs;
- deterministic cluster bootstrap resamples candidate member runs and
  historical batches, never nested task rows or a repetition cross-product;
- adding more tasks to one run cannot increase the independent sample count;
- a member with any missing/infrastructure-invalid task or missing guided
  isolation attestation is excluded wholesale from regression evidence;
- matched-task deltas only across identical task/content/scoring identity;
- guided/CLI cohorts are never comparable;
- infrastructure-invalid results excluded from numerator and denominator;
- quick scans always `insufficient_data`;
- `legacy_unknown` excluded from verified model baselines;
- default route compares only to default-route history;
- route identity, accepted provenance class, adapter contract, surface,
  suite/hash/scoring/analysis mismatch reject baseline membership;
- exact evidence ids, exclusions, cutoff, policy versions, and digest are
  frozen in `BaselineSnapshot`;
- Full creation atomically inserts candidate `created_at`/`baseline_as_of`,
  immutable snapshot/digest, batch plan, targets, and members; injected failure
  leaves none of them;
- no Full batch can be read or started without exactly one valid snapshot;
- Quick/Standard batches carry no regression snapshot and cannot emit a
  regression signal;
- absolute threshold alone cannot produce likely regression;
- relative threshold alone cannot produce likely regression;
- `delta = candidate - baseline`, and the confidence upper bound must be below
  the negative tolerated drop;
- `absolute_drop = baseline - candidate` and
  `relative_drop = absolute_drop / baseline`; non-positive baseline cannot
  satisfy the relative gate;
- insufficient independent clusters produce `watch` or `insufficient_data`;
- full sufficient evidence can produce `stable`, `watch`, or
  `likely_regression`;
- variant content hashes remain incompatible until a future task-family
  equivalence contract;
- NaN, infinity, malformed counts, and inconsistent stored summaries fail
  closed.

**Step 2 — GREEN pure analysis**

Implement:

```rust
pub enum RegressionSignal {
    InsufficientData, Stable, Watch, LikelyRegression
}
pub struct BatchAnalysis { /* distributions, matched deltas, uncertainty */ }
pub struct CalibrationPolicy { /* versioned thresholds and minima */ }
pub struct BaselineSnapshot {
    /* baseline_as_of, ordered ids/exclusions, window/policy, digest */
}
```

Keep the algorithm pure and deterministic. Task 7 enables
`ReliableFullV1` only after Full creation can select evidence strictly before
the candidate cutoff and insert candidate batch plus snapshot in the same
`BEGIN IMMEDIATE` transaction, before any member reservation. Each historical
batch contributes one
median summary; each candidate member contributes one summary. Task-level
deltas are diagnostic only. Persist the analysis version, calibration-policy
version, bootstrap seed/resample count, cutoff, evidence ids/exclusions, and
snapshot digest. Do not introduce an LLM judge.

Expose this as one narrow repository operation such as
`create_full_batch_with_baseline_snapshot`; there is no public
“insert Full now, attach snapshot later” path.

**Step 3 — calibration gate**

Before enabling `likely_regression` wording in production:

- run an explicitly authorized real-user calibration protocol;
- record false-positive/false-negative review;
- freeze the first policy version;
- obtain product/security and independent statistical-method review.

Without that evidence, map the strongest UI wording to “值得复测”.

**Step 4 — verify and commit**

```powershell
cargo test -p ability-core --test batch_analysis
$tests = cargo test -p ability-core --test batch_analysis -- --list
foreach ($name in @('full_creation_freezes_baseline_atomically','candidate_and_later_evidence_are_excluded','bootstrap_resamples_runs_and_batches_not_tasks','partial_member_is_not_regression_evidence')) { if (-not ($tests -match [regex]::Escape($name))) { throw "missing test: $name" } }
cargo test -p ability-core --all-targets
npm test --workspace apps/desktop -- src/domain/batch.test.ts src/api/runtimeValidation.test.ts
cargo fmt --all -- --check
git diff --check
git add crates/ability-core/src/batch.rs crates/ability-core/src/batch_analysis.rs crates/ability-core/tests/batch_analysis.rs crates/ability-core/src/lib.rs crates/ability-core/src/storage.rs apps/desktop/src-tauri/src/batch_commands.rs apps/desktop/src/api/backend.ts apps/desktop/src/api/runtimeValidation.ts apps/desktop/src/api/runtimeValidation.test.ts apps/desktop/src/domain/batch.ts apps/desktop/src/domain/batch.test.ts
git diff --cached --name-only
git commit -m "feat: analyze matched batch evidence"
```

---

### Task 8: Add Matrix Results, History, Export, and Data Lifecycle

**Files:**

- Create `apps/desktop/src/pages/BatchResultPage.tsx`
- Create `apps/desktop/src/pages/BatchResultPage.test.tsx`
- Modify `apps/desktop/src/pages/HistoryPage.tsx`
- Modify `apps/desktop/src/pages/HistoryPage.ui.test.tsx`
- Modify `apps/desktop/src/app/routes.tsx`
- Modify `apps/desktop/src-tauri/src/data_management.rs`
- Modify `apps/desktop/src-tauri/src/data_management_tests.rs`
- Modify `crates/ability-core/src/report.rs`
- Create `schemas/public-batch-report.schema.json`
- Create `crates/ability-core/tests/batch_report_schema.rs`

**Step 1 — RED tests**

Prove:

- every matrix cell exposes sample count and evidence drill-down;
- client and CLI cohorts render as separate, visibly non-comparable matrices;
- labels distinguish requested model, visible model, and provider default
  route;
- queued/running/completed/invalid/unavailable/insufficient states differ;
- provenance and baseline compatibility are visible;
- raw answers are absent from default export;
- aggregate export includes hashes, versions, counts, exclusions, uncertainty;
- a separate public batch-report schema rejects unknown/unsafe fields and
  round-trips the exact aggregate export;
- deleting an active batch is refused;
- unlink-only preserves runs;
- database ownership deletion is transactional;
- artifact deletion uses durable intent, quarantine/staged rename, database
  commit, final cleanup, and startup reconciliation;
- failures at each deletion phase are recoverable and idempotent;
- backup export includes batch rows/hashes and validates foreign keys;
- no restore/import capability or acceptance claim is added in this plan;
- retention cannot orphan batch evidence;
- keyboard and screen-reader navigation work for the matrix.

**Step 2 — GREEN lifecycle and pages**

Render a matrix as a view over validated evidence. Do not make a visual cell
the source of truth. Use the precision-radar design tokens from the completed
v0.2.2 UI plan. Extend the current data-management deletion intent/recovery
pattern rather than claiming SQLite can roll back filesystem operations.

**Step 3 — verify and commit**

```powershell
npm test --workspace apps/desktop -- src/pages/BatchResultPage.test.tsx src/pages/HistoryPage.ui.test.tsx src/test/accessibility.test.tsx
cargo test -p ability-core --all-targets
cargo test -p ability-core --test batch_report_schema
cargo test -p ability-radar --lib
$tests = cargo test -p ability-radar --lib -- --list
foreach ($name in @('data_management_tests::batch_backup_contains_valid_links','data_management_tests::batch_delete_reconciles_quarantine','batch_tests::active_batch_delete_is_rejected')) { if (-not ($tests -match [regex]::Escape($name))) { throw "missing test: $name" } }
npm run build --workspace apps/desktop
git diff --check
git add apps/desktop/src/pages/BatchResultPage.tsx apps/desktop/src/pages/BatchResultPage.test.tsx apps/desktop/src/pages/HistoryPage.tsx apps/desktop/src/pages/HistoryPage.ui.test.tsx apps/desktop/src/app/routes.tsx apps/desktop/src-tauri/src/data_management.rs apps/desktop/src-tauri/src/data_management_tests.rs crates/ability-core/src/report.rs crates/ability-core/tests/batch_report_schema.rs schemas/public-batch-report.schema.json
git diff --cached --name-only
git commit -m "feat: present and retain batch evidence"
```

---

### Task 9: Seal Phase Gates and Perform Local Acceptance

**Files:**

- Modify `README.md`
- Modify `docs/troubleshooting.md`
- Create `docs/batch-scan-methodology.md`
- Update repository contract tests as required

**Step 1 — automated full gate**

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --locked
npm test
npm run build --workspace apps/desktop
npm run validate:repository
git diff --check
```

All must pass from a clean tracked worktree.

**Step 2 — fake end-to-end acceptance**

Run a local fake-provider batch with:

- two CLI targets;
- three repetitions;
- deterministic sealed pack;
- one injected quota pause;
- target B completion while target A is deferred;
- explicit reauthorization and resume of the same target A run id;
- restart reconciliation at reserved/launching/running crash points;
- cancellation of a second batch;
- history/export and two-phase delete recovery.

Assert zero network/provider calls.

Also assert cost-policy version 1 at every exact boundary: 32 guided
interactions, CLI Quick 8/160/14,400, Standard 24/480/43,200, Full
50/1,000/90,000, 15-minute initial acknowledgement expiry, and the
4/8/24/72-hour execution windows. One-unit-over fixtures must fail before any
row or fake-provider event.

**Step 3 — explicitly authorized real-user acceptance**

Only after separate user confirmation:

- show task-launch count, maximum provider turns, guided interactions, expected
  and maximum time, unknown token/quota amount, and authorization expiry;
- run the smallest two-target homogeneous quick comparison;
- do not enable retries;
- verify route wording, model provenance, and adapter/CLI launch source;
- stop on auth/quota/network errors;
- inspect local results and clean owned processes.

This acceptance consumes the user's own subscription quota. It is never part
of CI and never charged to the repository maintainer.

Do not combine client and CLI targets in this acceptance. If both surfaces are
accepted, run two separate cohorts and label their scores non-comparable.

**Step 4 — documentation commit**

```powershell
git add README.md docs/troubleshooting.md docs/batch-scan-methodology.md scripts/repository-contracts.test.mjs scripts/validate-repository.mjs
git diff --cached --name-only
git diff --cached --check
git commit -m "docs: explain multi-target scan evidence"
```

Omit either script path when that file did not change. Confirm no unrelated
script is staged.

**Step 5 — whole-branch review**

Generate a fixed-base review package spanning all batch commits. Request:

- specification review;
- security/privacy review;
- statistical-method review;
- data-migration/lifecycle review;
- accessibility review.

Fix and re-review every Critical or Important finding before any packaging or
publication decision.
