use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, CodexAdapter, ExecutionRequest,
    OutputStream, ProcessEnvironment, ProcessError, ProcessOutput, ProcessRunner, ProcessSpec,
};
use ability_core::FailureKind;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
