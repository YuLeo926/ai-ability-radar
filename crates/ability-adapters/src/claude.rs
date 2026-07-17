use crate::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, ExecutionRequest, ProcessError,
    ProcessOutput, ProcessRunner, ProcessSpec, TargetAvailability, classify_cli_failure,
    is_agent_budget_exhaustion,
};
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct ClaudeCodeAdapter {
    runner: Arc<dyn ProcessRunner>,
}

impl ClaudeCodeAdapter {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }
}

#[async_trait]
impl AgentAdapter for ClaudeCodeAdapter {
    fn kind(&self) -> TargetKind {
        TargetKind::ClaudeCode
    }

    async fn detect(&self) -> TargetAvailability {
        let version = match self
            .runner
            .run(
                detection_spec(vec!["--version".into()]),
                CancellationToken::new(),
            )
            .await
        {
            Ok(output) if output.exit_code == Some(0) => Some(output.stdout.trim().to_owned()),
            _ => return unavailable(self.kind()),
        };

        let auth_state = match self
            .runner
            .run(
                detection_spec(vec!["auth".into(), "status".into()]),
                CancellationToken::new(),
            )
            .await
        {
            Ok(output) => auth_state(output.exit_code, &output.stdout),
            Err(_) => AuthState::Unknown,
        };

        TargetAvailability {
            kind: self.kind(),
            installed: true,
            version,
            auth_state,
            prerequisites: Vec::new(),
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        match self.runner.run(execution_spec(request), cancellation).await {
            Ok(output) => complete_or_classify(output),
            Err(ProcessError::TimedOut) => Err(AdapterError::AgentBudgetExceeded),
            Err(ProcessError::Cancelled) => Err(AdapterError::Cancelled),
            Err(ProcessError::Spawn(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(AdapterError::Unavailable)
            }
            Err(ProcessError::Spawn(error)) => infrastructure(error.to_string()),
            Err(ProcessError::Supervision(error)) => infrastructure(error.to_string()),
            Err(ProcessError::Wait(error)) => infrastructure(error.to_string()),
            Err(ProcessError::CaptureFailed) => {
                infrastructure("process output capture failed".into())
            }
            Err(ProcessError::OutputLimit { stream }) => {
                infrastructure(format!("process {stream:?} exceeded the capture limit"))
            }
            Err(ProcessError::TerminationFailed) => {
                infrastructure("process tree cleanup could not be confirmed".into())
            }
            Err(ProcessError::DurationOverflow) => {
                infrastructure("process duration exceeds the supported range".into())
            }
        }
    }
}

fn detection_spec(args: Vec<String>) -> ProcessSpec {
    ProcessSpec {
        program: "claude".into(),
        args,
        current_dir: std::env::temp_dir(),
        env: BTreeMap::new(),
        timeout: Duration::from_secs(10),
    }
}

fn unavailable(kind: TargetKind) -> TargetAvailability {
    TargetAvailability {
        kind,
        installed: false,
        version: None,
        auth_state: AuthState::Unknown,
        prerequisites: Vec::new(),
    }
}

fn auth_state(exit_code: Option<i32>, stdout: &str) -> AuthState {
    let Ok(status) = serde_json::from_str::<Value>(stdout) else {
        return AuthState::Unknown;
    };
    match (exit_code, status.get("loggedIn").and_then(Value::as_bool)) {
        (Some(0), Some(true)) => AuthState::Ready,
        (Some(1), Some(false)) => AuthState::NeedsLogin,
        _ => AuthState::Unknown,
    }
}

fn execution_spec(request: ExecutionRequest) -> ProcessSpec {
    let mut args = vec![
        "-p".into(),
        request.prompt,
        "--bare".into(),
        "--no-session-persistence".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--max-turns".into(),
        request.max_turns.to_string(),
        "--tools".into(),
        "Read,Edit,Write".into(),
        "--allowedTools".into(),
        "Read".into(),
        "Edit".into(),
        "Write".into(),
        "--permission-mode".into(),
        "dontAsk".into(),
    ];
    if let Some(model) = request.model {
        args.extend(["--model".into(), model]);
    }
    if let Some(effort) = request.reasoning_effort {
        args.extend(["--effort".into(), effort]);
    }
    ProcessSpec {
        program: "claude".into(),
        args,
        current_dir: request.workspace,
        env: BTreeMap::new(),
        timeout: Duration::from_secs(request.time_budget_secs),
    }
}

fn complete_or_classify(output: ProcessOutput) -> Result<AdapterCompletion, AdapterError> {
    match parse_stream(&output.stdout) {
        StreamState::MaxTurns => Err(AdapterError::AgentBudgetExceeded),
        StreamState::Success if output.exit_code == Some(0) => Ok(AdapterCompletion::Completed {
            duration_ms: output.duration_ms,
            stdout: output.stdout,
            stderr: output.stderr,
        }),
        StreamState::NonTerminal => failed_output(output),
        StreamState::Success | StreamState::Invalid => infrastructure_output(output),
    }
}

fn failed_output(output: ProcessOutput) -> Result<AdapterCompletion, AdapterError> {
    let detail = format!("{}\n{}", output.stderr, output.stdout);
    if is_agent_budget_exhaustion(&detail) {
        Err(AdapterError::AgentBudgetExceeded)
    } else {
        infrastructure_with_kind(classify_cli_failure(&detail), detail)
    }
}

fn infrastructure_output(output: ProcessOutput) -> Result<AdapterCompletion, AdapterError> {
    let detail = format!("{}\n{}", output.stderr, output.stdout);
    infrastructure_with_kind(classify_cli_failure(&detail), detail)
}

fn infrastructure(detail: String) -> Result<AdapterCompletion, AdapterError> {
    infrastructure_with_kind(FailureKind::AppInterrupted, detail)
}

fn infrastructure_with_kind(
    kind: FailureKind,
    detail: String,
) -> Result<AdapterCompletion, AdapterError> {
    Err(AdapterError::Infrastructure { kind, detail })
}

enum StreamState {
    Success,
    MaxTurns,
    NonTerminal,
    Invalid,
}

fn parse_stream(stdout: &str) -> StreamState {
    enum Terminal {
        Success,
        MaxTurns,
        Other,
    }

    let mut terminal = None;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if terminal.is_some() {
            return StreamState::Invalid;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return StreamState::Invalid;
        };
        let Some(event_type) = event.get("type").and_then(Value::as_str) else {
            return StreamState::Invalid;
        };
        if event_type != "result" {
            continue;
        }
        terminal = Some(match event.get("subtype").and_then(Value::as_str) {
            Some("success") => Terminal::Success,
            Some("error_max_turns") => Terminal::MaxTurns,
            _ => Terminal::Other,
        });
    }

    match terminal {
        Some(Terminal::Success) => StreamState::Success,
        Some(Terminal::MaxTurns) => StreamState::MaxTurns,
        Some(Terminal::Other) => StreamState::Invalid,
        None => StreamState::NonTerminal,
    }
}
