use ability_adapters::{
    AdapterCompletion, AdapterError, AgentAdapter, AuthState, AvailabilityStatus, ExecutionRequest,
    LaunchSource, PROMPTFOO_AGENT_CONTRACT_VERSION, ProcessEnvironment, ProcessError,
    ProcessOutput, ProcessRunner, ProcessSpec, PromptfooAgentAdapter, PromptfooRuntime,
};
use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const RUN_ID: Uuid = Uuid::from_u128(0x45c1c4a567ad422f90227fe4b6838f2e);

#[derive(Clone, Copy)]
enum ExecuteMode {
    Success,
    ProviderError(&'static str),
    Timeout,
    Cancelled,
    WrongRun,
}

struct FakeRunner {
    seen: Mutex<Vec<ProcessSpec>>,
    execute_mode: ExecuteMode,
}

impl FakeRunner {
    fn new(execute_mode: ExecuteMode) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            execute_mode,
        })
    }

    fn execution_spec(&self) -> ProcessSpec {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .find(|spec| {
                spec.args
                    .first()
                    .is_some_and(|arg| script_is(arg, "tools/promptfoo-runner/run.mjs"))
            })
            .cloned()
            .expect("runner execution spec")
    }
}

#[async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        _cancellation: CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        self.seen.lock().unwrap().push(spec.clone());
        if spec.args == ["--version"] {
            return Ok(output(0, "v24.1.0\n"));
        }
        if spec.args.first().is_some_and(|arg| {
            arg.replace('\\', "/")
                .ends_with("node_modules/@openai/codex/bin/codex.js")
        }) {
            return match spec.args.last().map(String::as_str) {
                Some("--version") => Ok(output(0, "codex-cli 0.147.0\n")),
                Some("status") => Ok(output(0, "Logged in using ChatGPT\n")),
                _ => unreachable!("unexpected bundled Codex probe"),
            };
        }
        if spec
            .args
            .first()
            .is_some_and(|arg| script_is(arg, "tools/promptfoo-runner/probe.mjs"))
        {
            let provider = spec.args.get(1).map(String::as_str).unwrap();
            let (provider_id, sdk_name, sdk_version) = match provider {
                "codex" => ("openai:codex-sdk", "@openai/codex-sdk", "0.147.0"),
                "claude" => (
                    "anthropic:claude-agent-sdk",
                    "@anthropic-ai/claude-agent-sdk",
                    "0.3.226",
                ),
                _ => unreachable!(),
            };
            return Ok(output(
                0,
                format!(
                    "{}\n",
                    json!({
                        "contract_version": PROMPTFOO_AGENT_CONTRACT_VERSION,
                        "provider": provider,
                        "provider_id": provider_id,
                        "promptfoo_version": "0.122.0",
                        "sdk_name": sdk_name,
                        "sdk_version": sdk_version,
                        "runner_ready": true,
                    })
                ),
            ));
        }
        assert!(
            spec.args
                .first()
                .is_some_and(|arg| script_is(arg, "tools/promptfoo-runner/run.mjs"))
        );
        match self.execute_mode {
            ExecuteMode::Timeout => Err(ProcessError::TimedOut),
            ExecuteMode::Cancelled => Err(ProcessError::Cancelled),
            ExecuteMode::Success | ExecuteMode::WrongRun => {
                let request: Value = serde_json::from_slice(spec.stdin.as_ref().unwrap()).unwrap();
                let run_id = if matches!(self.execute_mode, ExecuteMode::WrongRun) {
                    "dd87404f-d724-46e1-8ab2-c99dcd0d98ce"
                } else {
                    request["run_id"].as_str().unwrap()
                };
                let tool_summary = if request["provider"] == "claude" {
                    json!([
                        { "name": "Bash", "count": 2 },
                        { "name": "Read", "count": 1 }
                    ])
                } else {
                    json!([
                        { "name": "command_execution", "count": 2 },
                        { "name": "file_change", "count": 1 }
                    ])
                };
                Ok(output(
                    0,
                    format!(
                        "{}\n",
                        json!({
                            "contract_version": PROMPTFOO_AGENT_CONTRACT_VERSION,
                            "run_id": run_id,
                            "status": "success",
                            "final_text": "implemented and tested",
                            "session_id": "thread-123",
                            "tokens": { "input": 13, "output": 5, "total": 18 },
                            "tool_summary": tool_summary,
                            "model_evidence": {
                                "requested_model": request["requested_model"],
                                "observed_model": null,
                                "source": "request_only"
                            },
                            "provider_summary": {
                                "unknown_fields": [],
                                "discarded_field_count": 0
                            },
                            "provider_error_code": null
                        })
                    ),
                ))
            }
            ExecuteMode::ProviderError(code) => Ok(output(
                1,
                format!(
                    "{}\n",
                    json!({
                        "contract_version": PROMPTFOO_AGENT_CONTRACT_VERSION,
                        "run_id": RUN_ID,
                        "status": "error",
                        "final_text": "",
                        "session_id": null,
                        "tokens": { "input": null, "output": null, "total": null },
                        "tool_summary": [],
                        "model_evidence": {
                            "requested_model": "gpt-5.6-terra",
                            "observed_model": null,
                            "source": "unavailable"
                        },
                        "provider_summary": {
                            "unknown_fields": [],
                            "discarded_field_count": 0
                        },
                        "provider_error_code": code
                    })
                ),
            )),
        }
    }
}

fn script_is(value: &str, suffix: &str) -> bool {
    value.replace('\\', "/").ends_with(suffix)
}

fn output(exit_code: i32, stdout: impl Into<String>) -> ProcessOutput {
    ProcessOutput {
        exit_code: Some(exit_code),
        stdout: stdout.into(),
        stderr: String::new(),
        duration_ms: 250,
    }
}

struct DetectionAdapter {
    availability: TargetAvailability,
}

use ability_adapters::TargetAvailability;

#[async_trait]
impl AgentAdapter for DetectionAdapter {
    fn kind(&self) -> TargetKind {
        self.availability.kind
    }

    fn contract_version(&self) -> &'static str {
        match self.kind() {
            TargetKind::CodexCli => "codex-cli-v1",
            TargetKind::ClaudeCode => "claude-code-v1",
            _ => unreachable!(),
        }
    }

    async fn detect(&self) -> TargetAvailability {
        self.availability.clone()
    }

    async fn execute(
        &self,
        _request: ExecutionRequest,
        _cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        unreachable!()
    }
}

fn direct_availability(kind: TargetKind, version: &str) -> TargetAvailability {
    TargetAvailability {
        kind,
        installed: true,
        version: Some(version.into()),
        auth_state: AuthState::Ready,
        status: AvailabilityStatus::Ready,
        source: Some(LaunchSource::NativeExe),
        prerequisites: Vec::new(),
    }
}

fn adapter(
    kind: TargetKind,
    runner: Arc<dyn ProcessRunner>,
    version: &str,
    root: &Path,
) -> PromptfooAgentAdapter {
    PromptfooAgentAdapter::with_detection_adapter(
        runner,
        PromptfooRuntime::with_node_program(root, "node"),
        Arc::new(DetectionAdapter {
            availability: direct_availability(kind, version),
        }),
    )
    .unwrap()
}

fn request(workspace: PathBuf) -> ExecutionRequest {
    ExecutionRequest {
        run_id: RUN_ID,
        prompt: "Fix the repository without using the network.".into(),
        workspace,
        time_budget_secs: 600,
        max_turns: 20,
        model: Some("gpt-5.6-terra".into()),
        reasoning_effort: Some("ultra".into()),
    }
}

#[tokio::test]
async fn detects_the_full_runtime_identity_without_exposing_paths() {
    let root = tempdir().unwrap();
    let runner = FakeRunner::new(ExecuteMode::Success);
    let adapter = adapter(
        TargetKind::CodexCli,
        runner,
        "codex-cli 0.147.0",
        root.path(),
    );

    let availability = adapter.detect().await;

    assert_eq!(availability.status, AvailabilityStatus::Ready);
    assert_eq!(availability.source, Some(LaunchSource::ReviewedNpm));
    assert_eq!(adapter.contract_version(), PROMPTFOO_AGENT_CONTRACT_VERSION);
    assert_eq!(
        availability.version.as_deref(),
        Some("promptfoo 0.122.0 codex-sdk 0.147.0 openai-codex-sdk")
    );
    assert!(
        !serde_json::to_string(&availability)
            .unwrap()
            .contains(root.path().to_string_lossy().as_ref())
    );
    assert!(availability.prerequisites.iter().any(|item| {
        item.name == "Codex CLI" && item.version.as_deref() == Some("codex-cli 0.147.0")
    }));
}

#[tokio::test]
async fn old_codex_versions_are_not_reported_ready_for_scored_effort_runs() {
    let root = tempdir().unwrap();
    let runner = FakeRunner::new(ExecuteMode::Success);
    let adapter = adapter(
        TargetKind::CodexCli,
        runner,
        "codex-cli 0.143.0",
        root.path(),
    );

    let availability = adapter.detect().await;

    assert!(availability.installed);
    assert_eq!(availability.status, AvailabilityStatus::VersionUnsupported);
    assert!(
        availability
            .prerequisites
            .iter()
            .any(|item| { item.name == "Codex 推理档位兼容性" && !item.available })
    );
}

#[tokio::test]
async fn codex_prerelease_versions_use_their_semver_core_for_effort_compatibility() {
    let root = tempdir().unwrap();
    let runner = FakeRunner::new(ExecuteMode::Success);
    let adapter = adapter(
        TargetKind::CodexCli,
        runner,
        "codex-cli 0.147.0-alpha.6.5",
        root.path(),
    );

    assert_eq!(adapter.detect().await.status, AvailabilityStatus::Ready);
}

#[tokio::test]
async fn default_codex_adapter_probes_the_same_bundled_cli_used_by_the_sdk() {
    let root = tempdir().unwrap();
    let runner = FakeRunner::new(ExecuteMode::Success);
    let adapter = PromptfooAgentAdapter::new(
        runner.clone(),
        PromptfooRuntime::with_node_program(root.path(), "node"),
        TargetKind::CodexCli,
    )
    .unwrap();

    let availability = adapter.detect().await;

    assert_eq!(availability.status, AvailabilityStatus::Ready);
    assert!(availability.prerequisites.iter().any(|item| {
        item.name == "Codex CLI" && item.version.as_deref() == Some("codex-cli 0.147.0")
    }));
    let seen = runner.seen.lock().unwrap();
    assert!(seen.iter().any(|spec| {
        spec.args.first().is_some_and(|arg| {
            arg.replace('\\', "/")
                .ends_with("node_modules/@openai/codex/bin/codex.js")
        }) && spec.args.last().is_some_and(|arg| arg == "--version")
    }));
}

#[tokio::test]
async fn unknown_local_login_state_fails_closed_before_any_provider_call() {
    let root = tempdir().unwrap();
    let runner = FakeRunner::new(ExecuteMode::Success);
    let mut availability = direct_availability(TargetKind::CodexCli, "codex-cli 0.147.0");
    availability.auth_state = AuthState::Unknown;
    let adapter = PromptfooAgentAdapter::with_detection_adapter(
        runner,
        PromptfooRuntime::with_node_program(root.path(), "node"),
        Arc::new(DetectionAdapter { availability }),
    )
    .unwrap();

    let detected = adapter.detect().await;

    assert_eq!(detected.status, AvailabilityStatus::VersionProbeFailed);
    assert!(
        detected
            .prerequisites
            .iter()
            .any(|item| item.name == "本地登录状态" && !item.available)
    );
}

#[tokio::test]
async fn executes_one_stdin_request_and_returns_structured_evidence() {
    let root = tempdir().unwrap();
    let workspace = tempdir().unwrap();
    let runner = FakeRunner::new(ExecuteMode::Success);
    let adapter = adapter(
        TargetKind::CodexCli,
        runner.clone(),
        "codex-cli 0.147.0",
        root.path(),
    );
    assert_eq!(adapter.detect().await.status, AvailabilityStatus::Ready);

    let completion = adapter
        .execute(
            request(workspace.path().to_path_buf()),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let AdapterCompletion::Completed {
        evidence: Some(evidence),
        ..
    } = completion
    else {
        panic!("Promptfoo completion must include evidence")
    };
    assert_eq!(evidence.final_text, "implemented and tested");
    assert_eq!(evidence.tokens.total, Some(18));
    assert_eq!(evidence.tool_summary.len(), 2);
    assert_eq!(evidence.model_evidence.requested_model, "gpt-5.6-terra");

    let spec = runner.execution_spec();
    assert_eq!(spec.environment, ProcessEnvironment::Clear);
    assert_eq!(spec.args.len(), 1);
    assert!(!spec.args.join(" ").contains("Fix the repository"));
    let sent: Value = serde_json::from_slice(spec.stdin.as_ref().unwrap()).unwrap();
    assert_eq!(sent["provider"], "codex");
    assert_eq!(sent["reasoning_effort"], "ultra");
    assert_eq!(sent["max_turns"], Value::Null);
    assert_eq!(sent["run_id"], RUN_ID.to_string());
}

#[tokio::test]
async fn claude_request_preserves_its_turn_budget_and_separate_runtime_identity() {
    let root = tempdir().unwrap();
    let workspace = tempdir().unwrap();
    let runner = FakeRunner::new(ExecuteMode::Success);
    let adapter = adapter(
        TargetKind::ClaudeCode,
        runner.clone(),
        "2.1.211 (Claude Code)",
        root.path(),
    );

    let availability = adapter.detect().await;
    assert_eq!(availability.status, AvailabilityStatus::Ready);
    assert_eq!(
        availability.version.as_deref(),
        Some("promptfoo 0.122.0 claude-agent-sdk 0.3.226 anthropic-claude-agent-sdk")
    );
    adapter
        .execute(
            request(workspace.path().to_path_buf()),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let sent: Value =
        serde_json::from_slice(runner.execution_spec().stdin.as_ref().unwrap()).unwrap();
    assert_eq!(sent["provider"], "claude");
    assert_eq!(sent["max_turns"], 20);
}

#[tokio::test]
async fn maps_provider_failures_timeouts_cancellation_and_contract_mismatch() {
    let cases = [
        (
            ExecuteMode::ProviderError("auth"),
            Some(FailureKind::AuthExpired),
        ),
        (
            ExecuteMode::ProviderError("quota"),
            Some(FailureKind::QuotaExhausted),
        ),
        (
            ExecuteMode::ProviderError("network"),
            Some(FailureKind::Network),
        ),
        (
            ExecuteMode::ProviderError("model_unavailable"),
            Some(FailureKind::AppInterrupted),
        ),
        (ExecuteMode::WrongRun, Some(FailureKind::AppInterrupted)),
        (ExecuteMode::Timeout, None),
        (ExecuteMode::Cancelled, None),
    ];

    for (mode, failure) in cases {
        let root = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let runner = FakeRunner::new(mode);
        let adapter = adapter(
            TargetKind::CodexCli,
            runner,
            "codex-cli 0.147.0",
            root.path(),
        );
        assert_eq!(adapter.detect().await.status, AvailabilityStatus::Ready);
        let error = adapter
            .execute(
                request(workspace.path().to_path_buf()),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        match (mode, failure) {
            (ExecuteMode::Timeout, None) => {
                assert!(matches!(error, AdapterError::AgentBudgetExceeded))
            }
            (ExecuteMode::Cancelled, None) => assert!(matches!(error, AdapterError::Cancelled)),
            (_, Some(expected)) => assert!(matches!(
                error,
                AdapterError::Infrastructure { kind, .. } if kind == expected
            )),
            _ => unreachable!(),
        }
    }
}
