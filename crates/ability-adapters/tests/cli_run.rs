use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, AvailabilityStatus, CliRunError,
    CliRunService, ExecutionRequest, LaunchSource, PrerequisiteStatus, RunEvent, RunEventKind,
    TargetAvailability, VerificationGrade, WorkspaceVerifier, adapter_error_grade,
};
use ability_core::{
    EnvironmentFingerprint, FailureKind, LoadedPack, PackLoader, RunMode, RunRepository, RunStatus,
    StorageError, TargetKind, TargetSelection, TaskOutcome, TaskResult,
};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

enum AdapterStep {
    Complete { duration_ms: u64 },
    BudgetExhausted,
    Infrastructure(FailureKind),
    Cancelled,
}

struct FakeAdapter {
    kind: TargetKind,
    steps: Mutex<VecDeque<AdapterStep>>,
    calls: AtomicUsize,
    cancel_parent: Option<CancellationToken>,
}

impl FakeAdapter {
    fn new(kind: TargetKind, steps: impl IntoIterator<Item = AdapterStep>) -> Self {
        Self {
            kind,
            steps: Mutex::new(steps.into_iter().collect()),
            calls: AtomicUsize::new(0),
            cancel_parent: None,
        }
    }

    fn cancelling(
        kind: TargetKind,
        cancellation: CancellationToken,
        steps: impl IntoIterator<Item = AdapterStep>,
    ) -> Self {
        let mut adapter = Self::new(kind, steps);
        adapter.cancel_parent = Some(cancellation);
        adapter
    }
}

#[async_trait]
impl AgentAdapter for FakeAdapter {
    fn kind(&self) -> TargetKind {
        self.kind
    }

    async fn detect(&self) -> TargetAvailability {
        TargetAvailability {
            kind: self.kind(),
            installed: true,
            version: Some("fake-cli".into()),
            auth_state: AuthState::Unknown,
            status: AvailabilityStatus::Ready,
            source: Some(LaunchSource::ReviewedNpm),
            prerequisites: vec![PrerequisiteStatus {
                name: "fake-runtime".into(),
                available: true,
                version: Some("1".into()),
            }],
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        _cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(request.workspace.join("src/index.mjs").is_file());
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("fake adapter step");
        if let Some(cancellation) = &self.cancel_parent {
            cancellation.cancel();
        }
        match step {
            AdapterStep::Complete { duration_ms } => {
                fs::write(
                    request.workspace.join("src/index.mjs"),
                    "export const fixed = true;",
                )
                .unwrap();
                Ok(AdapterCompletion::Completed {
                    duration_ms,
                    stdout: "local agent stdout".into(),
                    stderr: "local agent stderr".into(),
                })
            }
            AdapterStep::BudgetExhausted => Err(AdapterError::AgentBudgetExceeded),
            AdapterStep::Infrastructure(kind) => Err(AdapterError::Infrastructure {
                kind,
                detail: "fake infrastructure failure".into(),
            }),
            AdapterStep::Cancelled => Err(AdapterError::Cancelled),
        }
    }
}

enum VerifierStep {
    Grade(VerificationGrade),
    Cancelled,
}

struct FakeVerifier {
    steps: Mutex<VecDeque<VerifierStep>>,
    calls: AtomicUsize,
    cancel_parent: Option<CancellationToken>,
}

impl FakeVerifier {
    fn new(steps: impl IntoIterator<Item = VerifierStep>) -> Self {
        Self {
            steps: Mutex::new(steps.into_iter().collect()),
            calls: AtomicUsize::new(0),
            cancel_parent: None,
        }
    }

    fn cancelling(
        cancellation: CancellationToken,
        steps: impl IntoIterator<Item = VerifierStep>,
    ) -> Self {
        let mut verifier = Self::new(steps);
        verifier.cancel_parent = Some(cancellation);
        verifier
    }
}

#[async_trait]
impl WorkspaceVerifier for FakeVerifier {
    async fn verify(
        &self,
        _verifier_id: &str,
        workspace: &Path,
        _cancellation: CancellationToken,
    ) -> VerificationGrade {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            fs::read_to_string(workspace.join("src/index.mjs")).unwrap(),
            "export const fixed = true;"
        );
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("fake verifier step");
        if let Some(cancellation) = &self.cancel_parent {
            cancellation.cancel();
        }
        match step {
            VerifierStep::Grade(grade) => grade,
            VerifierStep::Cancelled => cancelled_grade(),
        }
    }
}

fn passed_grade(duration_ms: u64) -> VerificationGrade {
    VerificationGrade {
        outcome: TaskOutcome::Passed,
        score: Some(100.0),
        failure_kind: None,
        detail: "hidden_tests:pass".into(),
        duration_ms,
    }
}

fn failed_grade(duration_ms: u64) -> VerificationGrade {
    VerificationGrade {
        outcome: TaskOutcome::Failed,
        score: Some(0.0),
        failure_kind: Some(FailureKind::WrongAnswer),
        detail: "hidden_tests:fail".into(),
        duration_ms,
    }
}

fn invalid_grade(kind: FailureKind) -> VerificationGrade {
    VerificationGrade {
        outcome: TaskOutcome::Invalid,
        score: None,
        failure_kind: Some(kind),
        detail: "fake verifier infrastructure failure".into(),
        duration_ms: 4,
    }
}

fn cancelled_grade() -> VerificationGrade {
    VerificationGrade {
        outcome: TaskOutcome::Cancelled,
        score: None,
        failure_kind: Some(FailureKind::UserCancelled),
        detail: "fake verifier cancelled".into(),
        duration_ms: 3,
    }
}

fn write_pack(root: &Path, tasks: usize, target: TargetKind, external_grader: bool) {
    fs::create_dir_all(root).unwrap();
    let target = match target {
        TargetKind::ChatGptClient => "chat_gpt_client",
        TargetKind::ClaudeClient => "claude_client",
        TargetKind::CodexCli => "codex_cli",
        TargetKind::ClaudeCode => "claude_code",
    };
    let tasks = (0..tasks)
        .map(|index| {
            let task = format!("task-{}", index + 1);
            fs::create_dir_all(root.join(&task).join("starter/src")).unwrap();
            fs::write(root.join(&task).join("prompt.md"), "Fix the repository.").unwrap();
            fs::write(
                root.join(&task).join("starter/src/index.mjs"),
                "export const fixed = false;",
            )
            .unwrap();
            let grader = if external_grader {
                serde_json::json!({
                    "type": "external_verifier",
                    "verifier_id": format!("fake-{}-v1", index + 1),
                })
            } else {
                serde_json::json!({"type": "exact_text", "expected": "done"})
            };
            serde_json::json!({
                "id": task,
                "category": "cli_coding",
                "prompt_file": format!("{task}/prompt.md"),
                "starter_dir": format!("{task}/starter"),
                "time_budget_secs": 60,
                "max_turns": 2,
                "grader": grader,
            })
        })
        .collect::<Vec<_>>();
    let manifest = serde_json::json!({
        "schema_version": 1,
        "id": "cli-smoke",
        "version": "1.0.0",
        "title": "CLI Smoke",
        "target_kinds": [target],
        "tasks": tasks,
    });
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

fn environment(pack: &LoadedPack) -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os_family: std::env::consts::OS.into(),
        os_version: "test".into(),
        app_version: "0.2.0".into(),
        cli_version: Some("fake-cli".into()),
        verifier_runtime_version: Some("fake-runtime".into()),
        suite_id: pack.manifest.id.clone(),
        suite_version: pack.manifest.version.clone(),
        suite_content_sha256: pack.content_sha256.clone(),
        scoring_rule_version: "ability-v1".into(),
        resumed: false,
    }
}

fn target(kind: TargetKind) -> TargetSelection {
    TargetSelection {
        kind,
        reported_model: "CLI current selection".into(),
        reasoning_effort: None,
    }
}

struct Fixture {
    _directory: TempDir,
    pack: Arc<LoadedPack>,
    repository: Arc<RunRepository>,
    artifact_root: PathBuf,
    service: CliRunService,
}

impl Fixture {
    fn new(tasks: usize) -> Self {
        let directory = tempdir().unwrap();
        let pack_root = directory.path().join("pack");
        write_pack(&pack_root, tasks, TargetKind::CodexCli, true);
        let pack = Arc::new(PackLoader::load(&pack_root).unwrap());
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let artifact_root = directory.path().join("artifacts");
        let service = CliRunService::new(repository.clone(), artifact_root.clone());
        Self {
            _directory: directory,
            pack,
            repository,
            artifact_root,
            service,
        }
    }

    fn prepare(&self) -> ability_core::RunRecord {
        self.service
            .prepare(
                self.pack.clone(),
                target(TargetKind::CodexCli),
                RunMode::Quick,
                environment(&self.pack),
            )
            .unwrap()
    }
}

fn collect_events(mut receiver: mpsc::UnboundedReceiver<RunEvent>) -> Vec<RunEvent> {
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn copies_starter_runs_agent_verifies_checkpoints_and_emits_ordered_events() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [
            AdapterStep::Complete { duration_ms: 50 },
            AdapterStep::Complete { duration_ms: 60 },
        ],
    ));
    let verifier = Arc::new(FakeVerifier::new([
        VerifierStep::Grade(passed_grade(10)),
        VerifierStep::Grade(passed_grade(20)),
    ]));
    let (sender, receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 2);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 2);
    let stored = fixture.repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Completed);
    assert_eq!(stored.completed_tasks, 2);
    assert_eq!(stored.score.unwrap().ability_score, 100.0);
    let results = fixture.repository.get_task_results(run.id).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].duration_ms, 60);
    assert_eq!(results[1].duration_ms, 80);
    for result in &results {
        let relative = result.answer_rel_path.as_ref().unwrap();
        assert!(!Path::new(relative).is_absolute());
        assert!(!relative.contains(&fixture._directory.path().display().to_string()));
        assert!(fixture.artifact_root.join(relative).is_file());
    }
    assert_eq!(
        fs::read_to_string(
            fixture
                .pack
                .tasks
                .first()
                .unwrap()
                .pack_root
                .join("task-1/starter/src/index.mjs")
        )
        .unwrap(),
        "export const fixed = false;"
    );

    let events = collect_events(receiver);
    assert_eq!(
        events
            .iter()
            .map(|event| (
                event.kind,
                event.task_id.as_deref(),
                event.completed_tasks,
                event.total_tasks
            ))
            .collect::<Vec<_>>(),
        vec![
            (RunEventKind::TaskStarted, Some("task-1"), 0, 2),
            (RunEventKind::TaskFinished, Some("task-1"), 1, 2),
            (RunEventKind::TaskStarted, Some("task-2"), 1, 2),
            (RunEventKind::TaskFinished, Some("task-2"), 2, 2),
            (RunEventKind::RunFinished, None, 2, 2),
        ]
    );
}

#[test]
fn prepare_rejects_environment_mismatches_without_persisting_a_run() {
    let fixture = Fixture::new(1);
    for field in ["id", "version", "hash"] {
        let mut fingerprint = environment(&fixture.pack);
        match field {
            "id" => fingerprint.suite_id = "other-suite".into(),
            "version" => fingerprint.suite_version = "9.9.9".into(),
            "hash" => fingerprint.suite_content_sha256 = "0".repeat(64),
            _ => unreachable!(),
        }
        assert!(matches!(
            fixture.service.prepare(
                fixture.pack.clone(),
                target(TargetKind::CodexCli),
                RunMode::Quick,
                fingerprint,
            ),
            Err(CliRunError::EnvironmentMismatch)
        ));
    }
    assert!(fixture.repository.list_runs().unwrap().is_empty());
}

#[test]
fn prepare_validates_target_tasks_paths_and_artifact_root_before_insert() {
    let fixture = Fixture::new(1);
    assert!(matches!(
        fixture.service.prepare(
            fixture.pack.clone(),
            target(TargetKind::ChatGptClient),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::WrongTarget)
    ));

    let mut unsupported = (*fixture.pack).clone();
    unsupported.tasks[0].definition.grader = ability_core::GraderSpec::ExactText {
        expected: "x".into(),
    };
    assert!(matches!(
        fixture.service.prepare(
            Arc::new(unsupported),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::UnsupportedGrader(_))
    ));

    let mut unsafe_task = (*fixture.pack).clone();
    unsafe_task.tasks[0].definition.id = "../escape".into();
    assert!(matches!(
        fixture.service.prepare(
            Arc::new(unsafe_task),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::UnsafeArtifactPath)
    ));

    let mut invalid_limits = (*fixture.pack).clone();
    invalid_limits.tasks[0].definition.time_budget_secs = 0;
    invalid_limits.tasks[0].definition.max_turns = 0;
    assert!(matches!(
        fixture.service.prepare(
            Arc::new(invalid_limits),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::PackMismatch)
    ));

    fs::remove_dir_all(
        fixture
            .pack
            .tasks
            .first()
            .unwrap()
            .pack_root
            .join("task-1/starter"),
    )
    .unwrap();
    assert!(matches!(
        fixture.service.prepare(
            fixture.pack.clone(),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::MissingStarter(_))
    ));
    assert!(fixture.repository.list_runs().unwrap().is_empty());

    let directory = tempdir().unwrap();
    let repository = Arc::new(RunRepository::open(&directory.path().join("relative.db")).unwrap());
    let relative_service =
        CliRunService::new(repository.clone(), PathBuf::from("relative/artifacts"));
    let pack_root = directory.path().join("pack");
    write_pack(&pack_root, 1, TargetKind::CodexCli, true);
    let pack = Arc::new(PackLoader::load(&pack_root).unwrap());
    assert!(matches!(
        relative_service.prepare(
            pack.clone(),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&pack),
        ),
        Err(CliRunError::UnsafeArtifactPath)
    ));
    assert!(repository.list_runs().unwrap().is_empty());
}

#[test]
fn prepare_rejects_an_artifact_root_that_is_an_existing_file() {
    let fixture = Fixture::new(1);
    let artifact_file = fixture._directory.path().join("artifact-file");
    fs::write(&artifact_file, "owned by another component").unwrap();
    let repository =
        Arc::new(RunRepository::open(&fixture._directory.path().join("file-root.db")).unwrap());
    let service = CliRunService::new(repository.clone(), artifact_file);

    assert!(matches!(
        service.prepare(
            fixture.pack.clone(),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::ArtifactConflict)
    ));
    assert!(repository.list_runs().unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn prepare_rejects_dot_components_in_the_configured_artifact_root() {
    let fixture = Fixture::new(1);
    let raw_root = PathBuf::from(format!(
        r"{}\.\artifacts",
        fixture._directory.path().display()
    ));
    let repository =
        Arc::new(RunRepository::open(&fixture._directory.path().join("dot-root.db")).unwrap());
    let service = CliRunService::new(repository.clone(), raw_root);

    assert!(matches!(
        service.prepare(
            fixture.pack.clone(),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::UnsafeArtifactPath)
    ));
    assert!(repository.list_runs().unwrap().is_empty());
}

#[tokio::test]
async fn execute_rebinds_stored_run_before_workspace_or_adapter_side_effects() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let other_root = fixture._directory.path().join("other-pack");
    write_pack(&other_root, 1, TargetKind::CodexCli, true);
    fs::write(other_root.join("task-1/prompt.md"), "Different content.").unwrap();
    let other_pack = Arc::new(PackLoader::load(&other_root).unwrap());
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 1 }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(passed_grade(1))]));
    let (sender, receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                other_pack,
                adapter.clone(),
                verifier,
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::PackMismatch)
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(
        collect_events(receiver)
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![RunEventKind::RunFinished]
    );

    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::ClaudeCode,
        [AdapterStep::Complete { duration_ms: 1 }],
    ));
    let (sender, _receiver) = mpsc::unbounded_channel();
    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::AdapterMismatch)
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
}

#[tokio::test]
async fn execute_rejects_terminal_or_checkpointed_runs_without_execution() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    fixture
        .repository
        .finish_without_score(run.id, RunStatus::Cancelled)
        .unwrap();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, _receiver) = mpsc::unbounded_channel();
    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::RunNotRunning(RunStatus::Cancelled))
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());

    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    fixture
        .repository
        .save_task_result(&TaskResult {
            run_id: run.id,
            task_id: "task-1".into(),
            category: ability_core::Category::CliCoding,
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            duration_ms: 1,
            answer_rel_path: None,
            detail: "preexisting checkpoint".into(),
        })
        .unwrap();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, receiver) = mpsc::unbounded_channel();
    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::UnexpectedCheckpoint)
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(collect_events(receiver).len(), 1);
}

#[tokio::test]
async fn cancellation_before_first_task_has_no_workspace_or_adapter_call() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            Arc::new(FakeVerifier::new([])),
            cancellation,
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());
    let stored = fixture.repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Cancelled);
    assert!(stored.finished_at.is_some());
    assert_eq!(stored.score, None);
    assert_eq!(
        collect_events(receiver)
            .iter()
            .map(|event| (event.kind, event.completed_tasks))
            .collect::<Vec<_>>(),
        vec![(RunEventKind::RunFinished, 0)]
    );
}

#[tokio::test]
async fn cancellation_during_adapter_checkpoints_cancelled_and_stops_next_task() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let cancellation = CancellationToken::new();
    let adapter = Arc::new(FakeAdapter::cancelling(
        TargetKind::CodexCli,
        cancellation.clone(),
        [AdapterStep::Cancelled],
    ));
    let verifier = Arc::new(FakeVerifier::new([]));
    let (sender, receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            cancellation,
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Cancelled
    );
    let results = fixture.repository.get_task_results(run.id).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, TaskOutcome::Cancelled);
    assert_eq!(
        collect_events(receiver)
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![
            RunEventKind::TaskStarted,
            RunEventKind::TaskFinished,
            RunEventKind::RunFinished
        ]
    );
}

#[tokio::test]
async fn cancellation_during_verifier_checkpoints_cancelled_and_stops_next_task() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let cancellation = CancellationToken::new();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 5 }],
    ));
    let verifier = Arc::new(FakeVerifier::cancelling(
        cancellation.clone(),
        [VerifierStep::Cancelled],
    ));
    let (sender, _receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            cancellation,
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Cancelled
    );
    let results = fixture.repository.get_task_results(run.id).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, TaskOutcome::Cancelled);
    assert_eq!(results[0].duration_ms, 8);
}

#[tokio::test]
async fn cancellation_triggered_inside_verifier_replaces_any_late_scored_grade() {
    for late_grade in [passed_grade(7), failed_grade(9)] {
        let fixture = Fixture::new(2);
        let run = fixture.prepare();
        let cancellation = CancellationToken::new();
        let expected_duration = 5 + late_grade.duration_ms;
        let adapter = Arc::new(FakeAdapter::new(
            TargetKind::CodexCli,
            [AdapterStep::Complete { duration_ms: 5 }],
        ));
        let verifier = Arc::new(FakeVerifier::cancelling(
            cancellation.clone(),
            [VerifierStep::Grade(late_grade)],
        ));
        let (sender, receiver) = mpsc::unbounded_channel();

        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter.clone(),
                verifier.clone(),
                cancellation,
                sender,
            )
            .await
            .unwrap();

        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
        let stored = fixture.repository.get_run(run.id).unwrap().unwrap();
        assert_eq!(stored.status, RunStatus::Cancelled);
        assert_eq!(stored.score, None);
        let results = fixture.repository.get_task_results(run.id).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, TaskOutcome::Cancelled);
        assert_eq!(results[0].score, None);
        assert_eq!(results[0].failure_kind, Some(FailureKind::UserCancelled));
        assert_eq!(results[0].duration_ms, expected_duration);
        assert_eq!(
            collect_events(receiver)
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                RunEventKind::TaskStarted,
                RunEventKind::TaskFinished,
                RunEventKind::RunFinished
            ]
        );
    }
}

#[tokio::test]
async fn budget_exhaustion_is_scored_zero_and_later_tasks_continue() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [
            AdapterStep::BudgetExhausted,
            AdapterStep::Complete { duration_ms: 5 },
        ],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(passed_grade(5))]));
    let (sender, _receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier,
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 2);
    let results = fixture.repository.get_task_results(run.id).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].score, Some(0.0));
    assert_eq!(
        results[0].failure_kind,
        Some(FailureKind::AgentBudgetExceeded)
    );
    let stored = fixture.repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Completed);
    assert_eq!(stored.score.unwrap().ability_score, 50.0);
    assert_eq!(
        adapter_error_grade(AdapterError::AgentBudgetExceeded, 60_000).score,
        Some(0.0)
    );
}

#[tokio::test]
async fn infrastructure_invalid_result_short_circuits_and_interrupts_for_retry() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Infrastructure(FailureKind::Network)],
    ));
    let verifier = Arc::new(FakeVerifier::new([]));
    let (sender, receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 0);
    let stored = fixture.repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Interrupted);
    assert_eq!(stored.completed_tasks, 1);
    assert_eq!(stored.score, None);
    let results = fixture.repository.get_task_results(run.id).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, TaskOutcome::Invalid);
    assert_eq!(results[0].score, None);
    assert_eq!(
        collect_events(receiver)
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![
            RunEventKind::TaskStarted,
            RunEventKind::TaskFinished,
            RunEventKind::RunFinished
        ]
    );
}

#[tokio::test]
async fn verifier_infrastructure_invalid_result_also_stops_later_tasks() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 5 }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(invalid_grade(
        FailureKind::VerifierError,
    ))]));
    let (sender, _receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier,
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(
        fixture.repository.get_task_results(run.id).unwrap()[0].outcome,
        TaskOutcome::Invalid
    );
}

#[tokio::test]
async fn cli_resume_retries_the_invalid_task_before_continuing_the_pack() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let first_adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Infrastructure(FailureKind::AppInterrupted)],
    ));
    let (first_sender, _first_receiver) = mpsc::unbounded_channel();
    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            first_adapter,
            Arc::new(FakeVerifier::new([])),
            CancellationToken::new(),
            first_sender,
        )
        .await
        .unwrap();

    let resumed = fixture
        .service
        .resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        )
        .unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    assert_eq!(resumed.completed_tasks, 0);
    assert!(
        fixture
            .repository
            .get_task_results(run.id)
            .unwrap()
            .is_empty()
    );

    let retry_adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [
            AdapterStep::Complete { duration_ms: 5 },
            AdapterStep::Complete { duration_ms: 6 },
        ],
    ));
    let retry_verifier = Arc::new(FakeVerifier::new([
        VerifierStep::Grade(passed_grade(5)),
        VerifierStep::Grade(passed_grade(6)),
    ]));
    let (retry_sender, retry_receiver) = mpsc::unbounded_channel();
    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            retry_adapter.clone(),
            retry_verifier,
            CancellationToken::new(),
            retry_sender,
        )
        .await
        .unwrap();

    assert_eq!(retry_adapter.calls.load(Ordering::SeqCst), 2);
    let stored = fixture.repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Completed);
    assert_eq!(stored.completed_tasks, 2);
    assert_eq!(
        fixture
            .repository
            .get_task_results(run.id)
            .unwrap()
            .into_iter()
            .map(|result| (result.task_id, result.outcome))
            .collect::<Vec<_>>(),
        vec![
            ("task-1".into(), TaskOutcome::Passed),
            ("task-2".into(), TaskOutcome::Passed),
        ]
    );
    assert_eq!(
        collect_events(retry_receiver)
            .into_iter()
            .map(|event| (event.kind, event.task_id, event.completed_tasks))
            .collect::<Vec<_>>(),
        vec![
            (RunEventKind::TaskStarted, Some("task-1".into()), 0),
            (RunEventKind::TaskFinished, Some("task-1".into()), 1),
            (RunEventKind::TaskStarted, Some("task-2".into()), 1),
            (RunEventKind::TaskFinished, Some("task-2".into()), 2),
            (RunEventKind::RunFinished, None, 2),
        ]
    );
}

#[cfg(windows)]
#[tokio::test]
async fn cli_retry_resume_rolls_back_marker_when_hostile_recovery_artifact_is_rejected() {
    use std::process::Command;

    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Infrastructure(FailureKind::AppInterrupted)],
    ));
    let (sender, _receiver) = mpsc::unbounded_channel();
    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter,
            Arc::new(FakeVerifier::new([])),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();
    let before_run = fixture.repository.get_run(run.id).unwrap().unwrap();
    let before_results = fixture.repository.get_task_results(run.id).unwrap();
    assert_eq!(before_results.len(), 1);
    assert_eq!(before_results[0].outcome, TaskOutcome::Invalid);

    let outside = tempdir().unwrap();
    let sentinel = outside.path().join("sentinel.txt");
    fs::write(&sentinel, "must remain untouched").unwrap();
    let run_root = fixture.artifact_root.join("runs").join(run.id.to_string());
    fs::create_dir_all(&run_root).unwrap();
    let hostile = run_root.join("hostile-reparse");
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            hostile.to_str().unwrap(),
            outside.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    assert!(matches!(
        fixture.service.resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        ),
        Err(CliRunError::NotResumable)
    ));

    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap(),
        before_run
    );
    assert_eq!(
        fixture.repository.get_task_results(run.id).unwrap(),
        before_results
    );
    assert_eq!(
        fs::read_to_string(&sentinel).unwrap(),
        "must remain untouched"
    );
    assert!(hostile.exists());
}

#[tokio::test]
async fn duration_overflow_is_a_coordinator_error_and_interrupts_the_run() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete {
            duration_ms: u64::MAX,
        }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(passed_grade(1))]));
    let (sender, receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter,
                verifier,
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::DurationOverflow)
    ));
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert!(
        fixture
            .repository
            .get_task_results(run.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        collect_events(receiver)
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![RunEventKind::TaskStarted, RunEventKind::RunFinished]
    );
}

#[tokio::test]
async fn checkpoint_storage_failure_interrupts_and_emits_one_final_event() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 1 }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(
        VerificationGrade {
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            detail: String::new(),
            duration_ms: 1,
        },
    )]));
    let (sender, receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter,
                verifier,
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::Storage(StorageError::InvalidData(_)))
    ));
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert!(
        fixture
            .repository
            .get_task_results(run.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        collect_events(receiver)
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        vec![RunEventKind::TaskStarted, RunEventKind::RunFinished]
    );
}

#[tokio::test]
async fn a_secondary_terminalization_failure_keeps_the_original_error_observable() {
    struct DeletingVerifier {
        database: PathBuf,
        run_id: Uuid,
    }

    #[async_trait]
    impl WorkspaceVerifier for DeletingVerifier {
        async fn verify(
            &self,
            _verifier_id: &str,
            _workspace: &Path,
            _cancellation: CancellationToken,
        ) -> VerificationGrade {
            let connection = rusqlite::Connection::open(&self.database).unwrap();
            connection
                .execute("DELETE FROM runs WHERE id=?1", [self.run_id.to_string()])
                .unwrap();
            passed_grade(1)
        }
    }

    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 1 }],
    ));
    let verifier = Arc::new(DeletingVerifier {
        database: fixture._directory.path().join("runs.db"),
        run_id: run.id,
    });
    let (sender, receiver) = mpsc::unbounded_channel();

    let error = fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter,
            verifier,
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CliRunError::TerminalizationFailed {
            original,
            terminalization: StorageError::RunNotFound(id),
        } if id == run.id && matches!(*original, CliRunError::Storage(_))
    ));
    assert!(fixture.repository.get_run(run.id).unwrap().is_none());
    assert!(
        collect_events(receiver)
            .iter()
            .all(|event| event.kind != RunEventKind::RunFinished)
    );
}

#[tokio::test]
async fn existing_workspace_is_never_deleted_or_replaced() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let workspace = fixture
        .artifact_root
        .join("runs")
        .join(run.id.to_string())
        .join("workspaces/task-1");
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("owner.txt"), "pre-existing").unwrap();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, _receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::ArtifactConflict)
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read_to_string(workspace.join("owner.txt")).unwrap(),
        "pre-existing"
    );
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
}

#[cfg(unix)]
#[tokio::test]
async fn source_symlinks_are_rejected_without_copying_outside_content() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let outside = fixture._directory.path().join("outside.txt");
    fs::write(&outside, "outside").unwrap();
    let starter = fixture.pack.tasks[0].pack_root.join("task-1/starter");
    symlink(&outside, starter.join("linked.txt")).unwrap();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, _receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::UnsafeArtifactPath)
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
}

#[cfg(windows)]
#[test]
fn prepare_rejects_an_artifact_root_reached_through_a_junction() {
    use std::process::Command;

    let fixture = Fixture::new(1);
    let outside = tempdir().unwrap();
    let junction = fixture._directory.path().join("artifact-junction");
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            outside.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let repository =
        Arc::new(RunRepository::open(&fixture._directory.path().join("junction.db")).unwrap());
    let service = CliRunService::new(repository.clone(), junction.join("artifacts"));

    assert!(matches!(
        service.prepare(
            fixture.pack.clone(),
            target(TargetKind::CodexCli),
            RunMode::Quick,
            environment(&fixture.pack),
        ),
        Err(CliRunError::UnsafeArtifactPath)
    ));
    assert!(repository.list_runs().unwrap().is_empty());
    assert!(!outside.path().join("artifacts").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn execute_rejects_a_new_destination_ancestor_junction() {
    use std::process::Command;

    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    fs::create_dir_all(&fixture.artifact_root).unwrap();
    let outside = tempdir().unwrap();
    let runs = fixture.artifact_root.join("runs");
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            runs.to_str().unwrap(),
            outside.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, _receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                fixture.pack.clone(),
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::UnsafeArtifactPath)
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!outside.path().join(run.id.to_string()).exists());
}

#[tokio::test]
async fn a_dropped_event_receiver_does_not_change_run_correctness() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 5 }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(passed_grade(5))]));
    let (sender, receiver) = mpsc::unbounded_channel();
    drop(receiver);

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter,
            verifier,
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(
        fixture.repository.get_task_results(run.id).unwrap().len(),
        1
    );
}

#[tokio::test]
async fn unknown_run_is_rejected_without_adapter_or_workspace_side_effects() {
    let fixture = Fixture::new(1);
    let unknown = Uuid::new_v4();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, _receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                unknown,
                fixture.pack.clone(),
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::Storage(StorageError::RunNotFound(id))) if id == unknown
    ));
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());
}

#[tokio::test]
async fn initial_run_decode_failure_interrupts_the_existing_row_and_emits_final_event() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let database = fixture._directory.path().join("runs.db");
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute(
                "UPDATE runs SET environment_json=?2 WHERE id=?1",
                rusqlite::params![run.id.to_string(), "not valid JSON"],
            )
            .unwrap();
    }
    let different_pack_root = fixture._directory.path().join("different-pack");
    write_pack(&different_pack_root, 2, TargetKind::CodexCli, true);
    let different_pack = Arc::new(PackLoader::load(&different_pack_root).unwrap());
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let (sender, receiver) = mpsc::unbounded_channel();

    assert!(matches!(
        fixture
            .service
            .execute(
                run.id,
                different_pack,
                adapter.clone(),
                Arc::new(FakeVerifier::new([])),
                CancellationToken::new(),
                sender,
            )
            .await,
        Err(CliRunError::Storage(StorageError::Database(_)))
    ));

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());
    let connection = rusqlite::Connection::open(database).unwrap();
    let (status, finished_at, score): (String, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT status_json,finished_at,score_json FROM runs WHERE id=?1",
            [run.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        status,
        serde_json::to_string(&RunStatus::Interrupted).unwrap()
    );
    assert!(finished_at.is_some());
    assert!(score.is_none());
    assert_eq!(
        collect_events(receiver)
            .iter()
            .map(|event| (event.kind, event.completed_tasks, event.total_tasks))
            .collect::<Vec<_>>(),
        vec![(RunEventKind::RunFinished, 0, 1)]
    );
}

fn durable_cli_checkpoint(run_id: Uuid, task_id: &str) -> TaskResult {
    TaskResult {
        run_id,
        task_id: task_id.into(),
        category: ability_core::Category::CliCoding,
        outcome: TaskOutcome::Passed,
        score: Some(100.0),
        failure_kind: None,
        duration_ms: 10,
        answer_rel_path: Some(format!("runs/{run_id}/logs/{task_id}.log")),
        detail: "hidden_tests:pass".into(),
    }
}

#[test]
fn cli_resume_rejects_every_target_snapshot_mismatch_before_mutation() {
    let mismatches: [fn(&mut TargetSelection); 3] = [
        |value| value.kind = TargetKind::ClaudeCode,
        |value| value.reported_model = "changed-model".into(),
        |value| value.reasoning_effort = Some("high".into()),
    ];

    for mutate in mismatches {
        let fixture = Fixture::new(1);
        let run = fixture.prepare();
        fixture
            .repository
            .finish_without_score(run.id, RunStatus::Interrupted)
            .unwrap();
        let mut expected_target = run.target.clone();
        mutate(&mut expected_target);

        assert!(matches!(
            fixture.service.resume(
                run.id,
                expected_target,
                &fixture.pack,
                environment(&fixture.pack),
            ),
            Err(CliRunError::NotResumable)
        ));
        assert_eq!(
            fixture.repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );
        assert!(!fixture.artifact_root.exists());
    }
}

#[tokio::test]
async fn cli_resume_removes_uncheckpointed_workspace_and_log_before_fresh_execution() {
    let fixture = Fixture::new(1);
    let run = fixture.prepare();
    let run_root = fixture.artifact_root.join("runs").join(run.id.to_string());
    let workspace = run_root.join("workspaces/task-1");
    let log = run_root.join("logs/task-1.log");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(workspace.join("stale.txt"), "interrupted workspace").unwrap();
    fs::write(&log, "published before checkpoint").unwrap();
    fixture
        .repository
        .finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();

    fixture
        .service
        .resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        )
        .unwrap();

    assert!(!workspace.exists());
    assert!(!log.exists());
    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 5 }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(passed_grade(5))]));
    let (sender, _receiver) = mpsc::unbounded_channel();
    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier,
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Completed
    );
}

#[test]
fn cli_resume_rejects_impossible_checkpoint_artifact_and_score_shapes() {
    for checkpoint in [
        TaskResult {
            run_id: Uuid::nil(),
            task_id: "task-1".into(),
            category: ability_core::Category::CliCoding,
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            duration_ms: 10,
            answer_rel_path: None,
            detail: "impossible missing log".into(),
        },
        TaskResult {
            run_id: Uuid::nil(),
            task_id: "task-1".into(),
            category: ability_core::Category::CliCoding,
            outcome: TaskOutcome::Failed,
            score: Some(0.0),
            failure_kind: Some(FailureKind::AgentBudgetExceeded),
            duration_ms: 10,
            answer_rel_path: Some("placeholder".into()),
            detail: "impossible budget log".into(),
        },
        TaskResult {
            run_id: Uuid::nil(),
            task_id: "task-1".into(),
            category: ability_core::Category::CliCoding,
            outcome: TaskOutcome::Failed,
            score: Some(50.0),
            failure_kind: Some(FailureKind::WrongAnswer),
            duration_ms: 10,
            answer_rel_path: Some("placeholder".into()),
            detail: "impossible partial verifier score".into(),
        },
    ] {
        let fixture = Fixture::new(1);
        let run = fixture.prepare();
        let mut checkpoint = checkpoint;
        checkpoint.run_id = run.id;
        if checkpoint.answer_rel_path.is_some() {
            checkpoint.answer_rel_path = Some(format!("runs/{}/logs/task-1.log", run.id));
        }
        fixture.repository.save_task_result(&checkpoint).unwrap();
        fixture
            .repository
            .finish_without_score(run.id, RunStatus::Interrupted)
            .unwrap();

        assert!(
            fixture
                .service
                .resume(
                    run.id,
                    run.target.clone(),
                    &fixture.pack,
                    environment(&fixture.pack),
                )
                .is_err(),
            "impossible checkpoint shape must fail closed: {checkpoint:?}"
        );
        assert_eq!(
            fixture.repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );
    }
}

#[tokio::test]
async fn resumed_cli_skips_exactly_validated_checkpoints_and_starts_progress_from_durable_count() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    fixture
        .repository
        .save_task_result(&durable_cli_checkpoint(run.id, "task-1"))
        .unwrap();
    fixture
        .repository
        .finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();

    let resumed = fixture
        .service
        .resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        )
        .unwrap();
    assert!(resumed.environment.resumed);
    assert_eq!(resumed.completed_tasks, 1);

    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 5 }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(passed_grade(5))]));
    let (sender, receiver) = mpsc::unbounded_channel();
    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
    assert!(
        !fixture
            .artifact_root
            .join("runs")
            .join(run.id.to_string())
            .join("workspaces/task-1")
            .exists()
    );
    assert!(
        fixture
            .artifact_root
            .join("runs")
            .join(run.id.to_string())
            .join("workspaces/task-2")
            .is_dir()
    );
    assert_eq!(
        collect_events(receiver)
            .into_iter()
            .map(|event| (event.kind, event.task_id, event.completed_tasks))
            .collect::<Vec<_>>(),
        vec![
            (RunEventKind::TaskStarted, Some("task-2".into()), 1),
            (RunEventKind::TaskFinished, Some("task-2".into()), 2),
            (RunEventKind::RunFinished, None, 2),
        ]
    );
}

#[tokio::test]
async fn cli_can_resume_a_second_interruption_without_replaying_its_checkpoint() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    fixture
        .repository
        .save_task_result(&durable_cli_checkpoint(run.id, "task-1"))
        .unwrap();
    fixture
        .repository
        .finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();

    fixture
        .service
        .resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        )
        .unwrap();
    fixture
        .repository
        .finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();
    let resumed_again = fixture
        .service
        .resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        )
        .unwrap();

    assert!(resumed_again.environment.resumed);
    assert_eq!(resumed_again.completed_tasks, 1);
    assert_eq!(
        fixture.repository.get_task_results(run.id).unwrap(),
        vec![durable_cli_checkpoint(run.id, "task-1")]
    );

    let adapter = Arc::new(FakeAdapter::new(
        TargetKind::CodexCli,
        [AdapterStep::Complete { duration_ms: 5 }],
    ));
    let verifier = Arc::new(FakeVerifier::new([VerifierStep::Grade(passed_grade(5))]));
    let (sender, receiver) = mpsc::unbounded_channel();
    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        collect_events(receiver)
            .into_iter()
            .map(|event| (event.kind, event.task_id, event.completed_tasks))
            .collect::<Vec<_>>(),
        vec![
            (RunEventKind::TaskStarted, Some("task-2".into()), 1),
            (RunEventKind::TaskFinished, Some("task-2".into()), 2),
            (RunEventKind::RunFinished, None, 2),
        ]
    );
}

#[tokio::test]
async fn fully_checkpointed_cli_resume_completes_without_adapter_verifier_or_workspace_calls() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    for task_id in ["task-1", "task-2"] {
        fixture
            .repository
            .save_task_result(&durable_cli_checkpoint(run.id, task_id))
            .unwrap();
    }
    fixture
        .repository
        .finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();
    fixture
        .service
        .resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        )
        .unwrap();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let verifier = Arc::new(FakeVerifier::new([]));
    let (sender, receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.artifact_root.exists());
    assert_eq!(
        fixture.repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(
        collect_events(receiver)
            .into_iter()
            .map(|event| (event.kind, event.completed_tasks))
            .collect::<Vec<_>>(),
        vec![(RunEventKind::RunFinished, 2)]
    );
}

#[tokio::test]
async fn cancelled_resumed_cli_reports_durable_progress_without_consuming_remaining_capacity() {
    let fixture = Fixture::new(2);
    let run = fixture.prepare();
    fixture
        .repository
        .save_task_result(&durable_cli_checkpoint(run.id, "task-1"))
        .unwrap();
    fixture
        .repository
        .finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();
    fixture
        .service
        .resume(
            run.id,
            run.target.clone(),
            &fixture.pack,
            environment(&fixture.pack),
        )
        .unwrap();
    let adapter = Arc::new(FakeAdapter::new(TargetKind::CodexCli, []));
    let verifier = Arc::new(FakeVerifier::new([]));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let (sender, receiver) = mpsc::unbounded_channel();

    fixture
        .service
        .execute(
            run.id,
            fixture.pack.clone(),
            adapter.clone(),
            verifier.clone(),
            cancellation,
            sender,
        )
        .await
        .unwrap();

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        collect_events(receiver)
            .into_iter()
            .map(|event| (event.kind, event.completed_tasks))
            .collect::<Vec<_>>(),
        vec![(RunEventKind::RunFinished, 1)]
    );
}
