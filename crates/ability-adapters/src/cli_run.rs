use crate::{
    AdapterCompletion, AdapterError, AgentAdapter, ExecutionRequest, VerificationGrade,
    WorkspaceVerifier,
};
use ability_core::{
    ArtifactStore, EnvironmentFingerprint, FailureKind, GraderSpec, LoadedPack, LoadedTask,
    RecoveryArtifactCheckpoint, RunMode, RunRecord, RunRepository, RunStatus, StorageError,
    TargetKind, TargetSelection, TaskOutcome, TaskResult, summarize_scores, validate_recovery,
    validate_recovery_checkpoints,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventKind {
    TaskStarted,
    TaskFinished,
    RunFinished,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: Uuid,
    pub kind: RunEventKind,
    pub task_id: Option<String>,
    pub completed_tasks: u32,
    pub total_tasks: u32,
}

#[derive(Debug, Error)]
pub enum CliRunError {
    #[error("target is not a CLI target or is unsupported by the pack")]
    WrongTarget,
    #[error("environment fingerprint does not match the loaded pack")]
    EnvironmentMismatch,
    #[error("loaded pack does not match the persisted run")]
    PackMismatch,
    #[error("adapter target does not match the persisted run")]
    AdapterMismatch,
    #[error("run is not running: {0:?}")]
    RunNotRunning(RunStatus),
    #[error("automatic CLI runs do not support pre-existing checkpoints")]
    UnexpectedCheckpoint,
    #[error("run cannot be resumed")]
    NotResumable,
    #[error("starter directory is missing for {0}")]
    MissingStarter(String),
    #[error("task does not use an external verifier: {0}")]
    UnsupportedGrader(String),
    #[error("artifact path is unsafe")]
    UnsafeArtifactPath,
    #[error("an artifact path already exists or is not an ordinary directory/file")]
    ArtifactConflict,
    #[error("task count exceeds the supported range")]
    CountOverflow,
    #[error("duration exceeds the supported range")]
    DurationOverflow,
    #[error("storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("workspace I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("artifact cleanup failed after {original}: {cleanup}")]
    CleanupFailed {
        original: Box<CliRunError>,
        cleanup: io::Error,
    },
    #[error("service-created workspace could not be safely removed after cancellation: {0}")]
    CancellationCleanup(io::Error),
    #[error("run terminalization failed after {original}: {terminalization}")]
    TerminalizationFailed {
        original: Box<CliRunError>,
        terminalization: StorageError,
    },
}

pub struct CliRunService {
    repository: Arc<RunRepository>,
    artifact_root: PathBuf,
    #[cfg(test)]
    after_workspace_copy_hook: Mutex<Option<WorkspaceCopyHook>>,
}

#[cfg(test)]
type WorkspaceCopyHook = Arc<dyn Fn(&Path) + Send + Sync>;

impl CliRunService {
    pub fn new(repository: Arc<RunRepository>, artifact_root: PathBuf) -> Self {
        Self {
            repository,
            artifact_root,
            #[cfg(test)]
            after_workspace_copy_hook: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn set_after_workspace_copy_hook_for_test(&self, hook: Arc<dyn Fn(&Path) + Send + Sync>) {
        *self.after_workspace_copy_hook.lock().unwrap() = Some(hook);
    }

    pub fn prepare(
        &self,
        pack: Arc<LoadedPack>,
        target: TargetSelection,
        mode: RunMode,
        environment: EnvironmentFingerprint,
    ) -> Result<RunRecord, CliRunError> {
        validate_artifact_root(&self.artifact_root)?;
        let total_tasks = validate_pack(&pack, target.kind)?;
        if environment.resumed
            || environment.suite_id != pack.manifest.id
            || environment.suite_version != pack.manifest.version
            || environment.suite_content_sha256 != pack.content_sha256
        {
            return Err(CliRunError::EnvironmentMismatch);
        }

        let mut run = RunRecord::new(
            target,
            mode,
            pack.manifest.id.clone(),
            pack.manifest.version.clone(),
            total_tasks,
            environment,
        );
        run.status = RunStatus::Running;
        self.repository.insert_run(&run)?;
        Ok(run)
    }

    pub fn resume(
        &self,
        run_id: Uuid,
        expected_target: TargetSelection,
        pack: &LoadedPack,
        current_environment: EnvironmentFingerprint,
    ) -> Result<RunRecord, CliRunError> {
        validate_artifact_root(&self.artifact_root)?;
        let artifact_store = ArtifactStore::new(self.artifact_root.clone());
        let pack_task_ids = pack
            .tasks
            .iter()
            .map(|task| task.definition.id.clone())
            .collect::<Vec<_>>();
        self.repository
            .resume_run(run_id, &expected_target, |run, results| {
                if run.target != expected_target {
                    return Err(StorageError::InvalidData(
                        "run target changed while recovery was being validated".into(),
                    ));
                }
                validate_pack(pack, run.target.kind).map_err(|_| {
                    StorageError::InvalidData("sealed CLI pack is not resumable".into())
                })?;
                validate_recovery(run, results, pack, &current_environment, true)?;
                let checkpoints = results
                    .iter()
                    .map(|result| RecoveryArtifactCheckpoint {
                        task_id: result.task_id.clone(),
                        raw_artifact: result.answer_rel_path.is_some(),
                    })
                    .collect::<Vec<_>>();
                artifact_store
                    .prepare_recovery_artifacts(
                        run.id,
                        run.target.kind,
                        &pack_task_ids,
                        &checkpoints,
                    )
                    .map_err(|_| {
                        StorageError::InvalidData(
                            "recovery artifact ownership is inconsistent".into(),
                        )
                    })
            })
            .map_err(|error| match error {
                StorageError::InvalidData(_) | StorageError::RunNotFound(_) => {
                    CliRunError::NotResumable
                }
                other => CliRunError::Storage(other),
            })
    }

    pub async fn execute(
        &self,
        run_id: Uuid,
        pack: Arc<LoadedPack>,
        adapter: Arc<dyn AgentAdapter>,
        verifier: Arc<dyn WorkspaceVerifier>,
        cancellation: CancellationToken,
        events: UnboundedSender<RunEvent>,
    ) -> Result<(), CliRunError> {
        let run = match self.repository.get_run(run_id) {
            Ok(Some(run)) => run,
            Ok(None) => return Err(CliRunError::Storage(StorageError::RunNotFound(run_id))),
            Err(error) => {
                let fallback_total_tasks = u32::try_from(pack.tasks.len()).unwrap_or(0);
                let fallback_completed_tasks = self
                    .repository
                    .get_task_results(run_id)
                    .ok()
                    .and_then(|results| u32::try_from(results.len()).ok())
                    .unwrap_or(0);
                let (completed_tasks, total_tasks) = self
                    .repository
                    .get_run_task_counts(run_id)
                    .ok()
                    .flatten()
                    .unwrap_or((fallback_completed_tasks, fallback_total_tasks));
                return Err(self.interrupt_after_error(
                    run_id,
                    CliRunError::Storage(error),
                    &events,
                    completed_tasks,
                    total_tasks,
                ));
            }
        };
        if run.status != RunStatus::Running {
            return Err(CliRunError::RunNotRunning(run.status));
        }

        let (total_tasks, completed_ids) = match self.bind_execution(&run, &pack, adapter.as_ref())
        {
            Ok(bound) => bound,
            Err(error) => {
                return Err(self.interrupt_after_error(
                    run_id,
                    error,
                    &events,
                    run.completed_tasks,
                    run.total_tasks,
                ));
            }
        };

        let mut completed_tasks =
            u32::try_from(completed_ids.len()).map_err(|_| CliRunError::CountOverflow)?;
        if cancellation.is_cancelled() {
            self.repository
                .finish_without_score(run_id, RunStatus::Cancelled)?;
            send_event(
                &events,
                run_id,
                RunEventKind::RunFinished,
                None,
                completed_tasks,
                total_tasks,
            );
            return Ok(());
        }

        for task in &pack.tasks {
            if completed_ids.contains(&task.definition.id) {
                continue;
            }
            if cancellation.is_cancelled() {
                self.repository
                    .finish_without_score(run_id, RunStatus::Cancelled)?;
                send_event(
                    &events,
                    run_id,
                    RunEventKind::RunFinished,
                    None,
                    completed_tasks,
                    total_tasks,
                );
                return Ok(());
            }

            let created_workspace = match self.create_workspace(run_id, task) {
                Ok(workspace) => workspace,
                Err(error) => {
                    return Err(self.interrupt_after_error(
                        run_id,
                        error,
                        &events,
                        completed_tasks,
                        total_tasks,
                    ));
                }
            };
            if cancellation.is_cancelled() {
                if let Err(error) = created_workspace.cleanup() {
                    return Err(self.interrupt_after_error(
                        run_id,
                        CliRunError::CancellationCleanup(error),
                        &events,
                        completed_tasks,
                        total_tasks,
                    ));
                }
                self.repository
                    .finish_without_score(run_id, RunStatus::Cancelled)?;
                send_event(
                    &events,
                    run_id,
                    RunEventKind::RunFinished,
                    None,
                    completed_tasks,
                    total_tasks,
                );
                return Ok(());
            }
            let workspace = created_workspace.keep();

            send_event(
                &events,
                run_id,
                RunEventKind::TaskStarted,
                Some(task.definition.id.clone()),
                completed_tasks,
                total_tasks,
            );

            let request = ExecutionRequest {
                prompt: task.prompt.clone(),
                workspace: workspace.clone(),
                time_budget_secs: task.definition.time_budget_secs,
                max_turns: task.definition.max_turns,
                model: (run.target.reported_model != "default")
                    .then(|| run.target.reported_model.clone()),
                reasoning_effort: run.target.reasoning_effort.clone(),
            };

            let mut result = match adapter.execute(request, cancellation.child_token()).await {
                Ok(AdapterCompletion::Completed {
                    duration_ms,
                    stdout,
                    stderr,
                }) => {
                    let log_relative =
                        match self.write_agent_log(run_id, &task.definition.id, &stdout, &stderr) {
                            Ok(relative) => relative,
                            Err(error) => {
                                return Err(self.interrupt_after_error(
                                    run_id,
                                    error,
                                    &events,
                                    completed_tasks,
                                    total_tasks,
                                ));
                            }
                        };
                    let grade = if cancellation.is_cancelled() {
                        cancelled_grade("agent_cancelled_after_completion")
                    } else {
                        let verifier_id = match external_verifier_id(task) {
                            Ok(verifier_id) => verifier_id,
                            Err(error) => {
                                return Err(self.interrupt_after_error(
                                    run_id,
                                    error,
                                    &events,
                                    completed_tasks,
                                    total_tasks,
                                ));
                            }
                        };
                        verifier
                            .verify(verifier_id, &workspace, cancellation.child_token())
                            .await
                    };
                    match task_result(run_id, task, grade, Some(log_relative), duration_ms) {
                        Ok(result) => result,
                        Err(error) => {
                            return Err(self.interrupt_after_error(
                                run_id,
                                error,
                                &events,
                                completed_tasks,
                                total_tasks,
                            ));
                        }
                    }
                }
                Err(error) => {
                    let budget_ms = match task.definition.time_budget_secs.checked_mul(1_000) {
                        Some(value) => value,
                        None => {
                            return Err(self.interrupt_after_error(
                                run_id,
                                CliRunError::DurationOverflow,
                                &events,
                                completed_tasks,
                                total_tasks,
                            ));
                        }
                    };
                    let grade = adapter_error_grade(error, budget_ms);
                    match task_result(run_id, task, grade, None, 0) {
                        Ok(result) => result,
                        Err(error) => {
                            return Err(self.interrupt_after_error(
                                run_id,
                                error,
                                &events,
                                completed_tasks,
                                total_tasks,
                            ));
                        }
                    }
                }
            };

            if cancellation.is_cancelled() && result.outcome != TaskOutcome::Cancelled {
                result.outcome = TaskOutcome::Cancelled;
                result.score = None;
                result.failure_kind = Some(FailureKind::UserCancelled);
                result.detail = "user_cancelled_after_task_work".into();
            }
            if let Err(error) = self.repository.save_task_result(&result) {
                return Err(self.interrupt_after_error(
                    run_id,
                    CliRunError::Storage(error),
                    &events,
                    completed_tasks,
                    total_tasks,
                ));
            }
            completed_tasks = match completed_tasks.checked_add(1) {
                Some(value) => value,
                None => {
                    return Err(self.interrupt_after_error(
                        run_id,
                        CliRunError::CountOverflow,
                        &events,
                        completed_tasks,
                        total_tasks,
                    ));
                }
            };
            send_event(
                &events,
                run_id,
                RunEventKind::TaskFinished,
                Some(result.task_id.clone()),
                completed_tasks,
                total_tasks,
            );

            if result.outcome == TaskOutcome::Cancelled || cancellation.is_cancelled() {
                self.repository
                    .finish_without_score(run_id, RunStatus::Cancelled)?;
                send_event(
                    &events,
                    run_id,
                    RunEventKind::RunFinished,
                    None,
                    completed_tasks,
                    total_tasks,
                );
                return Ok(());
            }
            if result.outcome == TaskOutcome::Invalid {
                break;
            }
        }

        let results = match self.repository.get_task_results(run_id) {
            Ok(results) => results,
            Err(error) => {
                return Err(self.interrupt_after_error(
                    run_id,
                    CliRunError::Storage(error),
                    &events,
                    completed_tasks,
                    total_tasks,
                ));
            }
        };
        let persisted_count = match u32::try_from(results.len()) {
            Ok(count) => count,
            Err(_) => {
                return Err(self.interrupt_after_error(
                    run_id,
                    CliRunError::CountOverflow,
                    &events,
                    completed_tasks,
                    total_tasks,
                ));
            }
        };
        let summary = summarize_scores(&results, total_tasks);
        if let Err(error) = self.repository.complete_run(run_id, summary.as_ref()) {
            return Err(self.interrupt_after_error(
                run_id,
                CliRunError::Storage(error),
                &events,
                persisted_count,
                total_tasks,
            ));
        }
        send_event(
            &events,
            run_id,
            RunEventKind::RunFinished,
            None,
            persisted_count,
            total_tasks,
        );
        Ok(())
    }

    fn bind_execution(
        &self,
        run: &RunRecord,
        pack: &LoadedPack,
        adapter: &dyn AgentAdapter,
    ) -> Result<(u32, BTreeSet<String>), CliRunError> {
        if !is_cli_target(run.target.kind) {
            return Err(CliRunError::WrongTarget);
        }
        if adapter.kind() != run.target.kind {
            return Err(CliRunError::AdapterMismatch);
        }
        if run.suite_id != pack.manifest.id
            || run.suite_version != pack.manifest.version
            || run.environment.suite_id != pack.manifest.id
            || run.environment.suite_version != pack.manifest.version
            || run.environment.suite_content_sha256 != pack.content_sha256
        {
            return Err(CliRunError::PackMismatch);
        }
        let total_tasks = validate_pack(pack, run.target.kind)?;
        if total_tasks != run.total_tasks {
            return Err(CliRunError::PackMismatch);
        }
        let results = self.repository.get_task_results(run.id)?;
        if run.environment.resumed {
            validate_recovery_checkpoints(run, &results, pack, true)
                .map_err(|_| CliRunError::NotResumable)?;
        } else if run.completed_tasks != 0 || !results.is_empty() {
            return Err(CliRunError::UnexpectedCheckpoint);
        }
        validate_artifact_root(&self.artifact_root)?;
        Ok((
            total_tasks,
            results.into_iter().map(|result| result.task_id).collect(),
        ))
    }

    fn create_workspace(
        &self,
        run_id: Uuid,
        task: &LoadedTask,
    ) -> Result<CreatedWorkspace, CliRunError> {
        let source = starter_path(task)?;
        validate_source_tree(&source)?;
        let destination = self
            .artifact_root
            .join("runs")
            .join(run_id.to_string())
            .join("workspaces")
            .join(&task.definition.id);
        if !destination.starts_with(&self.artifact_root) {
            return Err(CliRunError::UnsafeArtifactPath);
        }

        let mut created = CreatedArtifacts::default();
        let result = (|| {
            ensure_directory_chain(
                destination
                    .parent()
                    .ok_or(CliRunError::UnsafeArtifactPath)?,
                &mut created,
            )?;
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if is_link_or_reparse_point(&metadata) => {
                    return Err(CliRunError::UnsafeArtifactPath);
                }
                Ok(_) => return Err(CliRunError::ArtifactConflict),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(CliRunError::Io(error)),
            }
            fs::create_dir(&destination).map_err(map_create_error)?;
            created.directories.push(destination.clone());
            ensure_under_root(&self.artifact_root, &destination)?;
            copy_tree(&source, &destination, &mut created)?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                #[cfg(test)]
                if let Some(hook) = self
                    .after_workspace_copy_hook
                    .lock()
                    .unwrap()
                    .as_ref()
                    .cloned()
                {
                    hook(&destination);
                }
                Ok(CreatedWorkspace {
                    path: destination,
                    created,
                })
            }
            Err(error) => match created.cleanup() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(CliRunError::CleanupFailed {
                    original: Box::new(error),
                    cleanup,
                }),
            },
        }
    }

    fn write_agent_log(
        &self,
        run_id: Uuid,
        task_id: &str,
        stdout: &str,
        stderr: &str,
    ) -> Result<String, CliRunError> {
        validate_task_component(task_id)?;
        let relative = format!("runs/{run_id}/logs/{task_id}.log");
        let path = self.artifact_root.join(Path::new(&relative));
        if !path.starts_with(&self.artifact_root) {
            return Err(CliRunError::UnsafeArtifactPath);
        }

        let mut created = CreatedArtifacts::default();
        let result = (|| {
            ensure_directory_chain(
                path.parent().ok_or(CliRunError::UnsafeArtifactPath)?,
                &mut created,
            )?;
            ensure_under_root(
                &self.artifact_root,
                path.parent().ok_or(CliRunError::UnsafeArtifactPath)?,
            )?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(map_create_error)?;
            created.files.push(path.clone());
            use std::io::Write;
            writeln!(file, "STDOUT")?;
            writeln!(file, "{stdout}")?;
            writeln!(file, "STDERR")?;
            write!(file, "{stderr}")?;
            file.sync_all()?;
            Ok(())
        })();

        match result {
            Ok(()) => Ok(relative),
            Err(error) => match created.cleanup() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(CliRunError::CleanupFailed {
                    original: Box::new(error),
                    cleanup,
                }),
            },
        }
    }

    fn interrupt_after_error(
        &self,
        run_id: Uuid,
        original: CliRunError,
        events: &UnboundedSender<RunEvent>,
        completed_tasks: u32,
        total_tasks: u32,
    ) -> CliRunError {
        match self
            .repository
            .finish_without_score(run_id, RunStatus::Interrupted)
        {
            Ok(()) => {
                send_event(
                    events,
                    run_id,
                    RunEventKind::RunFinished,
                    None,
                    completed_tasks,
                    total_tasks,
                );
                original
            }
            Err(terminalization) => CliRunError::TerminalizationFailed {
                original: Box::new(original),
                terminalization,
            },
        }
    }
}

struct CreatedWorkspace {
    path: PathBuf,
    created: CreatedArtifacts,
}

impl CreatedWorkspace {
    fn keep(self) -> PathBuf {
        self.path
    }

    fn cleanup(mut self) -> io::Result<()> {
        self.created.cleanup()
    }
}

pub fn adapter_error_grade(error: AdapterError, budget_ms: u64) -> VerificationGrade {
    match error {
        AdapterError::AgentBudgetExceeded => VerificationGrade {
            outcome: TaskOutcome::Failed,
            score: Some(0.0),
            failure_kind: Some(FailureKind::AgentBudgetExceeded),
            detail: "agent_budget_exceeded".into(),
            duration_ms: budget_ms,
        },
        AdapterError::Cancelled => cancelled_grade("user_cancelled"),
        AdapterError::Unavailable => VerificationGrade {
            outcome: TaskOutcome::Invalid,
            score: None,
            failure_kind: Some(FailureKind::CliMissing),
            detail: "cli_unavailable".into(),
            duration_ms: 0,
        },
        AdapterError::Infrastructure { kind, detail } => VerificationGrade {
            outcome: TaskOutcome::Invalid,
            score: None,
            failure_kind: Some(kind),
            detail,
            duration_ms: 0,
        },
    }
}

fn task_result(
    run_id: Uuid,
    task: &LoadedTask,
    grade: VerificationGrade,
    answer_rel_path: Option<String>,
    agent_duration_ms: u64,
) -> Result<TaskResult, CliRunError> {
    let duration_ms = agent_duration_ms
        .checked_add(grade.duration_ms)
        .ok_or(CliRunError::DurationOverflow)?;
    Ok(TaskResult {
        run_id,
        task_id: task.definition.id.clone(),
        category: task.definition.category,
        outcome: grade.outcome,
        score: grade.score,
        failure_kind: grade.failure_kind,
        duration_ms,
        answer_rel_path,
        detail: grade.detail,
    })
}

fn cancelled_grade(detail: &str) -> VerificationGrade {
    VerificationGrade {
        outcome: TaskOutcome::Cancelled,
        score: None,
        failure_kind: Some(FailureKind::UserCancelled),
        detail: detail.into(),
        duration_ms: 0,
    }
}

fn validate_pack(pack: &LoadedPack, target: TargetKind) -> Result<u32, CliRunError> {
    if !is_cli_target(target) || !pack.manifest.target_kinds.contains(&target) {
        return Err(CliRunError::WrongTarget);
    }
    let total_tasks = u32::try_from(pack.tasks.len()).map_err(|_| CliRunError::CountOverflow)?;
    if total_tasks == 0 || pack.manifest.tasks.len() != pack.tasks.len() {
        return Err(CliRunError::PackMismatch);
    }

    let mut task_ids = BTreeSet::new();
    for task in &pack.tasks {
        validate_task_component(&task.definition.id)?;
        if !task_ids.insert(task.definition.id.as_str()) {
            return Err(CliRunError::PackMismatch);
        }
        external_verifier_id(task)?;
        if task.definition.time_budget_secs == 0
            || task.definition.time_budget_secs > 7_200
            || task.definition.max_turns == 0
            || task.definition.max_turns > 100
        {
            return Err(CliRunError::PackMismatch);
        }
        task.definition
            .time_budget_secs
            .checked_mul(1_000)
            .ok_or(CliRunError::DurationOverflow)?;
        let source = starter_path(task)?;
        validate_source_tree(&source)?;
    }
    Ok(total_tasks)
}

fn is_cli_target(target: TargetKind) -> bool {
    matches!(target, TargetKind::CodexCli | TargetKind::ClaudeCode)
}

fn external_verifier_id(task: &LoadedTask) -> Result<&str, CliRunError> {
    match &task.definition.grader {
        GraderSpec::ExternalVerifier { verifier_id } if !verifier_id.is_empty() => Ok(verifier_id),
        _ => Err(CliRunError::UnsupportedGrader(task.definition.id.clone())),
    }
}

fn starter_path(task: &LoadedTask) -> Result<PathBuf, CliRunError> {
    let relative = task
        .definition
        .starter_dir
        .as_deref()
        .ok_or_else(|| CliRunError::MissingStarter(task.definition.id.clone()))?;
    if !safe_relative_path(relative) {
        return Err(CliRunError::UnsafeArtifactPath);
    }
    let source = task.pack_root.join(relative);
    reject_existing_path_chain(&source)?;
    let canonical_root = task
        .pack_root
        .canonicalize()
        .map_err(|_| CliRunError::MissingStarter(task.definition.id.clone()))?;
    let canonical_source = source
        .canonicalize()
        .map_err(|_| CliRunError::MissingStarter(task.definition.id.clone()))?;
    if !canonical_source.starts_with(&canonical_root) || !canonical_source.is_dir() {
        return Err(CliRunError::MissingStarter(task.definition.id.clone()));
    }
    Ok(canonical_source)
}

fn validate_task_component(value: &str) -> Result<(), CliRunError> {
    if value.is_empty()
        || value.contains(['/', '\\', ':'])
        || value == "."
        || value == ".."
        || value.encode_utf16().count() > 251
    {
        return Err(CliRunError::UnsafeArtifactPath);
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['/', '\\'])
        && !value.contains(':')
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn validate_artifact_root(root: &Path) -> Result<(), CliRunError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(CliRunError::UnsafeArtifactPath);
    }
    #[cfg(windows)]
    validate_windows_local_path(root)?;
    reject_existing_path_chain(root)?;
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(CliRunError::ArtifactConflict),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliRunError::Io(error)),
    }
}

#[cfg(windows)]
fn validate_windows_local_path(path: &Path) -> Result<(), CliRunError> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Prefix;

    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut start = 0;
    for end in 0..=units.len() {
        if end != units.len() && units[end] != u16::from(b'\\') && units[end] != u16::from(b'/') {
            continue;
        }
        let component = &units[start..end];
        if component == [u16::from(b'.')] || component == [u16::from(b'.'), u16::from(b'.')] {
            return Err(CliRunError::UnsafeArtifactPath);
        }
        start = end + 1;
    }

    let mut components = path.components();
    match components.next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)) => {}
        _ => return Err(CliRunError::UnsafeArtifactPath),
    }
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(CliRunError::UnsafeArtifactPath);
    }
    let mut has_component = false;
    for component in components {
        let Component::Normal(value) = component else {
            return Err(CliRunError::UnsafeArtifactPath);
        };
        if value.is_empty()
            || value.to_string_lossy().contains(':')
            || value.encode_wide().count() > 255
        {
            return Err(CliRunError::UnsafeArtifactPath);
        }
        has_component = true;
    }
    if !has_component {
        return Err(CliRunError::UnsafeArtifactPath);
    }
    Ok(())
}

fn reject_existing_path_chain(path: &Path) -> Result<(), CliRunError> {
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if is_link_or_reparse_point(&metadata) {
                    return Err(CliRunError::UnsafeArtifactPath);
                }
                if ancestor != path && !metadata.is_dir() {
                    return Err(CliRunError::ArtifactConflict);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(CliRunError::Io(error)),
        }
    }
    Ok(())
}

fn validate_source_tree(root: &Path) -> Result<(), CliRunError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| CliRunError::MissingStarter(root.display().to_string()))?;
    if is_link_or_reparse_point(&metadata) {
        return Err(CliRunError::UnsafeArtifactPath);
    }
    if !metadata.is_dir() {
        return Err(CliRunError::MissingStarter(root.display().to_string()));
    }

    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if is_link_or_reparse_point(&metadata) {
                return Err(CliRunError::UnsafeArtifactPath);
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if !metadata.is_file() {
                return Err(CliRunError::ArtifactConflict);
            }
        }
    }
    Ok(())
}

fn ensure_directory_chain(path: &Path, created: &mut CreatedArtifacts) -> Result<(), CliRunError> {
    validate_artifact_root(path)?;
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_link_or_reparse_point(&metadata) {
                    return Err(CliRunError::UnsafeArtifactPath);
                }
                if !metadata.is_dir() {
                    return Err(CliRunError::ArtifactConflict);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(map_create_error)?;
                let metadata = fs::symlink_metadata(&current)?;
                if is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
                    return Err(CliRunError::UnsafeArtifactPath);
                }
                created.directories.push(current.clone());
            }
            Err(error) => return Err(CliRunError::Io(error)),
        }
    }
    Ok(())
}

fn ensure_under_root(root: &Path, child: &Path) -> Result<(), CliRunError> {
    let root = root.canonicalize()?;
    let child = child.canonicalize()?;
    if child.starts_with(&root) {
        Ok(())
    } else {
        Err(CliRunError::UnsafeArtifactPath)
    }
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    created: &mut CreatedArtifacts,
) -> Result<(), CliRunError> {
    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let from = entry.path();
        let metadata = fs::symlink_metadata(&from)?;
        if is_link_or_reparse_point(&metadata) {
            return Err(CliRunError::UnsafeArtifactPath);
        }
        let to = destination.join(entry.file_name());
        match fs::symlink_metadata(&to) {
            Ok(existing) if is_link_or_reparse_point(&existing) => {
                return Err(CliRunError::UnsafeArtifactPath);
            }
            Ok(_) => return Err(CliRunError::ArtifactConflict),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(CliRunError::Io(error)),
        }
        if metadata.is_dir() {
            fs::create_dir(&to).map_err(map_create_error)?;
            created.directories.push(to.clone());
            copy_tree(&from, &to, created)?;
        } else if metadata.is_file() {
            copy_new_file(&from, &to, created)?;
        } else {
            return Err(CliRunError::ArtifactConflict);
        }
    }
    Ok(())
}

fn copy_new_file(
    source: &Path,
    destination: &Path,
    created: &mut CreatedArtifacts,
) -> Result<(), CliRunError> {
    let mut input = File::open(source)?;
    let metadata = input.metadata()?;
    if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(CliRunError::UnsafeArtifactPath);
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(map_create_error)?;
    created.files.push(destination.to_path_buf());
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn map_create_error(error: io::Error) -> CliRunError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        CliRunError::ArtifactConflict
    } else {
        CliRunError::Io(error)
    }
}

#[derive(Default)]
struct CreatedArtifacts {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
}

impl CreatedArtifacts {
    fn cleanup(&mut self) -> io::Result<()> {
        for file in self.files.iter().rev() {
            remove_created_file(file)?;
        }
        for directory in self.directories.iter().rev() {
            remove_created_directory(directory)?;
        }
        Ok(())
    }
}

fn remove_created_file(path: &Path) -> io::Result<()> {
    reject_unsafe_cleanup_chain(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !is_link_or_reparse_point(&metadata) && metadata.is_file() => {
            fs::remove_file(path)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "created file was replaced by an unsafe artifact",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_created_directory(path: &Path) -> io::Result<()> {
    reject_unsafe_cleanup_chain(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !is_link_or_reparse_point(&metadata) && metadata.is_dir() => {
            fs::remove_dir(path)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "created directory was replaced by an unsafe artifact",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn reject_unsafe_cleanup_chain(path: &Path) -> io::Result<()> {
    reject_existing_path_chain(path)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))
}

#[cfg(windows)]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn send_event(
    sender: &UnboundedSender<RunEvent>,
    run_id: Uuid,
    kind: RunEventKind,
    task_id: Option<String>,
    completed_tasks: u32,
    total_tasks: u32,
) {
    let _ = sender.send(RunEvent {
        run_id,
        kind,
        task_id,
        completed_tasks,
        total_tasks,
    });
}

#[cfg(test)]
mod cancellation_copy_tests {
    use super::*;
    use crate::{AuthState, PrerequisiteStatus, TargetAvailability};
    use ability_core::PackLoader;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    struct CountingAdapter {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl AgentAdapter for CountingAdapter {
        fn kind(&self) -> TargetKind {
            TargetKind::CodexCli
        }

        async fn detect(&self) -> TargetAvailability {
            TargetAvailability {
                kind: self.kind(),
                installed: true,
                version: None,
                auth_state: AuthState::Unknown,
                prerequisites: Vec::<PrerequisiteStatus>::new(),
            }
        }

        async fn execute(
            &self,
            _request: ExecutionRequest,
            _cancellation: CancellationToken,
        ) -> Result<AdapterCompletion, AdapterError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(AdapterError::Unavailable)
        }
    }

    struct UnusedVerifier;

    #[async_trait]
    impl WorkspaceVerifier for UnusedVerifier {
        async fn verify(
            &self,
            _verifier_id: &str,
            _workspace: &Path,
            _cancellation: CancellationToken,
        ) -> VerificationGrade {
            panic!("verifier must not run after cancellation during copy")
        }
    }

    #[tokio::test]
    async fn cancellation_after_copy_removes_only_the_created_workspace_before_start() {
        let directory = tempdir().unwrap();
        let pack_root = directory.path().join("pack");
        fs::create_dir_all(pack_root.join("task/starter/src")).unwrap();
        fs::write(pack_root.join("task/prompt.md"), "Fix it.").unwrap();
        fs::write(
            pack_root.join("task/starter/src/index.mjs"),
            "export const fixed = false;",
        )
        .unwrap();
        fs::write(
            pack_root.join("manifest.json"),
            r#"{
              "schema_version":1,
              "id":"copy-cancel",
              "version":"1.0.0",
              "title":"Copy cancel",
              "target_kinds":["codex_cli"],
              "tasks":[{
                "id":"task-one",
                "category":"cli_coding",
                "prompt_file":"task/prompt.md",
                "starter_dir":"task/starter",
                "time_budget_secs":60,
                "max_turns":2,
                "grader":{"type":"external_verifier","verifier_id":"fake-v1"}
              }]
            }"#,
        )
        .unwrap();
        let pack = Arc::new(PackLoader::load(&pack_root).unwrap());
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let artifact_root = directory.path().join("artifacts");
        fs::create_dir(&artifact_root).unwrap();
        fs::write(artifact_root.join("owner.txt"), "preserve").unwrap();
        let service = CliRunService::new(repository.clone(), artifact_root.clone());
        let run = service
            .prepare(
                pack.clone(),
                TargetSelection {
                    kind: TargetKind::CodexCli,
                    reported_model: "default".into(),
                    reasoning_effort: None,
                },
                RunMode::Quick,
                EnvironmentFingerprint {
                    os_family: "windows".into(),
                    os_version: "test".into(),
                    app_version: "0.2.0".into(),
                    cli_version: Some("fake".into()),
                    verifier_runtime_version: Some("fake".into()),
                    suite_id: pack.manifest.id.clone(),
                    suite_version: pack.manifest.version.clone(),
                    suite_content_sha256: pack.content_sha256.clone(),
                    scoring_rule_version: "ability-v1".into(),
                    resumed: false,
                },
            )
            .unwrap();
        let cancellation = CancellationToken::new();
        let hook_cancellation = cancellation.clone();
        service.set_after_workspace_copy_hook_for_test(Arc::new(move |workspace| {
            assert!(workspace.join("src/index.mjs").is_file());
            hook_cancellation.cancel();
        }));
        let adapter = Arc::new(CountingAdapter {
            calls: AtomicUsize::new(0),
        });
        let (sender, mut receiver) = mpsc::unbounded_channel();

        service
            .execute(
                run.id,
                pack,
                adapter.clone(),
                Arc::new(UnusedVerifier),
                cancellation,
                sender,
            )
            .await
            .unwrap();

        assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Cancelled
        );
        assert!(repository.get_task_results(run.id).unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(artifact_root.join("owner.txt")).unwrap(),
            "preserve"
        );
        assert!(!artifact_root.join("runs").exists());
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.kind, RunEventKind::RunFinished);
        assert_eq!(event.completed_tasks, 0);
        assert_eq!(event.total_tasks, 1);
        assert!(receiver.try_recv().is_err());
    }
}
