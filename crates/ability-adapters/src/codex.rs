use crate::command_locator::{LaunchCommand, resolve_launch_command};
use crate::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, AvailabilityStatus, ExecutionRequest,
    LaunchSource, ProcessEnvironment, ProcessError, ProcessRunner, ProcessSpec, TargetAvailability,
    classify_cli_failure, is_agent_budget_exhaustion,
};
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct CodexAdapter {
    runner: Arc<dyn ProcessRunner>,
    launch: Mutex<Option<LaunchCommand>>,
}

impl CodexAdapter {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self {
            runner,
            launch: Mutex::new(None),
        }
    }

    pub fn with_resolved_command(
        runner: Arc<dyn ProcessRunner>,
        program: impl Into<std::path::PathBuf>,
        prefix_args: Vec<String>,
    ) -> Self {
        Self {
            runner,
            launch: Mutex::new(Some(LaunchCommand {
                program: program.into(),
                prefix_args,
            })),
        }
    }

    fn retained_launch(&self) -> std::io::Result<LaunchCommand> {
        let mut retained = self
            .launch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(launch) = retained.as_ref() {
            return Ok(launch.clone());
        }
        let path = std::env::var_os("PATH");
        let launch = resolve_launch_command(Path::new("codex"), path.as_deref())?;
        *retained = Some(launch.clone());
        Ok(launch)
    }
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    fn kind(&self) -> TargetKind {
        TargetKind::CodexCli
    }

    async fn detect(&self) -> TargetAvailability {
        let launch = match self.retained_launch() {
            Ok(launch) => launch,
            Err(_) => return unavailable(self.kind()),
        };
        let version = match self
            .runner
            .run(
                detection_spec(&launch, vec!["--version".into()]),
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
                detection_spec(&launch, vec!["login".into(), "status".into()]),
                CancellationToken::new(),
            )
            .await
        {
            Ok(output) if output.stdout.to_lowercase().contains("not logged in") => {
                AuthState::NeedsLogin
            }
            Ok(output)
                if output.exit_code == Some(0)
                    && output.stdout.to_lowercase().contains("logged in") =>
            {
                AuthState::Ready
            }
            _ => AuthState::Unknown,
        };

        TargetAvailability {
            kind: self.kind(),
            installed: true,
            version,
            auth_state,
            status: match auth_state {
                AuthState::NeedsLogin => AvailabilityStatus::NeedsLogin,
                AuthState::Unknown | AuthState::Ready => AvailabilityStatus::Ready,
            },
            source: Some(launch_source(&launch)),
            prerequisites: Vec::new(),
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        let launch = self
            .retained_launch()
            .map_err(|_| AdapterError::Unavailable)?;
        let spec = execution_spec(&launch, request);
        match self.runner.run(spec, cancellation).await {
            Ok(output) if output.exit_code == Some(0) && has_completed_turn(&output.stdout) => {
                Ok(AdapterCompletion::Completed {
                    duration_ms: output.duration_ms,
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
            Ok(output) => failed_output(output),
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

fn detection_spec(launch: &LaunchCommand, args: Vec<String>) -> ProcessSpec {
    let mut resolved_args = launch.prefix_args.clone();
    resolved_args.extend(args);
    ProcessSpec {
        program: launch.program.clone(),
        args: resolved_args,
        current_dir: std::env::temp_dir(),
        env: BTreeMap::new(),
        environment: ProcessEnvironment::Inherit,
        timeout: Duration::from_secs(10),
    }
}

fn unavailable(kind: TargetKind) -> TargetAvailability {
    TargetAvailability {
        kind,
        installed: false,
        version: None,
        auth_state: AuthState::Unknown,
        status: AvailabilityStatus::NotFound,
        source: None,
        prerequisites: Vec::new(),
    }
}

fn launch_source(launch: &LaunchCommand) -> LaunchSource {
    if launch.prefix_args.is_empty() {
        LaunchSource::NativeExe
    } else {
        LaunchSource::ReviewedNpm
    }
}

fn execution_spec(launch: &LaunchCommand, request: ExecutionRequest) -> ProcessSpec {
    let mut args = vec![
        "exec".into(),
        "--ephemeral".into(),
        "--json".into(),
        "--sandbox".into(),
        "workspace-write".into(),
        "--ignore-user-config".into(),
        "--ignore-rules".into(),
    ];
    if let Some(model) = request.model {
        args.extend(["--model".into(), model]);
    }
    if let Some(effort) = request.reasoning_effort {
        args.extend([
            "--config".into(),
            format!(
                "model_reasoning_effort={}",
                serde_json::to_string(&effort).expect("serializing a string cannot fail")
            ),
        ]);
    }
    args.push(request.prompt);
    let mut resolved_args = launch.prefix_args.clone();
    resolved_args.extend(args);
    ProcessSpec {
        program: launch.program.clone(),
        args: resolved_args,
        current_dir: request.workspace,
        env: BTreeMap::new(),
        environment: ProcessEnvironment::Inherit,
        timeout: Duration::from_secs(request.time_budget_secs),
    }
}

fn failed_output(output: crate::ProcessOutput) -> Result<AdapterCompletion, AdapterError> {
    let detail = format!("{}\n{}", output.stderr, output.stdout);
    if is_agent_budget_exhaustion(&detail) {
        Err(AdapterError::AgentBudgetExceeded)
    } else {
        infrastructure_with_kind(classify_cli_failure(&detail), detail)
    }
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

fn has_completed_turn(stdout: &str) -> bool {
    enum JsonlState {
        Open,
        Completed,
        Failed,
    }

    let mut state = JsonlState::Open;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if !matches!(state, JsonlState::Open) {
            return false;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        let Some(event_type) = value.get("type").and_then(Value::as_str) else {
            return false;
        };
        state = match event_type {
            "turn.completed" => JsonlState::Completed,
            "turn.failed" | "error" => JsonlState::Failed,
            _ => JsonlState::Open,
        };
    }
    matches!(state, JsonlState::Completed)
}
