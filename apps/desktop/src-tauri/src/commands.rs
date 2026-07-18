use crate::app_state::{
    probe_node, public_cli_version, supported_node_lts, AppState, CancellationRegistration,
    CancellationRegistry,
};
use crate::dto::{
    BootstrapDto, CliRunEventDto, PackSummaryDto, RunDetailDto, RunErrorEvent, StartRunInput,
    SubmitAnswerInput, TaskResultDto,
};
use ability_adapters::{
    AgentAdapter, AuthState, CliRunService, PrerequisiteStatus, TargetAvailability,
};
use ability_core::{
    EnvironmentFingerprint, LoadedPack, ManualStep, RunMode, RunRecord, RunRepository, RunStatus,
    TargetKind, TargetSelection,
};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartFamily {
    Manual,
    Cli,
}

struct ValidatedStart {
    target: TargetSelection,
    mode: RunMode,
}

struct CliReadiness {
    cli_version: String,
    node_version: String,
}

struct PreparedCliRun {
    run: RunRecord,
    cancellation: CancellationToken,
    _registration: CancellationRegistration,
}

fn validate_start(input: StartRunInput, family: StartFamily) -> Result<ValidatedStart, String> {
    if input.mode != RunMode::Quick {
        return Err("当前版本只支持快速体检；深度体检尚未实现".into());
    }
    let kind_allowed = match family {
        StartFamily::Manual => matches!(
            input.target.kind,
            TargetKind::ChatGptClient | TargetKind::ClaudeClient
        ),
        StartFamily::Cli => matches!(
            input.target.kind,
            TargetKind::CodexCli | TargetKind::ClaudeCode
        ),
    };
    if !kind_allowed {
        return Err(match family {
            StartFamily::Manual => "手动体检只支持 ChatGPT 或 Claude 客户端",
            StartFamily::Cli => "自动体检只支持 Codex CLI 或 Claude Code",
        }
        .into());
    }

    if input.target.reported_model.chars().any(char::is_control) {
        return Err("模型名称不能包含控制字符".into());
    }
    let reported_model = input.target.reported_model.trim().to_owned();
    if reported_model.is_empty()
        || reported_model.chars().count() > 120
        || reported_model.chars().any(char::is_control)
    {
        return Err("模型名称必须是 1–120 个可见字符".into());
    }
    if family == StartFamily::Cli && !safe_cli_model(&reported_model) {
        return Err(
            "CLI 模型名称只能包含 ASCII 字母、数字、点、下划线、冒号、斜杠或连字符，且必须以字母或数字开头"
                .into(),
        );
    }

    if input
        .target
        .reasoning_effort
        .as_ref()
        .is_some_and(|value| value.chars().any(char::is_control))
    {
        return Err("推理档位不能包含控制字符".into());
    }
    let reasoning_effort = input
        .target
        .reasoning_effort
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    if reasoning_effort
        .as_deref()
        .is_some_and(|value| !matches!(value, "low" | "medium" | "high"))
    {
        return Err("首版推理档位只能是 low、medium 或 high".into());
    }

    Ok(ValidatedStart {
        target: TargetSelection {
            kind: input.target.kind,
            reported_model,
            reasoning_effort,
        },
        mode: input.mode,
    })
}

fn safe_cli_model(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '/' | '-')
        })
}

fn validate_cli_readiness(
    expected_kind: TargetKind,
    availability: TargetAvailability,
    node: PrerequisiteStatus,
) -> Result<CliReadiness, String> {
    if availability.kind != expected_kind {
        return Err("CLI 探测结果与所选测试对象不一致".into());
    }
    if !availability.installed {
        return Err("未找到所选 CLI，请先安装并完成登录".into());
    }
    if availability.auth_state == AuthState::NeedsLogin {
        return Err("所选 CLI 尚未登录，请先在终端完成登录".into());
    }
    let cli_version = public_cli_version(availability.version)
        .ok_or_else(|| "CLI 返回了无效的版本信息".to_string())?;

    let node_version = node
        .version
        .filter(|version| node.available && supported_node_lts(version))
        .ok_or_else(|| "CLI 快速体检需要 Node.js 22 或 24 LTS".to_string())?;
    Ok(CliReadiness {
        cli_version,
        node_version,
    })
}

fn environment(
    pack: &LoadedPack,
    cli_version: Option<String>,
    verifier_runtime_version: Option<String>,
) -> EnvironmentFingerprint {
    let os = os_info::get();
    EnvironmentFingerprint {
        os_family: os.os_type().to_string(),
        os_version: os.version().to_string(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        cli_version,
        verifier_runtime_version,
        suite_id: pack.manifest.id.clone(),
        suite_version: pack.manifest.version.clone(),
        suite_content_sha256: pack.content_sha256.clone(),
        scoring_rule_version: "ability-v1".into(),
        resumed: false,
    }
}

fn prepare_cli_run(
    service: &CliRunService,
    pack: Arc<LoadedPack>,
    start: ValidatedStart,
    readiness: CliReadiness,
    cancellations: &CancellationRegistry,
) -> Result<PreparedCliRun, String> {
    let run = service
        .prepare(
            pack.clone(),
            start.target,
            start.mode,
            environment(
                &pack,
                Some(readiness.cli_version),
                Some(readiness.node_version),
            ),
        )
        .map_err(|error| error.to_string())?;
    let cancellation = CancellationToken::new();
    let registration = cancellations.register(run.id, cancellation.clone())?;
    Ok(PreparedCliRun {
        run,
        cancellation,
        _registration: registration,
    })
}

fn pack_summary(pack: &LoadedPack, estimated_minutes: &str) -> Result<PackSummaryDto, String> {
    Ok(PackSummaryDto {
        id: pack.manifest.id.clone(),
        version: pack.manifest.version.clone(),
        title: pack.manifest.title.clone(),
        task_count: u32::try_from(pack.tasks.len())
            .map_err(|_| "题目数量超过支持范围".to_string())?,
        estimated_minutes: estimated_minutes.into(),
    })
}

fn parse_run_id(value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| "无效的测试编号".into())
}

fn validate_task_id(value: &str) -> Result<&str, String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err("无效的题目编号".into());
    }
    Ok(value)
}

#[tauri::command]
pub async fn get_bootstrap(state: State<'_, AppState>) -> Result<BootstrapDto, String> {
    Ok(BootstrapDto {
        targets: state.target_availability().await,
        client_pack: pack_summary(&state.client_pack, "10–15")?,
        cli_pack: pack_summary(&state.cli_pack, "30–60")?,
    })
}

#[tauri::command]
pub fn start_manual_run(
    state: State<'_, AppState>,
    input: StartRunInput,
) -> Result<RunRecord, String> {
    let start = validate_start(input, StartFamily::Manual)?;
    state
        .manual_runs
        .start(
            state.client_pack.clone(),
            start.target,
            start.mode,
            environment(&state.client_pack, None, None),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn next_manual_step(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<ManualStep>, String> {
    state
        .manual_runs
        .next_step(parse_run_id(&run_id)?)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn submit_manual_answer(
    state: State<'_, AppState>,
    input: SubmitAnswerInput,
) -> Result<TaskResultDto, String> {
    let result = state
        .manual_runs
        .submit_answer(
            parse_run_id(&input.run_id)?,
            validate_task_id(&input.task_id)?,
            &input.answer,
        )
        .map_err(|error| error.to_string())?;
    TaskResultDto::try_from(result)
}

#[tauri::command]
pub async fn start_cli_run(
    app: AppHandle,
    state: State<'_, AppState>,
    input: StartRunInput,
) -> Result<RunRecord, String> {
    let start = validate_start(input, StartFamily::Cli)?;
    let adapter = state
        .adapters
        .get(&start.target.kind)
        .cloned()
        .ok_or_else(|| "该 CLI 暂不支持".to_string())?;
    let availability = adapter.detect().await;
    let node = probe_node(state.runner.clone()).await;
    let readiness = validate_cli_readiness(start.target.kind, availability, node)?;
    let prepared = prepare_cli_run(
        &state.cli_runs,
        state.cli_pack.clone(),
        start,
        readiness,
        &state.cancellations,
    )?;
    let run = prepared.run.clone();
    spawn_cli_run(app, &state, adapter, prepared);
    Ok(run)
}

fn spawn_cli_run(
    app: AppHandle,
    state: &AppState,
    adapter: Arc<dyn AgentAdapter>,
    prepared: PreparedCliRun,
) {
    let service = state.cli_runs.clone();
    let pack = state.cli_pack.clone();
    let verifier = state.verifier.clone();
    let repository = state.repository.clone();
    let run_id = prepared.run.id;
    let (sender, mut receiver) = mpsc::unbounded_channel::<CliRunEventDto>();
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let _ = event_app.emit("run://event", event);
        }
    });
    tauri::async_runtime::spawn(async move {
        let _registration = prepared._registration;
        let result = service
            .execute(
                run_id,
                pack,
                adapter,
                verifier,
                prepared.cancellation,
                sender,
            )
            .await
            .map_err(|error| error.to_string());
        if let Some(error) = finish_background(&repository, run_id, result) {
            let _ = app.emit("run://error", error);
        }
    });
}

fn finish_background(
    repository: &RunRepository,
    run_id: Uuid,
    result: Result<(), String>,
) -> Option<RunErrorEvent> {
    let primary = result.err()?;
    let terminalization_error = match repository.get_run(run_id) {
        Ok(Some(run)) if run.status == RunStatus::Running => {
            match repository.finish_without_score(run_id, RunStatus::Interrupted) {
                Ok(()) => None,
                Err(error) => match repository.get_run(run_id) {
                    Ok(Some(run)) if run.status != RunStatus::Running => None,
                    _ => Some(error.to_string()),
                },
            }
        }
        Ok(Some(_)) => None,
        Ok(None) => Some(format!("run {run_id} was not found")),
        Err(error) => Some(error.to_string()),
    };
    let message = terminalization_error.map_or(primary.clone(), |secondary| {
        format!("{primary}; terminalization failed: {secondary}")
    });
    Some(RunErrorEvent {
        run_id: run_id.to_string(),
        message,
    })
}

#[tauri::command]
pub fn cancel_run(state: State<'_, AppState>, run_id: String) -> Result<bool, String> {
    Ok(state.cancellations.cancel(parse_run_id(&run_id)?))
}

#[tauri::command]
pub fn list_runs(state: State<'_, AppState>) -> Result<Vec<RunRecord>, String> {
    state
        .repository
        .list_runs()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_run_detail(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<RunDetailDto>, String> {
    run_detail_from_repository(&state.repository, parse_run_id(&run_id)?)
}

fn run_detail_from_repository(
    repository: &RunRepository,
    run_id: Uuid,
) -> Result<Option<RunDetailDto>, String> {
    let Some(run) = repository
        .get_run(run_id)
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let task_results = repository
        .get_task_results(run_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(TaskResultDto::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(RunDetailDto { run, task_results }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::CancellationRegistry;
    use crate::dto::{StartRunInput, TargetSelectionInput};
    use ability_adapters::{AuthState, CliRunService, PrerequisiteStatus, TargetAvailability};
    use ability_core::{
        Category, EnvironmentFingerprint, FailureKind, LoadedPack, PackLoader, RunMode, RunRecord,
        RunRepository, RunStatus, TargetKind, TargetSelection, TaskOutcome, TaskResult,
    };
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn target_values_are_normalized_before_use() {
        let manual = validate_start(
            start_input(
                TargetKind::ChatGptClient,
                "  GPT-5 自定义  ",
                Some(" HIGH "),
                RunMode::Quick,
            ),
            StartFamily::Manual,
        )
        .unwrap();
        assert_eq!(manual.target.reported_model, "GPT-5 自定义");
        assert_eq!(manual.target.reasoning_effort.as_deref(), Some("high"));

        let cli = validate_start(
            start_input(
                TargetKind::CodexCli,
                "  openai/gpt-5.1-codex  ",
                Some(" medium "),
                RunMode::Quick,
            ),
            StartFamily::Cli,
        )
        .unwrap();
        assert_eq!(cli.target.reported_model, "openai/gpt-5.1-codex");
        assert_eq!(cli.target.reasoning_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn target_family_mode_and_unsafe_values_are_rejected() {
        assert!(validate_start(
            start_input(TargetKind::CodexCli, "default", None, RunMode::Quick,),
            StartFamily::Manual,
        )
        .is_err());
        assert!(validate_start(
            start_input(TargetKind::ChatGptClient, "GPT-5", None, RunMode::Quick,),
            StartFamily::Cli,
        )
        .is_err());
        assert!(validate_start(
            start_input(TargetKind::CodexCli, "default", None, RunMode::Deep,),
            StartFamily::Cli,
        )
        .is_err());

        for model in [
            "",
            " \t ",
            "-dangerous",
            "model name",
            "model;calc",
            "model\"value",
            "\nmodel",
            "model\t",
            "model\nvalue",
            "model\u{7f}value",
        ] {
            assert!(
                validate_start(
                    start_input(TargetKind::CodexCli, model, None, RunMode::Quick),
                    StartFamily::Cli,
                )
                .is_err(),
                "{model:?} should be rejected"
            );
        }
        for effort in ["ultra", "high\n", "médiúm"] {
            assert!(validate_start(
                start_input(
                    TargetKind::ClaudeCode,
                    "default",
                    Some(effort),
                    RunMode::Quick,
                ),
                StartFamily::Cli,
            )
            .is_err());
        }
    }

    #[test]
    fn quick_only_rejection_happens_before_persistence_or_registration() {
        let directory = tempdir().unwrap();
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let service = CliRunService::new(repository.clone(), directory.path().join("artifacts"));
        let cancellations = CancellationRegistry::default();
        let pack = cli_pack();
        let input = start_input(TargetKind::CodexCli, "default", None, RunMode::Deep);

        let validation = validate_start(input, StartFamily::Cli);

        assert!(validation.is_err());
        assert!(repository.list_runs().unwrap().is_empty());
        assert!(service_artifact_root_is_untouched(directory.path()));
        assert!(!cancellations.cancel(uuid::Uuid::new_v4()));
        drop(service);
        drop(pack);
    }

    #[test]
    fn successful_readiness_values_bind_the_persisted_environment() {
        let directory = tempdir().unwrap();
        let artifact_root = directory.path().join("artifacts");
        std::fs::create_dir(&artifact_root).unwrap();
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let service = CliRunService::new(repository.clone(), artifact_root);
        let cancellations = CancellationRegistry::default();
        let pack = cli_pack();
        let validated = validate_start(
            start_input(
                TargetKind::CodexCli,
                "default",
                Some("high"),
                RunMode::Quick,
            ),
            StartFamily::Cli,
        )
        .unwrap();
        let readiness = validate_cli_readiness(
            validated.target.kind,
            TargetAvailability {
                kind: TargetKind::CodexCli,
                installed: true,
                version: Some("codex-cli 1.2.3".into()),
                auth_state: AuthState::Ready,
                prerequisites: Vec::new(),
            },
            PrerequisiteStatus {
                name: "Node.js 22/24 LTS".into(),
                available: true,
                version: Some("v24.18.0".into()),
            },
        )
        .unwrap();

        let prepared =
            prepare_cli_run(&service, pack.clone(), validated, readiness, &cancellations).unwrap();

        assert_eq!(
            prepared.run.environment.cli_version.as_deref(),
            Some("codex-cli 1.2.3")
        );
        assert_eq!(
            prepared.run.environment.verifier_runtime_version.as_deref(),
            Some("v24.18.0")
        );
        assert_eq!(
            prepared.run.environment.suite_content_sha256,
            pack.content_sha256
        );
        assert_eq!(
            repository
                .get_run(prepared.run.id)
                .unwrap()
                .unwrap()
                .environment,
            prepared.run.environment
        );
        assert!(cancellations.cancel(prepared.run.id));
    }

    #[test]
    fn readiness_rejects_wrong_target_login_and_untrusted_versions() {
        let ready_node = || PrerequisiteStatus {
            name: "Node.js 22/24 LTS".into(),
            available: true,
            version: Some("v22.23.1".into()),
        };
        let availability =
            |kind, installed, version: Option<&str>, auth_state| TargetAvailability {
                kind,
                installed,
                version: version.map(str::to_owned),
                auth_state,
                prerequisites: Vec::new(),
            };

        assert!(validate_cli_readiness(
            TargetKind::CodexCli,
            availability(
                TargetKind::ClaudeCode,
                true,
                Some("claude 1"),
                AuthState::Ready,
            ),
            ready_node(),
        )
        .is_err());
        assert!(validate_cli_readiness(
            TargetKind::CodexCli,
            availability(TargetKind::CodexCli, false, None, AuthState::Unknown,),
            ready_node(),
        )
        .is_err());
        assert!(validate_cli_readiness(
            TargetKind::ClaudeCode,
            availability(
                TargetKind::ClaudeCode,
                true,
                Some("claude 1"),
                AuthState::NeedsLogin,
            ),
            ready_node(),
        )
        .is_err());
        assert!(validate_cli_readiness(
            TargetKind::CodexCli,
            availability(
                TargetKind::CodexCli,
                true,
                Some("codex\nforged"),
                AuthState::Ready,
            ),
            ready_node(),
        )
        .is_err());
        assert!(validate_cli_readiness(
            TargetKind::CodexCli,
            availability(
                TargetKind::CodexCli,
                true,
                Some("codex 1"),
                AuthState::Ready,
            ),
            PrerequisiteStatus {
                name: "Node.js 22/24 LTS".into(),
                available: true,
                version: Some("v20.20.0".into()),
            },
        )
        .is_err());
    }

    #[test]
    fn background_errors_interrupt_only_running_rows_and_preserve_primary_error() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let running = insert_run(&repository, RunStatus::Running);

        let event =
            finish_background(&repository, running.id, Err("primary failure".into())).unwrap();

        assert!(event.message.contains("primary failure"));
        assert_eq!(
            repository.get_run(running.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );

        let completed = insert_run(&repository, RunStatus::Completed);
        let event =
            finish_background(&repository, completed.id, Err("late failure".into())).unwrap();
        assert_eq!(event.message, "late failure");
        assert_eq!(
            repository.get_run(completed.id).unwrap().unwrap().status,
            RunStatus::Completed
        );
    }

    #[test]
    fn background_error_reports_secondary_terminalization_failure() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let missing = uuid::Uuid::new_v4();

        let event = finish_background(&repository, missing, Err("primary failure".into())).unwrap();

        assert!(event.message.contains("primary failure"));
        assert!(event.message.contains("terminalization"));
        assert!(event.message.contains(&missing.to_string()));
    }

    #[test]
    fn run_detail_command_path_omits_raw_repository_detail() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let run = insert_run(&repository, RunStatus::Running);
        let relative_artifact = format!("runs/{}/logs/dedupe-events.log", run.id);
        let credential = "AKIA_TASK13_REVIEW_SENTINEL_91Q7X";
        let absolute_path = r"C:\Users\Alice\.codex\auth.json";
        let raw_output = "RAW_STDOUT provider payload\nRAW_STDERR request-id=secret";
        repository
            .save_task_result(&TaskResult {
                run_id: run.id,
                task_id: "dedupe-events".into(),
                category: Category::CliCoding,
                outcome: TaskOutcome::Failed,
                score: Some(0.0),
                failure_kind: Some(FailureKind::WrongAnswer),
                duration_ms: 321,
                answer_rel_path: Some(relative_artifact.clone()),
                detail: format!("{credential}\n{absolute_path}\n{raw_output}"),
            })
            .unwrap();

        let detail = run_detail_from_repository(&repository, run.id)
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&detail).unwrap();

        assert!(!serialized.contains(credential));
        assert!(!serialized.contains(absolute_path));
        assert!(!serialized.contains("RAW_STDOUT"));
        assert!(!serialized.contains("RAW_STDERR"));
        assert!(!serialized.contains("\"detail\""));
        assert!(serialized.contains("\"outcome\":\"failed\""));
        assert!(serialized.contains("\"failureKind\":\"wrong_answer\""));
        assert!(serialized.contains(&relative_artifact));
    }

    fn start_input(
        kind: TargetKind,
        model: &str,
        effort: Option<&str>,
        mode: RunMode,
    ) -> StartRunInput {
        StartRunInput {
            target: TargetSelectionInput {
                kind,
                reported_model: model.into(),
                reasoning_effort: effort.map(str::to_owned),
            },
            mode,
        }
    }

    fn cli_pack() -> Arc<LoadedPack> {
        Arc::new(
            PackLoader::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../benchmark-packs/cli-quick-v1"),
            )
            .unwrap(),
        )
    }

    fn service_artifact_root_is_untouched(root: &Path) -> bool {
        !root.join("artifacts").exists()
    }

    fn insert_run(repository: &RunRepository, status: RunStatus) -> RunRecord {
        let pack = cli_pack();
        let mut run = RunRecord::new(
            TargetSelection {
                kind: TargetKind::CodexCli,
                reported_model: "default".into(),
                reasoning_effort: None,
            },
            RunMode::Quick,
            pack.manifest.id.clone(),
            pack.manifest.version.clone(),
            u32::try_from(pack.tasks.len()).unwrap(),
            EnvironmentFingerprint {
                os_family: "windows".into(),
                os_version: "test".into(),
                app_version: "0.1.0".into(),
                cli_version: Some("fake-cli".into()),
                verifier_runtime_version: Some("v22.0.0".into()),
                suite_id: pack.manifest.id.clone(),
                suite_version: pack.manifest.version.clone(),
                suite_content_sha256: pack.content_sha256.clone(),
                scoring_rule_version: "ability-v1".into(),
                resumed: false,
            },
        );
        run.status = RunStatus::Running;
        repository.insert_run(&run).unwrap();
        if status == RunStatus::Completed {
            repository.complete_run(run.id, None).unwrap();
        }
        repository.get_run(run.id).unwrap().unwrap()
    }
}
