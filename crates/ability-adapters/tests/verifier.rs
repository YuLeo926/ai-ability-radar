use ability_adapters::{
    NodeVerifier, OutputStream, ProcessEnvironment, ProcessError, ProcessOutput, ProcessRunner,
    ProcessSpec, TokioProcessRunner,
};
use ability_core::{FailureKind, TaskOutcome};
use async_trait::async_trait;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::{TempDir, tempdir};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct RecordingRunner {
    calls: Arc<Mutex<Vec<ProcessSpec>>>,
    response: RunnerResponse,
}

#[derive(Clone)]
enum RunnerResponse {
    Output(ProcessOutput),
    Error(ProcessErrorCase),
}

#[derive(Clone, Copy, Debug)]
enum ProcessErrorCase {
    SpawnNotFound,
    SpawnOther,
    Supervision,
    Wait,
    CaptureFailed,
    Cancelled,
    TimedOut,
    StdoutLimit,
    StderrLimit,
    TerminationFailed,
    DurationOverflow,
}

#[async_trait]
impl ProcessRunner for RecordingRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.calls.lock().unwrap().push(spec);
        match &self.response {
            RunnerResponse::Output(output) => Ok(output.clone()),
            RunnerResponse::Error(error) => Err(process_error(*error)),
        }
    }
}

fn process_error(error: ProcessErrorCase) -> ProcessError {
    match error {
        ProcessErrorCase::SpawnNotFound => {
            ProcessError::Spawn(io::Error::from(io::ErrorKind::NotFound))
        }
        ProcessErrorCase::SpawnOther => {
            ProcessError::Spawn(io::Error::from(io::ErrorKind::PermissionDenied))
        }
        ProcessErrorCase::Supervision => {
            ProcessError::Supervision(io::Error::from(io::ErrorKind::Other))
        }
        ProcessErrorCase::Wait => ProcessError::Wait(io::Error::from(io::ErrorKind::Other)),
        ProcessErrorCase::CaptureFailed => ProcessError::CaptureFailed,
        ProcessErrorCase::Cancelled => ProcessError::Cancelled,
        ProcessErrorCase::TimedOut => ProcessError::TimedOut,
        ProcessErrorCase::StdoutLimit => ProcessError::OutputLimit {
            stream: OutputStream::Stdout,
        },
        ProcessErrorCase::StderrLimit => ProcessError::OutputLimit {
            stream: OutputStream::Stderr,
        },
        ProcessErrorCase::TerminationFailed => ProcessError::TerminationFailed,
        ProcessErrorCase::DurationOverflow => ProcessError::DurationOverflow,
    }
}

struct VerifierFixture {
    _pack: TempDir,
    workspace: TempDir,
    pack_root: PathBuf,
}

impl VerifierFixture {
    fn new() -> Self {
        let pack = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        fs::create_dir_all(pack.path().join("tasks").join("dedupe-events")).unwrap();
        fs::create_dir_all(pack.path().join("tasks").join("retry-schedule")).unwrap();
        fs::write(
            pack.path()
                .join("tasks")
                .join("dedupe-events")
                .join("verify.mjs"),
            "console.log('TASK_PASSED');",
        )
        .unwrap();
        fs::write(
            pack.path()
                .join("tasks")
                .join("retry-schedule")
                .join("verify.mjs"),
            "console.log('TASK_PASSED');",
        )
        .unwrap();
        let pack_root = pack.path().to_path_buf();
        Self {
            _pack: pack,
            workspace,
            pack_root,
        }
    }
}

fn output(exit_code: i32, stdout: &str, stderr: &str) -> RunnerResponse {
    RunnerResponse::Output(ProcessOutput {
        exit_code: Some(exit_code),
        stdout: stdout.into(),
        stderr: stderr.into(),
        duration_ms: 10,
    })
}

fn verifier_with_response(
    fixture: &VerifierFixture,
    response: RunnerResponse,
) -> (NodeVerifier, Arc<Mutex<Vec<ProcessSpec>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runner = RecordingRunner {
        calls: calls.clone(),
        response,
    };
    (
        NodeVerifier::new(Arc::new(runner), fixture.pack_root.clone()),
        calls,
    )
}

#[tokio::test]
async fn a_zero_exit_hidden_verifier_passes() {
    let fixture = VerifierFixture::new();
    let (verifier, _) = verifier_with_response(&fixture, output(0, "TASK_PASSED", ""));
    let grade = verifier
        .verify(
            "dedupe-events-v1",
            fixture.workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(grade.outcome, TaskOutcome::Passed);
    assert_eq!(grade.score, Some(100.0));
    assert_eq!(grade.failure_kind, None);
}

struct CountingRunner(AtomicUsize);

#[async_trait]
impl ProcessRunner for CountingRunner {
    async fn run(
        &self,
        _spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        unreachable!("unknown verifier IDs must not execute")
    }
}

#[tokio::test]
async fn an_unknown_verifier_is_not_executed() {
    let fixture = VerifierFixture::new();
    let runner = Arc::new(CountingRunner(AtomicUsize::new(0)));
    let verifier = NodeVerifier::new(runner.clone(), fixture.pack_root);
    let grade = verifier
        .verify(
            "untrusted-command",
            fixture.workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(runner.0.load(Ordering::SeqCst), 0);
    assert_eq!(grade.outcome, TaskOutcome::Invalid);
    assert_eq!(grade.failure_kind, Some(FailureKind::VerifierError));
}

#[tokio::test]
async fn node_process_spec_is_direct_allowlisted_and_clears_the_environment() {
    let fixture = VerifierFixture::new();
    let (verifier, calls) = verifier_with_response(&fixture, output(0, "TASK_PASSED\n", ""));
    verifier
        .verify(
            "dedupe-events-v1",
            fixture.workspace.path(),
            CancellationToken::new(),
        )
        .await;

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let spec = &calls[0];
    let script = fixture
        .pack_root
        .join("tasks/dedupe-events/verify.mjs")
        .canonicalize()
        .unwrap();
    let workspace = fixture.workspace.path().canonicalize().unwrap();
    #[cfg(windows)]
    let script = PathBuf::from(script.to_string_lossy().trim_start_matches(r"\\?\"));
    #[cfg(windows)]
    let workspace = PathBuf::from(workspace.to_string_lossy().trim_start_matches(r"\\?\"));
    assert_eq!(spec.program, "node");
    assert_eq!(
        spec.args,
        vec![
            "--no-warnings",
            &script.to_string_lossy(),
            &workspace.to_string_lossy(),
        ]
    );
    assert_eq!(spec.current_dir, workspace);
    assert_eq!(spec.environment, ProcessEnvironment::Clear);
    #[cfg(windows)]
    assert_eq!(
        spec.env,
        [("SystemRoot".into(), std::env::var("SystemRoot").unwrap())].into()
    );
    #[cfg(not(windows))]
    assert!(spec.env.is_empty());
    assert_eq!(spec.timeout, Duration::from_secs(120));
}

#[tokio::test]
async fn strict_terminal_protocol_rejects_contradictory_injected_and_malformed_output() {
    let cases = [
        (0, "prefix TASK_PASSED", ""),
        (0, "TASK_PASSED\nTASK_PASSED\n", ""),
        (0, "TASK_PASSED\n", "warning"),
        (0, "", "TASK_FAILED\n"),
        (1, "TASK_PASSED\n", "TASK_FAILED\n"),
        (1, "", "prefix TASK_FAILED"),
        (2, "", "TASK_FAILED\n"),
        (1, "unexpected", "TASK_FAILED\n"),
    ];
    for (exit_code, stdout, stderr) in cases {
        let fixture = VerifierFixture::new();
        let (verifier, _) = verifier_with_response(&fixture, output(exit_code, stdout, stderr));
        let grade = verifier
            .verify(
                "dedupe-events-v1",
                fixture.workspace.path(),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            (grade.outcome, grade.failure_kind),
            (TaskOutcome::Invalid, Some(FailureKind::VerifierError)),
            "accepted protocol: exit={exit_code}, stdout={stdout:?}, stderr={stderr:?}"
        );
    }
}

#[tokio::test]
async fn expected_failure_marker_is_a_wrong_answer() {
    let fixture = VerifierFixture::new();
    let (verifier, _) = verifier_with_response(&fixture, output(1, "", "TASK_FAILED\r\n"));
    let grade = verifier
        .verify(
            "retry-schedule-v1",
            fixture.workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(grade.outcome, TaskOutcome::Failed);
    assert_eq!(grade.score, Some(0.0));
    assert_eq!(grade.failure_kind, Some(FailureKind::WrongAnswer));
}

#[tokio::test]
async fn every_process_error_maps_to_the_required_grade() {
    let cases = [
        (
            ProcessErrorCase::SpawnNotFound,
            TaskOutcome::Invalid,
            FailureKind::RuntimeMissing,
        ),
        (
            ProcessErrorCase::SpawnOther,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::Supervision,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::Wait,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::CaptureFailed,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::Cancelled,
            TaskOutcome::Cancelled,
            FailureKind::UserCancelled,
        ),
        (
            ProcessErrorCase::TimedOut,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::StdoutLimit,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::StderrLimit,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::TerminationFailed,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
        (
            ProcessErrorCase::DurationOverflow,
            TaskOutcome::Invalid,
            FailureKind::VerifierError,
        ),
    ];

    for (error, expected_outcome, expected_kind) in cases {
        let fixture = VerifierFixture::new();
        let (verifier, _) = verifier_with_response(&fixture, RunnerResponse::Error(error));
        let grade = verifier
            .verify(
                "dedupe-events-v1",
                fixture.workspace.path(),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            (grade.outcome, grade.failure_kind),
            (expected_outcome, Some(expected_kind)),
            "incorrect mapping for {error:?}"
        );
    }
}

#[cfg(windows)]
#[tokio::test]
async fn a_workspace_tree_containing_a_reparse_point_is_rejected_before_spawn() {
    use std::process::Command;

    let fixture = VerifierFixture::new();
    let target = tempdir().unwrap();
    let junction = fixture.workspace.path().join("linked");
    let status = Command::new("cmd")
        .args([
            "/c",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            target.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let (verifier, calls) = verifier_with_response(&fixture, output(0, "TASK_PASSED", ""));
    let grade = verifier
        .verify(
            "dedupe-events-v1",
            fixture.workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(grade.outcome, TaskOutcome::Invalid);
    assert_eq!(grade.failure_kind, Some(FailureKind::VerifierError));
}

fn bundled_pack_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmark-packs/cli-quick-v1")
}

#[tokio::test]
async fn both_bundled_starters_fail_with_the_local_node_runtime() {
    let verifier = NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root());
    for (verifier_id, starter) in [
        ("dedupe-events-v1", "tasks/dedupe-events/starter"),
        ("retry-schedule-v1", "tasks/retry-schedule/starter"),
    ] {
        let grade = verifier
            .verify(
                verifier_id,
                &bundled_pack_root().join(starter),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            (grade.outcome, grade.failure_kind),
            (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
            "{verifier_id}: {}",
            grade.detail
        );
    }
}

#[tokio::test]
async fn both_known_good_repositories_pass_with_the_local_node_runtime() {
    let verifier = NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root());

    let dedupe = tempdir().unwrap();
    fs::create_dir(dedupe.path().join("src")).unwrap();
    fs::write(
        dedupe.path().join("src/dedupeEvents.mjs"),
        r#"
export function dedupeEvents(events) {
  const latest = new Map();
  for (const event of events) {
    if (event === null || typeof event !== "object" ||
        typeof event.id !== "string" || event.id.length === 0 ||
        Number.isNaN(Date.parse(event.occurredAt))) continue;
    const previous = latest.get(event.id);
    if (!previous || Date.parse(event.occurredAt) >= Date.parse(previous.occurredAt)) {
      latest.set(event.id, event);
    }
  }
  return [...latest.values()].sort((left, right) =>
    Date.parse(left.occurredAt) - Date.parse(right.occurredAt) ||
    left.id.localeCompare(right.id));
}
"#,
    )
    .unwrap();
    let grade = verifier
        .verify("dedupe-events-v1", dedupe.path(), CancellationToken::new())
        .await;
    assert_eq!(
        (grade.outcome, grade.failure_kind),
        (TaskOutcome::Passed, None),
        "{}",
        grade.detail
    );

    let retry = tempdir().unwrap();
    fs::create_dir(retry.path().join("src")).unwrap();
    fs::write(
        retry.path().join("src/retrySchedule.mjs"),
        r#"
export function buildRetrySchedule({
  maxAttempts, baseDelayMs, maxDelayMs, retryAfterMs = [],
}) {
  if (![maxAttempts, baseDelayMs, maxDelayMs].every(Number.isInteger) ||
      maxAttempts < 1 || baseDelayMs < 1 || maxDelayMs < baseDelayMs ||
      !Array.isArray(retryAfterMs) ||
      !retryAfterMs.every((value) => Number.isInteger(value) && value >= 0)) {
    throw new TypeError("invalid retry options");
  }
  const result = [0];
  let elapsed = 0;
  for (let retryIndex = 1; retryIndex < maxAttempts; retryIndex += 1) {
    const base = Math.min(baseDelayMs * 2 ** (retryIndex - 1), maxDelayMs);
    elapsed += Math.max(base, retryAfterMs[retryIndex - 1] ?? 0);
    result.push(elapsed);
  }
  return result;
}
"#,
    )
    .unwrap();
    let grade = verifier
        .verify("retry-schedule-v1", retry.path(), CancellationToken::new())
        .await;
    assert_eq!(
        (grade.outcome, grade.failure_kind),
        (TaskOutcome::Passed, None),
        "{}",
        grade.detail
    );
}

#[tokio::test]
async fn candidate_syntax_and_execution_errors_are_wrong_answers() {
    let verifier = NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root());
    for source in [
        "export function dedupeEvents( {",
        "export function dedupeEvents() { throw new Error('candidate bug'); }",
    ] {
        let workspace = tempdir().unwrap();
        fs::create_dir(workspace.path().join("src")).unwrap();
        fs::write(workspace.path().join("src/dedupeEvents.mjs"), source).unwrap();
        let grade = verifier
            .verify(
                "dedupe-events-v1",
                workspace.path(),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            (grade.outcome, grade.failure_kind),
            (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
            "{}",
            grade.detail
        );
    }
}

#[tokio::test]
async fn candidate_cannot_forge_a_pass_marker_or_exit_early_during_import() {
    let verifier = NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root());
    let workspace = tempdir().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();
    fs::write(
        workspace.path().join("src/dedupeEvents.mjs"),
        r#"
console.log("TASK_PASSED");
process.exit(0);
export function dedupeEvents(events) {
  return events;
}
"#,
    )
    .unwrap();
    let grade = verifier
        .verify(
            "dedupe-events-v1",
            workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(
        (grade.outcome, grade.failure_kind),
        (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
        "{}",
        grade.detail
    );
}

#[tokio::test]
async fn candidate_really_exit_cannot_terminate_the_trusted_parent() {
    let verifier = NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root());
    let workspace = tempdir().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();
    fs::write(
        workspace.path().join("src/dedupeEvents.mjs"),
        r#"
console.log("TASK_PASSED");
process.reallyExit(0);
export function dedupeEvents(events) {
  return events;
}
"#,
    )
    .unwrap();
    let grade = verifier
        .verify(
            "dedupe-events-v1",
            workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(
        (grade.outcome, grade.failure_kind),
        (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
        "{}",
        grade.detail
    );
}

#[tokio::test]
async fn candidate_shared_assert_and_console_mutation_cannot_forge_a_pass() {
    let verifier = NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root());
    let workspace = tempdir().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();
    fs::write(
        workspace.path().join("src/dedupeEvents.mjs"),
        r#"
import assert from "node:assert/strict";
const trustedLog = console.log.bind(console);
assert.deepEqual = () => {};
console.log = (...args) => trustedLog(...args);
export function dedupeEvents() {
  return [];
}
"#,
    )
    .unwrap();
    let grade = verifier
        .verify(
            "dedupe-events-v1",
            workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(
        (grade.outcome, grade.failure_kind),
        (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
        "{}",
        grade.detail
    );
}

#[tokio::test]
async fn missing_trusted_candidate_runner_is_a_verifier_error() {
    let pack = tempdir().unwrap();
    let parent = pack.path().join("tasks/dedupe-events/verify.mjs");
    fs::create_dir_all(parent.parent().unwrap()).unwrap();
    fs::copy(
        bundled_pack_root().join("tasks/dedupe-events/verify.mjs"),
        &parent,
    )
    .unwrap();
    let workspace = tempdir().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();
    fs::write(
        workspace.path().join("src/dedupeEvents.mjs"),
        r#"
export function dedupeEvents(events) {
  const latest = new Map();
  for (const event of events) {
    if (event && typeof event === "object" && typeof event.id === "string" &&
        event.id.length > 0 && !Number.isNaN(Date.parse(event.occurredAt))) {
      const previous = latest.get(event.id);
      if (!previous || Date.parse(event.occurredAt) >= Date.parse(previous.occurredAt)) {
        latest.set(event.id, event);
      }
    }
  }
  return [...latest.values()].sort((left, right) =>
    Date.parse(left.occurredAt) - Date.parse(right.occurredAt) ||
    left.id.localeCompare(right.id));
}
"#,
    )
    .unwrap();
    let verifier = NodeVerifier::new(Arc::new(TokioProcessRunner), pack.path().to_path_buf());
    let grade = verifier
        .verify(
            "dedupe-events-v1",
            workspace.path(),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(
        (grade.outcome, grade.failure_kind),
        (TaskOutcome::Invalid, Some(FailureKind::VerifierError)),
        "{}",
        grade.detail
    );
    assert!(grade.detail.contains("VERIFIER_ERROR"), "{}", grade.detail);
}

async fn verify_candidate_source(
    verifier_id: &str,
    source_file: &str,
    source: &str,
) -> ability_adapters::VerificationGrade {
    let workspace = tempdir().unwrap();
    fs::create_dir(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src").join(source_file), source).unwrap();
    NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root())
        .verify(verifier_id, workspace.path(), CancellationToken::new())
        .await
}

#[tokio::test]
async fn dedupe_hidden_cases_reject_previously_undetected_bad_implementations() {
    let cases = [
        r#"
export function dedupeEvents(events) {
  const latest = new Map();
  for (const event of events) {
    if (event && typeof event === "object" && typeof event.id === "string" &&
        event.id.length > 0 && !Number.isNaN(Date.parse(event.occurredAt))) {
      const previous = latest.get(event.id);
      if (!previous || Date.parse(event.occurredAt) >= Date.parse(previous.occurredAt)) {
        latest.set(event.id, event);
      }
    }
  }
  const result = [...latest.values()].sort((left, right) =>
    Date.parse(left.occurredAt) - Date.parse(right.occurredAt) ||
    left.id.localeCompare(right.id));
  result.push(...events.filter((event) => event !== null && typeof event !== "object"));
  return result;
}
"#,
        r#"
export function dedupeEvents(events) {
  const latest = new Map();
  for (const event of events) {
    if (event && typeof event === "object" && event.id !== "" &&
        !Number.isNaN(Date.parse(event.occurredAt))) {
      const previous = latest.get(event.id);
      if (!previous || Date.parse(event.occurredAt) >= Date.parse(previous.occurredAt)) {
        latest.set(event.id, event);
      }
    }
  }
  return [...latest.values()].sort((left, right) =>
    Date.parse(left.occurredAt) - Date.parse(right.occurredAt) ||
    String(left.id).localeCompare(String(right.id)));
}
"#,
        r#"
export function dedupeEvents(events) {
  const latest = new Map();
  for (const event of events) {
    if (event && typeof event === "object" && typeof event.id === "string" &&
        event.id.length > 0 && !Number.isNaN(Date.parse(event.occurredAt))) {
      const previous = latest.get(event.id);
      if (!previous || Date.parse(event.occurredAt) >= Date.parse(previous.occurredAt)) {
        latest.set(event.id, event);
      }
    }
  }
  return [...latest.values()].sort((left, right) =>
    Date.parse(left.occurredAt) - Date.parse(right.occurredAt));
}
"#,
    ];
    for source in cases {
        let grade = verify_candidate_source("dedupe-events-v1", "dedupeEvents.mjs", source).await;
        assert_eq!(
            (grade.outcome, grade.failure_kind),
            (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
            "{}",
            grade.detail
        );
    }
}

#[tokio::test]
async fn retry_hidden_cases_reject_previously_undetected_bad_implementations() {
    let cases = [
        r#"
export function buildRetrySchedule({maxAttempts, baseDelayMs, maxDelayMs, retryAfterMs = []}) {
  if (![maxAttempts, baseDelayMs, maxDelayMs].every(Number.isInteger) ||
      maxAttempts < 1 || baseDelayMs < 1 || maxDelayMs < baseDelayMs ||
      !Array.isArray(retryAfterMs) ||
      !retryAfterMs.every((value) => Number.isInteger(value) && value >= 0)) {
    throw new TypeError("invalid");
  }
  const result = [0];
  let elapsed = 0;
  for (let index = 1; index < maxAttempts; index += 1) {
    elapsed += Math.max(baseDelayMs * 2 ** (index - 1), retryAfterMs[index - 1] ?? 0);
    result.push(elapsed);
  }
  return result;
}
"#,
        r#"
export function buildRetrySchedule({maxAttempts, baseDelayMs, maxDelayMs, retryAfterMs = []}) {
  if (![maxAttempts, baseDelayMs, maxDelayMs].every(Number.isInteger) ||
      maxAttempts < 1 || baseDelayMs < 1 || maxDelayMs < baseDelayMs) {
    throw new TypeError("invalid");
  }
  const result = [0];
  let elapsed = 0;
  for (let index = 1; index < maxAttempts; index += 1) {
    const base = Math.min(baseDelayMs * 2 ** (index - 1), maxDelayMs);
    elapsed += Math.max(base, retryAfterMs[index - 1] ?? 0);
    result.push(elapsed);
  }
  return result;
}
"#,
        r#"
export function buildRetrySchedule({maxAttempts, baseDelayMs, maxDelayMs, retryAfterMs = []}) {
  if (maxAttempts === 0 || baseDelayMs === 0 || maxDelayMs < baseDelayMs ||
      !Number.isInteger(maxAttempts)) {
    throw new TypeError("invalid");
  }
  const result = [0];
  let elapsed = 0;
  for (let index = 1; index < maxAttempts; index += 1) {
    const base = Math.min(baseDelayMs * 2 ** (index - 1), maxDelayMs);
    elapsed += Math.max(base, retryAfterMs[index - 1] ?? 0);
    result.push(elapsed);
  }
  return result;
}
"#,
    ];
    for source in cases {
        let grade = verify_candidate_source("retry-schedule-v1", "retrySchedule.mjs", source).await;
        assert_eq!(
            (grade.outcome, grade.failure_kind),
            (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
            "{}",
            grade.detail
        );
    }
}

#[tokio::test]
async fn candidate_child_has_only_read_permissions_and_a_minimal_environment() {
    let source = r#"
import path from "node:path";
const readGrants = process.execArgv.filter((arg) => arg.startsWith("--allow-fs-read="));
const prohibited = [
  "--allow-fs-write",
  "--allow-child-process",
  "--allow-worker",
  "--allow-addons",
  "--allow-wasi",
];
if (!process.execArgv.includes("--experimental-permission") ||
    readGrants.length !== 2 ||
    !readGrants.every((arg) => path.isAbsolute(arg.slice(arg.indexOf("=") + 1))) ||
    prohibited.some((flag) => process.execArgv.some((arg) => arg === flag || arg.startsWith(`${flag}=`)))) {
  throw new Error("unexpected candidate permissions");
}
const environmentKeys = Object.keys(process.env);
if (environmentKeys.some((key) => key !== "SystemRoot")) {
  throw new Error("unexpected inherited environment");
}
export function dedupeEvents(events) {
  const latest = new Map();
  for (const event of events) {
    if (event && typeof event === "object" && typeof event.id === "string" &&
        event.id.length > 0 && !Number.isNaN(Date.parse(event.occurredAt))) {
      const previous = latest.get(event.id);
      if (!previous || Date.parse(event.occurredAt) >= Date.parse(previous.occurredAt)) {
        latest.set(event.id, event);
      }
    }
  }
  return [...latest.values()].sort((left, right) =>
    Date.parse(left.occurredAt) - Date.parse(right.occurredAt) ||
    left.id.localeCompare(right.id));
}
"#;
    let grade = verify_candidate_source("dedupe-events-v1", "dedupeEvents.mjs", source).await;
    assert_eq!(
        (grade.outcome, grade.failure_kind),
        (TaskOutcome::Passed, None),
        "{}",
        grade.detail
    );
}

#[tokio::test]
async fn candidate_cannot_write_directly_or_through_a_child_process() {
    for attack in ["write", "child"] {
        let workspace = tempdir().unwrap();
        fs::create_dir(workspace.path().join("src")).unwrap();
        let sentinel = workspace.path().join(format!("{attack}-sentinel.txt"));
        let sentinel_json = serde_json::to_string(&sentinel.to_string_lossy()).unwrap();
        let child_script_json = serde_json::to_string(&format!(
            r#"require("node:fs").writeFileSync({sentinel_json}, "forged")"#
        ))
        .unwrap();
        let attack_source = if attack == "write" {
            format!(
                r#"
import fs from "node:fs";
try {{ fs.writeFileSync({sentinel_json}, "forged"); }} catch {{}}
export function dedupeEvents() {{ return []; }}
"#
            )
        } else {
            format!(
                r#"
import {{ execFileSync }} from "node:child_process";
try {{
  execFileSync(process.execPath, [
    "-e",
    {child_script_json},
  ]);
}} catch {{}}
export function dedupeEvents() {{ return []; }}
"#
            )
        };
        fs::write(workspace.path().join("src/dedupeEvents.mjs"), attack_source).unwrap();
        let grade = NodeVerifier::new(Arc::new(TokioProcessRunner), bundled_pack_root())
            .verify(
                "dedupe-events-v1",
                workspace.path(),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(
            (grade.outcome, grade.failure_kind),
            (TaskOutcome::Failed, Some(FailureKind::WrongAnswer)),
            "{attack}: {}",
            grade.detail
        );
        assert!(
            !sentinel.exists(),
            "{attack} escaped the permission boundary"
        );
    }
}
