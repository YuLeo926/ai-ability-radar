use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, AvailabilityStatus, CodexAdapter,
    ExecutionRequest, LaunchSource, OutputStream, ProcessEnvironment, ProcessError, ProcessOutput,
    ProcessRunner, ProcessSpec,
};
use ability_core::FailureKind;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
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

fn request() -> ExecutionRequest {
    ExecutionRequest {
        prompt: "Fix the repository and run its visible tests.".into(),
        workspace: PathBuf::from("C:/temp/task"),
        time_budget_secs: 600,
        max_turns: 20,
        model: Some("gpt-test".into()),
        reasoning_effort: Some("high".into()),
    }
}

fn test_adapter(runner: Arc<dyn ProcessRunner>) -> CodexAdapter {
    CodexAdapter::with_resolved_command(runner, "codex", Vec::new())
}

#[derive(Default)]
struct OrderedCandidateRunner {
    seen: Mutex<Vec<ProcessSpec>>,
}

impl OrderedCandidateRunner {
    fn execution_used_reviewed_npm(&self) -> bool {
        self.seen.lock().unwrap().iter().any(|spec| {
            spec.program == Path::new("node.exe")
                && spec.args.first().map(String::as_str)
                    == Some("npm/node_modules/@openai/codex/bin/codex.js")
                && spec.args.get(1).map(String::as_str) == Some("exec")
        })
    }
}

#[async_trait]
impl ProcessRunner for OrderedCandidateRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.seen.lock().unwrap().push(spec.clone());
        if spec.program == Path::new("windows-app/codex.exe") {
            return Err(ProcessError::Spawn(io::Error::from(
                io::ErrorKind::PermissionDenied,
            )));
        }
        let stdout = match spec.args.last().map(String::as_str) {
            Some("--version") => "codex-cli 0.142.5",
            Some("status") => "Logged in",
            _ => {
                return Ok(ProcessOutput {
                    exit_code: Some(0),
                    stdout: concat!(
                        "{\"type\":\"thread.started\",\"thread_id\":\"abc\"}\n",
                        "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":4}}\n"
                    )
                    .into(),
                    stderr: String::new(),
                    duration_ms: 1,
                });
            }
        };
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            duration_ms: 1,
        })
    }
}

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
    assert_eq!(availability.auth_state, AuthState::Ready);
    assert_eq!(availability.source, Some(LaunchSource::ReviewedNpm));
    assert_eq!(availability.version.as_deref(), Some("codex-cli 0.142.5"));
    adapter
        .execute(request(), CancellationToken::new())
        .await
        .unwrap();
    assert!(runner.execution_used_reviewed_npm());
}

#[tokio::test]
async fn codex_candidate_override_starts_unavailable_before_detection() {
    let adapter = CodexAdapter::with_candidate_commands(
        Arc::new(OrderedCandidateRunner::default()),
        vec![(
            PathBuf::from("candidate-codex.exe"),
            Vec::new(),
            LaunchSource::NativeExe,
        )],
        false,
    );

    let error = adapter
        .execute(request(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, AdapterError::Unavailable));
}

struct ConcurrentDetectionRunner {
    first_version_calls: AtomicUsize,
    first_auth_started: Semaphore,
    release_first_auth: Semaphore,
    execution_program: Mutex<Option<PathBuf>>,
}

impl Default for ConcurrentDetectionRunner {
    fn default() -> Self {
        Self {
            first_version_calls: AtomicUsize::new(0),
            first_auth_started: Semaphore::new(0),
            release_first_auth: Semaphore::new(0),
            execution_program: Mutex::new(None),
        }
    }
}

#[async_trait]
impl ProcessRunner for ConcurrentDetectionRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        if spec.args.last().map(String::as_str) == Some("--version") {
            if spec.program == Path::new("first-codex.exe") {
                if self.first_version_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(ProcessOutput {
                        exit_code: Some(0),
                        stdout: "codex-first 1.0".into(),
                        stderr: String::new(),
                        duration_ms: 1,
                    });
                }
                return Err(ProcessError::Spawn(io::Error::from(
                    io::ErrorKind::PermissionDenied,
                )));
            }
            return Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "codex-second 2.0".into(),
                stderr: String::new(),
                duration_ms: 1,
            });
        }

        if spec.args.last().map(String::as_str) == Some("status") {
            if spec.program == Path::new("first-codex.exe") {
                self.first_auth_started.add_permits(1);
                self.release_first_auth.acquire().await.unwrap().forget();
            }
            return Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "Logged in".into(),
                stderr: String::new(),
                duration_ms: 1,
            });
        }

        *self.execution_program.lock().unwrap() = Some(spec.program);
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"abc\"}\n",
                "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":4}}\n"
            )
            .into(),
            stderr: String::new(),
            duration_ms: 1,
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn codex_concurrent_detection_keeps_returned_success_and_execution_coherent() {
    let runner = Arc::new(ConcurrentDetectionRunner::default());
    let adapter = Arc::new(CodexAdapter::with_candidate_commands(
        runner.clone(),
        vec![
            (
                PathBuf::from("first-codex.exe"),
                Vec::new(),
                LaunchSource::NativeExe,
            ),
            (
                PathBuf::from("second-node.exe"),
                vec!["reviewed/codex.js".into()],
                LaunchSource::ReviewedNpm,
            ),
        ],
        false,
    ));

    let first_adapter = Arc::clone(&adapter);
    let first = tokio::spawn(async move {
        let availability = first_adapter.detect().await;
        let execution = first_adapter
            .execute(request(), CancellationToken::new())
            .await;
        (availability, execution)
    });
    runner.first_auth_started.acquire().await.unwrap().forget();

    let second_adapter = Arc::clone(&adapter);
    let second = tokio::spawn(async move { second_adapter.detect().await });
    tokio::task::yield_now().await;
    runner.release_first_auth.add_permits(1);

    let (first_availability, first_execution) = first.await.unwrap();
    let second_availability = second.await.unwrap();

    assert_eq!(
        first_availability.version.as_deref(),
        Some("codex-first 1.0")
    );
    assert_eq!(first_availability.source, Some(LaunchSource::NativeExe));
    assert!(matches!(
        first_execution,
        Ok(AdapterCompletion::Completed { .. })
    ));
    assert_eq!(
        *runner.execution_program.lock().unwrap(),
        Some(PathBuf::from("first-codex.exe"))
    );
    assert_eq!(second_availability.source, Some(LaunchSource::ReviewedNpm));
}

#[tokio::test]
async fn codex_uses_ephemeral_json_workspace_write() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter = test_adapter(Arc::new(FakeRunner { seen: seen.clone() }));
    let result = adapter
        .execute(request(), CancellationToken::new())
        .await
        .unwrap();

    assert!(matches!(result, AdapterCompletion::Completed { .. }));
    let specs = seen.lock().unwrap();
    assert_eq!(specs[0].program, PathBuf::from("codex"));
    assert_eq!(specs[0].environment, ProcessEnvironment::Inherit);
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

#[tokio::test]
async fn codex_preserves_shell_metacharacters_in_separate_arguments() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter = test_adapter(Arc::new(FakeRunner { seen: seen.clone() }));
    let mut request = request();
    request.model = Some("model & name".into());
    request.reasoning_effort = Some("high; $(unsafe)".into());
    request.prompt = "write & keep $HOME; spaces".into();
    adapter
        .execute(request, CancellationToken::new())
        .await
        .unwrap();

    let specs = seen.lock().unwrap();
    assert_eq!(specs[0].args[8], "model & name");
    assert_eq!(
        specs[0].args[10],
        "model_reasoning_effort=\"high; $(unsafe)\""
    );
    assert_eq!(specs[0].args[11], "write & keep $HOME; spaces");
}

#[tokio::test]
async fn codex_toml_encodes_reasoning_effort_as_one_config_argument() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter = test_adapter(Arc::new(FakeRunner { seen: seen.clone() }));
    let mut request = request();
    request.reasoning_effort = Some("high\"\\\n--config injected".into());
    adapter
        .execute(request, CancellationToken::new())
        .await
        .unwrap();

    let specs = seen.lock().unwrap();
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
            "model_reasoning_effort=\"high\\\"\\\\\\n--config injected\"",
            "Fix the repository and run its visible tests.",
        ]
    );
    assert_eq!(
        specs[0]
            .args
            .iter()
            .filter(|argument| argument.as_str() == "--config")
            .count(),
        1
    );
    assert_eq!(
        serde_json::from_str::<String>(&specs[0].args[10]["model_reasoning_effort=".len()..])
            .unwrap(),
        "high\"\\\n--config injected"
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
    let availability = test_adapter(Arc::new(ReadyDetectionRunner)).detect().await;
    assert!(availability.installed);
    assert_eq!(availability.version.as_deref(), Some("codex-cli 0.134.0"));
    assert_eq!(availability.auth_state, AuthState::Ready);
}

#[tokio::test]
async fn codex_failed_redetection_clears_the_retained_launch() {
    let runner = Arc::new(StaticRunner {
        result: Mutex::new(VecDeque::from([
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "codex-cli 1.0".into(),
                stderr: String::new(),
                duration_ms: 1,
            }),
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "Logged in".into(),
                stderr: String::new(),
                duration_ms: 1,
            }),
            Ok(ProcessOutput {
                exit_code: Some(1),
                stdout: "version probe failed".into(),
                stderr: String::new(),
                duration_ms: 1,
            }),
        ])),
    });
    let adapter = CodexAdapter::with_candidate_commands(
        runner,
        vec![(
            PathBuf::from("candidate-codex.exe"),
            Vec::new(),
            LaunchSource::NativeExe,
        )],
        false,
    );

    assert!(adapter.detect().await.installed);
    let failed = adapter.detect().await;
    let execute_error = adapter
        .execute(request(), CancellationToken::new())
        .await
        .unwrap_err();

    assert!(!failed.installed);
    assert_eq!(failed.status, AvailabilityStatus::VersionProbeFailed);
    assert!(matches!(execute_error, AdapterError::Unavailable));
}

#[tokio::test]
async fn codex_successful_version_with_unknown_auth_is_installed_and_ready() {
    let runner = Arc::new(StaticRunner {
        result: Mutex::new(VecDeque::from([
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "codex-cli 1.0".into(),
                stderr: String::new(),
                duration_ms: 1,
            }),
            Err(ProcessError::TimedOut),
        ])),
    });
    let adapter = CodexAdapter::with_candidate_commands(
        runner,
        vec![(
            PathBuf::from("candidate-codex.exe"),
            Vec::new(),
            LaunchSource::NativeExe,
        )],
        false,
    );

    let availability = adapter.detect().await;

    assert!(availability.installed);
    assert_eq!(availability.version.as_deref(), Some("codex-cli 1.0"));
    assert_eq!(availability.auth_state, AuthState::Unknown);
    assert_eq!(availability.status, AvailabilityStatus::Ready);
    assert_eq!(availability.source, Some(LaunchSource::NativeExe));
}

struct StaticRunner {
    result: Mutex<VecDeque<Result<ProcessOutput, ProcessError>>>,
}

impl StaticRunner {
    fn output(exit_code: Option<i32>, stdout: &str, stderr: &str) -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(VecDeque::from([Ok(ProcessOutput {
                exit_code,
                stdout: stdout.into(),
                stderr: stderr.into(),
                duration_ms: 1,
            })])),
        })
    }

    fn error(error: ProcessError) -> Arc<Self> {
        Arc::new(Self {
            result: Mutex::new(VecDeque::from([Err(error)])),
        })
    }
}

#[async_trait]
impl ProcessRunner for StaticRunner {
    async fn run(
        &self,
        _spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.result.lock().unwrap().pop_front().unwrap()
    }
}

async fn execute_output(exit_code: Option<i32>, stdout: &str, stderr: &str) -> AdapterError {
    test_adapter(StaticRunner::output(exit_code, stdout, stderr))
        .execute(request(), CancellationToken::new())
        .await
        .unwrap_err()
}

#[tokio::test]
async fn codex_rejects_malformed_nonblank_jsonl() {
    let error = execute_output(Some(0), "{\"type\":\"thread.started\"}\nnot-json\n", "").await;
    assert!(matches!(error, AdapterError::Infrastructure { .. }));
}

#[tokio::test]
async fn codex_rejects_a_nonterminal_completed_event() {
    let error = execute_output(
        Some(0),
        "{\"type\":\"turn.completed\"}\n{\"type\":\"thread.started\"}\n",
        "",
    )
    .await;
    assert!(matches!(error, AdapterError::Infrastructure { .. }));
}

#[tokio::test]
async fn codex_rejects_any_event_after_a_terminal_event() {
    for stdout in [
        "{\"type\":\"turn.completed\"}\n{\"type\":\"turn.completed\"}\n",
        "{\"type\":\"turn.completed\"}\n{\"type\":\"thread.started\"}\n{\"type\":\"turn.completed\"}\n",
    ] {
        let error = execute_output(Some(0), stdout, "").await;
        assert!(matches!(error, AdapterError::Infrastructure { .. }));
    }
}

#[tokio::test]
async fn codex_allows_trailing_blank_lines_after_completion() {
    let result = test_adapter(StaticRunner::output(
        Some(0),
        "{\"type\":\"thread.started\"}\n{\"type\":\"turn.completed\"}\n\n  \n\t\n",
        "",
    ))
    .execute(request(), CancellationToken::new())
    .await;
    assert!(matches!(result, Ok(AdapterCompletion::Completed { .. })));
}

#[tokio::test]
async fn codex_rejects_failed_and_error_events() {
    for event in ["turn.failed", "error"] {
        let stdout = format!("{{\"type\":\"{event}\"}}\n{{\"type\":\"turn.completed\"}}\n");
        let error = execute_output(Some(0), &stdout, "").await;
        assert!(matches!(error, AdapterError::Infrastructure { .. }));
    }
}

#[tokio::test]
async fn codex_rejects_missing_completion_or_nonzero_exit() {
    let missing = execute_output(Some(0), "{\"type\":\"thread.started\"}\n", "").await;
    assert!(matches!(missing, AdapterError::Infrastructure { .. }));

    let failed = execute_output(Some(1), "{\"type\":\"turn.completed\"}\n", "failed").await;
    assert!(matches!(failed, AdapterError::Infrastructure { .. }));
}

#[tokio::test]
async fn codex_checks_agent_budget_markers_before_infrastructure_classification() {
    let error = execute_output(Some(1), "", "maximum number of turns reached").await;
    assert!(matches!(error, AdapterError::AgentBudgetExceeded));
}

#[tokio::test]
async fn codex_maps_runner_errors_truthfully() {
    let cases = [
        (ProcessError::TimedOut, "agent_budget"),
        (ProcessError::Cancelled, "cancelled"),
        (ProcessError::CaptureFailed, "infrastructure"),
        (
            ProcessError::OutputLimit {
                stream: OutputStream::Stdout,
            },
            "infrastructure",
        ),
        (ProcessError::TerminationFailed, "infrastructure"),
        (ProcessError::DurationOverflow, "infrastructure"),
        (
            ProcessError::Spawn(io::Error::from(io::ErrorKind::PermissionDenied)),
            "infrastructure",
        ),
        (
            ProcessError::Supervision(io::Error::from(io::ErrorKind::Other)),
            "infrastructure",
        ),
        (
            ProcessError::Wait(io::Error::from(io::ErrorKind::Other)),
            "infrastructure",
        ),
    ];

    for (runner_error, expected) in cases {
        let error = test_adapter(StaticRunner::error(runner_error))
            .execute(request(), CancellationToken::new())
            .await
            .unwrap_err();
        match expected {
            "agent_budget" => assert!(matches!(error, AdapterError::AgentBudgetExceeded)),
            "cancelled" => assert!(matches!(error, AdapterError::Cancelled)),
            "infrastructure" => assert!(matches!(
                error,
                AdapterError::Infrastructure {
                    kind: FailureKind::AppInterrupted,
                    ..
                }
            )),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn codex_maps_missing_executable_to_unavailable() {
    let error = test_adapter(StaticRunner::error(ProcessError::Spawn(io::Error::from(
        io::ErrorKind::NotFound,
    ))))
    .execute(request(), CancellationToken::new())
    .await
    .unwrap_err();
    assert!(matches!(error, AdapterError::Unavailable));
}
