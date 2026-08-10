use crate::{
    AdapterCompletion, AdapterError, AgentAdapter, AgentExecutionEvidence, AgentModelEvidence,
    AgentProviderSummary, AgentTokenUsage, AgentToolUsage, AuthState, AvailabilityStatus,
    ClaudeCodeAdapter, CodexAdapter, ExecutionRequest, LaunchSource, ModelEvidenceSource,
    PrerequisiteStatus, ProcessEnvironment, ProcessError, ProcessOutput, ProcessRunner,
    ProcessSpec, TargetAvailability,
};
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const PROMPTFOO_AGENT_CONTRACT_VERSION: &str = "promptfoo-agent-v1";
const PROMPTFOO_VERSION: &str = "0.122.0";
const CODEX_SDK_VERSION: &str = "0.147.0";
const CLAUDE_SDK_VERSION: &str = "0.3.226";
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_FINAL_TEXT_BYTES: usize = 768 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptfooRuntime {
    project_root: PathBuf,
    node_program: PathBuf,
}

impl PromptfooRuntime {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self::with_node_program(project_root, "node")
    }

    pub fn with_node_program(
        project_root: impl Into<PathBuf>,
        node_program: impl Into<PathBuf>,
    ) -> Self {
        Self {
            project_root: project_root.into(),
            node_program: node_program.into(),
        }
    }

    pub fn source_checkout() -> Self {
        Self::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    fn runner_script(&self) -> PathBuf {
        self.project_root
            .join("tools")
            .join("promptfoo-runner")
            .join("run.mjs")
    }

    fn probe_script(&self) -> PathBuf {
        self.project_root
            .join("tools")
            .join("promptfoo-runner")
            .join("probe.mjs")
    }

    fn codex_cli_entry(&self) -> PathBuf {
        self.project_root
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin")
            .join("codex.js")
    }

    fn claude_cli_binary(&self) -> Option<PathBuf> {
        let package = claude_platform_package()?;
        let executable = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        Some(
            self.project_root
                .join("node_modules")
                .join("@anthropic-ai")
                .join(package)
                .join(executable),
        )
    }
}

pub struct PromptfooAgentAdapter {
    runner: Arc<dyn ProcessRunner>,
    runtime: PromptfooRuntime,
    detection_adapter: Arc<dyn AgentAdapter>,
    detected_runtime: Mutex<Option<RuntimeProbeResponse>>,
    detection_guard: tokio::sync::Mutex<()>,
}

impl PromptfooAgentAdapter {
    pub fn new(
        runner: Arc<dyn ProcessRunner>,
        runtime: PromptfooRuntime,
        kind: TargetKind,
    ) -> Result<Self, AdapterError> {
        let detection_adapter: Arc<dyn AgentAdapter> = match kind {
            TargetKind::CodexCli => {
                let entry =
                    path_argument(&runtime.codex_cli_entry()).ok_or(AdapterError::Unavailable)?;
                Arc::new(CodexAdapter::with_candidate_commands(
                    runner.clone(),
                    vec![(
                        runtime.node_program.clone(),
                        vec![entry],
                        LaunchSource::ReviewedNpm,
                    )],
                    false,
                ))
            }
            TargetKind::ClaudeCode => match runtime.claude_cli_binary() {
                Some(binary) => Arc::new(ClaudeCodeAdapter::with_candidate_commands(
                    runner.clone(),
                    vec![(binary, Vec::new(), LaunchSource::ReviewedNpm)],
                    false,
                )),
                None => Arc::new(ClaudeCodeAdapter::new(runner.clone())),
            },
            _ => return Err(AdapterError::Unavailable),
        };
        Self::with_detection_adapter(runner, runtime, detection_adapter)
    }

    pub fn with_detection_adapter(
        runner: Arc<dyn ProcessRunner>,
        runtime: PromptfooRuntime,
        detection_adapter: Arc<dyn AgentAdapter>,
    ) -> Result<Self, AdapterError> {
        if !matches!(
            detection_adapter.kind(),
            TargetKind::CodexCli | TargetKind::ClaudeCode
        ) {
            return Err(AdapterError::Unavailable);
        }
        Ok(Self {
            runner,
            runtime,
            detection_adapter,
            detected_runtime: Mutex::new(None),
            detection_guard: tokio::sync::Mutex::new(()),
        })
    }

    fn retained_runtime(&self) -> Option<RuntimeProbeResponse> {
        self.detected_runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    async fn detect_node(&self) -> Result<String, AvailabilityStatus> {
        let output = self
            .runner
            .run(
                ProcessSpec {
                    program: self.runtime.node_program.clone(),
                    args: vec!["--version".into()],
                    current_dir: self.runtime.project_root.clone(),
                    env: safe_runtime_environment(),
                    environment: ProcessEnvironment::Clear,
                    stdin: None,
                    timeout: Duration::from_secs(10),
                },
                CancellationToken::new(),
            )
            .await
            .map_err(|_| AvailabilityStatus::RuntimeMissing)?;
        let version = output.stdout.trim();
        if output.exit_code != Some(0)
            || !output.stderr.trim().is_empty()
            || !supported_node_version(version)
        {
            return Err(AvailabilityStatus::RuntimeMissing);
        }
        Ok(version.to_owned())
    }

    async fn probe_runtime(&self) -> Result<RuntimeProbeResponse, AvailabilityStatus> {
        let provider = provider_name(self.kind());
        let probe = path_argument(&self.runtime.probe_script())
            .ok_or(AvailabilityStatus::RuntimeMissing)?;
        let output = self
            .runner
            .run(
                ProcessSpec {
                    program: self.runtime.node_program.clone(),
                    args: vec![probe, provider.into()],
                    current_dir: self.runtime.project_root.clone(),
                    env: safe_runtime_environment(),
                    environment: ProcessEnvironment::Clear,
                    stdin: None,
                    timeout: PROBE_TIMEOUT,
                },
                CancellationToken::new(),
            )
            .await
            .map_err(|_| AvailabilityStatus::RuntimeMissing)?;
        if output.exit_code != Some(0) || !output.stderr.trim().is_empty() {
            return Err(AvailabilityStatus::RuntimeMissing);
        }
        let response: RuntimeProbeResponse = parse_single_json_line(&output.stdout)
            .map_err(|_| AvailabilityStatus::VersionProbeFailed)?;
        response
            .validate_for(self.kind())
            .map_err(|_| AvailabilityStatus::VersionProbeFailed)?;
        Ok(response)
    }

    fn unavailable(
        &self,
        status: AvailabilityStatus,
        prerequisites: Vec<PrerequisiteStatus>,
    ) -> TargetAvailability {
        TargetAvailability {
            kind: self.kind(),
            installed: false,
            version: None,
            auth_state: AuthState::Unknown,
            status,
            source: None,
            prerequisites,
        }
    }
}

#[async_trait]
impl AgentAdapter for PromptfooAgentAdapter {
    fn kind(&self) -> TargetKind {
        self.detection_adapter.kind()
    }

    fn contract_version(&self) -> &'static str {
        PROMPTFOO_AGENT_CONTRACT_VERSION
    }

    async fn detect(&self) -> TargetAvailability {
        let _detection = self.detection_guard.lock().await;
        *self
            .detected_runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

        let node_version = match self.detect_node().await {
            Ok(version) => version,
            Err(status) => return self.unavailable(status, Vec::new()),
        };
        let mut prerequisites = vec![PrerequisiteStatus {
            name: "Node.js 22.22+/24 LTS".into(),
            available: true,
            version: Some(node_version),
        }];
        let runtime = match self.probe_runtime().await {
            Ok(runtime) => runtime,
            Err(status) => return self.unavailable(status, prerequisites),
        };
        prerequisites.extend(runtime.prerequisites());

        let mut cli = self.detection_adapter.detect().await;
        let cli_name = match self.kind() {
            TargetKind::CodexCli => "Codex CLI",
            TargetKind::ClaudeCode => "Claude Code",
            _ => unreachable!("constructor rejects non-CLI targets"),
        };
        prerequisites.push(PrerequisiteStatus {
            name: cli_name.into(),
            available: cli.installed && cli.status == AvailabilityStatus::Ready,
            version: cli.version.clone(),
        });
        prerequisites.append(&mut cli.prerequisites);
        if cli.status != AvailabilityStatus::Ready {
            cli.prerequisites = prerequisites;
            return cli;
        }
        if cli.auth_state != AuthState::Ready {
            prerequisites.push(PrerequisiteStatus {
                name: "本地登录状态".into(),
                available: false,
                version: None,
            });
            cli.status = if cli.auth_state == AuthState::NeedsLogin {
                AvailabilityStatus::NeedsLogin
            } else {
                AvailabilityStatus::VersionProbeFailed
            };
            cli.prerequisites = prerequisites;
            return cli;
        }

        if self.kind() == TargetKind::CodexCli
            && !cli
                .version
                .as_deref()
                .is_some_and(codex_supports_reasoning_effort)
        {
            prerequisites.push(PrerequisiteStatus {
                name: "Codex 推理档位兼容性".into(),
                available: false,
                version: Some("需要 codex-cli 0.144.0 或更高版本".into()),
            });
            return TargetAvailability {
                kind: self.kind(),
                installed: true,
                version: cli.version,
                auth_state: cli.auth_state,
                status: AvailabilityStatus::VersionUnsupported,
                source: cli.source,
                prerequisites,
            };
        }
        if self.kind() == TargetKind::CodexCli {
            prerequisites.push(PrerequisiteStatus {
                name: "Codex 推理档位兼容性".into(),
                available: true,
                version: Some("codex-cli 0.144.0+".into()),
            });
        }

        let public_version = runtime.public_version();
        *self
            .detected_runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(runtime);
        TargetAvailability {
            kind: self.kind(),
            installed: true,
            version: Some(public_version),
            auth_state: AuthState::Ready,
            status: AvailabilityStatus::Ready,
            source: Some(LaunchSource::ReviewedNpm),
            prerequisites,
        }
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        let runtime = self.retained_runtime().ok_or(AdapterError::Unavailable)?;
        runtime
            .validate_for(self.kind())
            .map_err(|_| AdapterError::Infrastructure {
                kind: FailureKind::AppInterrupted,
                detail: "Promptfoo runtime identity changed after detection".into(),
            })?;
        let promptfoo_request = RunnerRequest::from_execution(self.kind(), request)?;
        let stdin =
            serde_json::to_vec(&promptfoo_request).map_err(|_| AdapterError::Infrastructure {
                kind: FailureKind::AppInterrupted,
                detail: "Promptfoo request could not be encoded".into(),
            })?;
        let runner = path_argument(&self.runtime.runner_script()).ok_or_else(|| {
            AdapterError::Infrastructure {
                kind: FailureKind::RuntimeMissing,
                detail: "Promptfoo runner path is not representable".into(),
            }
        })?;
        let output = self
            .runner
            .run(
                ProcessSpec {
                    program: self.runtime.node_program.clone(),
                    args: vec![runner],
                    current_dir: self.runtime.project_root.clone(),
                    env: safe_runtime_environment(),
                    environment: ProcessEnvironment::Clear,
                    stdin: Some(stdin),
                    timeout: Duration::from_secs(promptfoo_request.time_budget_seconds),
                },
                cancellation,
            )
            .await
            .map_err(map_process_error)?;
        classify_runner_output(output, &promptfoo_request)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
struct RunnerRequest {
    provider: &'static str,
    workspace: PathBuf,
    prompt: String,
    requested_model: String,
    reasoning_effort: Option<String>,
    time_budget_seconds: u64,
    max_turns: Option<u32>,
    run_id: Uuid,
}

impl RunnerRequest {
    fn from_execution(kind: TargetKind, request: ExecutionRequest) -> Result<Self, AdapterError> {
        if request.time_budget_secs == 0 || request.time_budget_secs > 3_600 {
            return Err(AdapterError::Infrastructure {
                kind: FailureKind::AppInterrupted,
                detail: "Promptfoo time budget is outside the execution contract".into(),
            });
        }
        let max_turns = match kind {
            TargetKind::CodexCli => None,
            TargetKind::ClaudeCode if (1..=200).contains(&request.max_turns) => {
                Some(request.max_turns)
            }
            TargetKind::ClaudeCode => {
                return Err(AdapterError::Infrastructure {
                    kind: FailureKind::AppInterrupted,
                    detail: "Claude turn budget is outside the execution contract".into(),
                });
            }
            _ => return Err(AdapterError::Unavailable),
        };
        Ok(Self {
            provider: provider_name(kind),
            workspace: request.workspace,
            prompt: request.prompt,
            requested_model: request.model.unwrap_or_else(|| "default".into()),
            reasoning_effort: request.reasoning_effort,
            time_budget_seconds: request.time_budget_secs,
            max_turns,
            run_id: request.run_id,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeProbeResponse {
    contract_version: String,
    provider: String,
    provider_id: String,
    promptfoo_version: String,
    sdk_name: String,
    sdk_version: String,
    runner_ready: bool,
}

impl RuntimeProbeResponse {
    fn validate_for(&self, kind: TargetKind) -> Result<(), ()> {
        let (provider, provider_id, sdk_name, sdk_version) = runtime_identity(kind);
        if self.contract_version != PROMPTFOO_AGENT_CONTRACT_VERSION
            || self.provider != provider
            || self.provider_id != provider_id
            || self.promptfoo_version != PROMPTFOO_VERSION
            || self.sdk_name != sdk_name
            || self.sdk_version != sdk_version
            || !self.runner_ready
        {
            return Err(());
        }
        Ok(())
    }

    fn public_version(&self) -> String {
        let sdk_label = match self.provider.as_str() {
            "codex" => "codex-sdk",
            "claude" => "claude-agent-sdk",
            _ => "unknown-sdk",
        };
        format!(
            "promptfoo {} {} {} {}",
            self.promptfoo_version,
            sdk_label,
            self.sdk_version,
            self.provider_id.replace(':', "-")
        )
    }

    fn prerequisites(&self) -> Vec<PrerequisiteStatus> {
        vec![
            PrerequisiteStatus {
                name: "Promptfoo".into(),
                available: true,
                version: Some(self.promptfoo_version.clone()),
            },
            PrerequisiteStatus {
                name: self.sdk_name.clone(),
                available: true,
                version: Some(self.sdk_version.clone()),
            },
            PrerequisiteStatus {
                name: "Promptfoo provider".into(),
                available: true,
                version: Some(self.provider_id.clone()),
            },
            PrerequisiteStatus {
                name: "执行契约".into(),
                available: true,
                version: Some(self.contract_version.clone()),
            },
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RunnerStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderErrorCode {
    Auth,
    Quota,
    Network,
    ModelUnavailable,
    Runtime,
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunnerModelEvidence {
    requested_model: Option<String>,
    observed_model: Option<String>,
    source: ModelEvidenceSource,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunnerResponse {
    contract_version: String,
    run_id: Option<Uuid>,
    status: RunnerStatus,
    final_text: String,
    session_id: Option<String>,
    tokens: AgentTokenUsage,
    tool_summary: Vec<AgentToolUsage>,
    model_evidence: RunnerModelEvidence,
    provider_summary: AgentProviderSummary,
    provider_error_code: Option<ProviderErrorCode>,
}

fn classify_runner_output(
    output: ProcessOutput,
    request: &RunnerRequest,
) -> Result<AdapterCompletion, AdapterError> {
    let response: RunnerResponse =
        parse_single_json_line(&output.stdout).map_err(|_| AdapterError::Infrastructure {
            kind: FailureKind::AppInterrupted,
            detail: "Promptfoo runner returned a malformed response".into(),
        })?;
    validate_runner_response(&response, request)?;
    if response.contract_version != PROMPTFOO_AGENT_CONTRACT_VERSION
        || response.run_id != Some(request.run_id)
        || response.model_evidence.requested_model.as_deref()
            != Some(request.requested_model.as_str())
    {
        return Err(AdapterError::Infrastructure {
            kind: FailureKind::AppInterrupted,
            detail: "Promptfoo runner response identity did not match the request".into(),
        });
    }
    match (
        response.status,
        output.exit_code,
        response.provider_error_code,
    ) {
        (RunnerStatus::Success, Some(0), None) => {
            let model_evidence = AgentModelEvidence {
                requested_model: response
                    .model_evidence
                    .requested_model
                    .expect("validated above"),
                observed_model: response.model_evidence.observed_model,
                source: response.model_evidence.source,
            };
            if (model_evidence.source == ModelEvidenceSource::Provider)
                != model_evidence.observed_model.is_some()
                || model_evidence.source == ModelEvidenceSource::Unavailable
            {
                return Err(AdapterError::Infrastructure {
                    kind: FailureKind::AppInterrupted,
                    detail: "Promptfoo model evidence was inconsistent".into(),
                });
            }
            let evidence = AgentExecutionEvidence {
                contract_version: response.contract_version,
                run_id: request.run_id,
                final_text: response.final_text,
                session_id: response.session_id,
                tokens: response.tokens,
                tool_summary: response.tool_summary,
                model_evidence,
                provider_summary: response.provider_summary,
            };
            Ok(AdapterCompletion::Completed {
                duration_ms: output.duration_ms,
                stdout: output.stdout,
                stderr: output.stderr,
                evidence: Some(evidence),
            })
        }
        (RunnerStatus::Error, Some(code), Some(provider_error)) if code != 0 => {
            Err(AdapterError::Infrastructure {
                kind: provider_error.failure_kind(),
                detail: format!("Promptfoo provider failed: {}", provider_error.label()),
            })
        }
        _ => Err(AdapterError::Infrastructure {
            kind: FailureKind::AppInterrupted,
            detail: "Promptfoo runner exit status did not match its response".into(),
        }),
    }
}

fn validate_runner_response(
    response: &RunnerResponse,
    request: &RunnerRequest,
) -> Result<(), AdapterError> {
    let invalid = || AdapterError::Infrastructure {
        kind: FailureKind::AppInterrupted,
        detail: "Promptfoo runner returned invalid evidence".into(),
    };
    if response.final_text.len() > MAX_FINAL_TEXT_BYTES
        || response
            .session_id
            .as_deref()
            .is_some_and(|value| !safe_label(value, 256))
        || response.tool_summary.len() > 64
        || response.tool_summary.iter().any(|tool| {
            !safe_tool_name(&tool.name)
                || tool.count == 0
                || tool.count > 10_000
                || (request.provider == "claude"
                    && !matches!(
                        tool.name.as_str(),
                        "Read" | "Grep" | "Glob" | "Edit" | "Write" | "Bash"
                    ))
        })
        || response.provider_summary.unknown_fields.len() > 64
        || response
            .provider_summary
            .unknown_fields
            .iter()
            .any(|field| !safe_summary_field(field))
        || response
            .model_evidence
            .requested_model
            .as_deref()
            .is_some_and(|model| !safe_label(model, 120))
        || response
            .model_evidence
            .observed_model
            .as_deref()
            .is_some_and(|model| !safe_label(model, 120))
    {
        return Err(invalid());
    }
    if response.status == RunnerStatus::Error
        && (!response.final_text.is_empty()
            || response.session_id.is_some()
            || response.tokens.input.is_some()
            || response.tokens.output.is_some()
            || response.tokens.total.is_some()
            || !response.tool_summary.is_empty()
            || response.model_evidence.source != ModelEvidenceSource::Unavailable)
    {
        return Err(invalid());
    }
    Ok(())
}

impl ProviderErrorCode {
    fn failure_kind(self) -> FailureKind {
        match self {
            Self::Auth => FailureKind::AuthExpired,
            Self::Quota => FailureKind::QuotaExhausted,
            Self::Network => FailureKind::Network,
            Self::ModelUnavailable | Self::Runtime | Self::Unknown => FailureKind::AppInterrupted,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Quota => "quota",
            Self::Network => "network",
            Self::ModelUnavailable => "model_unavailable",
            Self::Runtime => "runtime",
            Self::Unknown => "unknown",
        }
    }
}

fn map_process_error(error: ProcessError) -> AdapterError {
    match error {
        ProcessError::TimedOut => AdapterError::AgentBudgetExceeded,
        ProcessError::Cancelled => AdapterError::Cancelled,
        ProcessError::Spawn(error) if error.kind() == std::io::ErrorKind::NotFound => {
            AdapterError::Unavailable
        }
        ProcessError::Spawn(_)
        | ProcessError::Supervision(_)
        | ProcessError::Wait(_)
        | ProcessError::CaptureFailed
        | ProcessError::StdinFailed
        | ProcessError::StdinLimit
        | ProcessError::OutputLimit { .. }
        | ProcessError::TerminationFailed
        | ProcessError::DurationOverflow => AdapterError::Infrastructure {
            kind: FailureKind::AppInterrupted,
            detail: "Promptfoo runner process failed".into(),
        },
    }
}

fn parse_single_json_line<T: for<'de> Deserialize<'de>>(stdout: &str) -> Result<T, ()> {
    let line = stdout.strip_suffix('\n').ok_or(())?;
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() || line.contains(['\r', '\n']) {
        return Err(());
    }
    serde_json::from_str(line).map_err(|_| ())
}

fn safe_label(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= maximum
        && value.trim() == value
        && value
            .chars()
            .all(|character| !character.is_control() && character != '\u{200b}')
}

fn safe_tool_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && value.len() <= 128
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | ':' | '-')
        })
}

fn safe_summary_field(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && value.len() <= 128
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        })
}

fn provider_name(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::CodexCli => "codex",
        TargetKind::ClaudeCode => "claude",
        _ => unreachable!("Promptfoo adapter supports CLI targets only"),
    }
}

fn runtime_identity(kind: TargetKind) -> (&'static str, &'static str, &'static str, &'static str) {
    match kind {
        TargetKind::CodexCli => (
            "codex",
            "openai:codex-sdk",
            "@openai/codex-sdk",
            CODEX_SDK_VERSION,
        ),
        TargetKind::ClaudeCode => (
            "claude",
            "anthropic:claude-agent-sdk",
            "@anthropic-ai/claude-agent-sdk",
            CLAUDE_SDK_VERSION,
        ),
        _ => unreachable!("Promptfoo adapter supports CLI targets only"),
    }
}

fn path_argument(path: &Path) -> Option<String> {
    path.to_str().map(str::to_owned)
}

fn supported_node_version(value: &str) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    let mut parts = version.split('.');
    let (Some(major), Some(minor), Some(_patch), None) = (
        parts.next().and_then(|part| part.parse::<u32>().ok()),
        parts.next().and_then(|part| part.parse::<u32>().ok()),
        parts.next().and_then(|part| part.parse::<u32>().ok()),
        parts.next(),
    ) else {
        return false;
    };
    (major == 22 && minor >= 22) || major == 24
}

fn codex_supports_reasoning_effort(value: &str) -> bool {
    let version = value.strip_prefix("codex-cli ").unwrap_or(value);
    let core = version
        .split_once('-')
        .map_or(version, |(core, _prerelease)| core);
    semver_at_least(core, (0, 144, 0))
}

fn semver_at_least(value: &str, minimum: (u32, u32, u32)) -> bool {
    let mut parts = value.split('.');
    let (Some(major), Some(minor), Some(patch), None) = (
        parts.next().and_then(|part| part.parse::<u32>().ok()),
        parts.next().and_then(|part| part.parse::<u32>().ok()),
        parts.next().and_then(|part| part.parse::<u32>().ok()),
        parts.next(),
    ) else {
        return false;
    };
    (major, minor, patch) >= minimum
}

fn safe_runtime_environment() -> BTreeMap<String, String> {
    const KEYS: &[&str] = &[
        "APPDATA",
        "CLAUDE_CONFIG_DIR",
        "CODEX_HOME",
        "COMSPEC",
        "HOME",
        "HOMEDRIVE",
        "HOMEPATH",
        "LANG",
        "LC_ALL",
        "LOCALAPPDATA",
        "NODE_EXTRA_CA_CERTS",
        "PATH",
        "PATHEXT",
        "SHELL",
        "SSL_CERT_DIR",
        "SSL_CERT_FILE",
        "SYSTEMROOT",
        "TEMP",
        "TERM",
        "TMP",
        "TMPDIR",
        "USER",
        "USERNAME",
        "USERPROFILE",
    ];
    std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.to_str()?;
            let value = value.to_str()?;
            (KEYS.iter().any(|allowed| key.eq_ignore_ascii_case(allowed))
                && !value.is_empty()
                && !value.contains('\0'))
            .then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn claude_platform_package() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("claude-agent-sdk-win32-x64"),
        ("windows", "aarch64") => Some("claude-agent-sdk-win32-arm64"),
        ("macos", "x86_64") => Some("claude-agent-sdk-darwin-x64"),
        ("macos", "aarch64") => Some("claude-agent-sdk-darwin-arm64"),
        ("linux", "x86_64") if cfg!(target_env = "musl") => Some("claude-agent-sdk-linux-x64-musl"),
        ("linux", "aarch64") if cfg!(target_env = "musl") => {
            Some("claude-agent-sdk-linux-arm64-musl")
        }
        ("linux", "x86_64") => Some("claude-agent-sdk-linux-x64"),
        ("linux", "aarch64") => Some("claude-agent-sdk-linux-arm64"),
        _ => None,
    }
}
