use crate::app_state::{
    fresh_provider_adapters, probe_node, public_cli_version, supported_node_lts, AppState,
    CancellationRegistration, CancellationRegistry, LocalDataGate, LocalDataMutationClaim,
    RunOperationRegistry,
};
use crate::dto::{
    BootstrapDto, CliRunEventDto, DataSettingsDto, DeleteTargetHistoryInput, ExportReportInput,
    FullBackupInput, PackSummaryDto, ResumeRunInput, ResumeTargetSelectionInput, RunDetailDto,
    RunErrorEvent, RunIdInput, SetRetentionInput, StartRunInput, SubmitAnswerInput, TaskResultDto,
};
use ability_adapters::{
    AgentAdapter, AuthState, CliRunService, PrerequisiteStatus, ProcessRunner, TargetAvailability,
};
use ability_core::{
    contains_forbidden_display_character, is_valid_reported_model, ArtifactStore,
    EnvironmentFingerprint, LoadedPack, ManualRunService, ManualStep, RunMode, RunRecord,
    RunRepository, RunStatus, TargetKind, TargetSelection,
};
use std::collections::BTreeMap;
use std::fs;
#[cfg(windows)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
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

const SAFE_BACKGROUND_ERROR: &str =
    "CLI 运行被中断；本次不会作为能力失败计分，请查看本地记录后重试。";

const KNOWN_REASONING_EFFORTS: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

fn normalize_reasoning_effort(
    value: Option<String>,
    family: StartFamily,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if contains_forbidden_display_character(&value) {
        return Err("\u{63a8}\u{7406}\u{6863}\u{4f4d}\u{4e0d}\u{80fd}\u{5305}\u{542b}\u{63a7}\u{5236}\u{5b57}\u{7b26}\u{3001}\u{683c}\u{5f0f}\u{5b57}\u{7b26}\u{6216}\u{4e0d}\u{53ef}\u{89c1}\u{5b57}\u{7b26}".into());
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(
            "\u{8bf7}\u{586b}\u{5199}\u{81ea}\u{5b9a}\u{4e49}\u{63a8}\u{7406}\u{6863}\u{4f4d}"
                .into(),
        );
    }
    let canonical = trimmed.to_ascii_lowercase();
    if KNOWN_REASONING_EFFORTS.contains(&canonical.as_str()) {
        return Ok(Some(canonical));
    }

    match family {
        StartFamily::Manual => {
            if trimmed.chars().count() > 40 {
                return Err("\u{81ea}\u{5b9a}\u{4e49}\u{63a8}\u{7406}\u{6863}\u{4f4d}\u{5fc5}\u{987b}\u{662f} 1\u{2013}40 \u{4e2a}\u{53ef}\u{89c1}\u{5b57}\u{7b26}".into());
            }
            Ok(Some(trimmed.to_owned()))
        }
        StartFamily::Cli => {
            if canonical.len() > 32
                || !canonical
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return Err(
                    "CLI \u{63a8}\u{7406}\u{6863}\u{4f4d}\u{53ea}\u{80fd}\u{5305}\u{542b} 1\u{2013}32 \u{4e2a} ASCII \u{5b57}\u{6bcd}\u{3001}\u{6570}\u{5b57}\u{3001}\u{4e0b}\u{5212}\u{7ebf}\u{6216}\u{8fde}\u{5b57}\u{7b26}"
                        .into(),
                );
            }
            Ok(Some(canonical))
        }
    }
}

fn validate_stored_reasoning_effort(
    value: Option<String>,
    family: StartFamily,
) -> Result<Option<String>, String> {
    let normalized = normalize_reasoning_effort(value.clone(), family)?;
    if normalized != value {
        return Err("\u{6062}\u{590d}\u{76ee}\u{6807}\u{5305}\u{542b}\u{672a}\u{89c4}\u{8303}\u{5316}\u{7684}\u{63a8}\u{7406}\u{6863}\u{4f4d}\u{3002}".into());
    }
    Ok(normalized)
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

    if contains_forbidden_display_character(&input.target.reported_model) {
        return Err("模型名称不能包含控制字符、格式字符或不可见字符".into());
    }
    let reported_model = input.target.reported_model.trim().to_owned();
    if !is_valid_reported_model(&reported_model) {
        return Err("模型名称必须是 1–120 个可见字符".into());
    }
    if family == StartFamily::Cli && !safe_cli_model(&reported_model) {
        return Err(
            "CLI 模型名称只能包含 ASCII 字母、数字、点、下划线、冒号、斜杠或连字符，且必须以字母或数字开头"
                .into(),
        );
    }

    let reasoning_effort = normalize_reasoning_effort(input.target.reasoning_effort, family)?;

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

fn target_kind_is_in_family(kind: TargetKind, family: StartFamily) -> bool {
    match family {
        StartFamily::Manual => {
            matches!(kind, TargetKind::ChatGptClient | TargetKind::ClaudeClient)
        }
        StartFamily::Cli => matches!(kind, TargetKind::CodexCli | TargetKind::ClaudeCode),
    }
}

fn validate_resume_target(
    input: ResumeTargetSelectionInput,
    family: StartFamily,
) -> Result<TargetSelection, String> {
    if !target_kind_is_in_family(input.kind, family) {
        return Err("恢复目标不属于当前体检类型。".into());
    }
    if !is_valid_reported_model(&input.reported_model) {
        return Err("恢复目标包含无效的模型名称。".into());
    }
    if family == StartFamily::Cli && !safe_cli_model(&input.reported_model) {
        return Err("恢复目标包含无效的 CLI 模型名称。".into());
    }
    let reasoning_effort = validate_stored_reasoning_effort(input.reasoning_effort, family)?;
    Ok(TargetSelection {
        kind: input.kind,
        reported_model: input.reported_model,
        reasoning_effort,
    })
}

fn load_matching_resume_run(
    repository: &RunRepository,
    run_id: Uuid,
    expected_target: &TargetSelection,
    family: StartFamily,
) -> Result<RunRecord, String> {
    let stored = repository
        .get_run(run_id)
        .map_err(|_| "无法读取这次体检，请稍后重试。".to_string())?
        .ok_or_else(|| "没有找到这次体检。".to_string())?;
    if stored.status != RunStatus::Interrupted
        || !target_kind_is_in_family(stored.target.kind, family)
        || stored.target != *expected_target
    {
        return Err("恢复请求与原体检配置不一致。".into());
    }
    Ok(stored)
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
    let _local_data = state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
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
pub fn resume_manual_run(
    state: State<'_, AppState>,
    input: ResumeRunInput,
) -> Result<RunRecord, String> {
    let run_id = parse_run_id(&input.run_id)?;
    let _local_data = state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let _operation = state
        .run_operations
        .claim([run_id])
        .map_err(|_| "这次体检正在恢复、运行或清理数据，请勿重复操作。".to_string())?;
    let expected_target = validate_resume_target(input.expected_target, StartFamily::Manual)?;
    load_matching_resume_run(
        &state.repository,
        run_id,
        &expected_target,
        StartFamily::Manual,
    )?;
    state
        .manual_runs
        .resume(
            run_id,
            expected_target,
            state.client_pack.clone(),
            environment(&state.client_pack, None, None),
        )
        .map_err(|_| "无法恢复这次体检；本地检查点或运行环境已变化。".to_string())
}

#[tauri::command]
pub fn next_manual_step(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Option<ManualStep>, String> {
    next_manual_step_for(
        &state.manual_runs,
        &state.run_operations,
        &state.local_data_gate,
        parse_run_id(&run_id)?,
    )
}

fn next_manual_step_for(
    service: &ManualRunService,
    operations: &RunOperationRegistry,
    gate: &LocalDataGate,
    run_id: Uuid,
) -> Result<Option<ManualStep>, String> {
    let _local_data = gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let _operation = operations
        .claim([run_id])
        .map_err(|_| "这次体检正在恢复、运行或清理数据，请勿重复操作。".to_string())?;
    service.next_step(run_id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn submit_manual_answer(
    state: State<'_, AppState>,
    input: SubmitAnswerInput,
) -> Result<TaskResultDto, String> {
    submit_manual_answer_for(
        &state.manual_runs,
        &state.run_operations,
        &state.local_data_gate,
        parse_run_id(&input.run_id)?,
        validate_task_id(&input.task_id)?,
        &input.answer,
    )
}

fn submit_manual_answer_for(
    service: &ManualRunService,
    operations: &RunOperationRegistry,
    gate: &LocalDataGate,
    run_id: Uuid,
    task_id: &str,
    answer: &str,
) -> Result<TaskResultDto, String> {
    let _local_data = gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let _operation = operations
        .claim([run_id])
        .map_err(|_| "这次体检正在恢复、运行或清理数据，请勿重复操作。".to_string())?;
    let result = service
        .submit_answer(run_id, task_id, answer)
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
    let adapters = fresh_provider_adapters(state.runner.clone());
    let adapter = adapters
        .get(&start.target.kind)
        .cloned()
        .ok_or_else(|| "该 CLI 暂不支持".to_string())?;
    let availability = adapter.detect().await;
    let node = probe_node(state.runner.clone()).await;
    let readiness = validate_cli_readiness(start.target.kind, availability, node)?;
    let local_data = state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let prepared = prepare_cli_run(
        &state.cli_runs,
        state.cli_pack.clone(),
        start,
        readiness,
        &state.cancellations,
    )?;
    let run = prepared.run.clone();
    spawn_cli_run(app, &state, adapter, prepared, local_data);
    Ok(run)
}

#[tauri::command]
pub async fn resume_cli_run(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ResumeRunInput,
) -> Result<RunRecord, String> {
    let run_id = parse_run_id(&input.run_id)?;
    let local_data = state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let _operation = state
        .run_operations
        .claim([run_id])
        .map_err(|_| "这次体检正在恢复、运行或清理数据，请勿重复操作。".to_string())?;
    let expected_target = validate_resume_target(input.expected_target, StartFamily::Cli)?;
    let adapters = fresh_provider_adapters(state.runner.clone());
    resume_cli_run_with(
        CliResumeContext {
            repository: &state.repository,
            service: &state.cli_runs,
            pack: state.cli_pack.clone(),
            adapters: &adapters,
            runner: state.runner.clone(),
            cancellations: &state.cancellations,
        },
        run_id,
        expected_target,
        |adapter, prepared| spawn_cli_run(app, &state, adapter, prepared, local_data),
    )
    .await
}

struct CliResumeContext<'a> {
    repository: &'a RunRepository,
    service: &'a CliRunService,
    pack: Arc<LoadedPack>,
    adapters: &'a BTreeMap<TargetKind, Arc<dyn AgentAdapter>>,
    runner: Arc<dyn ProcessRunner>,
    cancellations: &'a CancellationRegistry,
}

async fn resume_cli_run_with<S>(
    context: CliResumeContext<'_>,
    run_id: Uuid,
    expected_target: TargetSelection,
    spawn: S,
) -> Result<RunRecord, String>
where
    S: FnOnce(Arc<dyn AgentAdapter>, PreparedCliRun),
{
    let CliResumeContext {
        repository,
        service,
        pack,
        adapters,
        runner,
        cancellations,
    } = context;
    let stored = load_matching_resume_run(repository, run_id, &expected_target, StartFamily::Cli)?;
    let expected_kind = stored.target.kind;
    let cancellation = CancellationToken::new();
    let registration = cancellations
        .register(run_id, cancellation.clone())
        .map_err(|_| "这次体检正在恢复或运行，请勿重复启动。".to_string())?;
    let adapter = adapters
        .get(&expected_kind)
        .cloned()
        .ok_or_else(|| "当前版本不支持这个 CLI。".to_string())?;
    let availability = adapter.detect().await;
    let node = probe_node(runner).await;
    let readiness = validate_cli_readiness(expected_kind, availability, node)?;
    let run = service
        .resume(
            run_id,
            expected_target,
            &pack,
            environment(
                &pack,
                Some(readiness.cli_version),
                Some(readiness.node_version),
            ),
        )
        .map_err(|_| "无法恢复这次 CLI 体检；检查点或运行环境已变化。".to_string())?;
    let prepared = PreparedCliRun {
        run: run.clone(),
        cancellation,
        _registration: registration,
    };
    spawn(adapter, prepared);
    Ok(run)
}

fn spawn_cli_run(
    app: AppHandle,
    state: &AppState,
    adapter: Arc<dyn AgentAdapter>,
    prepared: PreparedCliRun,
    local_data: LocalDataMutationClaim,
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
        let _local_data = local_data;
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
            .map_err(|_| SAFE_BACKGROUND_ERROR.to_string());
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
    result.err()?;
    match repository.get_run(run_id) {
        Ok(Some(run)) if run.status == RunStatus::Running => {
            match repository.finish_without_score(run_id, RunStatus::Interrupted) {
                Ok(()) => {}
                Err(error) => match repository.get_run(run_id) {
                    Ok(Some(run)) if run.status != RunStatus::Running => {}
                    _ => {
                        let _ = error;
                    }
                },
            }
        }
        Ok(Some(_)) | Ok(None) | Err(_) => {}
    }
    Some(RunErrorEvent {
        run_id: run_id.to_string(),
        message: SAFE_BACKGROUND_ERROR.into(),
    })
}

#[tauri::command]
pub fn cancel_run(state: State<'_, AppState>, run_id: String) -> Result<bool, String> {
    cancel_run_for(
        &state.cancellations,
        &state.manual_runs,
        &state.run_operations,
        &state.local_data_gate,
        parse_run_id(&run_id)?,
    )
}

fn cancel_run_for(
    cancellations: &CancellationRegistry,
    manual_runs: &ManualRunService,
    operations: &RunOperationRegistry,
    gate: &LocalDataGate,
    run_id: Uuid,
) -> Result<bool, String> {
    let _local_data = gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    if cancellations.cancel(run_id) {
        return Ok(true);
    }
    let _operation = operations
        .claim([run_id])
        .map_err(|_| "这次体检正在恢复、运行或清理数据，请勿重复操作。".to_string())?;
    manual_runs
        .cancel(run_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn interrupt_manual_run(state: State<'_, AppState>, run_id: String) -> Result<bool, String> {
    interrupt_manual_run_for(
        &state.manual_runs,
        &state.run_operations,
        &state.local_data_gate,
        parse_run_id(&run_id)?,
    )
}

fn interrupt_manual_run_for(
    manual_runs: &ManualRunService,
    operations: &RunOperationRegistry,
    gate: &LocalDataGate,
    run_id: Uuid,
) -> Result<bool, String> {
    let _local_data = gate
        .claim_mutating()
        .map_err(|_| "local data is busy".to_string())?;
    let _operation = operations
        .claim([run_id])
        .map_err(|_| "run operation is busy".to_string())?;
    manual_runs
        .interrupt(run_id)
        .map_err(|error| error.to_string())
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

#[tauri::command]
pub fn delete_raw_artifacts(state: State<'_, AppState>, input: RunIdInput) -> Result<(), String> {
    let run_id = parse_run_id(&input.run_id)?;
    let _local_data = state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let store = ArtifactStore::new(state.artifact_root.clone());
    delete_raw_artifacts_for(&state.repository, &store, &state.run_operations, run_id)
}

#[tauri::command]
pub fn delete_run(state: State<'_, AppState>, input: RunIdInput) -> Result<bool, String> {
    let run_id = parse_run_id(&input.run_id)?;
    let _local_data = state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let store = ArtifactStore::new(state.artifact_root.clone());
    delete_run_for(&state.repository, &store, &state.run_operations, run_id)
}

#[tauri::command]
pub fn delete_target_history(
    state: State<'_, AppState>,
    input: DeleteTargetHistoryInput,
) -> Result<u32, String> {
    let expected = parse_expected_run_ids(&input.expected_run_ids)?;
    let _local_data = state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    let store = ArtifactStore::new(state.artifact_root.clone());
    delete_target_history_for(
        &state.repository,
        &store,
        &state.run_operations,
        input.target,
        &expected,
    )
}

fn parse_expected_run_ids(values: &[String]) -> Result<Vec<Uuid>, String> {
    if values.len() > 10_000 {
        return Err("确认的历史记录数量超过支持范围。".into());
    }
    let mut ids = values
        .iter()
        .map(|value| parse_run_id(value))
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("确认的历史记录包含重复项，请重新检查。".into());
    }
    Ok(ids)
}

fn ensure_inactive_run(
    repository: &RunRepository,
    run_id: Uuid,
) -> Result<Option<RunRecord>, String> {
    let run = repository
        .get_run(run_id)
        .map_err(|_| "无法读取本地记录，请稍后重试。".to_string())?;
    if run
        .as_ref()
        .is_some_and(|record| record.status == RunStatus::Running)
    {
        return Err("运行中的体检不能删除或清理数据。".into());
    }
    Ok(run)
}

fn delete_raw_artifacts_for(
    repository: &RunRepository,
    store: &ArtifactStore,
    operations: &RunOperationRegistry,
    run_id: Uuid,
) -> Result<(), String> {
    let _operation = operations
        .claim([run_id])
        .map_err(|_| "这次体检正在恢复、运行或清理数据，当前不能删除。".to_string())?;
    let run =
        ensure_inactive_run(repository, run_id)?.ok_or_else(|| "没有找到这次体检。".to_string())?;
    store
        .delete_run_artifacts(run_id, run.target.kind)
        .map_err(|_| "无法安全删除本地原始数据；未更改体检记录。".to_string())?;
    repository
        .clear_artifact_references(run_id)
        .map_err(|_| "原始数据已移除，但记录更新失败；可以安全重试。".to_string())?;
    Ok(())
}

fn delete_run_for(
    repository: &RunRepository,
    store: &ArtifactStore,
    operations: &RunOperationRegistry,
    run_id: Uuid,
) -> Result<bool, String> {
    let _operation = operations
        .claim([run_id])
        .map_err(|_| "这次体检正在恢复、运行或清理数据，当前不能删除。".to_string())?;
    let Some(run) = ensure_inactive_run(repository, run_id)? else {
        return Ok(false);
    };
    store
        .delete_run_artifacts(run_id, run.target.kind)
        .map_err(|_| "无法安全删除本地原始数据；体检记录仍被保留。".to_string())?;
    repository
        .delete_run(run_id)
        .map_err(|_| "原始数据已移除，但体检记录删除失败；可以安全重试。".to_string())
}

fn delete_target_history_for(
    repository: &RunRepository,
    store: &ArtifactStore,
    operations: &RunOperationRegistry,
    target: TargetKind,
    expected_run_ids: &[Uuid],
) -> Result<u32, String> {
    let _operations = operations
        .claim(expected_run_ids.iter().copied())
        .map_err(|_| "部分体检正在恢复、运行或清理数据，当前不能批量删除。".to_string())?;
    let runs = repository
        .list_runs()
        .map_err(|_| "无法读取本地历史，请稍后重试。".to_string())?;
    let mut current = runs
        .iter()
        .filter(|run| run.target.kind == target)
        .map(|run| run.id)
        .collect::<Vec<_>>();
    current.sort_unstable();
    let mut expected = expected_run_ids.to_vec();
    expected.sort_unstable();
    if current != expected {
        return Err("历史记录在确认后发生了变化，请重新检查再删除。".into());
    }
    if runs
        .iter()
        .any(|run| expected.binary_search(&run.id).is_ok() && run.status == RunStatus::Running)
    {
        return Err("运行中的体检不能删除。".into());
    }
    for run_id in &expected {
        let persisted_target = runs
            .iter()
            .find(|run| run.id == *run_id)
            .map(|run| run.target.kind)
            .ok_or_else(|| "历史记录在确认后发生了变化，请重新检查再删除。".to_string())?;
        store
            .delete_run_artifacts(*run_id, persisted_target)
            .map_err(|_| "无法安全删除全部本地原始数据；历史记录仍被保留。".to_string())?;
    }
    repository
        .delete_target_history(target, &expected)
        .map_err(|_| "原始数据已移除，但历史记录删除失败；可以安全重试。".to_string())
}

#[tauri::command]
pub async fn export_public_report(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ExportReportInput,
) -> Result<Option<String>, String> {
    let run_id = parse_run_id(&input.run_id)?;
    let selected = app
        .dialog()
        .file()
        .set_title("导出可分享报告")
        .add_filter("HTML report", &["html"])
        .set_file_name(default_report_file_name(run_id))
        .blocking_save_file();
    let destination = match selected {
        None => None,
        Some(selected) => Some(
            selected
                .into_path()
                .map_err(|_| "仅支持保存到本地 HTML 文件。".to_string())?,
        ),
    };
    export_report_to_selected_path_with_gate(
        &state.repository,
        &state.local_data_gate,
        run_id,
        destination,
    )
}

fn default_report_file_name(run_id: Uuid) -> String {
    let key = run_id.simple().to_string();
    format!("ability-radar-{}.html", &key[..8])
}

#[cfg(test)]
fn export_report_to_selected_path(
    repository: &RunRepository,
    run_id: Uuid,
    destination: Option<PathBuf>,
) -> Result<Option<String>, String> {
    export_report_to_selected_path_with_gate(
        repository,
        &LocalDataGate::default(),
        run_id,
        destination,
    )
}

fn export_report_to_selected_path_with_gate(
    repository: &RunRepository,
    gate: &LocalDataGate,
    run_id: Uuid,
    destination: Option<PathBuf>,
) -> Result<Option<String>, String> {
    let Some(destination) = destination else {
        return Ok(None);
    };
    let _local_data = gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    validate_export_destination(&destination)?;
    let run = repository
        .get_run(run_id)
        .map_err(|_| "无法读取这次体检，请稍后重试。".to_string())?
        .ok_or_else(|| "没有找到这次体检。".to_string())?;
    let tasks = repository
        .get_task_results(run_id)
        .map_err(|_| "无法读取这次体检，请稍后重试。".to_string())?;
    let report = ability_core::build_public_report(&run, &tasks).map_err(public_report_error)?;
    let html = ability_core::render_public_report_html(&report).map_err(public_report_error)?;
    write_new_report(&destination, html.as_bytes())?;
    let report_hash = ability_core::public_report_sha256(&html);
    let _ = repository.record_publication(report.report_id, run_id, &report_hash, "local_html");
    Ok(Some(report.report_id.to_string()))
}

#[tauri::command]
pub fn get_data_settings(state: State<'_, AppState>) -> Result<DataSettingsDto, String> {
    Ok(DataSettingsDto {
        raw_retention_days: state
            .repository
            .raw_retention_days()
            .map_err(|_| "无法读取本地数据设置，请稍后重试。".to_string())?,
        cleanup_pending: state.cleanup_pending.load(Ordering::SeqCst),
    })
}

#[tauri::command]
pub fn set_raw_retention(
    state: State<'_, AppState>,
    input: SetRetentionInput,
) -> Result<u32, String> {
    set_retention_for(
        &state.repository,
        &ArtifactStore::new(state.artifact_root.clone()),
        &state.run_operations,
        &state.local_data_gate,
        &state.cleanup_pending,
        input.raw_retention_days,
        chrono::Utc::now(),
    )
}

fn set_retention_for(
    repository: &RunRepository,
    store: &ArtifactStore,
    operations: &RunOperationRegistry,
    gate: &LocalDataGate,
    cleanup_pending: &AtomicBool,
    days: Option<u32>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<u32, String> {
    if !matches!(days, None | Some(7 | 30 | 90)) {
        return Err("保留期限只能是永久、7、30 或 90 天。".into());
    }
    let _local_data = gate
        .claim_mutating()
        .map_err(|_| "本地数据正在备份，请稍后重试。".to_string())?;
    repository
        .set_raw_retention_days(days)
        .map_err(|_| "无法保存本地数据设置，请稍后重试。".to_string())?;
    match crate::data_management::prune_expired_artifacts(repository, store, operations, now) {
        Ok(removed) => {
            cleanup_pending.store(false, Ordering::SeqCst);
            Ok(removed)
        }
        Err(_) => {
            cleanup_pending.store(true, Ordering::SeqCst);
            Err("保留期限已保存，但原始数据清理尚未完成，请稍后重试。".into())
        }
    }
}

#[tauri::command]
pub async fn export_full_backup(
    app: AppHandle,
    state: State<'_, AppState>,
    input: FullBackupInput,
) -> Result<bool, String> {
    if !input.acknowledged_unencrypted_raw_data {
        return Err("请先确认备份未加密并包含原始回答和日志。".into());
    }
    let selected = app
        .dialog()
        .file()
        .set_title("导出完整本地备份")
        .add_filter("ZIP backup", &["zip"])
        .set_file_name(format!(
            "ability-radar-full-backup-{}.zip",
            chrono::Utc::now().format("%Y%m%d")
        ))
        .blocking_save_file();
    let destination = match selected {
        None => None,
        Some(selected) => Some(
            selected
                .into_path()
                .map_err(|_| "请选择新的本地 ZIP 文件。".to_string())?,
        ),
    };
    export_full_backup_to_selected_path(
        &state.repository,
        &ArtifactStore::new(state.artifact_root.clone()),
        &state.run_operations,
        &state.local_data_gate,
        &state.app_data,
        destination,
        chrono::Utc::now(),
    )
}

fn export_full_backup_to_selected_path(
    repository: &RunRepository,
    store: &ArtifactStore,
    operations: &RunOperationRegistry,
    gate: &LocalDataGate,
    app_data: &Path,
    destination: Option<PathBuf>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, String> {
    let Some(destination) = destination else {
        return Ok(false);
    };
    validate_backup_destination(&destination, app_data)?;
    let _exclusive = gate
        .claim_exclusive()
        .map_err(|_| "本地数据正在变更，请稍后重试备份。".to_string())?;
    if operations.any_active() {
        return Err("本地数据正在变更，请稍后重试备份。".into());
    }
    if repository
        .has_running_runs()
        .map_err(|_| "无法安全检查本地数据状态，请稍后重试。".to_string())?
    {
        return Err("仍有体检正在运行，请结束后再备份。".into());
    }
    write_full_backup_to_destination(repository, store, app_data, &destination, now)?;
    Ok(true)
}

fn validate_backup_destination(destination: &Path, app_data: &Path) -> Result<(), String> {
    let is_zip = destination
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("zip"));
    if !destination.is_absolute()
        || !is_zip
        || validate_destination_platform(destination).is_err()
        || path_is_within(destination, app_data)
    {
        return Err("备份必须保存为应用数据目录之外的新本地 .zip 文件。".into());
    }
    Ok(())
}

fn path_is_within(path: &Path, parent: &Path) -> bool {
    fn components(path: &Path) -> Option<Vec<String>> {
        path.components()
            .map(|component| match component {
                std::path::Component::Prefix(value) => {
                    Some(value.as_os_str().to_string_lossy().to_ascii_lowercase())
                }
                std::path::Component::RootDir => Some(String::new()),
                std::path::Component::Normal(value) => {
                    Some(value.to_string_lossy().to_ascii_lowercase())
                }
                std::path::Component::CurDir | std::path::Component::ParentDir => None,
            })
            .collect()
    }
    match (components(path), components(parent)) {
        (Some(path), Some(parent)) => path.starts_with(&parent),
        _ => true,
    }
}

#[cfg(windows)]
fn write_full_backup_to_destination(
    repository: &RunRepository,
    store: &ArtifactStore,
    app_data: &Path,
    destination: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    write_new_backup(destination, app_data, |temporary| {
        windows_report_file::with_private_snapshot(app_data, |snapshot_path, snapshot_file| {
            crate::data_management::create_full_backup(
                repository,
                store,
                snapshot_path,
                snapshot_file,
                temporary,
                now,
                env!("CARGO_PKG_VERSION"),
            )
            .map_err(std::io::Error::other)
        })
    })
}

#[cfg(not(windows))]
fn write_full_backup_to_destination(
    _repository: &RunRepository,
    _store: &ArtifactStore,
    _app_data: &Path,
    _destination: &Path,
    _now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    Err("当前版本仅支持在 Windows 上安全导出完整备份。".into())
}

#[cfg(windows)]
fn write_new_backup<F>(destination: &Path, app_data: &Path, writer: F) -> Result<(), String>
where
    F: FnOnce(&mut fs::File) -> Result<(), windows_report_file::NativeWriteError>,
{
    let private_cleanup_incomplete = std::cell::Cell::new(false);
    let result = windows_report_file::write_new_file_outside(
        destination,
        app_data,
        |temporary| match writer(temporary) {
            Ok(()) => Ok(()),
            Err(windows_report_file::NativeWriteError::Operation(error)) => Err(error),
            Err(windows_report_file::NativeWriteError::CleanupIncomplete) => {
                private_cleanup_incomplete.set(true);
                Err(std::io::Error::other("private snapshot cleanup incomplete"))
            }
        },
        |_| {},
    );
    if private_cleanup_incomplete.get() {
        Err(map_backup_write_error(
            windows_report_file::NativeWriteError::CleanupIncomplete,
        ))
    } else {
        result.map_err(map_backup_write_error)
    }
}

#[cfg(windows)]
fn map_backup_write_error(error: windows_report_file::NativeWriteError) -> String {
    match error {
        windows_report_file::NativeWriteError::CleanupIncomplete => {
            "备份未完成，临时私密数据可能尚未清理；请关闭应用并联系支持。".into()
        }
        windows_report_file::NativeWriteError::Operation(_) => {
            "无法安全写入新的本地备份；请重新选择位置。".into()
        }
    }
}

fn public_report_error(error: ability_core::ReportError) -> String {
    match error {
        ability_core::ReportError::SensitiveText(field) => {
            format!("无法导出：公开字段 {field} 可能包含敏感信息。")
        }
        _ => "无法导出：本地结果不完整或不一致。".into(),
    }
}

fn validate_export_destination(destination: &Path) -> Result<(), String> {
    let html_extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("html"));
    if !destination.is_absolute()
        || !html_extension
        || validate_destination_platform(destination).is_err()
    {
        return Err("报告必须保存为本机上的新 .html 文件。".into());
    }

    Ok(())
}

#[cfg(windows)]
fn validate_destination_platform(destination: &Path) -> std::io::Result<()> {
    windows_report_file::validate_destination_path(destination)
}

#[cfg(not(windows))]
fn validate_destination_platform(destination: &Path) -> std::io::Result<()> {
    if destination.is_absolute() && !destination.as_os_str().to_string_lossy().starts_with("//") {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "report destination is not a local absolute path",
        ))
    }
}

#[cfg(windows)]
fn write_new_report(destination: &Path, contents: &[u8]) -> Result<(), String> {
    write_new_report_with(destination, |temporary| temporary.write_all(contents))
}

#[cfg(windows)]
fn write_new_report_with<F>(destination: &Path, writer: F) -> Result<(), String>
where
    F: FnOnce(&mut fs::File) -> std::io::Result<()>,
{
    windows_report_file::write_new_file(destination, writer, |_| {})
        .map_err(|_| "无法安全写入新的本机报告文件，请重新选择位置。".to_string())
}

#[cfg(all(test, windows))]
fn write_new_report_with_hook<F, H>(
    destination: &Path,
    writer: F,
    after_component_open: H,
) -> Result<(), String>
where
    F: FnOnce(&mut fs::File) -> std::io::Result<()>,
    H: FnMut(&Path),
{
    windows_report_file::write_new_file(destination, writer, after_component_open)
        .map_err(|_| "无法安全写入新的本机报告文件，请重新选择位置。".to_string())
}

#[cfg(not(windows))]
fn write_new_report(_destination: &Path, _contents: &[u8]) -> Result<(), String> {
    Err("当前版本仅支持在 Windows 上安全导出报告。".into())
}

#[cfg(windows)]
pub(crate) mod windows_report_file {
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::ptr::{null, null_mut};
    use uuid::Uuid;
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FileDispositionInformation, FileRenameInformation, NtCreateFile, NtSetInformationFile,
        FILE_CREATE, FILE_DIRECTORY_FILE, FILE_DISPOSITION_INFORMATION, FILE_NON_DIRECTORY_FILE,
        FILE_OPEN, FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{
        RtlNtStatusToDosError, INVALID_HANDLE_VALUE, OBJ_CASE_INSENSITIVE, STATUS_SUCCESS,
        UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::DELETE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetDriveTypeW, GetFileInformationByHandle, GetFinalPathNameByHandleW,
        BY_HANDLE_FILE_INFORMATION, FILE_ADD_FILE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, OPEN_EXISTING,
        SYNCHRONIZE,
    };
    use windows_sys::Win32::System::WindowsProgramming::{
        DRIVE_CDROM, DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOTE, DRIVE_REMOVABLE,
        DRIVE_UNKNOWN,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    const DIRECTORY_OPEN_ACCESS: u32 =
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE;
    const DIRECTORY_WRITE_ACCESS: u32 = DIRECTORY_OPEN_ACCESS | FILE_ADD_FILE;
    const DIRECTORY_SHARING_WITHOUT_DELETE: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;
    const REPORT_FILE_ACCESS: u32 =
        FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES | FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE;
    const REPORT_FILE_SHARE_NONE: u32 = 0;
    const PRIVATE_FILE_ACCESS: u32 = FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
    const PRIVATE_FILE_SHARING: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum DriveClass {
        WritableLocal,
        Remote,
        Unsupported,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(super) struct HandleSnapshot {
        pub(super) attributes: u32,
        pub(super) volume_serial_number: u32,
        pub(super) final_path: String,
        pub(super) file_index: u64,
    }

    #[derive(Debug)]
    pub(super) enum NativeWriteError {
        Operation(io::Error),
        CleanupIncomplete,
    }

    impl From<io::Error> for NativeWriteError {
        fn from(error: io::Error) -> Self {
            Self::Operation(error)
        }
    }

    impl std::fmt::Display for NativeWriteError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Operation(_) => formatter.write_str("native file operation failed"),
                Self::CleanupIncomplete => formatter.write_str(
                    "report write failed and the opened temporary handle could not be deleted",
                ),
            }
        }
    }

    impl std::error::Error for NativeWriteError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::Operation(error) => Some(error),
                Self::CleanupIncomplete => None,
            }
        }
    }

    struct OpenParent {
        _handles: Vec<File>,
        drive: u8,
        volume_serial_number: u32,
        final_path: String,
        final_name: OsString,
    }

    impl OpenParent {
        fn directory(&self) -> &File {
            self._handles
                .last()
                .expect("the destination parent handle is retained")
        }
    }

    pub(super) fn classify_drive_type(raw: u32) -> DriveClass {
        match raw {
            DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK => DriveClass::WritableLocal,
            DRIVE_REMOTE => DriveClass::Remote,
            DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR | DRIVE_CDROM => DriveClass::Unsupported,
            _ => DriveClass::Unsupported,
        }
    }

    pub(super) fn validate_destination_path(destination: &Path) -> io::Result<()> {
        let (drive, _) = local_drive_components(destination)?;
        require_writable_local_drive(drive)
    }

    pub(super) fn write_new_file<F, H>(
        destination: &Path,
        writer: F,
        after_component_open: H,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
        H: FnMut(&Path),
    {
        write_new_file_with_inspector(destination, writer, after_component_open, inspect_handle)
    }

    pub(super) fn write_new_file_outside<F, H>(
        destination: &Path,
        forbidden_directory: &Path,
        writer: F,
        after_component_open: H,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
        H: FnMut(&Path),
    {
        write_new_file_with_authority(
            destination,
            Some(forbidden_directory),
            writer,
            after_component_open,
            inspect_handle,
            delete_file_handle,
        )
    }

    pub(super) fn write_new_file_with_inspector<F, H, I>(
        destination: &Path,
        writer: F,
        after_component_open: H,
        inspector: I,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
        H: FnMut(&Path),
        I: FnMut(&File) -> io::Result<HandleSnapshot>,
    {
        write_new_file_with_inspector_and_cleanup(
            destination,
            writer,
            after_component_open,
            inspector,
            delete_file_handle,
        )
    }

    pub(super) fn write_new_file_with_inspector_and_cleanup<F, H, I, D>(
        destination: &Path,
        writer: F,
        after_component_open: H,
        inspector: I,
        cleanup: D,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
        H: FnMut(&Path),
        I: FnMut(&File) -> io::Result<HandleSnapshot>,
        D: FnMut(&File) -> io::Result<()>,
    {
        write_new_file_with_authority(
            destination,
            None,
            writer,
            after_component_open,
            inspector,
            cleanup,
        )
    }

    pub(super) fn with_private_snapshot<F>(
        app_data: &Path,
        operation: F,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&Path, &mut File) -> io::Result<()>,
    {
        with_private_snapshot_with_hooks(app_data, operation, |_| {}, delete_file_handle)
    }

    #[cfg(test)]
    pub(super) fn with_private_snapshot_and_cleanup<F, D>(
        app_data: &Path,
        operation: F,
        cleanup: D,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&Path, &mut File) -> io::Result<()>,
        D: FnMut(&File) -> io::Result<()>,
    {
        with_private_snapshot_with_hooks(app_data, operation, |_| {}, cleanup)
    }

    #[cfg(test)]
    pub(super) fn with_private_snapshot_with_release_hook<F, H>(
        app_data: &Path,
        operation: F,
        after_release: H,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&Path, &mut File) -> io::Result<()>,
        H: FnOnce(&Path),
    {
        with_private_snapshot_with_hooks(app_data, operation, after_release, delete_file_handle)
    }

    fn with_private_snapshot_with_hooks<F, H, D>(
        app_data: &Path,
        operation: F,
        after_release: H,
        mut cleanup: D,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&Path, &mut File) -> io::Result<()>,
        H: FnOnce(&Path),
        D: FnMut(&File) -> io::Result<()>,
    {
        let name = OsString::from(format!(
            ".ability-radar-backup-snapshot-{}.sqlite",
            Uuid::new_v4()
        ));
        let path = app_data.join(&name);
        let mut no_hook: fn(&Path) = |_| {};
        let parent = open_parent(&path, &mut no_hook)?;
        let mut snapshot_file = create_new_private_file(parent.directory(), &name)?;
        let opened = inspect_handle(&snapshot_file).and_then(|snapshot| {
            validate_private_snapshot(
                &snapshot,
                parent.drive,
                parent.volume_serial_number,
                &parent.final_path,
                &name,
            )?;
            Ok(snapshot)
        });
        let operation_result = match &opened {
            Ok(_) => operation(&path, &mut snapshot_file),
            Err(error) => Err(io::Error::new(error.kind(), "snapshot authority failed")),
        };
        let completed_result = match &opened {
            Ok(opened) => (|| {
                let completed = inspect_handle(&snapshot_file)?;
                validate_private_snapshot(
                    &completed,
                    parent.drive,
                    parent.volume_serial_number,
                    &parent.final_path,
                    &name,
                )?;
                if opened.file_index != completed.file_index {
                    return Err(unsafe_destination());
                }
                Ok(())
            })(),
            Err(_) => Err(unsafe_destination()),
        };
        let operation_result = operation_result.and(completed_result);
        let expected = opened.ok();
        drop(snapshot_file);
        after_release(&path);
        let cleanup_result = expected
            .ok_or_else(unsafe_destination)
            .and_then(|expected| {
                delete_private_file(
                    parent.directory(),
                    &name,
                    &expected,
                    parent.drive,
                    parent.volume_serial_number,
                    &parent.final_path,
                    &mut cleanup,
                )
            });
        match cleanup_result {
            Err(_) => Err(NativeWriteError::CleanupIncomplete),
            Ok(()) => operation_result.map_err(NativeWriteError::Operation),
        }
    }

    fn write_new_file_with_authority<F, H, I, D>(
        destination: &Path,
        forbidden_directory: Option<&Path>,
        writer: F,
        mut after_component_open: H,
        mut inspector: I,
        mut cleanup: D,
    ) -> Result<(), NativeWriteError>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
        H: FnMut(&Path),
        I: FnMut(&File) -> io::Result<HandleSnapshot>,
        D: FnMut(&File) -> io::Result<()>,
    {
        let parent = open_parent(destination, &mut after_component_open)?;
        let forbidden_parent = if let Some(forbidden_directory) = forbidden_directory {
            let boundary = forbidden_directory.join(".ability-radar-authority-boundary");
            let mut no_hook: fn(&Path) = |_| {};
            let forbidden_parent = open_parent(&boundary, &mut no_hook)?;
            if parent.volume_serial_number == forbidden_parent.volume_serial_number
                && final_path_is_within(&parent.final_path, &forbidden_parent.final_path)
            {
                return Err(unsafe_destination().into());
            }
            Some(forbidden_parent)
        } else {
            None
        };
        let temporary_name = OsString::from(format!(".ability-radar-{}.tmp", Uuid::new_v4()));
        let mut temporary = create_new_file(parent.directory(), &temporary_name)?;

        let snapshot = inspector(&temporary)
            .or_else(|error| fail_and_delete_with(&temporary, error, &mut cleanup))?;
        if let Err(error) = validate_opened_file_snapshot(
            &snapshot,
            parent.drive,
            parent.volume_serial_number,
            &parent.final_path,
            &temporary_name,
        ) {
            return fail_and_delete_with(&temporary, error, &mut cleanup);
        }

        if let Err(error) = writer(&mut temporary).and_then(|()| temporary.sync_all()) {
            return fail_and_delete_with(&temporary, error, &mut cleanup);
        }

        let before_publish = inspector(&temporary)
            .or_else(|error| fail_and_delete_with(&temporary, error, &mut cleanup))?;
        if let Err(error) = validate_opened_file_snapshot(
            &before_publish,
            parent.drive,
            parent.volume_serial_number,
            &parent.final_path,
            &temporary_name,
        ) {
            return fail_and_delete_with(&temporary, error, &mut cleanup);
        }

        if let Err(error) = rename_no_replace(
            &temporary,
            parent.directory(),
            parent.final_name.as_os_str(),
        ) {
            return fail_and_delete_with(&temporary, error, &mut cleanup);
        }

        let published = inspector(&temporary)
            .or_else(|error| fail_and_delete_with(&temporary, error, &mut cleanup))?;
        if let Err(error) = validate_opened_file_snapshot(
            &published,
            parent.drive,
            parent.volume_serial_number,
            &parent.final_path,
            &parent.final_name,
        ) {
            return fail_and_delete_with(&temporary, error, &mut cleanup);
        }
        drop(forbidden_parent);
        Ok(())
    }

    fn final_path_is_within(path: &str, directory: &str) -> bool {
        let path = path.trim_end_matches(['\\', '/']);
        let directory = directory.trim_end_matches(['\\', '/']);
        let Some(prefix) = path.get(..directory.len()) else {
            return false;
        };
        if !prefix.eq_ignore_ascii_case(directory) {
            return false;
        }
        path.len() == directory.len()
            || path
                .as_bytes()
                .get(directory.len())
                .is_some_and(|byte| is_separator_char(*byte))
    }

    fn open_parent<H>(destination: &Path, after_component_open: &mut H) -> io::Result<OpenParent>
    where
        H: FnMut(&Path),
    {
        let (drive, mut components) = local_drive_components(destination)?;
        require_writable_local_drive(drive)?;
        let final_name = components.pop().ok_or_else(unsafe_destination)?;
        let root_access = if components.is_empty() {
            DIRECTORY_WRITE_ACCESS
        } else {
            DIRECTORY_OPEN_ACCESS
        };
        let root = open_drive_root(drive, root_access)?;
        let root_snapshot = inspect_handle(&root)?;
        validate_snapshot_common(&root_snapshot, drive, root_snapshot.volume_serial_number)?;

        let volume_serial_number = root_snapshot.volume_serial_number;
        let mut parent_final_path = root_snapshot.final_path;
        let mut current_path = drive_root_path(drive);
        let mut handles = vec![root];
        let last_component_index = components.len().saturating_sub(1);
        for (index, component) in components.iter().enumerate() {
            let access = if index == last_component_index {
                DIRECTORY_WRITE_ACCESS
            } else {
                DIRECTORY_OPEN_ACCESS
            };
            let directory = open_directory(
                handles
                    .last()
                    .expect("the previous destination component handle is retained"),
                component,
                access,
            )?;
            let snapshot = inspect_handle(&directory)?;
            validate_opened_directory_snapshot(
                &snapshot,
                drive,
                volume_serial_number,
                &parent_final_path,
            )?;
            parent_final_path = snapshot.final_path;
            handles.push(directory);
            current_path.push(component);
            after_component_open(&current_path);
        }

        Ok(OpenParent {
            _handles: handles,
            drive,
            volume_serial_number,
            final_path: parent_final_path,
            final_name,
        })
    }

    fn local_drive_components(path: &Path) -> io::Result<(u8, Vec<OsString>)> {
        if has_dot_component(path) {
            return Err(unsafe_destination());
        }
        let mut components = path.components();
        let drive = match components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) => letter.to_ascii_uppercase(),
                _ => return Err(unsafe_destination()),
            },
            _ => return Err(unsafe_destination()),
        };
        if !matches!(components.next(), Some(Component::RootDir)) {
            return Err(unsafe_destination());
        }
        let mut normal = Vec::new();
        for component in components {
            let Component::Normal(value) = component else {
                return Err(unsafe_destination());
            };
            if value.is_empty() || value.to_string_lossy().contains(':') {
                return Err(unsafe_destination());
            }
            native_component_length(value)?;
            normal.push(value.to_os_string());
        }
        if normal.is_empty() {
            return Err(unsafe_destination());
        }
        Ok((drive, normal))
    }

    fn has_dot_component(path: &Path) -> bool {
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        let mut start = 0;
        for end in 0..=units.len() {
            if end != units.len() && units[end] != u16::from(b'\\') && units[end] != u16::from(b'/')
            {
                continue;
            }
            let component = &units[start..end];
            if component == [u16::from(b'.')] || component == [u16::from(b'.'), u16::from(b'.')] {
                return true;
            }
            start = end + 1;
        }
        false
    }

    fn require_writable_local_drive(drive: u8) -> io::Result<()> {
        let root = drive_root_path(drive);
        let wide = wide(root.as_os_str());
        // SAFETY: `wide` is NUL-terminated and remains valid for this synchronous call.
        let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
        match classify_drive_type(drive_type) {
            DriveClass::WritableLocal => Ok(()),
            DriveClass::Remote | DriveClass::Unsupported => Err(unsafe_destination()),
        }
    }

    fn drive_root_path(drive: u8) -> PathBuf {
        PathBuf::from(format!("{}:\\", char::from(drive)))
    }

    fn open_drive_root(drive: u8, access: u32) -> io::Result<File> {
        let root = drive_root_path(drive);
        let wide = wide(root.as_os_str());
        // SAFETY: `wide` is NUL-terminated and remains alive for the synchronous call.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                DIRECTORY_SHARING_WITHOUT_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateFileW returned a newly owned, non-invalid handle.
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }

    fn open_directory(parent: &File, name: &OsStr, access: u32) -> io::Result<File> {
        open_relative(
            parent,
            name,
            access,
            FILE_OPEN,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            DIRECTORY_SHARING_WITHOUT_DELETE,
        )
    }

    fn create_new_file(parent: &File, name: &OsStr) -> io::Result<File> {
        open_relative(
            parent,
            name,
            REPORT_FILE_ACCESS,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            REPORT_FILE_SHARE_NONE,
        )
    }

    fn create_new_private_file(parent: &File, name: &OsStr) -> io::Result<File> {
        open_relative(
            parent,
            name,
            PRIVATE_FILE_ACCESS,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            PRIVATE_FILE_SHARING,
        )
    }

    fn delete_private_file(
        parent: &File,
        name: &OsStr,
        expected: &HandleSnapshot,
        drive: u8,
        volume_serial_number: u32,
        parent_final_path: &str,
        cleanup: &mut impl FnMut(&File) -> io::Result<()>,
    ) -> io::Result<()> {
        let cleanup_handle = open_relative(
            parent,
            name,
            FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            REPORT_FILE_SHARE_NONE,
        )?;
        let cleanup_snapshot = inspect_handle(&cleanup_handle)?;
        validate_private_snapshot(
            &cleanup_snapshot,
            drive,
            volume_serial_number,
            parent_final_path,
            name,
        )?;
        if expected.file_index != cleanup_snapshot.file_index
            || expected.volume_serial_number != cleanup_snapshot.volume_serial_number
        {
            return Err(unsafe_destination());
        }
        cleanup(&cleanup_handle)
    }

    fn open_relative(
        parent: &File,
        name: &OsStr,
        access: u32,
        disposition: u32,
        options: u32,
        sharing: u32,
    ) -> io::Result<File> {
        let name_length = native_component_length(name)?;
        let mut name_storage = wide(name);
        let mut name = UNICODE_STRING {
            Length: name_length,
            MaximumLength: name_length,
            Buffer: name_storage.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle() as _,
            ObjectName: &mut name,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: null(),
            SecurityQualityOfService: null(),
        };
        let mut handle = INVALID_HANDLE_VALUE;
        // SAFETY: zero is a valid initial IO_STATUS_BLOCK state for synchronous NtCreateFile.
        let mut status = unsafe { zeroed::<IO_STATUS_BLOCK>() };
        // SAFETY: all referenced buffers and the root handle remain live for this synchronous call.
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &attributes,
                &mut status,
                null(),
                FILE_ATTRIBUTE_NORMAL,
                sharing,
                disposition,
                options,
                null(),
                0,
            )
        };
        if result != STATUS_SUCCESS {
            return Err(nt_error(result));
        }
        // SAFETY: NtCreateFile returned a newly owned handle on STATUS_SUCCESS.
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }

    pub(super) fn inspect_handle(file: &File) -> io::Result<HandleSnapshot> {
        // SAFETY: zero initialization is valid for this output-only Windows structure.
        let mut information = unsafe { zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        // SAFETY: `file` is live and `information` is a correctly sized writable buffer.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut final_path = vec![0_u16; 32_768];
        // SAFETY: the handle is live and the buffer is writable for the advertised length.
        let mut length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle() as _,
                final_path.as_mut_ptr(),
                final_path.len() as u32,
                0,
            )
        };
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length as usize >= final_path.len() {
            final_path.resize(length as usize + 1, 0);
            // SAFETY: the resized buffer is writable for the advertised length.
            length = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle() as _,
                    final_path.as_mut_ptr(),
                    final_path.len() as u32,
                    0,
                )
            };
            if length == 0 || length as usize >= final_path.len() {
                return Err(io::Error::last_os_error());
            }
        }
        let final_path =
            String::from_utf16(&final_path[..length as usize]).map_err(|_| unsafe_destination())?;
        Ok(HandleSnapshot {
            attributes: information.dwFileAttributes,
            volume_serial_number: information.dwVolumeSerialNumber,
            final_path,
            file_index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        })
    }

    fn validate_opened_directory_snapshot(
        snapshot: &HandleSnapshot,
        expected_drive: u8,
        expected_volume_serial_number: u32,
        expected_parent: &str,
    ) -> io::Result<()> {
        validate_snapshot_common(snapshot, expected_drive, expected_volume_serial_number)?;
        let (actual_parent, _) =
            split_final_path(&snapshot.final_path).ok_or_else(unsafe_destination)?;
        if same_windows_path(actual_parent, expected_parent) {
            Ok(())
        } else {
            Err(unsafe_destination())
        }
    }

    pub(super) fn validate_opened_file_snapshot<N>(
        snapshot: &HandleSnapshot,
        expected_drive: u8,
        expected_volume_serial_number: u32,
        expected_parent: &str,
        expected_name: N,
    ) -> io::Result<()>
    where
        N: AsRef<OsStr>,
    {
        validate_snapshot_common(snapshot, expected_drive, expected_volume_serial_number)?;
        let (actual_parent, actual_name) =
            split_final_path(&snapshot.final_path).ok_or_else(unsafe_destination)?;
        let expected_name = expected_name.as_ref().to_string_lossy();
        if same_windows_path(actual_parent, expected_parent)
            && actual_name.eq_ignore_ascii_case(&expected_name)
        {
            Ok(())
        } else {
            Err(unsafe_destination())
        }
    }

    fn validate_private_snapshot<N>(
        snapshot: &HandleSnapshot,
        expected_drive: u8,
        expected_volume_serial_number: u32,
        expected_parent: &str,
        expected_name: N,
    ) -> io::Result<()>
    where
        N: AsRef<OsStr>,
    {
        validate_opened_file_snapshot(
            snapshot,
            expected_drive,
            expected_volume_serial_number,
            expected_parent,
            expected_name,
        )?;
        if snapshot.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            Err(unsafe_destination())
        } else {
            Ok(())
        }
    }

    fn validate_snapshot_common(
        snapshot: &HandleSnapshot,
        expected_drive: u8,
        expected_volume_serial_number: u32,
    ) -> io::Result<()> {
        if snapshot.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || snapshot.volume_serial_number != expected_volume_serial_number
            || final_path_drive(&snapshot.final_path) != Some(expected_drive.to_ascii_uppercase())
        {
            Err(unsafe_destination())
        } else {
            Ok(())
        }
    }

    fn final_path_drive(path: &str) -> Option<u8> {
        let path = path
            .strip_prefix(r"\\?\")
            .or_else(|| path.strip_prefix(r"\??\"))
            .unwrap_or(path);
        let bytes = path.as_bytes();
        (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && is_separator_char(bytes[2]))
        .then(|| bytes[0].to_ascii_uppercase())
    }

    fn split_final_path(path: &str) -> Option<(&str, &str)> {
        let separator = path.rfind(['\\', '/'])?;
        let name = path.get(separator + 1..)?;
        (!name.is_empty()).then(|| (&path[..separator], name))
    }

    fn same_windows_path(left: &str, right: &str) -> bool {
        left.trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(right.trim_end_matches(['\\', '/']))
    }

    fn is_separator_char(byte: u8) -> bool {
        matches!(byte, b'\\' | b'/')
    }

    fn rename_no_replace(file: &File, directory: &File, name: &OsStr) -> io::Result<()> {
        let name_length = native_component_length(name)?;
        let mut storage = information_buffer::<FILE_RENAME_INFORMATION>(usize::from(name_length));
        let information = storage.as_mut_ptr() as *mut FILE_RENAME_INFORMATION;
        let name_utf16 = name.encode_wide().collect::<Vec<_>>();
        // SAFETY: `storage` is aligned and large enough for the trailing UTF-16 name.
        unsafe {
            (*information).Anonymous.ReplaceIfExists = false;
            (*information).RootDirectory = directory.as_raw_handle() as _;
            (*information).FileNameLength = u32::try_from(name_utf16.len() * size_of::<u16>())
                .map_err(|_| unsafe_destination())?;
            std::ptr::copy_nonoverlapping(
                name_utf16.as_ptr(),
                (*information).FileName.as_mut_ptr(),
                name_utf16.len(),
            );
        }
        set_information(
            file,
            information.cast(),
            storage.len() * size_of::<u64>(),
            FileRenameInformation,
        )
    }

    fn delete_file_handle(file: &File) -> io::Result<()> {
        let information = FILE_DISPOSITION_INFORMATION { DeleteFile: true };
        set_information(
            file,
            (&information as *const FILE_DISPOSITION_INFORMATION).cast(),
            size_of::<FILE_DISPOSITION_INFORMATION>(),
            FileDispositionInformation,
        )
    }

    fn fail_and_delete_with<T, D>(
        file: &File,
        primary: io::Error,
        cleanup: &mut D,
    ) -> Result<T, NativeWriteError>
    where
        D: FnMut(&File) -> io::Result<()>,
    {
        match cleanup(file) {
            Ok(()) => Err(NativeWriteError::Operation(primary)),
            Err(_) => Err(NativeWriteError::CleanupIncomplete),
        }
    }

    fn set_information(
        file: &File,
        information: *const core::ffi::c_void,
        length: usize,
        class: i32,
    ) -> io::Result<()> {
        // SAFETY: zero is a valid initial IO_STATUS_BLOCK state for synchronous requests.
        let mut status = unsafe { zeroed::<IO_STATUS_BLOCK>() };
        // SAFETY: the file and caller-provided information buffer remain live for this call.
        let result = unsafe {
            NtSetInformationFile(
                file.as_raw_handle() as _,
                &mut status,
                information,
                length as u32,
                class,
            )
        };
        if result == STATUS_SUCCESS {
            Ok(())
        } else {
            Err(nt_error(result))
        }
    }

    fn information_buffer<T>(name_utf16_length: usize) -> Vec<u64> {
        let name_bytes = name_utf16_length * size_of::<u16>();
        let bytes = size_of::<T>() + name_bytes.saturating_sub(size_of::<u16>());
        vec![0_u64; bytes.div_ceil(size_of::<u64>())]
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn native_component_length(value: &OsStr) -> io::Result<u16> {
        const MAX_WINDOWS_COMPONENT_UTF16: usize = 255;
        let units = value.encode_wide().count();
        if units == 0 || units > MAX_WINDOWS_COMPONENT_UTF16 {
            return Err(unsafe_destination());
        }
        let bytes = units
            .checked_mul(size_of::<u16>())
            .ok_or_else(unsafe_destination)?;
        u16::try_from(bytes).map_err(|_| unsafe_destination())
    }

    fn unsafe_destination() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "report destination failed local handle validation",
        )
    }

    fn nt_error(status: i32) -> io::Error {
        // SAFETY: RtlNtStatusToDosError is a pure conversion for the supplied NTSTATUS.
        io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
    }
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
    use ability_adapters::{
        AdapterCompletion, AdapterError, AuthState, CliRunService, ExecutionRequest,
        PrerequisiteStatus, ProcessError, ProcessOutput, ProcessRunner, ProcessSpec,
        TargetAvailability,
    };
    use ability_core::{
        summarize_scores, Category, EnvironmentFingerprint, FailureKind, LoadedPack,
        ManualRunService, PackLoader, RunMode, RunRecord, RunRepository, RunStatus, TargetKind,
        TargetSelection, TaskOutcome, TaskResult,
    };
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

    struct CountingAdapter {
        detect_calls: AtomicUsize,
        execute_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl AgentAdapter for CountingAdapter {
        fn kind(&self) -> TargetKind {
            TargetKind::CodexCli
        }

        async fn detect(&self) -> TargetAvailability {
            self.detect_calls.fetch_add(1, Ordering::SeqCst);
            TargetAvailability {
                kind: TargetKind::CodexCli,
                installed: true,
                version: Some("codex-cli 1.2.3".into()),
                auth_state: AuthState::Ready,
                prerequisites: Vec::new(),
            }
        }

        async fn execute(
            &self,
            _request: ExecutionRequest,
            _cancellation: CancellationToken,
        ) -> Result<AdapterCompletion, AdapterError> {
            self.execute_calls.fetch_add(1, Ordering::SeqCst);
            Ok(AdapterCompletion::Completed {
                duration_ms: 1,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    struct CountingRunner {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ProcessRunner for CountingRunner {
        async fn run(
            &self,
            _spec: ProcessSpec,
            _cancellation: CancellationToken,
        ) -> Result<ProcessOutput, ProcessError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: "v22.0.0".into(),
                stderr: String::new(),
                duration_ms: 1,
            })
        }
    }

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
    fn manual_reasoning_accepts_all_known_values_and_preserves_custom_labels() {
        for value in [
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ] {
            let padded = format!(" {value} ");
            let start = validate_start(
                start_input(
                    TargetKind::ChatGptClient,
                    "GPT-5.6",
                    Some(&padded),
                    RunMode::Quick,
                ),
                StartFamily::Manual,
            )
            .unwrap();
            assert_eq!(start.target.reasoning_effort.as_deref(), Some(value));
        }

        let custom = validate_start(
            start_input(
                TargetKind::ClaudeClient,
                "Claude",
                Some("  \u{6269}\u{5c55}\u{601d}\u{8003}\u{ff08}\u{5b9e}\u{9a8c}\u{ff09} "),
                RunMode::Quick,
            ),
            StartFamily::Manual,
        )
        .unwrap();
        assert_eq!(
            custom.target.reasoning_effort.as_deref(),
            Some("\u{6269}\u{5c55}\u{601d}\u{8003}\u{ff08}\u{5b9e}\u{9a8c}\u{ff09}")
        );
    }

    #[test]
    fn cli_reasoning_accepts_known_and_safe_custom_tokens() {
        for value in [
            "none",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh",
            "max",
            "ultra",
            "frontier_2",
            "deep-preview",
        ] {
            let padded = format!(" {value} ");
            let start = validate_start(
                start_input(
                    TargetKind::CodexCli,
                    "default",
                    Some(&padded),
                    RunMode::Quick,
                ),
                StartFamily::Cli,
            )
            .unwrap();
            assert_eq!(start.target.reasoning_effort.as_deref(), Some(value));
        }
    }

    #[test]
    fn reasoning_rejects_control_overflow_and_unsafe_cli_values() {
        let manual_overflow = "x".repeat(41);
        for value in ["bad\nvalue".to_owned(), manual_overflow] {
            assert!(validate_start(
                start_input(
                    TargetKind::ChatGptClient,
                    "GPT",
                    Some(&value),
                    RunMode::Quick,
                ),
                StartFamily::Manual,
            )
            .is_err());
        }

        let cli_overflow = "a".repeat(33);
        for value in [
            "\u{6781}\u{9ad8}",
            "high;calc",
            "high value",
            cli_overflow.as_str(),
        ] {
            assert!(validate_start(
                start_input(TargetKind::CodexCli, "default", Some(value), RunMode::Quick,),
                StartFamily::Cli,
            )
            .is_err());
        }
    }

    #[test]
    fn manual_reasoning_requires_visible_text_and_rejects_unicode_invisibles() {
        for value in [
            "\u{0}",
            "\u{200b}",
            "\u{202e}",
            "\u{2060}",
            " \u{200b} ",
            "\u{53ef}\u{200b}\u{89c1}",
        ] {
            assert!(
                normalize_reasoning_effort(Some(value.into()), StartFamily::Manual).is_err(),
                "{value:?}"
            );
        }

        for value in [
            "\u{6269}\u{5c55}\u{601d}\u{8003}\u{ff08}\u{5b9e}\u{9a8c}\u{ff09}",
            "é",
        ] {
            assert_eq!(
                normalize_reasoning_effort(Some(value.into()), StartFamily::Manual)
                    .unwrap()
                    .as_deref(),
                Some(value)
            );
        }
        assert!(normalize_reasoning_effort(Some(" ".into()), StartFamily::Manual).is_err());
        assert!(normalize_reasoning_effort(Some("想".repeat(40)), StartFamily::Manual).is_ok());
        assert!(normalize_reasoning_effort(Some("想".repeat(41)), StartFamily::Manual).is_err());
    }

    #[test]
    fn resume_requires_the_already_normalized_family_specific_value() {
        let valid = validate_resume_target(
            ResumeTargetSelectionInput {
                kind: TargetKind::ClaudeClient,
                reported_model: "Claude".into(),
                reasoning_effort: Some("\u{6269}\u{5c55}\u{601d}\u{8003}".into()),
            },
            StartFamily::Manual,
        )
        .unwrap();
        assert_eq!(
            valid.reasoning_effort.as_deref(),
            Some("\u{6269}\u{5c55}\u{601d}\u{8003}")
        );

        for value in [" XHIGH ", "high;calc"] {
            assert!(validate_resume_target(
                ResumeTargetSelectionInput {
                    kind: TargetKind::CodexCli,
                    reported_model: "default".into(),
                    reasoning_effort: Some(value.into()),
                },
                StartFamily::Cli,
            )
            .is_err());
        }
    }

    #[test]
    fn reported_model_requires_visible_unicode_for_start_and_resume() {
        let invalid_models = [
            "\u{200b}",
            "\u{202e}",
            "\u{2060}",
            "\u{200b}\u{2060}",
            "GPT\u{200b}-5",
        ];
        for model in invalid_models {
            assert!(
                validate_start(
                    start_input(TargetKind::ChatGptClient, model, None, RunMode::Quick,),
                    StartFamily::Manual,
                )
                .is_err(),
                "start accepted {model:?}"
            );
            assert!(
                validate_resume_target(
                    ResumeTargetSelectionInput {
                        kind: TargetKind::ChatGptClient,
                        reported_model: model.into(),
                        reasoning_effort: None,
                    },
                    StartFamily::Manual,
                )
                .is_err(),
                "resume accepted {model:?}"
            );
        }

        for model in ["模型-α".to_owned(), "模".repeat(120)] {
            assert!(
                validate_start(
                    start_input(TargetKind::ChatGptClient, &model, None, RunMode::Quick,),
                    StartFamily::Manual,
                )
                .is_ok(),
                "start rejected {model:?}"
            );
            assert!(
                validate_resume_target(
                    ResumeTargetSelectionInput {
                        kind: TargetKind::ChatGptClient,
                        reported_model: model.clone(),
                        reasoning_effort: None,
                    },
                    StartFamily::Manual,
                )
                .is_ok(),
                "resume rejected {model:?}"
            );
        }

        let too_long = "模".repeat(121);
        assert!(validate_start(
            start_input(TargetKind::ChatGptClient, &too_long, None, RunMode::Quick,),
            StartFamily::Manual,
        )
        .is_err());
        assert!(validate_resume_target(
            ResumeTargetSelectionInput {
                kind: TargetKind::ChatGptClient,
                reported_model: too_long,
                reasoning_effort: None,
            },
            StartFamily::Manual,
        )
        .is_err());
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
        for effort in ["high\n", "médiúm"] {
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

    #[tokio::test]
    async fn cli_resume_target_mismatches_call_no_adapter_probe_spawn_or_registration() {
        let mismatches: [fn(&mut TargetSelection); 3] = [
            |target| target.kind = TargetKind::ClaudeCode,
            |target| target.reported_model = "changed-model".into(),
            |target| target.reasoning_effort = Some("high".into()),
        ];

        for mutate in mismatches {
            let directory = tempdir().unwrap();
            let repository =
                Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
            let artifact_root = directory.path().join("artifacts");
            let service = CliRunService::new(repository.clone(), artifact_root.clone());
            let pack = cli_pack();
            let run = insert_run(&repository, RunStatus::Running);
            repository
                .finish_without_score(run.id, RunStatus::Interrupted)
                .unwrap();
            let mut expected_target = run.target.clone();
            mutate(&mut expected_target);

            let adapter = Arc::new(CountingAdapter {
                detect_calls: AtomicUsize::new(0),
                execute_calls: AtomicUsize::new(0),
            });
            let mut adapters = BTreeMap::<TargetKind, Arc<dyn AgentAdapter>>::new();
            adapters.insert(TargetKind::CodexCli, adapter.clone());
            let runner = Arc::new(CountingRunner {
                calls: AtomicUsize::new(0),
            });
            let cancellations = CancellationRegistry::default();
            let spawn_calls = Arc::new(AtomicUsize::new(0));
            let spawn_counter = spawn_calls.clone();

            let result = resume_cli_run_with(
                CliResumeContext {
                    repository: &repository,
                    service: &service,
                    pack,
                    adapters: &adapters,
                    runner: runner.clone(),
                    cancellations: &cancellations,
                },
                run.id,
                expected_target,
                move |_, _| {
                    spawn_counter.fetch_add(1, Ordering::SeqCst);
                },
            )
            .await;

            assert!(result.is_err());
            assert_eq!(adapter.detect_calls.load(Ordering::SeqCst), 0);
            assert_eq!(adapter.execute_calls.load(Ordering::SeqCst), 0);
            assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
            assert_eq!(spawn_calls.load(Ordering::SeqCst), 0);
            assert!(!cancellations.cancel(run.id));
            assert_eq!(
                repository.get_run(run.id).unwrap().unwrap().status,
                RunStatus::Interrupted
            );
            assert!(!artifact_root.exists());
        }
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
    fn background_errors_interrupt_only_running_rows_and_expose_only_safe_copy() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let running = insert_run(&repository, RunStatus::Running);

        let event =
            finish_background(&repository, running.id, Err("primary failure".into())).unwrap();

        assert_eq!(event.message, SAFE_BACKGROUND_ERROR);
        assert!(!event.message.contains("primary failure"));
        assert_eq!(
            repository.get_run(running.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );

        let completed = insert_run(&repository, RunStatus::Completed);
        let event =
            finish_background(&repository, completed.id, Err("late failure".into())).unwrap();
        assert_eq!(event.message, SAFE_BACKGROUND_ERROR);
        assert!(!event.message.contains("late failure"));
        assert_eq!(
            repository.get_run(completed.id).unwrap().unwrap().status,
            RunStatus::Completed
        );
    }

    #[test]
    fn background_error_never_exposes_secondary_terminalization_details() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let missing = uuid::Uuid::new_v4();

        let event = finish_background(&repository, missing, Err("primary failure".into())).unwrap();

        assert_eq!(event.message, SAFE_BACKGROUND_ERROR);
        assert!(!event.message.contains("primary failure"));
        assert!(!event.message.contains("terminalization"));
        assert!(!event.message.contains(&missing.to_string()));
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

    #[test]
    fn raw_delete_is_retryable_when_database_reference_cleanup_is_injected_to_fail() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("runs.db");
        let repository = RunRepository::open(&database).unwrap();
        let run = insert_run(&repository, RunStatus::Running);
        let relative = format!("runs/{}/logs/dedupe-events.log", run.id);
        repository
            .save_task_result(&TaskResult {
                run_id: run.id,
                task_id: "dedupe-events".into(),
                category: Category::CliCoding,
                outcome: TaskOutcome::Passed,
                score: Some(100.0),
                failure_kind: None,
                duration_ms: 1,
                answer_rel_path: Some(relative),
                detail: "pass".into(),
            })
            .unwrap();
        repository
            .finish_without_score(run.id, RunStatus::Interrupted)
            .unwrap();
        let artifact_root = directory.path().join("artifacts");
        let raw = artifact_root
            .join("runs")
            .join(run.id.to_string())
            .join("logs/dedupe-events.log");
        std::fs::create_dir_all(raw.parent().unwrap()).unwrap();
        std::fs::write(&raw, "private CLI log").unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_clear BEFORE UPDATE OF answer_rel_path ON task_results
                 BEGIN SELECT RAISE(ABORT, 'C:\\Users\\Alice\\secret'); END;",
            )
            .unwrap();
        drop(connection);
        let store = ability_core::ArtifactStore::new(artifact_root);
        let operations = RunOperationRegistry::default();

        let error = delete_raw_artifacts_for(&repository, &store, &operations, run.id).unwrap_err();
        assert!(!error.contains("Alice"));
        assert!(!raw.exists());
        assert!(repository.get_task_results(run.id).unwrap()[0]
            .answer_rel_path
            .is_some());

        let connection = rusqlite::Connection::open(database).unwrap();
        connection.execute_batch("DROP TRIGGER fail_clear").unwrap();
        drop(connection);
        delete_raw_artifacts_for(&repository, &store, &operations, run.id).unwrap();
        assert_eq!(
            repository.get_task_results(run.id).unwrap()[0].answer_rel_path,
            None
        );
    }

    #[test]
    fn destructive_helpers_reject_active_and_stale_confirmations_before_file_deletion() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let active = insert_run(&repository, RunStatus::Running);
        let artifact_root = directory.path().join("artifacts");
        let active_raw = artifact_root
            .join("runs")
            .join(active.id.to_string())
            .join("logs/active.log");
        std::fs::create_dir_all(active_raw.parent().unwrap()).unwrap();
        std::fs::write(&active_raw, "active").unwrap();
        let store = ability_core::ArtifactStore::new(artifact_root.clone());
        let operations = RunOperationRegistry::default();

        assert!(delete_run_for(&repository, &store, &operations, active.id).is_err());
        assert!(active_raw.exists());

        repository
            .finish_without_score(active.id, RunStatus::Interrupted)
            .unwrap();
        let recovery_claim = operations.claim([active.id]).unwrap();
        assert!(
            delete_run_for(&repository, &store, &operations, active.id).is_err(),
            "a recovery claim must reject deletion before artifact access"
        );
        assert!(active_raw.exists());
        drop(recovery_claim);

        let new_run = insert_run(&repository, RunStatus::Running);
        repository
            .finish_without_score(new_run.id, RunStatus::Interrupted)
            .unwrap();
        let new_raw = artifact_root
            .join("runs")
            .join(new_run.id.to_string())
            .join("logs/new.log");
        std::fs::create_dir_all(new_raw.parent().unwrap()).unwrap();
        std::fs::write(&new_raw, "new").unwrap();

        let one_batch_member = operations.claim([new_run.id]).unwrap();
        assert!(
            delete_target_history_for(
                &repository,
                &store,
                &operations,
                TargetKind::CodexCli,
                &[active.id, new_run.id],
            )
            .is_err(),
            "a conflicting batch claim must reject before touching any artifact"
        );
        assert!(active_raw.exists());
        assert!(new_raw.exists());
        drop(one_batch_member);

        assert!(delete_target_history_for(
            &repository,
            &store,
            &operations,
            TargetKind::CodexCli,
            &[active.id],
        )
        .is_err());
        assert!(active_raw.exists());
        assert!(new_raw.exists());
        assert!(repository.get_run(active.id).unwrap().is_some());
        assert!(repository.get_run(new_run.id).unwrap().is_some());
    }

    #[test]
    fn cancelling_the_native_selection_short_circuits_before_loading_or_generating() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let missing = Uuid::new_v4();
        let before = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();

        let result = export_report_to_selected_path(&repository, missing, None).unwrap();
        let after = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();

        assert_eq!(result, None);
        assert_eq!(after, before, "cancellation must not create a report file");
    }

    #[test]
    fn retention_policy_remains_effective_and_reports_cleanup_pending_after_safe_failure() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-19T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut expired = insert_run(&repository, RunStatus::Completed);
        expired.finished_at = Some(now - chrono::Duration::days(8));
        let connection = rusqlite::Connection::open(directory.path().join("ability.db")).unwrap();
        connection
            .execute(
                "UPDATE runs SET finished_at=?2 WHERE id=?1",
                rusqlite::params![
                    expired.id.to_string(),
                    expired.finished_at.unwrap().to_rfc3339()
                ],
            )
            .unwrap();
        let artifact_root = directory.path().join("artifacts");
        let hostile = artifact_root.join("runs").join(expired.id.to_string());
        std::fs::create_dir_all(&hostile).unwrap();
        std::fs::write(hostile.join("owner.bin"), "unsafe layout").unwrap();
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let error = set_retention_for(
            &repository,
            &ArtifactStore::new(artifact_root),
            &RunOperationRegistry::default(),
            &crate::app_state::LocalDataGate::default(),
            &pending,
            Some(7),
            now,
        )
        .unwrap_err();
        assert_eq!(repository.raw_retention_days().unwrap(), Some(7));
        assert!(pending.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            error,
            "保留期限已保存，但原始数据清理尚未完成，请稍后重试。"
        );
        assert!(!error.contains(directory.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn next_manual_step_enters_the_global_gate_before_the_service_can_complete_a_run() {
        let directory = tempdir().unwrap();
        let repository =
            Arc::new(RunRepository::open(&directory.path().join("ability.db")).unwrap());
        let service = ManualRunService::new(repository, directory.path().join("artifacts"));
        let gate = crate::app_state::LocalDataGate::default();
        let backup = gate.claim_exclusive().unwrap();

        let error = next_manual_step_for(
            &service,
            &RunOperationRegistry::default(),
            &gate,
            Uuid::new_v4(),
        )
        .unwrap_err();

        assert_eq!(error, "本地数据正在备份，请稍后重试。");
        drop(backup);
    }

    #[test]
    fn manual_cancel_uses_the_exact_run_claim_and_respects_the_backup_gate() {
        let directory = tempdir().unwrap();
        let repository =
            Arc::new(RunRepository::open(&directory.path().join("ability.db")).unwrap());
        let pack = Arc::new(
            PackLoader::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../benchmark-packs/client-quick-v1"),
            )
            .unwrap(),
        );
        let service = ManualRunService::new(repository.clone(), directory.path().join("artifacts"));
        let run = service
            .start(
                pack.clone(),
                TargetSelection {
                    kind: TargetKind::ChatGptClient,
                    reported_model: "GPT-5".into(),
                    reasoning_effort: None,
                },
                RunMode::Quick,
                environment(&pack, None, None),
            )
            .unwrap();
        let cancellations = CancellationRegistry::default();
        let operations = RunOperationRegistry::default();
        let gate = LocalDataGate::default();

        let operation = operations.claim([run.id]).unwrap();
        assert!(cancel_run_for(&cancellations, &service, &operations, &gate, run.id).is_err());
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Running
        );
        drop(operation);

        let backup = gate.claim_exclusive().unwrap();
        assert!(cancel_run_for(&cancellations, &service, &operations, &gate, run.id).is_err());
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Running
        );
        drop(backup);

        assert!(cancel_run_for(&cancellations, &service, &operations, &gate, run.id).unwrap());
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Cancelled
        );
        assert!(!repository.has_running_runs().unwrap());
        assert!(matches!(
            service.resume(
                run.id,
                run.target.clone(),
                pack.clone(),
                environment(&pack, None, None),
            ),
            Err(ability_core::RunServiceError::NotResumable(_))
        ));
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Cancelled
        );
        #[cfg(windows)]
        {
            let output = tempdir().unwrap();
            let destination = output.path().join("post-cancel-backup.zip");
            assert!(export_full_backup_to_selected_path(
                &repository,
                &ArtifactStore::new(directory.path().join("artifacts")),
                &operations,
                &gate,
                directory.path(),
                Some(destination.clone()),
                chrono::Utc::now(),
            )
            .unwrap());
            assert!(destination.exists());
        }
    }

    #[test]
    fn manual_interrupt_uses_the_exact_run_claim_and_preserves_recovery() {
        let directory = tempdir().unwrap();
        let repository =
            Arc::new(RunRepository::open(&directory.path().join("ability.db")).unwrap());
        let pack = Arc::new(
            PackLoader::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../benchmark-packs/client-quick-v1"),
            )
            .unwrap(),
        );
        let service = ManualRunService::new(repository.clone(), directory.path().join("artifacts"));
        let run = service
            .start(
                pack.clone(),
                TargetSelection {
                    kind: TargetKind::ChatGptClient,
                    reported_model: "GPT-5".into(),
                    reasoning_effort: None,
                },
                RunMode::Quick,
                environment(&pack, None, None),
            )
            .unwrap();
        let operations = RunOperationRegistry::default();
        let gate = LocalDataGate::default();

        let operation = operations.claim([run.id]).unwrap();
        assert!(interrupt_manual_run_for(&service, &operations, &gate, run.id).is_err());
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Running
        );
        drop(operation);

        let backup = gate.claim_exclusive().unwrap();
        assert!(interrupt_manual_run_for(&service, &operations, &gate, run.id).is_err());
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Running
        );
        drop(backup);

        assert!(interrupt_manual_run_for(&service, &operations, &gate, run.id).unwrap());
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );
        assert!(!repository.has_running_runs().unwrap());
        assert!(service
            .resume(
                run.id,
                run.target,
                pack.clone(),
                environment(&pack, None, None),
            )
            .is_ok());
    }

    #[test]
    fn cli_cancel_signals_only_the_exact_registered_token() {
        let directory = tempdir().unwrap();
        let repository =
            Arc::new(RunRepository::open(&directory.path().join("ability.db")).unwrap());
        let service = ManualRunService::new(repository, directory.path().join("artifacts"));
        let cancellations = CancellationRegistry::default();
        let selected = Uuid::new_v4();
        let other = Uuid::new_v4();
        let selected_token = CancellationToken::new();
        let other_token = CancellationToken::new();
        let _selected = cancellations
            .register(selected, selected_token.clone())
            .unwrap();
        let _other = cancellations.register(other, other_token.clone()).unwrap();

        assert!(cancel_run_for(
            &cancellations,
            &service,
            &RunOperationRegistry::default(),
            &LocalDataGate::default(),
            selected,
        )
        .unwrap());
        assert!(selected_token.is_cancelled());
        assert!(!other_token.is_cancelled());
    }

    #[test]
    fn manual_submit_respects_the_exact_run_operation_claim() {
        let directory = tempdir().unwrap();
        let repository =
            Arc::new(RunRepository::open(&directory.path().join("ability.db")).unwrap());
        let pack = Arc::new(
            PackLoader::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../benchmark-packs/client-quick-v1"),
            )
            .unwrap(),
        );
        let service = ManualRunService::new(repository.clone(), directory.path().join("artifacts"));
        let run = service
            .start(
                pack.clone(),
                TargetSelection {
                    kind: TargetKind::ChatGptClient,
                    reported_model: "GPT-5".into(),
                    reasoning_effort: None,
                },
                RunMode::Quick,
                environment(&pack, None, None),
            )
            .unwrap();
        let operations = RunOperationRegistry::default();
        let _delete = operations.claim([run.id]).unwrap();

        assert!(submit_manual_answer_for(
            &service,
            &operations,
            &LocalDataGate::default(),
            run.id,
            &pack.tasks[0].definition.id,
            "answer",
        )
        .is_err());
        assert!(repository.get_task_results(run.id).unwrap().is_empty());
    }

    #[test]
    fn backup_cancel_and_busy_checks_create_no_destination_or_gate_claim_window() {
        let app_data = tempdir().unwrap();
        let output = tempdir().unwrap();
        let repository = RunRepository::open(&app_data.path().join("ability.db")).unwrap();
        let store = ArtifactStore::new(app_data.path().join("artifacts"));
        let operations = RunOperationRegistry::default();
        let gate = crate::app_state::LocalDataGate::default();
        let mutation = gate.claim_mutating().unwrap();

        assert!(!export_full_backup_to_selected_path(
            &repository,
            &store,
            &operations,
            &gate,
            app_data.path(),
            None,
            chrono::Utc::now(),
        )
        .unwrap());
        drop(mutation);

        let run_id = Uuid::new_v4();
        let operation = operations.claim([run_id]).unwrap();
        let destination = output.path().join("backup.zip");
        let error = export_full_backup_to_selected_path(
            &repository,
            &store,
            &operations,
            &gate,
            app_data.path(),
            Some(destination.clone()),
            chrono::Utc::now(),
        )
        .unwrap_err();
        assert_eq!(error, "本地数据正在变更，请稍后重试备份。");
        assert!(!destination.exists());
        drop(operation);
        assert!(gate.claim_mutating().is_ok());
    }

    #[test]
    fn backup_rejects_running_run_before_opening_destination() {
        let app_data = tempdir().unwrap();
        let output = tempdir().unwrap();
        let repository = RunRepository::open(&app_data.path().join("ability.db")).unwrap();
        insert_run(&repository, RunStatus::Running);
        let destination = output.path().join("backup.zip");

        let error = export_full_backup_to_selected_path(
            &repository,
            &ArtifactStore::new(app_data.path().join("artifacts")),
            &RunOperationRegistry::default(),
            &crate::app_state::LocalDataGate::default(),
            app_data.path(),
            Some(destination.clone()),
            chrono::Utc::now(),
        )
        .unwrap_err();
        assert_eq!(error, "仍有体检正在运行，请结束后再备份。");
        assert!(!destination.exists());
    }

    #[cfg(windows)]
    #[test]
    fn backup_streams_through_the_retained_writer_and_publishes_only_the_final_zip() {
        let app_data = tempdir().unwrap();
        let output = tempdir().unwrap();
        let repository = RunRepository::open(&app_data.path().join("ability.db")).unwrap();
        let destination = output.path().join("backup.zip");

        assert!(export_full_backup_to_selected_path(
            &repository,
            &ArtifactStore::new(app_data.path().join("artifacts")),
            &RunOperationRegistry::default(),
            &crate::app_state::LocalDataGate::default(),
            app_data.path(),
            Some(destination.clone()),
            chrono::Utc::now(),
        )
        .unwrap());

        assert!(std::fs::read(&destination)
            .unwrap()
            .starts_with(b"PK\x03\x04"));
        assert_eq!(
            std::fs::read_dir(output.path()).unwrap().count(),
            1,
            "successful publication must leave no randomized destination temporary"
        );
        assert!(std::fs::read_dir(app_data.path()).unwrap().all(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            !name.starts_with(".ability-radar-backup-snapshot-") && !name.ends_with("-journal")
        }));
    }

    #[cfg(windows)]
    #[test]
    fn backup_writer_rejects_a_handle_bound_app_data_parent_before_creating_a_temporary() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let app_data = tempdir().unwrap();
        let destination = app_data.path().join("backup.zip");
        let writer_called = AtomicBool::new(false);

        let result = windows_report_file::write_new_file_outside(
            &destination,
            app_data.path(),
            |_| {
                writer_called.store(true, Ordering::SeqCst);
                Ok(())
            },
            |_| {},
        );

        assert!(result.is_err());
        assert!(!writer_called.load(Ordering::SeqCst));
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(app_data.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn unsafe_artifact_enumeration_removes_the_destination_temporary_before_returning() {
        let app_data = tempdir().unwrap();
        let output = tempdir().unwrap();
        let repository = RunRepository::open(&app_data.path().join("ability.db")).unwrap();
        let run = insert_exportable_run(&repository, "safe-model");
        let hostile = app_data
            .path()
            .join("artifacts/runs")
            .join(run.id.to_string());
        std::fs::create_dir_all(&hostile).unwrap();
        std::fs::write(hostile.join("owner.bin"), "private attacker-shaped bytes").unwrap();
        let destination = output.path().join("backup.zip");

        let error = export_full_backup_to_selected_path(
            &repository,
            &ArtifactStore::new(app_data.path().join("artifacts")),
            &RunOperationRegistry::default(),
            &crate::app_state::LocalDataGate::default(),
            app_data.path(),
            Some(destination.clone()),
            chrono::Utc::now(),
        )
        .unwrap_err();

        assert_eq!(error, "无法安全写入新的本地备份；请重新选择位置。");
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 0);
        assert!(std::fs::read_dir(app_data.path()).unwrap().all(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            !name.starts_with(".ability-radar-backup-snapshot-") && !name.ends_with("-journal")
        }));
        assert!(!error.contains(app_data.path().to_string_lossy().as_ref()));
        assert!(!error.contains("owner.bin"));
    }

    #[test]
    fn publication_audit_failure_after_safe_write_is_explicitly_best_effort() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("ability.db");
        let repository = RunRepository::open(&database).unwrap();
        let run = insert_exportable_run(&repository, "safe-model");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_publication BEFORE INSERT ON publications
                 BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;",
            )
            .unwrap();
        let destination = directory.path().join("published.html");

        let result = export_report_to_selected_path_with_gate(
            &repository,
            &crate::app_state::LocalDataGate::default(),
            run.id,
            Some(destination.clone()),
        )
        .unwrap();
        assert!(result.is_some());
        assert!(destination.exists());
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM publications", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn export_command_helper_writes_only_a_privacy_checked_offline_report() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let run = insert_exportable_run(&repository, "safe-model");
        let destination = directory.path().join("public-report.html");

        let report_id =
            export_report_to_selected_path(&repository, run.id, Some(destination.clone()))
                .unwrap()
                .unwrap();
        let html = std::fs::read_to_string(destination).unwrap();

        assert_ne!(report_id, run.id.to_string());
        assert!(!html.contains(&run.id.to_string()));
        assert!(!html.contains("private raw answer"));
        assert!(!html.contains(r"C:\Users\Alice"));
        assert!(!html.contains("src=\"http"));
        assert!(!html.contains("href=\"http"));
        assert!(html.contains("safe-model"));
    }

    #[test]
    fn export_destination_is_absolute_local_html_only_and_writer_refuses_existing_file() {
        let directory = tempdir().unwrap();
        let valid = directory.path().join("new-report.HTML");
        assert!(validate_export_destination(&valid).is_ok());

        assert!(validate_export_destination(Path::new("relative/report.html")).is_err());
        assert!(validate_export_destination(&directory.path().join("report.txt")).is_err());
        assert!(validate_export_destination(Path::new(r"\\server\share\report.html")).is_err());

        let existing = directory.path().join("existing.html");
        std::fs::write(&existing, "existing").unwrap();
        assert!(validate_export_destination(&existing).is_ok());
        assert!(write_new_report(&existing, b"replacement").is_err());
        assert_eq!(std::fs::read_to_string(existing).unwrap(), "existing");
    }

    #[cfg(windows)]
    #[test]
    fn drive_type_classification_accepts_only_writable_local_volumes() {
        use windows_report_file::{classify_drive_type, DriveClass};
        use windows_sys::Win32::System::WindowsProgramming::{
            DRIVE_CDROM, DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOTE,
            DRIVE_REMOVABLE, DRIVE_UNKNOWN,
        };

        for raw in [DRIVE_FIXED, DRIVE_REMOVABLE, DRIVE_RAMDISK] {
            assert_eq!(classify_drive_type(raw), DriveClass::WritableLocal);
        }
        assert_eq!(classify_drive_type(DRIVE_REMOTE), DriveClass::Remote);
        for raw in [DRIVE_UNKNOWN, DRIVE_NO_ROOT_DIR, DRIVE_CDROM, u32::MAX] {
            assert_eq!(classify_drive_type(raw), DriveClass::Unsupported);
        }
    }

    #[cfg(windows)]
    #[test]
    fn opened_handle_contract_rejects_reparse_wrong_volume_and_wrong_parent() {
        use windows_report_file::{validate_opened_file_snapshot, HandleSnapshot};

        let safe = HandleSnapshot {
            attributes: 0,
            volume_serial_number: 91,
            final_path: r"\\?\C:\safe\report.tmp".into(),
            file_index: 7,
        };
        assert!(
            validate_opened_file_snapshot(&safe, b'C', 91, r"\\?\C:\safe", "report.tmp",).is_ok()
        );

        let reparse = HandleSnapshot {
            attributes: 0x0400,
            ..safe.clone()
        };
        assert!(
            validate_opened_file_snapshot(&reparse, b'C', 91, r"\\?\C:\safe", "report.tmp",)
                .is_err()
        );

        let wrong_volume = HandleSnapshot {
            volume_serial_number: 92,
            ..safe.clone()
        };
        assert!(validate_opened_file_snapshot(
            &wrong_volume,
            b'C',
            91,
            r"\\?\C:\safe",
            "report.tmp",
        )
        .is_err());

        let wrong_parent = HandleSnapshot {
            final_path: r"\\?\C:\outside\report.tmp".into(),
            ..safe
        };
        assert!(validate_opened_file_snapshot(
            &wrong_parent,
            b'C',
            91,
            r"\\?\C:\safe",
            "report.tmp",
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn secure_writer_rejects_a_junction_parent_before_writing_report_bytes() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let directory = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let junction = directory.path().join("redirected");
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                junction.to_str().unwrap(),
                outside.path().to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let writer_called = AtomicBool::new(false);

        let result = write_new_report_with(&junction.join("report.html"), |_| {
            writer_called.store(true, Ordering::SeqCst);
            Ok(())
        });

        assert!(result.is_err());
        assert!(!writer_called.load(Ordering::SeqCst));
        assert!(!outside.path().join("report.html").exists());
    }

    #[cfg(windows)]
    #[test]
    fn held_parent_handles_block_a_deterministic_ancestor_junction_swap() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let directory = tempdir().unwrap();
        let middle = directory.path().join("middle");
        std::fs::create_dir(&middle).unwrap();
        let detached = directory.path().join("detached");
        let outside = tempdir().unwrap();
        let destination = middle.join("report.html");
        let replaced = AtomicBool::new(false);

        write_new_report_with_hook(
            &destination,
            |temporary| temporary.write_all(b"safe report"),
            |opened| {
                if opened == middle && std::fs::rename(&middle, &detached).is_ok() {
                    let status = std::process::Command::new("cmd")
                        .args([
                            "/C",
                            "mklink",
                            "/J",
                            middle.to_str().unwrap(),
                            outside.path().to_str().unwrap(),
                        ])
                        .status()
                        .unwrap();
                    assert!(status.success());
                    replaced.store(true, Ordering::SeqCst);
                }
            },
        )
        .unwrap();

        assert!(!replaced.load(Ordering::SeqCst));
        assert_eq!(std::fs::read_to_string(destination).unwrap(), "safe report");
        assert!(!outside.path().join("report.html").exists());
    }

    #[cfg(windows)]
    #[test]
    fn every_file_inspection_failure_deletes_the_same_opened_report_handle() {
        use std::cell::Cell;

        for failed_inspection in 1..=3 {
            let directory = tempdir().unwrap();
            let destination = directory.path().join("report.html");
            let inspection_count = Cell::new(0);
            let writer_called = Cell::new(false);

            let result = windows_report_file::write_new_file_with_inspector(
                &destination,
                |temporary| {
                    writer_called.set(true);
                    temporary.write_all(b"partial private report")
                },
                |_| {},
                |file| {
                    let current = inspection_count.get() + 1;
                    inspection_count.set(current);
                    if current == failed_inspection {
                        Err(std::io::Error::other("injected inspection failure"))
                    } else {
                        windows_report_file::inspect_handle(file)
                    }
                },
            );

            assert!(result.is_err(), "inspection {failed_inspection} must fail");
            assert_eq!(inspection_count.get(), failed_inspection);
            assert_eq!(writer_called.get(), failed_inspection != 1);
            assert!(
                !destination.exists(),
                "inspection {failed_inspection} left the published destination"
            );
            assert_eq!(
                std::fs::read_dir(directory.path()).unwrap().count(),
                0,
                "inspection {failed_inspection} left a randomized temporary file"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_failure_is_surfaced_generically_without_hiding_the_private_temporary() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("backup.zip");

        let error = windows_report_file::write_new_file_with_inspector_and_cleanup(
            &destination,
            |temporary| {
                temporary.write_all(b"private raw sentinel")?;
                Err(std::io::Error::other(
                    "C:\\Users\\Alice\\private.zip SQL secret",
                ))
            },
            |_| {},
            windows_report_file::inspect_handle,
            |_| Err(std::io::Error::other("injected cleanup failure")),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "report write failed and the opened temporary handle could not be deleted"
        );
        assert!(!destination.exists());
        let temporaries = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(temporaries.len(), 1);
        assert!(temporaries[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".ability-radar-"));
        std::fs::remove_file(&temporaries[0]).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn backup_writer_maps_cleanup_incomplete_separately_without_sensitive_details() {
        let cleanup =
            map_backup_write_error(windows_report_file::NativeWriteError::CleanupIncomplete);
        assert_eq!(
            cleanup,
            "备份未完成，临时私密数据可能尚未清理；请关闭应用并联系支持。"
        );

        let ordinary = map_backup_write_error(windows_report_file::NativeWriteError::Operation(
            std::io::Error::other("C:\\Users\\Alice\\private.zip SQL raw sentinel"),
        ));
        assert_eq!(ordinary, "无法安全写入新的本地备份；请重新选择位置。");
        assert!(!ordinary.contains("Alice"));
        assert!(!ordinary.contains("SQL"));
        assert!(!ordinary.contains("sentinel"));
    }

    #[cfg(windows)]
    #[test]
    fn private_backup_snapshot_retains_authority_and_cleans_every_exit() {
        let directory = tempdir().unwrap();
        let observed_path = std::cell::RefCell::new(None);

        let error =
            windows_report_file::with_private_snapshot(directory.path(), |path, _retained| {
                observed_path.replace(Some(path.to_path_buf()));
                std::fs::write(path, b"private SQLite sentinel")?;
                assert_eq!(std::fs::read(path)?, b"private SQLite sentinel");
                Err(std::io::Error::other("injected archive failure"))
            })
            .unwrap_err();

        assert!(matches!(
            error,
            windows_report_file::NativeWriteError::Operation(_)
        ));
        assert!(!observed_path.borrow().as_ref().unwrap().exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn private_snapshot_blocks_rename_replacement_before_sqlite_opens_the_path() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
        let detached = directory.path().join("detached.sqlite");
        let rename_succeeded = std::cell::Cell::new(false);

        let result =
            windows_report_file::with_private_snapshot(directory.path(), |path, retained| {
                let before = windows_report_file::inspect_handle(retained)?;
                if std::fs::rename(path, &detached).is_ok() {
                    rename_succeeded.set(true);
                    std::fs::write(path, b"attacker replacement")?;
                }
                repository
                    .snapshot_to_backup_file(path)
                    .map_err(std::io::Error::other)?;
                let after = windows_report_file::inspect_handle(retained)?;
                if before.volume_serial_number != after.volume_serial_number
                    || before.file_index != after.file_index
                {
                    return Err(std::io::Error::other("snapshot identity changed"));
                }
                Ok(())
            });

        assert!(!rename_succeeded.get(), "retained handle allowed rename");
        assert!(result.is_ok());
        assert!(!detached.exists());
    }

    #[cfg(windows)]
    #[test]
    fn private_backup_snapshot_classifies_cleanup_failure() {
        let directory = tempdir().unwrap();

        let error = windows_report_file::with_private_snapshot_and_cleanup(
            directory.path(),
            |path, _retained| std::fs::write(path, b"private SQLite sentinel"),
            |_| Err(std::io::Error::other("injected cleanup failure")),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            windows_report_file::NativeWriteError::CleanupIncomplete
        ));
        let residue = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(residue.len(), 1);
        assert_eq!(
            std::fs::read(&residue[0]).unwrap(),
            b"private SQLite sentinel"
        );
        std::fs::remove_file(&residue[0]).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn post_release_snapshot_swap_is_cleanup_incomplete_and_never_publishes_zip() {
        let app_data = tempdir().unwrap();
        let output = tempdir().unwrap();
        let destination = output.path().join("backup.zip");
        let detached = app_data.path().join("detached-original.sqlite");
        let replacement_path = std::cell::RefCell::new(None);

        let error = write_new_backup(&destination, app_data.path(), |temporary| {
            windows_report_file::with_private_snapshot_with_release_hook(
                app_data.path(),
                |path, _retained| {
                    std::fs::write(path, b"original private SQLite")?;
                    temporary.write_all(b"partial private ZIP")
                },
                |path| {
                    std::fs::rename(path, &detached).unwrap();
                    std::fs::write(path, b"attacker replacement").unwrap();
                    replacement_path.replace(Some(path.to_path_buf()));
                },
            )
        })
        .unwrap_err();

        assert_eq!(
            error,
            "备份未完成，临时私密数据可能尚未清理；请关闭应用并联系支持。"
        );
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 0);
        let replacement = replacement_path.borrow();
        let replacement = replacement.as_ref().unwrap();
        assert_eq!(std::fs::read(replacement).unwrap(), b"attacker replacement");
        assert_eq!(
            std::fs::read(&detached).unwrap(),
            b"original private SQLite"
        );
        std::fs::remove_file(replacement).unwrap();
        std::fs::remove_file(detached).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_write_cleans_the_opened_temporary_without_deleting_a_raced_final_file() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("report.html");
        let attacker_path = destination.clone();

        let result = write_new_report_with(&destination, |temporary| {
            temporary.write_all(b"partial private report")?;
            std::fs::write(&attacker_path, "attacker-owned")?;
            Err(std::io::Error::other("injected write failure"))
        });

        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "attacker-owned"
        );
        let names = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![std::ffi::OsString::from("report.html")]);
    }

    #[test]
    fn export_errors_never_echo_database_file_or_sensitive_values() {
        let directory = tempdir().unwrap();
        let repository = RunRepository::open(&directory.path().join("runs.db")).unwrap();
        let run = insert_exportable_run(&repository, r"C:\Users\Alice\sk-ant-private-model");
        let destination = directory.path().join("blocked.html");

        let error = export_report_to_selected_path(&repository, run.id, Some(destination.clone()))
            .unwrap_err();

        assert!(error.contains("reportedModel"));
        assert!(!error.contains("Alice"));
        assert!(!error.contains("sk-ant-private-model"));
        assert!(!error.contains(directory.path().to_string_lossy().as_ref()));
        assert!(!destination.exists());
    }

    #[test]
    fn default_report_name_is_bounded_and_contains_no_path() {
        let run_id = Uuid::parse_str("39d9f772-2e12-4b2d-af13-94c32d36f2d3").unwrap();

        let name = default_report_file_name(run_id);

        assert_eq!(name, "ability-radar-39d9f772.html");
        assert!(!name.contains(['/', '\\', ':']));
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
                app_version: "0.2.0".into(),
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
            let results = pack
                .tasks
                .iter()
                .map(|task| TaskResult {
                    run_id: run.id,
                    task_id: task.definition.id.clone(),
                    category: task.definition.category,
                    outcome: TaskOutcome::Passed,
                    score: Some(100.0),
                    failure_kind: None,
                    duration_ms: 1,
                    answer_rel_path: None,
                    detail: "coherent completed fixture".into(),
                })
                .collect::<Vec<_>>();
            for result in &results {
                repository.save_task_result(result).unwrap();
            }
            let score = summarize_scores(&results, run.total_tasks).unwrap();
            repository.complete_run(run.id, Some(&score)).unwrap();
        } else {
            assert_eq!(status, RunStatus::Running);
        }
        repository.get_run(run.id).unwrap().unwrap()
    }

    fn insert_exportable_run(repository: &RunRepository, model: &str) -> RunRecord {
        let pack = cli_pack();
        let mut run = RunRecord::new(
            TargetSelection {
                kind: TargetKind::CodexCli,
                reported_model: model.into(),
                reasoning_effort: Some("high".into()),
            },
            RunMode::Quick,
            pack.manifest.id.clone(),
            pack.manifest.version.clone(),
            2,
            EnvironmentFingerprint {
                os_family: "Windows".into(),
                os_version: "11 Pro C:\\Users\\Alice".into(),
                app_version: "0.2.0".into(),
                cli_version: Some("codex-cli 1.2.3".into()),
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
        let results = [
            TaskResult {
                run_id: run.id,
                task_id: "one".into(),
                category: Category::CliCoding,
                outcome: TaskOutcome::Passed,
                score: Some(100.0),
                failure_kind: None,
                duration_ms: 10,
                answer_rel_path: Some("runs/private/one.txt".into()),
                detail: "private raw answer C:\\Users\\Alice".into(),
            },
            TaskResult {
                run_id: run.id,
                task_id: "two".into(),
                category: Category::CliCoding,
                outcome: TaskOutcome::Failed,
                score: Some(0.0),
                failure_kind: Some(FailureKind::WrongAnswer),
                duration_ms: 20,
                answer_rel_path: Some("runs/private/two.txt".into()),
                detail: "private raw answer sk-ant-never-export".into(),
            },
        ];
        for result in &results {
            repository.save_task_result(result).unwrap();
        }
        let score = ability_core::summarize_scores(&results, 2).unwrap();
        repository.complete_run(run.id, Some(&score)).unwrap();
        repository.get_run(run.id).unwrap().unwrap()
    }
}
