use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, AvailabilityStatus,
    ClaudeCodeAdapter, ExecutionRequest, LaunchSource, OutputStream, ProcessEnvironment,
    ProcessError, ProcessOutput, ProcessRunner, ProcessSpec,
};
use ability_core::FailureKind;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
            stdout: "{\"type\":\"result\",\"subtype\":\"success\"}\n".into(),
            stderr: String::new(),
            duration_ms: 300,
        })
    }
}

fn request() -> ExecutionRequest {
    ExecutionRequest {
        prompt: "Fix the repository.".into(),
        workspace: PathBuf::from("C:/temp/task"),
        time_budget_secs: 600,
        max_turns: 20,
        model: Some("sonnet".into()),
        reasoning_effort: Some("high".into()),
    }
}

fn test_adapter(runner: Arc<dyn ProcessRunner>) -> ClaudeCodeAdapter {
    ClaudeCodeAdapter::with_resolved_command(runner, "claude", Vec::new())
}

#[derive(Default)]
struct OrderedCandidateRunner {
    seen: Mutex<Vec<ProcessSpec>>,
}

impl OrderedCandidateRunner {
    fn execution_used_reviewed_npm(&self) -> bool {
        self.seen.lock().unwrap().iter().any(|spec| {
            spec.program == PathBuf::from("node.exe")
                && spec.args.first().map(String::as_str)
                    == Some("npm/node_modules/@anthropic-ai/claude-code/cli.js")
                && spec.args.get(1).map(String::as_str) == Some("-p")
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
        if spec.program == PathBuf::from("windows-app/claude.exe") {
            return Err(ProcessError::Spawn(io::Error::from(
                io::ErrorKind::PermissionDenied,
            )));
        }
        let (exit_code, stdout) = match spec.args.last().map(String::as_str) {
            Some("--version") => (Some(0), "2.1.211"),
            Some("status") => (Some(0), r#"{"loggedIn":true}"#),
            _ => (Some(0), "{\"type\":\"result\",\"subtype\":\"success\"}\n"),
        };
        Ok(ProcessOutput {
            exit_code,
            stdout: stdout.into(),
            stderr: String::new(),
            duration_ms: 1,
        })
    }
}

#[tokio::test]
async fn claude_detection_skips_an_inaccessible_candidate_and_retains_npm() {
    let runner = Arc::new(OrderedCandidateRunner::default());
    let adapter = ClaudeCodeAdapter::with_candidate_commands(
        runner.clone(),
        vec![
            (
                PathBuf::from("windows-app/claude.exe"),
                Vec::new(),
                LaunchSource::NativeExe,
            ),
            (
                PathBuf::from("node.exe"),
                vec!["npm/node_modules/@anthropic-ai/claude-code/cli.js".into()],
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
    assert_eq!(availability.version.as_deref(), Some("2.1.211"));
    adapter
        .execute(request(), CancellationToken::new())
        .await
        .unwrap();
    assert!(runner.execution_used_reviewed_npm());
}

#[tokio::test]
async fn claude_candidate_override_starts_unavailable_before_detection() {
    let adapter = ClaudeCodeAdapter::with_candidate_commands(
        Arc::new(OrderedCandidateRunner::default()),
        vec![(
            PathBuf::from("candidate-claude.exe"),
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
            if spec.program == PathBuf::from("first-claude.exe") {
                if self.first_version_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(ProcessOutput {
                        exit_code: Some(0),
                        stdout: "claude-first 1.0".into(),
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
                stdout: "claude-second 2.0".into(),
                stderr: String::new(),
                duration_ms: 1,
            });
        }

        if spec.args.last().map(String::as_str) == Some("status") {
            if spec.program == PathBuf::from("first-claude.exe") {
                self.first_auth_started.add_permits(1);
                self.release_first_auth.acquire().await.unwrap().forget();
            }
            return Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: r#"{"loggedIn":true}"#.into(),
                stderr: String::new(),
                duration_ms: 1,
            });
        }

        *self.execution_program.lock().unwrap() = Some(spec.program);
        Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: "{\"type\":\"result\",\"subtype\":\"success\"}\n".into(),
            stderr: String::new(),
            duration_ms: 1,
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn claude_concurrent_detection_keeps_returned_success_and_execution_coherent() {
    let runner = Arc::new(ConcurrentDetectionRunner::default());
    let adapter = Arc::new(ClaudeCodeAdapter::with_candidate_commands(
        runner.clone(),
        vec![
            (
                PathBuf::from("first-claude.exe"),
                Vec::new(),
                LaunchSource::NativeExe,
            ),
            (
                PathBuf::from("second-node.exe"),
                vec!["reviewed/claude.js".into()],
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
        Some("claude-first 1.0")
    );
    assert_eq!(first_availability.source, Some(LaunchSource::NativeExe));
    assert!(matches!(
        first_execution,
        Ok(AdapterCompletion::Completed { .. })
    ));
    assert_eq!(
        *runner.execution_program.lock().unwrap(),
        Some(PathBuf::from("first-claude.exe"))
    );
    assert_eq!(second_availability.source, Some(LaunchSource::ReviewedNpm));
}

#[tokio::test]
async fn claude_uses_only_constrained_noninteractive_arguments() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter = test_adapter(Arc::new(FakeRunner { seen: seen.clone() }));
    let result = adapter
        .execute(request(), CancellationToken::new())
        .await
        .unwrap();

    assert!(matches!(result, AdapterCompletion::Completed { .. }));
    let specs = seen.lock().unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].program, PathBuf::from("claude"));
    assert_eq!(specs[0].current_dir, PathBuf::from("C:/temp/task"));
    assert_eq!(specs[0].timeout, Duration::from_secs(600));
    assert!(specs[0].env.is_empty());
    assert_eq!(specs[0].environment, ProcessEnvironment::Inherit);
    assert_eq!(
        specs[0].args,
        vec![
            "-p",
            "Fix the repository.",
            "--bare",
            "--no-session-persistence",
            "--output-format",
            "stream-json",
            "--max-turns",
            "20",
            "--tools",
            "Read,Edit,Write",
            "--allowedTools",
            "Read",
            "Edit",
            "Write",
            "--permission-mode",
            "dontAsk",
            "--model",
            "sonnet",
            "--effort",
            "high",
        ]
    );
    assert!(
        !specs[0]
            .args
            .contains(&"--dangerously-skip-permissions".into())
    );
    assert!(!specs[0].args.contains(&"bypassPermissions".into()));
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
            (
                Some(0),
                r#"{"loggedIn":true,"email":"private@example.com"}"#,
            )
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
async fn claude_detection_parses_auth_status_to_a_public_readiness_decision() {
    let availability = test_adapter(Arc::new(ReadyClaudeDetectionRunner))
        .detect()
        .await;
    assert!(availability.installed);
    assert_eq!(availability.version.as_deref(), Some("2.1.211"));
    assert_eq!(availability.auth_state, AuthState::Ready);
}

#[tokio::test]
async fn claude_failed_redetection_clears_the_retained_launch() {
    let runner = Arc::new(StaticRunner {
        result: Mutex::new(VecDeque::from([
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "claude 1.0".into(),
                stderr: String::new(),
                duration_ms: 1,
            }),
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: r#"{"loggedIn":true}"#.into(),
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
    let adapter = ClaudeCodeAdapter::with_candidate_commands(
        runner,
        vec![(
            PathBuf::from("candidate-claude.exe"),
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
async fn claude_successful_version_with_unknown_auth_is_installed_and_ready() {
    let runner = Arc::new(StaticRunner {
        result: Mutex::new(VecDeque::from([
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "claude 1.0".into(),
                stderr: String::new(),
                duration_ms: 1,
            }),
            Err(ProcessError::TimedOut),
        ])),
    });
    let adapter = ClaudeCodeAdapter::with_candidate_commands(
        runner,
        vec![(
            PathBuf::from("candidate-claude.exe"),
            Vec::new(),
            LaunchSource::NativeExe,
        )],
        false,
    );

    let availability = adapter.detect().await;

    assert!(availability.installed);
    assert_eq!(availability.version.as_deref(), Some("claude 1.0"));
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
async fn claude_accepts_only_one_final_success_result_event() {
    let result = test_adapter(StaticRunner::output(
        Some(0),
        "{\"type\":\"assistant\"}\n{\"type\":\"result\",\"subtype\":\"success\"}\n\n \t",
        "",
    ))
    .execute(request(), CancellationToken::new())
    .await;
    assert!(matches!(result, Ok(AdapterCompletion::Completed { .. })));

    for stdout in [
        "not-json\n",
        "[]\n{\"type\":\"result\",\"subtype\":\"success\"}\n",
        "{\"not\":\"an event\"}\n{\"type\":\"result\",\"subtype\":\"success\"}\n",
        "{\"type\":\"assistant\"}\n",
        "{\"type\":\"result\",\"subtype\":\"error\"}\n",
        "{\"type\":\"result\",\"subtype\":\"success\"}\n{\"type\":\"assistant\"}\n",
        "{\"type\":\"result\",\"subtype\":\"success\"}\n{\"type\":\"result\",\"subtype\":\"success\"}\n",
    ] {
        let error = execute_output(Some(0), stdout, "").await;
        assert!(
            matches!(error, AdapterError::Infrastructure { .. }),
            "{stdout}"
        );
    }
}

#[tokio::test]
async fn claude_detects_structured_max_turns_before_text_fallback() {
    let error = execute_output(
        Some(1),
        r#"{"type":"result","subtype":"error_max_turns"}"#,
        "unrelated failure",
    )
    .await;
    assert!(matches!(error, AdapterError::AgentBudgetExceeded));

    let error = execute_output(Some(1), "", "maximum number of turns reached").await;
    assert!(matches!(error, AdapterError::AgentBudgetExceeded));
}

#[tokio::test]
async fn claude_never_uses_budget_text_for_an_invalid_stream() {
    for stdout in [
        "{\"type\":\"result\",\"subtype\":\"success\"}\n{\"type\":\"result\",\"subtype\":\"error_max_turns\"}\n",
        "{\"type\":\"result\",\"subtype\":\"error_max_turns\"}\n{\"type\":\"result\",\"subtype\":\"error_max_turns\"}\n",
        "error_max_turns\n",
    ] {
        let error = execute_output(Some(1), stdout, "error_max_turns").await;
        assert!(
            matches!(error, AdapterError::Infrastructure { .. }),
            "{stdout}"
        );
    }
}

#[tokio::test]
async fn claude_rejects_nonzero_exit_even_with_success_result() {
    let error = execute_output(
        Some(1),
        r#"{"type":"result","subtype":"success"}"#,
        "failed",
    )
    .await;
    assert!(matches!(error, AdapterError::Infrastructure { .. }));
}

#[tokio::test]
async fn claude_maps_every_process_error_truthfully() {
    let cases = [
        (ProcessError::TimedOut, "agent_budget"),
        (ProcessError::Cancelled, "cancelled"),
        (ProcessError::CaptureFailed, "infrastructure"),
        (
            ProcessError::OutputLimit {
                stream: OutputStream::Stderr,
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

    let missing = test_adapter(StaticRunner::error(ProcessError::Spawn(io::Error::from(
        io::ErrorKind::NotFound,
    ))))
    .execute(request(), CancellationToken::new())
    .await
    .unwrap_err();
    assert!(matches!(missing, AdapterError::Unavailable));
}
