use crate::app_state::{
    probe_node, public_cli_version, supported_node_lts, AppState, CancellationRegistration,
    CancellationRegistry,
};
use crate::dto::{
    BootstrapDto, CliRunEventDto, ExportReportInput, PackSummaryDto, RunDetailDto, RunErrorEvent,
    StartRunInput, SubmitAnswerInput, TaskResultDto,
};
use ability_adapters::{
    AgentAdapter, AuthState, CliRunService, PrerequisiteStatus, TargetAvailability,
};
use ability_core::{
    EnvironmentFingerprint, LoadedPack, ManualStep, RunMode, RunRecord, RunRepository, RunStatus,
    TargetKind, TargetSelection,
};
use std::fs;
#[cfg(windows)]
use std::io::Write;
use std::path::{Path, PathBuf};
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
    export_report_to_selected_path(&state.repository, run_id, destination)
}

fn default_report_file_name(run_id: Uuid) -> String {
    let key = run_id.simple().to_string();
    format!("ability-radar-{}.html", &key[..8])
}

fn export_report_to_selected_path(
    repository: &RunRepository,
    run_id: Uuid,
    destination: Option<PathBuf>,
) -> Result<Option<String>, String> {
    let Some(destination) = destination else {
        return Ok(None);
    };
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
    Ok(Some(report.report_id.to_string()))
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

    match fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err("为避免覆盖或跟随链接，请选择一个新的报告文件名。".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("无法安全检查所选位置，请重新选择。".into()),
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
mod windows_report_file {
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
        BY_HANDLE_FILE_INFORMATION, FILE_ADD_FILE, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_TRAVERSE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, OPEN_EXISTING, SYNCHRONIZE,
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
    ) -> io::Result<()>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
        H: FnMut(&Path),
    {
        write_new_file_with_inspector(destination, writer, after_component_open, inspect_handle)
    }

    pub(super) fn write_new_file_with_inspector<F, H, I>(
        destination: &Path,
        writer: F,
        mut after_component_open: H,
        mut inspector: I,
    ) -> io::Result<()>
    where
        F: FnOnce(&mut File) -> io::Result<()>,
        H: FnMut(&Path),
        I: FnMut(&File) -> io::Result<HandleSnapshot>,
    {
        let parent = open_parent(destination, &mut after_component_open)?;
        let temporary_name = OsString::from(format!(".ability-radar-{}.tmp", Uuid::new_v4()));
        let mut temporary = create_new_file(parent.directory(), &temporary_name)?;

        let snapshot = inspector(&temporary).or_else(|error| fail_and_delete(&temporary, error))?;
        if let Err(error) = validate_opened_file_snapshot(
            &snapshot,
            parent.drive,
            parent.volume_serial_number,
            &parent.final_path,
            &temporary_name,
        ) {
            return fail_and_delete(&temporary, error);
        }

        if let Err(error) = writer(&mut temporary).and_then(|()| temporary.sync_all()) {
            return fail_and_delete(&temporary, error);
        }

        let before_publish =
            inspector(&temporary).or_else(|error| fail_and_delete(&temporary, error))?;
        if let Err(error) = validate_opened_file_snapshot(
            &before_publish,
            parent.drive,
            parent.volume_serial_number,
            &parent.final_path,
            &temporary_name,
        ) {
            return fail_and_delete(&temporary, error);
        }

        if let Err(error) = rename_no_replace(
            &temporary,
            parent.directory(),
            parent.final_name.as_os_str(),
        ) {
            return fail_and_delete(&temporary, error);
        }

        let published =
            inspector(&temporary).or_else(|error| fail_and_delete(&temporary, error))?;
        if let Err(error) = validate_opened_file_snapshot(
            &published,
            parent.drive,
            parent.volume_serial_number,
            &parent.final_path,
            &parent.final_name,
        ) {
            return fail_and_delete(&temporary, error);
        }
        Ok(())
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

    fn fail_and_delete<T>(file: &File, primary: io::Error) -> io::Result<T> {
        match delete_file_handle(file) {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(io::Error::new(
                cleanup.kind(),
                "report write failed and the opened temporary handle could not be deleted",
            )),
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
    fn export_destination_is_absolute_new_local_html_only() {
        let directory = tempdir().unwrap();
        let valid = directory.path().join("new-report.HTML");
        assert!(validate_export_destination(&valid).is_ok());

        assert!(validate_export_destination(Path::new("relative/report.html")).is_err());
        assert!(validate_export_destination(&directory.path().join("report.txt")).is_err());
        assert!(validate_export_destination(Path::new(r"\\server\share\report.html")).is_err());

        let existing = directory.path().join("existing.html");
        std::fs::write(&existing, "existing").unwrap();
        assert!(
            validate_export_destination(&existing).is_err(),
            "rejecting every existing target also closes the symlink-following race"
        );
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
