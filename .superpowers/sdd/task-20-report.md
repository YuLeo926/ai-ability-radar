# Task 20 completion report

## Delivery

- Fixed base: `785f272e68859f092225dbfcc6344509ddb574aa`
- Required commit subject: `feat: add local retention and full backup`
- Scope: raw-data retention, safe pruning, app-wide backup consistency,
  handle-bound artifact traversal and native publication, strict IPC DTOs,
  publication audit metadata, startup retry state, and History local-data UI.
- Test data: temporary SQLite databases, fake runners/adapters, deterministic
  clocks/hooks, disposable artifact trees, and synthetic raw sentinels only.
- Prohibited operations were not performed: no real model, Codex/Claude CLI,
  provider API, credential, subscription, native user-data deletion, upload,
  GitHub operation, or network endpoint was used. The dependency audit was
  explicitly offline.

## Implemented behavior

### Repository and retention

- Migration `0002_settings.sql` adds the default-forever
  `raw_retention_days` setting and the path-free `publications` audit table.
- The repository enforces exactly `null`, `7`, `30`, or `90`, rejects corrupt
  JSON, keeps `secure_delete` enabled, and uses only terminal
  `Completed`/`Cancelled` rows with real `finished_at`.
- Expiry is inclusive at `finished_at <= now - days`; future, active, created,
  resumable interrupted, and unfinished rows do not expire.
- Candidate cleanup rechecks policy, status, target snapshot, timestamp, and
  cutoff inside an immediate transaction before clearing raw references.
- Pruning claims the complete candidate set before artifact access, removes
  only the target-specific app-owned raw tree, and preserves runs, task
  evidence, scores, and summaries. Missing trees are retry-safe.
- A saved policy remains effective if filesystem cleanup fails. Startup and
  explicit retry use the same safe implementation and expose only a generic
  `cleanupPending` state.

### Backup and filesystem authority

- Full backup takes an exclusive non-blocking app-wide local-data claim after
  native picker selection. Picker cancellation creates no gate claim,
  snapshot, archive, destination temporary, or mutation.
- The exclusive claim is held through running/recovery rechecks, SQLite
  snapshot, artifact enumeration/open, ZIP streaming, `sync_all`, no-replace
  publication, post-publication inspection, and cleanup.
- Every write-capable Tauri/background entry participates in the global gate:
  manual start/resume/next-step completion/submit, CLI prepare/resume and its
  complete background lifetime, cancel, raw/run/target deletion, retention,
  and post-export publication. Acquisition order is global gate before any
  per-run claim.
- The SQLite snapshot and its exact canonical run UUID/target bindings are
  captured together. No later live run list is used for artifact binding.
- Windows artifact traversal enumerates from retained handles, opens children
  relative to retained parents with reparse following disabled, validates
  opened handle kinds, rejects unknown UUID/layout/root entries and duplicate
  identities, and streams from retained ordinary-file handles.
- The destination writer retains no-delete-share ancestor handles, rejects
  remote/unknown/optical/reparse/wrong-volume/dot paths, creates a random
  same-directory file with create-new, syncs, publishes handle-relatively
  without replacement, and inspects the same handle after publication.
- Full backup additionally retains the app-data directory authority during
  publication and compares volume plus final handle paths before creating the
  destination temporary. This closes lexical/NTFS short-name alias bypasses of
  the outside-app-data rule.
- Cleanup failure is a fixed generic incomplete-cleanup error. Injected
  enumeration, streaming, inspection, write, and cleanup failures are tested;
  private temporaries are removed whenever deletion succeeds and are never
  silently hidden when injected deletion itself fails.

### ZIP, publication, IPC, and UI

- The archive is streamed directly to the destination temporary. It contains
  exactly one `ability-radar.sqlite`, one `backup-manifest.json`, and canonical
  sorted files below `artifacts/runs/<uuid>/...`.
- ZIP names reject absolute/drive/UNC forms, backslashes, colon/ADS,
  traversal/dot/empty/control components, unknown top levels, and duplicates.
- The manifest contains only fixed schema/app/time and truthful
  `containsRawAnswersAndLogs=true`, `encrypted=false` fields.
- Publication rows contain only canonical UUIDs, UTC time, lowercase SHA-256,
  and fixed `local_html`; report publication audit insertion is explicitly
  best-effort after successful publication, so an audit-row failure cannot
  turn a published report into a false export failure.
- Retention and full-backup inputs use `deny_unknown_fields`. Retention is a
  required-nullable allowlisted field. Backup requires an exact `true`
  acknowledgement and accepts no path.
- The invoke surface is exactly the reviewed 17 commands. The TypeScript
  bridge sends exact camelCase payloads and no destination path.
- History exposes an accessible local-data section even with empty history.
  It separates effective and pending retention, requires explicit shortening
  confirmation, preserves score/evidence copy, reloads after partial cleanup,
  treats picker cancellation as silent, gates backup behind an unchecked
  acknowledgement, suppresses duplicate clicks, rejects stale completions,
  and renders only fixed safe errors.

## TDD evidence

### Inherited work

The continuation handoff reported valid RED then GREEN slices for repository
settings/candidates/transactions, snapshot/publication, ArtifactStore, the
consistency gate, data management, strict DTOs, Tauri commands, invoke
allowlist, TypeScript bridge, and startup cleanup-pending. Those RED outputs
were not persisted. This report does not invent them; the inherited reasons
are recorded only as reported in `task-20-continuation.md` (principally missing
policy/candidate/snapshot/publication/module/DTO/command/bridge APIs, plus the
required-nullable Serde behavior).

The inherited History implementation already made the added stale,
confirmation, duplicate-click, cancellation, partial-cleanup, and safe-error
characterization tests pass. They are recorded as GREEN coverage, not claimed
as newly observed RED.

### RED/GREEN observed during continuation

1. Missing global gate on `next_manual_step`
   - RED command:
     `cargo test -p ability-radar next_manual_step_enters_the_global_gate_before_the_service_can_complete_a_run`
   - After correcting a test-only missing import, the valid RED was exit 1,
     `E0425`: missing `next_manual_step_for`.
   - Root cause: `ManualRunService::next_step` can call
     `complete_active_run` on a retry path, so the apparently read-like
     command can mutate the run.
   - GREEN: helper acquires the global mutation claim before calling the
     service; 1 passed, 0 failed.

2. Deterministic destination cleanup-failure injection
   - RED command:
     `cargo test -p ability-radar cleanup_failure_is_surfaced_generically_without_hiding_the_private_temporary`
   - RED: exit 1, `E0425`, missing injectable
     `write_new_file_with_inspector_and_cleanup`.
   - GREEN: production deletion remains `delete_file_handle`; the test injects
     only the cleanup function. Fixed generic error and visible private
     temporary behavior passed, 1 passed, 0 failed.

3. Handle-bound outside-app-data enforcement
   - RED command:
     `cargo test -p ability-radar backup_writer_rejects_a_handle_bound_app_data_parent_before_creating_a_temporary`
   - RED: exit 1, `E0425`, missing `write_new_file_outside`.
   - One intermediate compile attempt exposed a closure-lifetime/unused-mut
     implementation error and was not counted as GREEN.
   - GREEN: backup publication retains both authorities and rejects a same or
     descendant final handle path before temp creation; focused containment
     and successful external publication tests both passed.

## Focused verification

- First required History run:
  `npm test -- --run History`
  - exit 0; 2/2 files, 31/31 tests, 0 failed.
  - npm emitted only a forward-looking argument-parsing warning.
- Final History run:
  `npm test -- --run History`
  - exit 0; 2/2 files, 35/35 tests, 0 failed.
- Repository retention:
  `cargo test -p ability-core --test storage retention`
  - 3 passed, 0 failed.
- Snapshot:
  `cargo test -p ability-core --test storage backup_snapshot`
  - 1 passed, 0 failed.
- Publication repository:
  `cargo test -p ability-core --test storage publication_rows`
  - 1 passed, 0 failed.
- Handle-relative artifact backup:
  `cargo test -p ability-core --test artifact_deletion backup_enumeration`
  - 5 passed, 0 failed.
- Data management:
  `cargo test -p ability-radar data_management_tests`
  - 4 passed, 0 failed.
- Strict DTOs:
  retention input 1 passed; full-backup input 1 passed.
- Local-data gate:
  2 passed, 0 failed.
- Backup-related filter after the added publication test:
  8 passed, 0 failed.
- Publication best-effort, retention cleanup-pending, startup pending, invoke
  allowlist, `next_manual_step` gate, cleanup injection, and outside-app-data
  containment each passed their focused test with 0 failures.

All Rust commands were run from the Visual Studio 2022 Build Tools amd64
developer environment (`-arch=amd64 -host_arch=amd64`).

## Final required gates

These results are from the fresh run after the final outside-app-data
production change:

- `cargo test --workspace --all-targets`
  - exit 0; 257 passed, 0 failed.
  - Breakdown: ability-adapters 82, ability-core 113, desktop 62.
  - Rust emitted a non-failing localized MSVC linker-output warning while
    producing the desktop import library; clippy with warnings denied passed.
- `cargo fmt --all -- --check`
  - exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`
  - exit 0.
- `npm test`
  - exit 0; 10/10 files, 148/148 tests, 0 failed.
- `npm run build`
  - exit 0; TypeScript and Vite production build completed, 59 modules.
- `npm audit --offline --audit-level=high`
  - exit 0; 0 vulnerabilities.
- `git diff --check`
  - exit 0. Git printed only configured LF-to-CRLF working-copy notices.

## Programmatic ZIP inspection

The verification ZIP was generated from the focused Rust test using
`ABILITY_RADAR_TEST_ZIP_OUT=target/task20-verification.zip`. An initial command
with an overly strict `--exact` filter ran 0 tests and is not counted. The
corrected command ran the target test: 1 passed, 0 failed.

An independent PowerShell `System.IO.Compression.ZipFile` inspection asserted:

- 3 unique entries:
  `ability-radar.sqlite`, `backup-manifest.json`, and one canonical
  `artifacts/runs/<uuid>/answer.txt`;
- no absolute, drive, backslash, empty, dot, or traversal entry;
- exact manifest property allowlist and
  `containsRawAnswersAndLogs=true`, `encrypted=false`, `schemaVersion=1`;
- SQLite header `SQLite format 3`.

The generating Rust test also wrote the SQLite entry to a disposable path,
opened it with rusqlite, and successfully queried the `runs` table, proving the
snapshot is readable rather than relying only on the header.

## Manual/native and residual concerns

- No real native picker interaction or production-data workflow was performed;
  picker and publication behavior is covered by helper and Windows handle
  regression tests.
- No packaging installer was built because it was not a required gate.
- Residual implementation concern: none known from the reviewed Task 20
  requirements. The only observed environmental noise is the non-failing MSVC
  localized linker-output warning described above.

## Reviewer fix wave 1

This section appends the evidence for
`.superpowers/sdd/task-20-review-fix-1.md` and supersedes the earlier final-gate
counts above. No prohibited real model, CLI, API, network, credential, upload,
or production-data operation was used.

### Cleanup-incomplete classification

- Frontend RED:
  `npm test --workspace apps/desktop -- HistoryPage.ui.test.tsx`
  failed 1 of 31 because the cleanup-incomplete backend message rendered the
  ordinary backup failure.
- Rust RED:
  `cargo test -p ability-radar backup_writer_maps_cleanup_incomplete_separately_without_sensitive_details`
  failed to compile with missing `NativeWriteError` and
  `map_backup_write_error`.
- GREEN: native publication now returns typed `Operation` or
  `CleanupIncomplete`. Backup maps cleanup failure to the fixed private-data
  warning, maps all ordinary failures to the unchanged safe message, and never
  includes native/SQLite/path details. HTML export still maps both variants to
  its unchanged generic report error.
- Focused GREEN: Rust 1/1; HistoryPage 31/31. The UI distinguishes only the
  exact fixed cleanup message, keeps ordinary failures generic, preserves
  stale-completion suppression, and renders failures with `role="alert"`.

### File-backed private SQLite snapshot

- Storage RED:
  `cargo test -p ability-core --test storage backup_snapshot_binds_exact_run_identities_and_is_a_readable_sqlite_database`
  failed with missing `snapshot_to_backup_file`.
- Windows authority RED:
  `cargo test -p ability-radar private_backup_snapshot_`
  failed with missing `with_private_snapshot` and
  `with_private_snapshot_and_cleanup`.
- GREEN: the repository now backs up directly to a caller-created SQLite file
  and derives canonical run/target bindings by querying that exact snapshot.
  The former in-memory database, `serialize`, and full `Vec<u8>` allocation
  path is removed.
- The desktop creates a random create-new snapshot below app data while
  retaining the no-delete-share parent chain and a read authority handle.
  SQLite writes through the same random path; ZIP reads the same file through
  the retained handle. Journal mode is disabled for the private snapshot, and
  tests assert no snapshot or journal residue on success and failure.
- Cleanup opens the file relative to the retained parent, validates the same
  volume and file index against the retained handle, and then marks that exact
  handle for deletion. Every post-create exit attempts cleanup; an injected
  cleanup failure returns the typed incomplete-cleanup result.
- An intermediate full-gate run exposed Windows share incompatibility because
  the first retained handle requested `DELETE` while SQLite did not share it.
  That run is not counted as final evidence. The corrected authority handle is
  read-only and the relative cleanup handle alone requests `DELETE`.
- Focused GREEN after correction: storage 1/1, private snapshot 2/2, full
  backup success 1/1, and data-management backup 2/2.

### Artifact source drive and volume authority

- RED:
  `cargo test -p ability-core artifact_store::windows::tests::source_`
  failed with missing source-drive classifier, `VolumeAuthority`,
  `HandleSnapshot`, and validator.
- GREEN: `GetDriveTypeW` allows only fixed, removable, and RAM-disk sources.
  Remote, unknown, missing-root, optical, and all other types fail closed.
- The opened root binds the source volume serial. Every subsequently opened
  root component, `runs` directory, run directory, recursive directory, and
  ordinary file is validated as non-reparse and on that same volume before
  backup, deletion, or recovery logic uses it.
- Focused GREEN: authority 2/2 and `artifact_deletion` 10/10.

### Windows-canonical ZIP components and collisions

- Component RED:
  `cargo test -p ability-core windows_zip_component_key_`
  failed with missing canonical Windows ZIP-name validation.
- Collision RED:
  `cargo test -p ability-radar streaming_zip_refuses_windows_case_collisions_before_writing_an_entry`
  demonstrated that `Answer.txt` and `answer.TXT` were both accepted.
- GREEN: one shared canonical validator rejects Windows device basenames,
  trailing dot/space, separators, colon, control, empty, dot, and traversal
  components and returns a case-folded key.
- Artifact enumeration checks canonical sibling collisions in every directory
  and canonical full ZIP-name collisions. `StreamingZip` uses the same key and
  rejects a collision before writing any bytes for the second entry.
- Focused GREEN: component 1/1, streaming collision 1/1,
  `artifact_deletion` 10/10, and end-to-end unsafe-artifact publication 1/1
  with no final destination or destination temporary.

### Fresh final gates after reviewer fixes

All Rust commands used the Visual Studio 2022 Build Tools amd64 developer
environment.

- `cargo test --workspace --all-targets`
  - exit 0; 264 passed, 0 failed.
  - Breakdown: ability-adapters 82, ability-core 116, desktop 66.
- `cargo fmt --all -- --check`
  - exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`
  - exit 0.
- `npm test`
  - exit 0; 10/10 files, 149/149 tests, 0 failed.
- `npm run build`
  - exit 0; TypeScript and Vite production build completed, 59 modules.
- `npm audit --offline --audit-level=high`
  - exit 0; 0 vulnerabilities.
- `git diff --check`
  - exit 0; only configured LF-to-CRLF notices were printed.

### Fresh independent ZIP verification

The focused generator test passed 1/1 and wrote
`target/task20-final-verify.zip`. An initial inspection attempt failed only
because PowerShell had not loaded the `System.IO.Compression.FileSystem`
assembly; it is not counted as evidence. The corrected independent inspection
returned:

`ZIP_VERIFY_OK entries=3 unique_windows_keys=3 manifest_schema=1 sqlite_header=ok`

It checked exact and Windows-case uniqueness, every component against
device/trailing/dot/traversal rules, required manifest truth fields, and the
16-byte SQLite header. The generator test also opened the extracted SQLite
entry and queried `runs`. The disposable verification ZIP was removed.

## Reviewer fix wave 2

This section appends the evidence for
`.superpowers/sdd/task-20-review-fix-2.md`. It does not alter or invent the
earlier inherited RED history. No real model, Codex/Claude CLI, provider API,
network, credential, subscription, upload, or production-data operation was
used.

### Retained snapshot no-delete authority

- Pre-SQLite swap RED:
  `cargo test -p ability-radar private_snapshot_blocks_rename_replacement_before_sqlite_opens_the_path -- --nocapture`
  failed at runtime with the explicit assertion
  `retained handle allowed rename`.
- Root cause: the retained snapshot handle shared delete. Windows therefore
  permitted the random create-new file to be renamed between creation and
  SQLite opening the same path, allowing path identity and retained-handle
  identity to diverge.
- GREEN: the retained handle requests only read-data/read-attributes/synchronize
  and shares only read/write. It remains open, without delete sharing, through
  SQLite backup, binding query, artifact enumeration, and ZIP streaming.
  The deterministic rename/replacement attempt now fails and the focused
  regression passes 1/1.

### Post-release cleanup identity

- RED:
  `cargo test -p ability-radar post_release_snapshot_swap_is_cleanup_incomplete_and_never_publishes_zip`
  failed to compile because
  `with_private_snapshot_with_release_hook` did not exist.
- GREEN: after the operation, the retained handle is inspected again and the
  expected non-reparse ordinary-file facts, volume, file index, final parent,
  and random name are retained. The read handle is then closed.
- From the still-retained parent authority, cleanup opens the random name
  relative with DELETE access, validates all expected facts and exact identity,
  and only then deletes that opened handle.
- The deterministic after-release hook renames the original and creates a
  replacement. Cleanup detects the identity mismatch, returns typed
  `CleanupIncomplete`, removes the destination ZIP temporary, publishes no
  final ZIP, and deletes neither the attacker replacement nor the detached
  original. Focused GREEN: 1/1.
- Cleanup injection remains handle-scoped after identity validation. Success
  and every ordinary failure path attempt cleanup; open, mismatch, inspection,
  injected deletion, and native deletion failures fail closed as
  `CleanupIncomplete`.

### Cross-platform name-resolution contract

- `rustup target list --installed` showed only
  `x86_64-pc-windows-msvc`; no target was downloaded and no network was used.
  The review-permitted source-level compile contract was therefore used.
- The first attempt to run that contract was blocked at compilation by the
  still-missing release-hook symbol from the preceding RED and is not counted
  as the cross-platform RED.
- After restoring compilation, the valid RED command
  `cargo test -p ability-radar cross_platform_backup_path_resolves_only_paired_cfg_helpers -- --nocapture`
  failed because the paired `write_full_backup_to_destination` helpers were
  absent.
- GREEN: the cross-platform export function references only
  `write_full_backup_to_destination`. Its Windows implementation contains the
  private snapshot and streaming logic. Its non-Windows implementation has the
  same cross-platform signature, returns only the fixed unsupported error, and
  performs no repository, database, or file operation. Contract GREEN: 1/1.

### Focused regression evidence

All Rust commands used the Visual Studio 2022 Build Tools amd64 developer
environment.

- Snapshot filter: 5/5 passed.
- Data-management full backup: 2/2 passed.
- Native command/writer/backup regressions: 36/36 passed.
- ArtifactStore native traversal/deletion regressions: 10/10 passed.
- Repository file-backed snapshot: 1/1 passed.

### Fresh final gates after reviewer fix wave 2

- `cargo test --workspace --all-targets`
  - exit 0; 267 passed, 0 failed.
  - Breakdown: ability-adapters 82, ability-core 116, desktop 69.
- `cargo fmt --all -- --check`
  - exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`
  - exit 0.
- `npm test`
  - exit 0; 10/10 files, 149/149 tests, 0 failed.
- `npm run build`
  - exit 0; TypeScript and Vite production build completed, 59 modules.
- `npm audit --offline --audit-level=high`
  - exit 0; 0 vulnerabilities.
- `git diff --check`
  - exit 0; only configured LF-to-CRLF notices were printed.

### Fresh programmatic ZIP verification

The focused generator passed 1/1. Independent
`System.IO.Compression.ZipFile` inspection returned:

`ZIP_FIX2_FINAL_OK entries=3 unique_windows_keys=3 manifest_schema=1 sqlite_header=ok`

It checked required and unique Windows-safe entries, manifest truth fields,
and the SQLite header. The generator test also opened and queried the extracted
snapshot. The disposable verification ZIP was removed.
