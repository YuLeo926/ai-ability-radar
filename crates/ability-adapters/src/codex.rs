use crate::command_locator::{LaunchCommand, LaunchDiscovery};
use crate::provider_detection::{probe_launch_candidates, probe_provider_launches};
use crate::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, AvailabilityStatus, ExecutionRequest,
    LaunchSource, ProcessEnvironment, ProcessError, ProcessRunner, ProcessSpec, TargetAvailability,
    classify_cli_failure, is_agent_budget_exhaustion,
};
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct CodexAdapter {
    runner: Arc<dyn ProcessRunner>,
    launch: Mutex<Option<LaunchCommand>>,
    discovery_override: Option<LaunchDiscovery>,
}

impl CodexAdapter {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self {
            runner,
            launch: Mutex::new(None),
            discovery_override: None,
        }
    }

    pub fn with_resolved_command(
        runner: Arc<dyn ProcessRunner>,
        program: impl Into<std::path::PathBuf>,
        prefix_args: Vec<String>,
    ) -> Self {
        let launch = LaunchCommand {
            program: program.into(),
            prefix_args,
            source: LaunchSource::NativeExe,
        };
        Self {
            runner,
            launch: Mutex::new(Some(launch.clone())),
            discovery_override: Some(LaunchDiscovery {
                candidates: vec![launch],
                reviewed_npm_without_node: false,
            }),
        }
    }

    pub fn with_candidate_commands(
        runner: Arc<dyn ProcessRunner>,
        candidates: Vec<(PathBuf, Vec<String>, LaunchSource)>,
        reviewed_npm_without_node: bool,
    ) -> Self {
        Self {
            runner,
            launch: Mutex::new(None),
            discovery_override: Some(LaunchDiscovery {
                candidates: candidates
                    .into_iter()
                    .map(|(program, prefix_args, source)| LaunchCommand {
                        program,
                        prefix_args,
                        source,
                    })
                    .collect(),
                reviewed_npm_without_node,
            }),
        }
    }

    fn retained_launch(&self) -> Option<LaunchCommand> {
        self.launch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl AgentAdapter for CodexAdapter {
    fn kind(&self) -> TargetKind {
        TargetKind::CodexCli
    }

    async fn detect(&self) -> TargetAvailability {
        *self
            .launch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let working = match self.discovery_override.clone() {
            Some(discovery) => probe_launch_candidates(discovery, Arc::clone(&self.runner)).await,
            None => probe_provider_launches("codex", Arc::clone(&self.runner)).await,
        };
        let working = match working {
            Ok(working) => working,
            Err(status) => return unavailable(self.kind(), status),
        };
        *self
            .launch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(working.launch.clone());
        let auth_state = match self
            .runner
            .run(
                detection_spec(&working.launch, vec!["login".into(), "status".into()]),
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
            version: Some(working.version),
            auth_state,
            status: if auth_state == AuthState::NeedsLogin {
                AvailabilityStatus::NeedsLogin
            } else {
                AvailabilityStatus::Ready
            },
            source: Some(working.launch.source),
            prerequisites: Vec::new(),
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        let launch = self.retained_launch().ok_or(AdapterError::Unavailable)?;
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

fn unavailable(kind: TargetKind, status: AvailabilityStatus) -> TargetAvailability {
    TargetAvailability {
        kind,
        installed: false,
        version: None,
        auth_state: AuthState::Unknown,
        status,
        source: None,
        prerequisites: Vec::new(),
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
