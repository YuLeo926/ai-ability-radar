use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, ClaudeCodeAdapter, ExecutionRequest,
    OutputStream, ProcessError, ProcessOutput, ProcessRunner, ProcessSpec,
};
use ability_core::FailureKind;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
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

#[tokio::test]
async fn claude_uses_only_constrained_noninteractive_arguments() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let adapter = ClaudeCodeAdapter::new(Arc::new(FakeRunner { seen: seen.clone() }));
    let result = adapter
        .execute(request(), CancellationToken::new())
        .await
        .unwrap();

    assert!(matches!(result, AdapterCompletion::Completed { .. }));
    let specs = seen.lock().unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].program, "claude");
    assert_eq!(specs[0].current_dir, PathBuf::from("C:/temp/task"));
    assert_eq!(specs[0].timeout, Duration::from_secs(600));
    assert!(specs[0].env.is_empty());
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
    let availability = ClaudeCodeAdapter::new(Arc::new(ReadyClaudeDetectionRunner))
        .detect()
        .await;
    assert!(availability.installed);
    assert_eq!(availability.version.as_deref(), Some("2.1.211"));
    assert_eq!(availability.auth_state, AuthState::Ready);
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
    ClaudeCodeAdapter::new(StaticRunner::output(exit_code, stdout, stderr))
        .execute(request(), CancellationToken::new())
        .await
        .unwrap_err()
}

#[tokio::test]
async fn claude_accepts_only_one_final_success_result_event() {
    let result = ClaudeCodeAdapter::new(StaticRunner::output(
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
        let error = ClaudeCodeAdapter::new(StaticRunner::error(runner_error))
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

    let missing = ClaudeCodeAdapter::new(StaticRunner::error(ProcessError::Spawn(
        io::Error::from(io::ErrorKind::NotFound),
    )))
    .execute(request(), CancellationToken::new())
    .await
    .unwrap_err();
    assert!(matches!(missing, AdapterError::Unavailable));
}
