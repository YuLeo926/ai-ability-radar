# Multi-Target Batch Scan Design

**Date:** 2026-07-23  
**Status:** Approved direction; implementation follows the v0.2.2 identity
and provenance foundation  
**Related design:** `2026-07-17-ai-ability-radar-design.md`

## 1. Purpose

Add a detection method that can compare several model and reasoning-effort
combinations in one controlled, homogeneous execution-surface cohort. The
product may show client and CLI cohorts in one campaign overview, but it must
not compare their scores until a future cross-surface pack is implemented.
The product must answer two different questions without conflating them:

1. How did the configured targets perform on the same versioned question pack
   in the same scan window?
2. Did one target perform materially worse than its own compatible historical
   baseline?

The second question is the product's actual “降智” signal. A cross-model
leaderboard alone is not evidence of degradation.

The screenshot supplied by the user is treated as a method reference for a
multi-target, multi-session scan. Its undisclosed internal implementation is
not assumed or copied.

## 2. Non-goals

- Do not identify a hidden foundation model from answer style or self-report.
- Do not claim a single failed question proves degradation.
- Do not compare scores produced by different pack content or scoring rules as
  if they were directly compatible.
- Do not silently spend subscription quota.
- Do not make the project maintainer pay for a user's local scan.
- Do not automate desktop clients under the read-only model-identification
  permission.
- Do not use an LLM judge in the first batch implementation.
- Do not run real subscription scans in ordinary GitHub CI.

## 3. Terminology

- **Target:** One exact test configuration: provider surface, model label,
  reasoning effort, and provenance.
- **Route identity:** A canonical provider/surface/model-or-route/reasoning
  identity. It is independent of how the label was obtained.
- **Provenance evidence:** The exact source and verification quality attached
  to a route identity.
- **Comparison cohort:** Targets that use the same execution surface, sealed
  pack content, grader contract, and session policy.
- **Campaign:** One or more cohorts shown together. Cohorts with incompatible
  packs are explicitly non-comparable.
- **Member run:** One complete run of one target against one sealed pack.
- **Repetition:** A new isolated member run of the same target.
- **Batch:** A group of member runs created from one target set, pack, mode,
  seed, and cost acknowledgement.
- **Matched task:** The same task id and content hash evaluated for different
  targets or repetitions.
- **Compatible baseline:** Historical evidence with the same target
  route identity, accepted provenance class, adapter identity, pack
  id/version/content hash, and scoring-rule version.
- **Baseline snapshot:** A frozen, content-addressed list of historical
  evidence selected strictly before the candidate batch.

## 4. Detection modes

| Mode | Targets | Repetitions per target | Purpose | Regression conclusion |
|---|---:|---:|---|---|
| Single quick check | 1 | 1 | Existing v0.2 flow | No |
| Quick comparison | 2–4 | 1 | Fast same-window contrast | No |
| Standard scan | 2–4 | 3 | Repeatable CLI-cohort comparison | Provisional |
| Full scan | 2–5 | 5 | Stronger CLI-cohort baseline comparison | Yes, when evidence is sufficient |

Limits are hard safety limits, not UI suggestions. The initial implementation
also caps a cohort at 25 member runs. Surface-specific caps additionally limit
task launches, maximum provider turns, guided interactions, summed task-time
budgets, and scan-window duration. Checked arithmetic must reject overflow
before any run is reserved.

A cohort is homogeneous:

- guided ChatGPT/Claude client targets use the same sealed client pack and
  deterministic in-process graders;
- automated Codex CLI/Claude Code targets use the same sealed CLI pack,
  workspace tasks, and external verifier contract;
- guided and CLI targets can appear in one campaign only as separate,
  explicitly non-comparable cohorts.

Cross-surface score comparison is deferred until a versioned benchmark exists
whose identical task payloads and graders are implemented by both runners.

Full mode is a backend capability, not a UI-only flag. Before the reliable
analysis phase is installed, estimate/create/start commands reject Full and
create no batch, run, or provider action. Enabling Full requires the
candidate's `created_at`, `baseline_as_of`, immutable baseline snapshot, and
snapshot digest to be inserted in the same transaction before any member can
be reserved. Quick and Standard batches carry no regression snapshot and
cannot emit a regression signal.

## 5. Target, adapter, and provenance identity

`TargetRouteIdentity` includes:

- `TargetKind`;
- normalized requested/visible model label or the explicit default-route
  sentinel;
- normalized reasoning effort;
- execution surface (`guided_client` or `automated_cli`).

`ExecutionAdapterIdentity` is a core-owned, path-free evidence object:

- execution surface and provider family;
- reviewed launch kind (`guided_client`, `native_exe`, or `reviewed_npm`);
- public client/CLI version when available;
- adapter contract version.

Exact `ModelSource` and `ModelVerification` values remain attached as
provenance evidence, but do not unnecessarily split the route identity.
Baseline policy decides which verified provenance classes are equivalent. For
example, a user-confirmed manual label and a user-confirmed accessibility
prefill may be accepted together while still displaying their exact sources.

All identity payloads exclude user names, installation paths, package paths,
raw window labels, credentials, and mutable timestamps. Adapter identity is
threaded through the run environment, resume validation, export, and baseline
compatibility. A change to adapter contract version invalidates compatibility;
a reviewed launch-kind or public CLI-version change remains visible and is
included or excluded according to the versioned baseline policy.

Rules:

- `default_route` is a valid route target but not a concrete model identity.
  It can be compared with its own default-route history only.
- `legacy_unknown` records remain visible but cannot seed a verified
  model-specific baseline.
- A cohort rejects duplicate route identities.
- A target shown as a concrete model must satisfy the strict provenance
  combination matrix delivered by the client-identification plan.
- A visible client selector may prefill a target, but starting the scan is the
  user confirmation boundary.
- UI and exports say “requested model” for `cli_requested`, “visible model” for
  a user-confirmed client selector, and “provider default route” for
  `default_route`. A default-route signal is route-level, never
  model-specific.

## 6. Session isolation

Every member run declares an isolation policy and records how it was enforced.

- Stateless client questions require the user to attest to a new blank
  conversation per task immediately before the answer is accepted.
- A task explicitly declared multi-turn owns one new conversation for that
  scenario only.
- CLI tasks use a fresh service-owned workspace and provider session; this is
  machine-enforced and recorded separately from user attestation.
- No answer or conversation id is reused between targets or repetitions.
- Prompt text never contains another target's result, score, or evaluation.

The guided app cannot verify the user's client conversation because the
read-only permission may not inspect transcripts or conversation identity.
It therefore persists `user_attested` plus the isolation-policy version and
attestation time for each task. Missing or declined attestation makes that
task invalid for regression evidence. The product never labels guided
isolation as machine verified.

The scheduler creates a deterministic, balanced target order from the sealed
cohort seed. It rotates the first target between repetitions and reverses
alternate rounds so one provider is not always tested first or last. The seed
and resulting order are persisted for audit and resume.

## 7. Execution surfaces

### 7.1 Guided clients

The first client batch is guided:

1. The app shows the exact target and task.
2. The user opens the indicated client/model in a new blank conversation.
3. The app copies or displays the prompt.
4. The user pastes the complete answer back.
5. Deterministic local grading runs immediately.

Read-only Windows model identification may prefill the model and reasoning
effort. It may not click, switch, type, send, or read the conversation.

An action-capable Windows client adapter is a later v0.8 experiment. It
requires a separate permission, visible start/stop controls, a provider
allowlist, no credential access, and an always-available guided fallback.

### 7.2 Automated CLIs

Codex CLI and Claude Code may be queued automatically through their reviewed
local adapters.

- Default concurrency is one member run.
- The initial release does not expose higher concurrency.
- A trusted launch identity established during detection is retained for the
  member run.
- Each task launch has independent cancellation and bounded turn/time budgets.
- A member run may contain several task-level provider launches; estimates and
  events distinguish member runs from task launches and provider turns.
- Authentication, quota, network, runtime, and infrastructure failures are
  invalid evidence and never score as wrong answers.
- A target failure may pause that target while allowing the user to continue
  other targets; it must not silently retry and spend more quota.

## 8. Cost and consent

Before creation, the estimator displays checked totals for:

- target count;
- repetitions;
- tasks per member run;
- planned member runs;
- task/adapter launches;
- guided interactions;
- maximum provider turns from sealed task budgets;
- summed task-time upper bound;
- expected and maximum elapsed-time bands;
- scan-window expiry;
- subscription-usage level;
- automatic versus guided work;
- automatic retry budget, initially zero;
- token/quota amount as unknown unless the provider reports it through a
  reviewed bounded interface.

The estimate is derived from the sealed plan and is stored with the batch.
Changing targets, repetitions, pack version, or retry policy invalidates the
acknowledgement and requires a new estimate.

Task-launch count is never described as provider-request or token count. A
full guided cohort is rejected when its interaction or scan-window cap would
make a same-window comparison misleading. Initial guided support is quick
comparison; standard guided support requires a separately calibrated smaller
pack or an explicitly accepted interaction budget. Full mode initially applies
to automated CLI cohorts only.

Quick comparison requires one explicit start action. Standard and full scans
require a separate acknowledgement that they may consume significant
subscription quota. An acknowledgement expires before the maximum scan window
and cannot authorize later launches. No recurring schedule is enabled by
default.

### 8.1 Cost policy v1

Cost policy v1 applies only to the sealed `client-quick@1.0.0` and
`cli-quick@1.0.0` content hashes shipped with the release. A pack/content
change requires a new policy version.

| Surface/mode | Targets × repetitions | Member cap | Task-launch / guided-interaction cap | Max provider turns | Summed task-budget cap | Authorization wall-clock window |
|---|---:|---:|---:|---:|---:|---:|
| Guided client / quick | `2–4 × 1` | 4 | 32 guided interactions | 32 user prompt turns | 4,320 s | 4 h |
| Automated CLI / quick | `2–4 × 1` | 4 | 8 task launches | 160 | 14,400 s | 8 h |
| Automated CLI / standard | `2–4 × 3` | 12 | 24 task launches | 480 | 43,200 s | 24 h |
| Automated CLI / full | `2–5 × 5` | 25 | 50 task launches | 1,000 | 90,000 s | 72 h |

Guided standard/full modes are unsupported by policy v1. The formulas are:

```text
member_runs = targets × repetitions
task_launches = member_runs × sealed_task_count
provider_turn_ceiling = member_runs × Σ(task.max_turns)
summed_task_budget_secs = member_runs × Σ(task.time_budget_secs)
guided_interactions = task_launches for guided clients, otherwise 0
```

All multiplication and addition are checked. A plan must satisfy both its row
limits and the computed values. The initial estimate acknowledgement expires
15 minutes after issuance. Once execution starts, its authorization expires at
`started_at + wall_clock_window`; pauses do not stop that clock. After expiry,
no new task/member launch is allowed until the remaining plan receives a new
estimate and explicit authorization.

Expected elapsed time is a labelled estimate, calculated sequentially as the
pack's reviewed per-member band multiplied by member runs:

- guided client: `10–15 minutes × member_runs`;
- automated CLI: `30–60 minutes × member_runs`.

The provider-execution ceiling is
`summed_task_budget_secs + (member_runs × 300 seconds)` for reviewed local
orchestration overhead. The authorization wall-clock window is displayed
separately and is the hard latest launch time. Token and subscription quota
units remain “unknown”; none of these counts is relabelled as tokens.

## 9. Storage model

Add local coordination concepts while retaining existing `RunRecord` as the
source of task evidence:

```text
ScanBatch
  id
  mode
  suite identity and hashes
  scoring-rule version
  deterministic seed
  status
  cost estimate + initial acknowledgement hash/expiry
  planned/completed member counts
  timestamps

ScanBatchTarget
  batch id
  stable position
  complete TargetSelection
  route identity
  provenance evidence
  execution adapter identity
  execution surface

ScanBatchMember
  batch id
  stable ordinal
  target position
  repetition index
  preallocated RunRecord id
  durable launch/attempt state
  member status
  non-scoring failure classification

ScanBatchTaskIsolation
  member/run/task identity
  policy version
  user_attested or machine_enforced
  recorded time

ScanExecutionAuthorization
  batch/member scope
  attempt number
  additional turn/time budget
  acknowledgement hash and expiry

BaselineSnapshot
  candidate batch
  baseline_as_of
  ordered evidence ids and exclusions
  cohort/window/policy version
  content digest

ScanDeletionIntent
  batch/run/artifact ownership
  quarantine path token (relative, validated)
  lifecycle phase
  timestamps
```

Existing run and task-result rows stay canonical. Batch rows only link and
coordinate them. Before any provider launch, the repository preallocates a run
id and inserts the `RunRecord`, member ownership, target binding, and initial
member state in one `BEGIN IMMEDIATE` transaction. Manual and CLI services are
refactored to execute this already-owned run rather than inserting a new run.
Foreign keys and transactional state changes prevent a batch from referring
to a run owned by another batch.

The launch states are durable:

```text
planned -> reserved -> launching -> running
                               -> interrupted
                      running  -> completed | invalid | interrupted
planned/reserved/deferred      -> cancelled
```

`launching` is written before crossing the provider boundary. A restart treats
both `launching` and `running` as potentially consumed work, attaches the same
run id, and never creates or launches a replacement automatically.

Old databases migrate without rewriting existing run JSON. Target and suite
identity garbage collection includes batch-only references, including a batch
that has no launched runs.

Database-row ownership changes are transactional. Filesystem artifact deletion
cannot be SQLite-atomic and uses a recoverable two-phase lifecycle: durable
deletion intent, quarantine/staged rename, transactional row change, final
cleanup, and startup reconciliation. Batch-aware backup export is included;
backup import/restore remains deferred until the application has a general
restore architecture.

Guided task evidence uses one narrow repository transaction:
`save_guided_task_result_with_isolation`. It validates batch/member/run/task
ownership, active execution authorization, isolation-policy version, and
positive attestation, then inserts the isolation row and canonical
`TaskResult` together. A constraint or injected failure on either write rolls
back both. The manual service writes the answer artifact first and invokes this
transaction as its checkpoint; on failure it follows the existing recoverable
artifact cleanup/interrupt path. A score can never survive without its
attestation, and an attestation can never survive without its score.

## 10. State machine and recovery

Batch statuses:

```text
created -> running -> completed
                  -> paused -> running
                  -> interrupted -> running
                  -> cancelled
```

Invalid transitions fail closed.

- Member lifecycle categories are explicit:
  - runnable: `planned` with a valid unexpired authorization;
  - active: `reserved`, `launching`, or `running`;
  - deferred: interrupted/auth/quota/network states awaiting a user decision;
  - evidence terminal: `completed`;
  - non-evidence terminal: `invalid`, `unavailable`, or `cancelled`.
- The scheduler claims the earliest runnable ordinal. It may pass a deferred
  member while preserving the stored order; the deferred ordinal re-enters
  only after a new user authorization.
- A batch is complete only when every member is evidence-terminal or
  non-evidence-terminal.
- Aggregate batch status is derived with this precedence:
  - `running` while any member is active or runnable, even when another member
    is deferred;
  - `paused` only when no member is active/runnable and at least one is
    deferred;
  - `completed` only when all members are terminal and the batch was not
    cancelled as a whole;
  - `cancelled` only after an explicit batch cancellation and all active work
    is terminalized;
  - `created` before any valid execution authorization/reservation;
  - `interrupted` is a persisted startup-reconciliation state, not a state from
    which the runner may launch automatically.
- Closing the app marks a running batch interrupted after active run
  terminalization.
- Startup reconciliation handles `reserved`, `launching`, `running`, terminal
  run/member disagreement, and a run attached before launch. Ambiguous
  `launching`/`running` work is never replayed automatically.
- Resume validates the batch plan hash, route/adapter identities, pack hashes,
  scoring-rule version, completed member evidence, and next ordinal.
- Completed member runs are never repeated automatically.
- A quota/auth/network stop resumes only after a new user action.
- Cancelling a batch cancels the active run and marks all not-started members
  cancelled without fabricating run rows.

The immutable schedule has zero automatic retries. An explicit resume/retry is
an append-only `ScanExecutionAuthorization` for the same preallocated run,
with a new estimate, acknowledgement, attempt number, exact allowed failure
class, additional turn/time budget, and expiry. It never creates a new member
or silently changes the original order. Initially only interrupted,
authentication, quota, network, and infrastructure markers are retryable;
wrong-answer evidence is not.

## 11. Scoring and degradation analysis

### 11.1 Deterministic evidence first

The existing local graders and executable verifiers remain canonical.
Infrastructure failures are excluded. Raw model prose is not sent to another
model for judging.

Descriptive cohort aggregates use:

- valid member-run score distribution;
- category medians;
- matched-task pass/score deltas;
- completion and invalid-evidence counts;
- median absolute deviation;

Matched-task rows are descriptive and are never treated as independent samples
for a regression confidence interval. The aggregation algorithm is versioned.

### 11.2 Baseline compatibility

A historical member may enter a baseline only when all of these match:

- route identity and accepted provenance class;
- execution-adapter compatibility policy;
- homogeneous execution surface;
- suite id;
- suite version;
- suite content SHA-256;
- scoring-rule version;
- batch-analysis version.

Resumed runs remain eligible when their stored environment and execution
authorization pass recovery validation. Results from `legacy_unknown`
provenance are shown but excluded from verified model-specific baselines.

### 11.3 Frozen regression protocol

At candidate full-scan creation, obtain and freeze `baseline_as_of` before the
candidate row is inserted in the same transaction. Eligible evidence must have
`finished_at < baseline_as_of`; the candidate id is also explicitly excluded.
Freeze:

- `baseline_as_of` and candidate creation time/order;
- a 90-day lookback window;
- at most the most recent 12 compatible full batches;
- at most one eligible baseline batch per UTC calendar day, selecting the
  latest completed compatible batch before the cutoff for that day and then
  the 12 most recent selected days;
- ordered included batch/member ids;
- ordered exclusions and reasons;
- analysis/calibration versions;
- a digest over the complete baseline snapshot.

The candidate batch, all of its runs, all evidence at or after
`baseline_as_of`, and duplicate evidence ids are excluded. A production
regression signal requires at least five compatible historical full batches
spanning at least three UTC days, plus five valid candidate member runs for the
target.

The regression statistic operates on independent clusters:

1. Each candidate member run produces one canonical ability/category summary;
   tasks inside that run are not independent samples.
2. Each historical batch produces one median summary per target from its valid
   member runs.
3. The candidate statistic is the median of its five member-run summaries.
4. The baseline statistic is the median of the eligible historical-batch
   medians.
5. A deterministic cluster bootstrap resamples candidate member runs and
   historical batches, never individual task rows and never a cross-product of
   repetitions.
6. Define `delta = candidate - baseline`; negative is worse. A likely
   regression requires
   `absolute_drop = baseline - candidate >= policy.min_absolute_drop`,
   `relative_drop = absolute_drop / baseline >= policy.min_relative_drop`,
   and the upper bound of the delta interval to be at or below
   `-policy.min_absolute_drop`. A non-positive baseline cannot satisfy the
   relative-drop gate.

A member is eligible for regression only when it is completed, every required
task has canonical scoreable evidence, and guided tasks have accepted
isolation attestation. Any missing or infrastructure-invalid task makes the
whole member non-evidence for regression rather than allowing a biased partial
score. It remains visible in invalid-evidence counts. Matched-task/category
detail is shown for diagnosis but cannot increase the effective independent
sample count.

Variant and anti-memorization packs are deferred. Exact suite content hashes
remain required until a separately reviewed, versioned task-family equivalence
contract and paired-variant goldens exist.

### 11.4 Signal levels

- `insufficient_data`: quick scan, too few valid members, or no compatible
  baseline.
- `stable`: observed difference remains inside the versioned tolerance.
- `watch`: the effect exceeds a provisional threshold but frozen-baseline,
  confidence, calibration, or repeat
  evidence is insufficient.
- `likely_regression`: a full scan has sufficient independent clustered
  evidence, the
  versioned absolute and relative drop thresholds are both exceeded, and the
  confidence interval excludes the tolerated drop.

Threshold formulas, interval level, resample count, seed derivation, and
minimum counts live in a versioned calibration policy. Goldens cover
self-exclusion, evidence de-duplication, cutoff/window selection, nested
dependence, direction/sign, missing members, and exact-hash incompatibility.
Until real-user calibration and independent statistical review are complete,
all modes are capped at descriptive `insufficient_data` or “值得复测”; the
product does not use “likely regression” or “confirmed downgrade”.

## 12. Presentation contract

Each homogeneous cohort can be rendered as a target-by-category matrix, but
every cell must have a drill-down to:

- valid/invalid sample count;
- score distribution;
- matched task evidence;
- pack/scoring versions;
- model provenance;
- time window;
- baseline compatibility reason;
- uncertainty.

The UI must distinguish:

- untested;
- queued;
- running;
- completed;
- invalid infrastructure result;
- insufficient evidence;
- unavailable target.

A campaign with client and CLI cohorts uses separate matrices with an explicit
“题包不同，不可直接比较” boundary. Requested models and provider default
routes use those exact labels. A matrix is a view of evidence, not the evidence
store itself.

## 13. Privacy and export

- Raw answers stay local under the existing retention policy.
- Batch history stores no credentials, process paths, raw accessibility tree,
  conversation title, or account identifier.
- Public export excludes raw answers by default.
- Exported aggregate reports contain target provenance, pack/scoring hashes,
  sample counts, invalid-evidence counts, and analysis version.
- Deleting a batch offers either unlink-only or deleting its owned runs and raw
  artifacts; active data cannot be deleted. Database ownership changes are
  transactional while artifacts use the recoverable two-phase lifecycle.
- Backup export includes batch tables and hashes. Import/restore is not claimed
  by this feature until the product adds a separately designed restore flow.

## 14. Delivery phases

### Phase A — v0.2.2 foundation

- Trusted local CLI detection and retained launch identity.
- Honest model/reasoning provenance.
- Single-target manual and CLI runs.
- No multi-target execution yet.

### Phase B — v0.3 guided comparison

- Batch domain, migration, estimator, deterministic schedule, and recovery.
- Two to four guided client targets using one sealed client pack, one
  repetition.
- Per-task user-attested conversation isolation.
- Matrix history with `insufficient_data`; no degradation claim.

### Phase C — v0.4 automated CLI batches

- Sequential Codex CLI/Claude Code member-run queue.
- CLI-only homogeneous cohorts using one sealed CLI pack and verifier contract.
- Standard three-repetition scans.
- Pause/resume on auth, quota, network, or user action.
- Matched-task and category distributions.

### Phase D — v0.5 reliable detection

- Five-repetition full scan.
- Compatible historical baselines.
- Versioned cluster-aware analysis, matched-task diagnostics, uncertainty, and
  calibrated signal levels.
- Frozen baseline snapshots and cluster-aware analysis.
- Variant packs remain deferred until task-family equivalence is reviewed.

### Phase E — v0.8 optional Windows client automation

- Separate action permission and security review.
- Visible, interruptible provider-specific automation.
- Guided fallback remains fully supported.

## 15. Acceptance criteria

- A batch cannot reserve or launch a run without an immutable estimate and
  valid execution acknowledgement.
- Two targets inside one comparison cohort receive exactly the same sealed
  tasks and scoring rules.
- Client and CLI cohorts with different packs are visibly non-comparable.
- Target order is deterministic and balanced for a fixed seed.
- Guided isolation is accepted only with per-task user attestation;
  CLI task/session/workspace isolation is machine-enforced.
- Duplicate or incoherent target provenance is rejected.
- Default-route evidence never masquerades as a concrete model.
- Infrastructure failures never reduce the ability score.
- Quick comparison never emits a degradation conclusion.
- Full regression wording requires sufficient compatible clustered evidence.
- Baselines are frozen before the candidate and exclude candidate, later, and
  duplicate evidence.
- Uncertainty resamples member runs and historical batches, never nested tasks
  as independent evidence.
- Interrupted batches keep the same preallocated run/member ownership and
  never automatically repeat ambiguous or completed work.
- Cancellation prevents future member launches.
- Estimates expose task launches, provider-turn/time upper bounds, guided
  interactions, unknown token/quota amounts, and expiry.
- Automated tests use fake adapters and synthetic records only.
- Normal CI makes zero real provider requests and consumes zero subscription
  quota.
