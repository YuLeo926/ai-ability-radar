use crate::{
    ArtifactStore, EnvironmentFingerprint, FailureKind, GraderSpec, LoadedPack,
    RecoveryArtifactCheckpoint, RunMode, RunRecord, RunRepository, RunStatus, StorageError,
    TargetKind, TargetSelection, TaskOutcome, TaskResult, grade_submission, summarize_scores,
};
#[cfg(test)]
use crate::{ModelSource, ModelVerification};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use thiserror::Error;
use uuid::Uuid;

#[cfg(windows)]
use std::fs::File;
#[cfg(windows)]
use std::io::Write;

const MAX_ANSWER_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualStep {
    pub run_id: Uuid,
    pub task_id: String,
    pub task_number: u32,
    pub total_tasks: u32,
    pub prompt: String,
}

#[derive(Debug, Error)]
pub enum RunServiceError {
    #[error("run not found: {0}")]
    RunNotFound(Uuid),
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("target is not a client target")]
    WrongTarget,
    #[error("target is not supported by this pack")]
    UnsupportedTarget,
    #[error("environment does not match the selected pack")]
    EnvironmentMismatch,
    #[error("run cannot be resumed: {0}")]
    NotResumable(String),
    #[error("manual runs do not support task {task_id} because it requires an external verifier")]
    UnsupportedGrader { task_id: String },
    #[error("answer was submitted out of order")]
    OutOfOrder,
    #[error("answer exceeds the 256 KiB local limit")]
    AnswerTooLarge,
    #[error("artifact path is unsafe")]
    UnsafeArtifactPath,
    #[error("artifact already exists")]
    ArtifactConflict,
    #[error("artifact file name is too long for Windows")]
    ArtifactNameTooLong,
    #[error("manual artifact writes require the Windows capability implementation")]
    UnsupportedPlatform,
    #[error("elapsed task duration exceeds the supported range")]
    ElapsedOverflow,
    #[error("storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("artifact I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("checkpoint storage failed and artifact cleanup failed: {storage}; {cleanup}")]
    CheckpointCleanup {
        storage: Box<StorageError>,
        cleanup: std::io::Error,
    },
    #[error(
        "checkpoint storage failed, artifact cleanup failed, and interruption status failed: {storage}; {cleanup}; {status}"
    )]
    CheckpointCleanupTerminal {
        storage: Box<StorageError>,
        cleanup: std::io::Error,
        status: Box<StorageError>,
    },
    #[error("service state lock is poisoned")]
    Poisoned,
}

struct ActiveManualRun {
    pack: Arc<LoadedPack>,
    task_started: Instant,
}

pub struct ManualRunService {
    repository: Arc<RunRepository>,
    artifact_root: PathBuf,
    active: Mutex<HashMap<Uuid, ActiveManualRun>>,
    #[cfg(all(test, windows))]
    artifact_write_hooks: Mutex<TestArtifactWriteHooks>,
}

#[cfg(all(test, windows))]
type ArtifactWriteHook = Arc<dyn Fn(&Path) + Send + Sync>;

#[cfg(all(test, windows))]
#[derive(Default)]
struct TestArtifactWriteHooks {
    before_artifact_write: Option<ArtifactWriteHook>,
    before_publish: Option<ArtifactWriteHook>,
    after_publish: Option<ArtifactWriteHook>,
    after_root_component_open: Option<ArtifactWriteHook>,
    force_checkpoint_failure: bool,
    force_cleanup_failure: bool,
    force_status_failure: bool,
}

impl ManualRunService {
    pub fn new(repository: Arc<RunRepository>, artifact_root: PathBuf) -> Self {
        Self {
            repository,
            artifact_root,
            active: Mutex::new(HashMap::new()),
            #[cfg(all(test, windows))]
            artifact_write_hooks: Mutex::new(TestArtifactWriteHooks::default()),
        }
    }

    pub fn start(
        &self,
        pack: Arc<LoadedPack>,
        target: TargetSelection,
        mode: RunMode,
        environment: EnvironmentFingerprint,
    ) -> Result<RunRecord, RunServiceError> {
        validate_artifact_root(&self.artifact_root)?;
        if !matches!(
            target.kind,
            TargetKind::ChatGptClient | TargetKind::ClaudeClient
        ) {
            return Err(RunServiceError::WrongTarget);
        }
        if !pack.manifest.target_kinds.contains(&target.kind) {
            return Err(RunServiceError::UnsupportedTarget);
        }
        if environment.suite_id != pack.manifest.id
            || environment.suite_version != pack.manifest.version
            || environment.suite_content_sha256 != pack.content_sha256
        {
            return Err(RunServiceError::EnvironmentMismatch);
        }
        if let Some(task) = pack
            .tasks
            .iter()
            .find(|task| matches!(task.definition.grader, GraderSpec::ExternalVerifier { .. }))
        {
            return Err(RunServiceError::UnsupportedGrader {
                task_id: task.definition.id.clone(),
            });
        }
        for task in &pack.tasks {
            validate_task_artifact_names(&task.definition.id)?;
        }

        // Acquire state before persistence: if the mutex is poisoned, no Running row is created.
        let mut active = self.active.lock().map_err(|_| RunServiceError::Poisoned)?;
        let mut run = RunRecord::new(
            target,
            mode,
            pack.manifest.id.clone(),
            pack.manifest.version.clone(),
            u32::try_from(pack.tasks.len()).map_err(|_| RunServiceError::ElapsedOverflow)?,
            environment,
        );
        run.status = RunStatus::Running;
        self.repository.insert_run(&run)?;
        active.insert(
            run.id,
            ActiveManualRun {
                pack,
                task_started: Instant::now(),
            },
        );
        Ok(run)
    }

    pub fn resume(
        &self,
        run_id: Uuid,
        expected_target: TargetSelection,
        pack: Arc<LoadedPack>,
        current_environment: EnvironmentFingerprint,
    ) -> Result<RunRecord, RunServiceError> {
        validate_artifact_root(&self.artifact_root)?;
        for task in &pack.tasks {
            validate_task_artifact_names(&task.definition.id)?;
        }
        if pack
            .tasks
            .iter()
            .any(|task| matches!(task.definition.grader, GraderSpec::ExternalVerifier { .. }))
        {
            return Err(RunServiceError::NotResumable(
                "the sealed client task pack is not supported".into(),
            ));
        }

        let mut active = self.active.lock().map_err(|_| RunServiceError::Poisoned)?;
        if active.contains_key(&run_id) {
            return Err(RunServiceError::NotResumable(
                "the run is already active".into(),
            ));
        }
        let artifact_store = ArtifactStore::new(self.artifact_root.clone());
        let pack_task_ids = pack
            .tasks
            .iter()
            .map(|task| task.definition.id.clone())
            .collect::<Vec<_>>();
        let resumed = self
            .repository
            .resume_run(run_id, &expected_target, |run, results| {
                if run.target != expected_target {
                    return Err(StorageError::InvalidData(
                        "run target changed while recovery was being validated".into(),
                    ));
                }
                validate_recovery(run, results, &pack, &current_environment, false)?;
                let checkpoints = results
                    .iter()
                    .map(|result| RecoveryArtifactCheckpoint {
                        task_id: result.task_id.clone(),
                        raw_artifact: true,
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
            .map_err(map_resume_storage_error)?;
        active.insert(
            run_id,
            ActiveManualRun {
                pack,
                task_started: Instant::now(),
            },
        );
        Ok(resumed)
    }

    pub fn next_step(&self, run_id: Uuid) -> Result<Option<ManualStep>, RunServiceError> {
        let mut active = self.active.lock().map_err(|_| RunServiceError::Poisoned)?;
        let (completed, total_tasks) = {
            let state = match active.get(&run_id) {
                Some(state) => state,
                None => {
                    return match self.repository.get_run(run_id)? {
                        Some(run) if run.status == RunStatus::Completed => Ok(None),
                        _ => Err(RunServiceError::RunNotFound(run_id)),
                    };
                }
            };
            let run = self
                .repository
                .get_run(run_id)?
                .ok_or(RunServiceError::RunNotFound(run_id))?;
            if run.status != RunStatus::Running {
                return Err(RunServiceError::RunNotFound(run_id));
            }
            (
                self.repository.get_task_results(run_id)?.len(),
                state.pack.tasks.len(),
            )
        };

        if completed == total_tasks {
            self.complete_active_run(run_id, &active)?;
            active.remove(&run_id);
            return Ok(None);
        }
        let state = active.get(&run_id).expect("active state checked above");
        Ok(state.pack.tasks.get(completed).map(|task| ManualStep {
            run_id,
            task_id: task.definition.id.clone(),
            task_number: u32::try_from(completed + 1).unwrap_or(u32::MAX),
            total_tasks: u32::try_from(total_tasks).unwrap_or(u32::MAX),
            prompt: task.prompt.clone(),
        }))
    }

    pub fn cancel(&self, run_id: Uuid) -> Result<bool, RunServiceError> {
        let mut active = self.active.lock().map_err(|_| RunServiceError::Poisoned)?;
        if !active.contains_key(&run_id) {
            return Ok(false);
        }
        self.repository
            .finish_without_score(run_id, RunStatus::Cancelled)?;
        active.remove(&run_id);
        Ok(true)
    }

    pub fn interrupt(&self, run_id: Uuid) -> Result<bool, RunServiceError> {
        let mut active = self.active.lock().map_err(|_| RunServiceError::Poisoned)?;
        if !active.contains_key(&run_id) {
            return Ok(false);
        }
        self.repository
            .finish_without_score(run_id, RunStatus::Interrupted)?;
        active.remove(&run_id);
        Ok(true)
    }

    pub fn submit_answer(
        &self,
        run_id: Uuid,
        task_id: &str,
        answer: &str,
    ) -> Result<TaskResult, RunServiceError> {
        if answer.len() > MAX_ANSWER_BYTES {
            return Err(RunServiceError::AnswerTooLarge);
        }

        let mut active = self.active.lock().map_err(|_| RunServiceError::Poisoned)?;
        let (task_id_to_save, category, grader, duration_ms, total_tasks, completed) = {
            let state = active
                .get(&run_id)
                .ok_or(RunServiceError::RunNotFound(run_id))?;
            let run = self
                .repository
                .get_run(run_id)?
                .ok_or(RunServiceError::RunNotFound(run_id))?;
            if run.status != RunStatus::Running {
                return Err(RunServiceError::RunNotFound(run_id));
            }
            let completed = self.repository.get_task_results(run_id)?.len();
            if completed == state.pack.tasks.len() {
                self.complete_active_run(run_id, &active)?;
                active.remove(&run_id);
                return Err(RunServiceError::RunNotFound(run_id));
            }
            let task = state
                .pack
                .tasks
                .get(completed)
                .ok_or_else(|| RunServiceError::TaskNotFound(task_id.into()))?;
            if task.definition.id != task_id {
                return Err(RunServiceError::OutOfOrder);
            }
            (
                task.definition.id.clone(),
                task.definition.category,
                task.definition.grader.clone(),
                u64::try_from(state.task_started.elapsed().as_millis())
                    .map_err(|_| RunServiceError::ElapsedOverflow)?,
                state.pack.tasks.len(),
                completed,
            )
        };

        let answer_rel_path = answer_relative_path(run_id, &task_id_to_save)?;
        let artifact = self.write_answer_atomically(run_id, &task_id_to_save, answer)?;
        #[cfg(all(test, windows))]
        let mut artifact = artifact;
        #[cfg(all(test, windows))]
        {
            let hooks = self.artifact_write_hooks.lock().expect("test hook lock");
            artifact.force_cleanup_failure = hooks.force_cleanup_failure;
        }
        let grade = grade_submission(&grader, answer);
        let result = TaskResult {
            run_id,
            task_id: task_id_to_save,
            category,
            outcome: if grade.passed {
                TaskOutcome::Passed
            } else {
                TaskOutcome::Failed
            },
            score: Some(grade.score),
            failure_kind: if grade.passed {
                None
            } else {
                Some(FailureKind::WrongAnswer)
            },
            duration_ms,
            answer_rel_path: Some(answer_rel_path),
            detail: grade.detail,
        };
        #[cfg(all(test, windows))]
        let checkpoint_result = if self
            .artifact_write_hooks
            .lock()
            .expect("test hook lock")
            .force_checkpoint_failure
        {
            Err(StorageError::InvalidData(
                "forced checkpoint failure".into(),
            ))
        } else {
            self.repository.save_task_result(&result)
        };
        #[cfg(not(all(test, windows)))]
        let checkpoint_result = self.repository.save_task_result(&result);

        if let Err(error) = checkpoint_result {
            return match artifact.remove_after_checkpoint_failure() {
                Ok(()) => Err(error.into()),
                Err(cleanup) => {
                    active.remove(&run_id);
                    #[cfg(all(test, windows))]
                    let status_result = if self
                        .artifact_write_hooks
                        .lock()
                        .expect("test hook lock")
                        .force_status_failure
                    {
                        Err(StorageError::RunNotFound(run_id))
                    } else {
                        self.repository
                            .interrupt_running_after_checkpoint_cleanup(run_id)
                    };
                    #[cfg(not(all(test, windows)))]
                    let status_result = self
                        .repository
                        .interrupt_running_after_checkpoint_cleanup(run_id);
                    match status_result {
                        Ok(()) => Err(RunServiceError::CheckpointCleanup {
                            storage: Box::new(error),
                            cleanup,
                        }),
                        Err(status) => Err(RunServiceError::CheckpointCleanupTerminal {
                            storage: Box::new(error),
                            cleanup,
                            status: Box::new(status),
                        }),
                    }
                }
            };
        }
        artifact.keep_after_checkpoint();

        if completed + 1 == total_tasks {
            // Preserve the active state unless the repository confirms completion, so a
            // subsequent next_step call can safely retry a transient completion failure.
            self.complete_active_run(run_id, &active)?;
            active.remove(&run_id);
        } else {
            active
                .get_mut(&run_id)
                .expect("active state remains until completion")
                .task_started = Instant::now();
        }
        Ok(result)
    }

    fn complete_active_run(
        &self,
        run_id: Uuid,
        active: &HashMap<Uuid, ActiveManualRun>,
    ) -> Result<(), RunServiceError> {
        let state = active
            .get(&run_id)
            .ok_or(RunServiceError::RunNotFound(run_id))?;
        let results = self.repository.get_task_results(run_id)?;
        let summary = summarize_scores(
            &results,
            u32::try_from(state.pack.tasks.len()).map_err(|_| RunServiceError::ElapsedOverflow)?,
        );
        self.repository.complete_run(run_id, summary.as_ref())?;
        Ok(())
    }

    #[cfg(all(test, windows))]
    fn set_before_artifact_write_hook_for_test(&self, hook: ArtifactWriteHook) {
        self.artifact_write_hooks
            .lock()
            .expect("test hook lock")
            .before_artifact_write = Some(hook);
    }

    #[cfg(all(test, windows))]
    fn set_before_publish_hook_for_test(&self, hook: ArtifactWriteHook) {
        self.artifact_write_hooks
            .lock()
            .expect("test hook lock")
            .before_publish = Some(hook);
    }

    #[cfg(all(test, windows))]
    fn set_after_publish_hook_for_test(&self, hook: ArtifactWriteHook) {
        self.artifact_write_hooks
            .lock()
            .expect("test hook lock")
            .after_publish = Some(hook);
    }

    #[cfg(all(test, windows))]
    fn set_after_root_component_open_hook_for_test(&self, hook: ArtifactWriteHook) {
        self.artifact_write_hooks
            .lock()
            .expect("test hook lock")
            .after_root_component_open = Some(hook);
    }

    #[cfg(all(test, windows))]
    fn force_checkpoint_and_cleanup_failure_for_test(&self) {
        let mut hooks = self.artifact_write_hooks.lock().expect("test hook lock");
        hooks.force_checkpoint_failure = true;
        hooks.force_cleanup_failure = true;
    }

    #[cfg(all(test, windows))]
    fn force_checkpoint_cleanup_and_status_failure_for_test(&self) {
        let mut hooks = self.artifact_write_hooks.lock().expect("test hook lock");
        hooks.force_checkpoint_failure = true;
        hooks.force_cleanup_failure = true;
        hooks.force_status_failure = true;
    }

    #[cfg(all(test, windows))]
    fn run_test_hook(&self, phase: TestArtifactWriteHookPhase, path: &Path) {
        let hook = match phase {
            TestArtifactWriteHookPhase::BeforeArtifactWrite => self
                .artifact_write_hooks
                .lock()
                .expect("test hook lock")
                .before_artifact_write
                .clone(),
            TestArtifactWriteHookPhase::BeforePublish => self
                .artifact_write_hooks
                .lock()
                .expect("test hook lock")
                .before_publish
                .clone(),
            TestArtifactWriteHookPhase::AfterPublish => self
                .artifact_write_hooks
                .lock()
                .expect("test hook lock")
                .after_publish
                .clone(),
        };
        if let Some(hook) = hook {
            hook(path);
        }
    }

    #[cfg(windows)]
    fn write_answer_atomically(
        &self,
        run_id: Uuid,
        task_id: &str,
        answer: &str,
    ) -> Result<ArtifactPublication, RunServiceError> {
        #[cfg(all(test, windows))]
        let run_path = self.artifact_root.join("runs").join(run_id.to_string());
        let run_directory = WindowsRunDirectory::open(
            &self.artifact_root,
            run_id,
            #[cfg(all(test, windows))]
            &self.artifact_write_hooks,
        )?;
        #[cfg(all(test, windows))]
        self.run_test_hook(TestArtifactWriteHookPhase::BeforeArtifactWrite, &run_path);

        let temporary_name = format!(".{task_id}.{}.tmp", Uuid::new_v4());
        let mut temporary = run_directory
            .create_temporary(&temporary_name)
            .map_err(|error| contextual_io("create temporary artifact", error))?;
        if let Err(error) = temporary
            .write_all(answer.as_bytes())
            .and_then(|()| temporary.sync_all())
        {
            let _ = windows_artifacts::delete_file_handle(&temporary);
            return Err(error.into());
        }
        #[cfg(all(test, windows))]
        self.run_test_hook(TestArtifactWriteHookPhase::BeforePublish, &run_path);

        let final_name = format!("{task_id}.txt");
        match run_directory.publish_no_replace(&temporary, &final_name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = windows_artifacts::delete_file_handle(&temporary);
                return Err(RunServiceError::ArtifactConflict);
            }
            Err(error) => {
                let _ = windows_artifacts::delete_file_handle(&temporary);
                return Err(error.into());
            }
        }
        #[cfg(all(test, windows))]
        self.run_test_hook(TestArtifactWriteHookPhase::AfterPublish, &run_path);
        Ok(ArtifactPublication {
            temporary,
            directory: run_directory,
            #[cfg(all(test, windows))]
            force_cleanup_failure: false,
        })
    }

    #[cfg(not(windows))]
    fn write_answer_atomically(
        &self,
        _run_id: Uuid,
        _task_id: &str,
        _answer: &str,
    ) -> Result<ArtifactPublication, RunServiceError> {
        Err(RunServiceError::UnsupportedPlatform)
    }
}

fn map_resume_storage_error(error: StorageError) -> RunServiceError {
    match error {
        StorageError::InvalidData(_) => {
            RunServiceError::NotResumable("stored recovery data did not pass validation".into())
        }
        other => RunServiceError::Storage(other),
    }
}

pub fn validate_recovery(
    run: &RunRecord,
    results: &[TaskResult],
    pack: &LoadedPack,
    current_environment: &EnvironmentFingerprint,
    cli: bool,
) -> Result<(), StorageError> {
    validate_recovery_checkpoints(run, results, pack, cli)?;
    let mut persisted_environment = run.environment.clone();
    persisted_environment.resumed = false;
    if current_environment.resumed || persisted_environment != *current_environment {
        return Err(StorageError::InvalidData(
            "run or environment recovery identity is inconsistent".into(),
        ));
    }
    Ok(())
}

pub fn validate_recovery_checkpoints(
    run: &RunRecord,
    results: &[TaskResult],
    pack: &LoadedPack,
    cli: bool,
) -> Result<(), StorageError> {
    let expected_target = if cli {
        matches!(
            run.target.kind,
            TargetKind::CodexCli | TargetKind::ClaudeCode
        )
    } else {
        matches!(
            run.target.kind,
            TargetKind::ChatGptClient | TargetKind::ClaudeClient
        )
    };
    let expected_count = u32::try_from(pack.tasks.len())
        .map_err(|_| StorageError::InvalidData("task count exceeds supported range".into()))?;
    if !expected_target
        || run.mode != RunMode::Quick
        || !pack.manifest.target_kinds.contains(&run.target.kind)
        || run.suite_id != pack.manifest.id
        || run.suite_version != pack.manifest.version
        || run.total_tasks != expected_count
        || run.completed_tasks != u32::try_from(results.len()).unwrap_or(u32::MAX)
        || run.score.is_some()
        || run.environment.suite_id != pack.manifest.id
        || run.environment.suite_version != pack.manifest.version
        || run.environment.suite_content_sha256 != pack.content_sha256
        || run.environment.scoring_rule_version != "ability-v1"
    {
        return Err(StorageError::InvalidData(
            "run or environment recovery identity is inconsistent".into(),
        ));
    }

    let mut checkpoints = HashMap::with_capacity(results.len());
    for result in results {
        if result.run_id != run.id
            || checkpoints
                .insert(result.task_id.as_str(), result)
                .is_some()
            || !valid_recovery_result(result, cli)
        {
            return Err(StorageError::InvalidData(
                "checkpoint evidence is inconsistent".into(),
            ));
        }
    }

    for task in pack.tasks.iter().take(results.len()) {
        let result = checkpoints
            .get(task.definition.id.as_str())
            .ok_or_else(|| {
                StorageError::InvalidData(
                    "checkpoint is not an exact prefix of the sealed pack".into(),
                )
            })?;
        if result.category != task.definition.category {
            return Err(StorageError::InvalidData(
                "checkpoint evidence is inconsistent".into(),
            ));
        }
        let expected_artifact = if cli {
            match result.failure_kind {
                Some(FailureKind::AgentBudgetExceeded) => None,
                _ => Some(format!("runs/{}/logs/{}.log", run.id, result.task_id)),
            }
        } else {
            Some(format!("runs/{}/{}.txt", run.id, result.task_id))
        };
        if result.answer_rel_path != expected_artifact {
            return Err(StorageError::InvalidData(
                "checkpoint artifact ownership is inconsistent".into(),
            ));
        }
    }
    Ok(())
}

fn valid_recovery_result(result: &TaskResult, cli: bool) -> bool {
    match (cli, result.outcome) {
        (_, TaskOutcome::Passed) => result.score == Some(100.0) && result.failure_kind.is_none(),
        (false, TaskOutcome::Failed) => {
            result.score == Some(0.0) && result.failure_kind == Some(FailureKind::WrongAnswer)
        }
        (true, TaskOutcome::Failed) => {
            result.score == Some(0.0)
                && matches!(
                    result.failure_kind,
                    Some(FailureKind::WrongAnswer | FailureKind::AgentBudgetExceeded)
                )
        }
        (_, TaskOutcome::Invalid | TaskOutcome::Cancelled) => false,
    }
}

fn answer_relative_path(run_id: Uuid, task_id: &str) -> Result<String, RunServiceError> {
    if task_id.is_empty()
        || task_id.contains(['/', '\\', ':'])
        || task_id
            .split(['/', '\\'])
            .any(|component| component == "..")
    {
        return Err(RunServiceError::UnsafeArtifactPath);
    }
    Ok(format!("runs/{run_id}/{task_id}.txt"))
}

#[cfg(windows)]
fn validate_artifact_root(artifact_root: &Path) -> Result<(), RunServiceError> {
    windows_artifacts::local_drive_components(artifact_root).map(|_| ())
}

#[cfg(not(windows))]
fn validate_artifact_root(_artifact_root: &Path) -> Result<(), RunServiceError> {
    Err(RunServiceError::UnsupportedPlatform)
}

fn validate_task_artifact_names(task_id: &str) -> Result<(), RunServiceError> {
    const MAX_WINDOWS_COMPONENT_UTF16: usize = 255;
    for component in [
        format!("{task_id}.txt"),
        format!(".{task_id}.{}.tmp", Uuid::nil()),
    ] {
        if component.encode_utf16().count() > MAX_WINDOWS_COMPONENT_UTF16 {
            return Err(RunServiceError::ArtifactNameTooLong);
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
enum TestArtifactWriteHookPhase {
    BeforeArtifactWrite,
    BeforePublish,
    AfterPublish,
}

#[cfg(windows)]
struct ArtifactPublication {
    temporary: File,
    directory: WindowsRunDirectory,
    #[cfg(all(test, windows))]
    force_cleanup_failure: bool,
}

#[cfg(windows)]
impl ArtifactPublication {
    fn keep_after_checkpoint(self) {
        drop(self.temporary);
        drop(self.directory);
    }

    fn remove_after_checkpoint_failure(self) -> std::io::Result<()> {
        #[cfg(all(test, windows))]
        if self.force_cleanup_failure {
            return Err(std::io::Error::other("forced artifact cleanup failure"));
        }
        windows_artifacts::delete_file_handle(&self.temporary)?;
        drop(self.temporary);
        drop(self.directory);
        Ok(())
    }
}

#[cfg(windows)]
struct WindowsRunDirectory {
    _artifact_root: File,
    _runs: File,
    directory: File,
}

#[cfg(windows)]
impl WindowsRunDirectory {
    fn open(
        artifact_root: &Path,
        run_id: Uuid,
        #[cfg(all(test, windows))] hooks: &Mutex<TestArtifactWriteHooks>,
    ) -> Result<Self, RunServiceError> {
        let (drive, components) = windows_artifacts::local_drive_components(artifact_root)?;
        let artifact_root = windows_artifacts::open_artifact_root(
            drive,
            &components,
            #[cfg(all(test, windows))]
            hooks,
        )
        .map_err(|error| artifact_directory_error("open artifact root component", error))?;
        let runs = windows_artifacts::open_or_create_directory(
            &artifact_root,
            std::ffi::OsStr::new("runs"),
        )
        .map_err(|error| artifact_directory_error("open runs directory", error))?;
        let run_component = run_id.to_string();
        let directory = windows_artifacts::open_or_create_directory(
            &runs,
            std::ffi::OsStr::new(&run_component),
        )
        .map_err(|error| artifact_directory_error("open run directory", error))?;
        Ok(Self {
            _artifact_root: artifact_root,
            _runs: runs,
            directory,
        })
    }

    fn create_temporary(&self, name: &str) -> std::io::Result<File> {
        windows_artifacts::create_new_file(&self.directory, name)
    }

    fn publish_no_replace(&self, temporary: &File, name: &str) -> std::io::Result<()> {
        windows_artifacts::rename_no_replace(temporary, &self.directory, name)
    }
}

#[cfg(windows)]
fn contextual_io(context: &str, error: std::io::Error) -> RunServiceError {
    RunServiceError::Io(std::io::Error::new(
        error.kind(),
        format!("{context}: {error}"),
    ))
}

#[cfg(windows)]
fn artifact_directory_error(context: &str, error: std::io::Error) -> RunServiceError {
    if error.kind() == std::io::ErrorKind::InvalidInput {
        RunServiceError::UnsafeArtifactPath
    } else {
        contextual_io(context, error)
    }
}

#[cfg(windows)]
mod windows_artifacts {
    use super::RunServiceError;
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::ptr::{null, null_mut};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_CREATE, FILE_DIRECTORY_FILE, FILE_DISPOSITION_INFORMATION, FILE_NON_DIRECTORY_FILE,
        FILE_OPEN, FILE_OPEN_IF, FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION,
        FILE_SYNCHRONOUS_IO_NONALERT, FileDispositionInformation, FileRenameInformation,
        NtCreateFile, NtSetInformationFile,
    };
    use windows_sys::Win32::Foundation::{
        INVALID_HANDLE_VALUE, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, STATUS_SUCCESS,
        UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::DELETE;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
        FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA,
        GetFileInformationByHandle, OPEN_EXISTING, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    const DIRECTORY_ACCESS: u32 = FILE_LIST_DIRECTORY
        | FILE_ADD_FILE
        | FILE_ADD_SUBDIRECTORY
        | FILE_READ_ATTRIBUTES
        | FILE_TRAVERSE
        | SYNCHRONIZE;
    const DIRECTORY_OPEN_ACCESS: u32 =
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE;
    const FILE_ACCESS: u32 =
        FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES | FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE;
    const DIRECTORY_SHARING: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;
    const FILE_SHARE_NONE: u32 = 0;

    pub fn local_drive_components(path: &Path) -> Result<(u8, Vec<OsString>), RunServiceError> {
        if has_dot_component(path) {
            return Err(RunServiceError::UnsafeArtifactPath);
        }
        let mut components = path.components();
        let drive = match components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) => letter,
                _ => return Err(RunServiceError::UnsafeArtifactPath),
            },
            _ => return Err(RunServiceError::UnsafeArtifactPath),
        };
        if !matches!(components.next(), Some(Component::RootDir)) {
            return Err(RunServiceError::UnsafeArtifactPath);
        }
        let mut normal = Vec::new();
        for component in components {
            let Component::Normal(value) = component else {
                return Err(RunServiceError::UnsafeArtifactPath);
            };
            if value.is_empty() || value.to_string_lossy().contains(':') {
                return Err(RunServiceError::UnsafeArtifactPath);
            }
            native_component_length(value).map_err(RunServiceError::Io)?;
            normal.push(value.to_os_string());
        }
        if normal.is_empty() {
            return Err(RunServiceError::UnsafeArtifactPath);
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

    pub fn drive_root_path(drive: u8) -> PathBuf {
        PathBuf::from(format!("{}:\\", char::from(drive)))
    }

    pub fn open_drive_root(drive: u8) -> io::Result<File> {
        let root = drive_root_path(drive);
        let wide = wide(root.as_os_str());
        // SAFETY: `wide` is NUL-terminated and remains alive for the synchronous call.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                DIRECTORY_OPEN_ACCESS,
                DIRECTORY_SHARING,
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
        let file = unsafe { File::from_raw_handle(handle as RawHandle) };
        reject_reparse_handle(&file)?;
        Ok(file)
    }

    pub fn open_artifact_root(
        drive: u8,
        components: &[OsString],
        #[cfg(all(test, windows))] hooks: &std::sync::Mutex<super::TestArtifactWriteHooks>,
    ) -> io::Result<File> {
        let mut handles = vec![open_drive_root(drive)?];
        let mut current_path = drive_root_path(drive);
        for (index, component) in components.iter().enumerate() {
            let parent = handles.last().expect("drive root is retained");
            let child = match open_directory(parent, component, DIRECTORY_OPEN_ACCESS) {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let parent = upgrade_directory_access(&mut handles, components, index)?;
                    create_directory(parent, component)?
                }
                Err(error) => return Err(error),
            };
            handles.push(child);
            current_path.push(component);
            #[cfg(all(test, windows))]
            {
                let hook = hooks
                    .lock()
                    .expect("test hook lock")
                    .after_root_component_open
                    .clone();
                if let Some(hook) = hook {
                    hook(&current_path);
                }
            }
        }
        let last = handles.len() - 1;
        let _ = upgrade_directory_access(&mut handles, components, last)?;
        Ok(handles.pop().expect("artifact root handle is retained"))
    }

    fn upgrade_directory_access<'a>(
        handles: &'a mut [File],
        components: &[OsString],
        handle_index: usize,
    ) -> io::Result<&'a File> {
        if handle_index == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "artifact root cannot be created directly beneath a drive root",
            ));
        }
        let replacement = open_directory(
            &handles[handle_index - 1],
            &components[handle_index - 1],
            DIRECTORY_ACCESS,
        )?;
        handles[handle_index] = replacement;
        Ok(&handles[handle_index])
    }

    fn open_directory(parent: &File, name: &OsStr, access: u32) -> io::Result<File> {
        let file = open_relative(
            parent,
            name,
            access,
            FILE_OPEN,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            DIRECTORY_SHARING,
        )?;
        reject_reparse_handle(&file)?;
        Ok(file)
    }

    fn create_directory(parent: &File, name: &OsStr) -> io::Result<File> {
        let file = open_relative(
            parent,
            name,
            DIRECTORY_ACCESS,
            FILE_CREATE,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            DIRECTORY_SHARING,
        )?;
        reject_reparse_handle(&file)?;
        Ok(file)
    }

    pub fn open_or_create_directory(parent: &File, name: &OsStr) -> io::Result<File> {
        let file = open_relative(
            parent,
            name,
            DIRECTORY_ACCESS,
            FILE_OPEN_IF,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            DIRECTORY_SHARING,
        )?;
        reject_reparse_handle(&file)?;
        Ok(file)
    }

    pub fn create_new_file(parent: &File, name: &str) -> io::Result<File> {
        let file = open_relative(
            parent,
            OsStr::new(name),
            FILE_ACCESS,
            FILE_CREATE,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            FILE_SHARE_NONE,
        )?;
        reject_reparse_handle(&file)?;
        Ok(file)
    }

    pub fn rename_no_replace(file: &File, directory: &File, name: &str) -> io::Result<()> {
        let name = OsStr::new(name);
        let name_length = native_component_length(name)?;
        let mut storage = information_buffer::<FILE_RENAME_INFORMATION>(usize::from(name_length));
        let info = storage.as_mut_ptr() as *mut FILE_RENAME_INFORMATION;
        let name_utf16 = name.encode_wide().collect::<Vec<_>>();
        // SAFETY: `storage` is suitably aligned, large enough for the trailing UTF-16 name,
        // and stays alive until NtSetInformationFile returns synchronously.
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = directory.as_raw_handle() as _;
            (*info).FileNameLength =
                u32::try_from(name_utf16.len() * size_of::<u16>()).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Windows artifact component length exceeds rename metadata",
                    )
                })?;
            std::ptr::copy_nonoverlapping(
                name_utf16.as_ptr(),
                (*info).FileName.as_mut_ptr(),
                name_utf16.len(),
            );
        }
        set_information(
            file,
            info.cast(),
            storage.len() * size_of::<u64>(),
            FileRenameInformation,
        )
    }

    pub fn delete_file_handle(file: &File) -> io::Result<()> {
        let information = FILE_DISPOSITION_INFORMATION { DeleteFile: true };
        set_information(
            file,
            (&information as *const FILE_DISPOSITION_INFORMATION).cast(),
            size_of::<FILE_DISPOSITION_INFORMATION>(),
            FileDispositionInformation,
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
        // SAFETY: `attributes`, `name_storage`, and `status` remain valid for this synchronous
        // call; the root handle is a directory handle opened without delete sharing.
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
            let error = nt_error(result);
            return Err(io::Error::new(
                error.kind(),
                format!("NtCreateFile status {result:#x}: {error}"),
            ));
        }
        // SAFETY: NtCreateFile returned a newly owned handle on STATUS_SUCCESS.
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }

    fn reject_reparse_handle(file: &File) -> io::Result<()> {
        // SAFETY: zero initialization is valid for this output-only Windows structure.
        let mut information = unsafe { zeroed::<BY_HANDLE_FILE_INFORMATION>() };
        // SAFETY: `file` is live and `information` is a writable output buffer of the exact type.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reparse points are not valid artifact directories or files",
            ));
        }
        Ok(())
    }

    fn set_information(
        file: &File,
        information: *const core::ffi::c_void,
        length: usize,
        class: i32,
    ) -> io::Result<()> {
        // SAFETY: zero is a valid initial IO_STATUS_BLOCK state for synchronous requests.
        let mut status = unsafe { zeroed::<IO_STATUS_BLOCK>() };
        // SAFETY: the file handle is live and the caller supplies a correctly sized buffer that
        // remains valid for the synchronous NtSetInformationFile call.
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
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows artifact component is too long",
            ));
        }
        let bytes = units.checked_mul(size_of::<u16>()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows component length overflow",
            )
        })?;
        u16::try_from(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows component exceeds UNICODE_STRING length",
            )
        })
    }

    fn nt_error(status: i32) -> io::Error {
        // SAFETY: RtlNtStatusToDosError is a pure conversion for the supplied NTSTATUS value.
        io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
    }
}

#[cfg(not(windows))]
struct ArtifactPublication;

#[cfg(not(windows))]
impl ArtifactPublication {
    fn keep_after_checkpoint(self) {}

    fn remove_after_checkpoint_failure(self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{PackLoader, TargetSelection};
    use rusqlite::Connection;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::tempdir;

    fn pack(root: &Path) -> Arc<LoadedPack> {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("one.txt"), "Only output 4").unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{
              "schema_version":1,"id":"manual-race","version":"1.0.0","title":"Race",
              "target_kinds":["chat_gpt_client"],"tasks":[{
                "id":"one","category":"logic","prompt_file":"one.txt","starter_dir":null,
                "time_budget_secs":60,"max_turns":1,
                "grader":{"type":"exact_text","expected":"4"}
              }]
            }"#,
        )
        .unwrap();
        Arc::new(PackLoader::load(root).unwrap())
    }

    fn environment(pack: &LoadedPack) -> EnvironmentFingerprint {
        EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "11".into(),
            app_version: "test".into(),
            cli_version: None,
            verifier_runtime_version: None,
            suite_id: pack.manifest.id.clone(),
            suite_version: pack.manifest.version.clone(),
            suite_content_sha256: pack.content_sha256.clone(),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        }
    }

    fn start_service() -> (
        tempfile::TempDir,
        Arc<RunRepository>,
        ManualRunService,
        RunRecord,
    ) {
        let directory = tempdir().unwrap();
        let loaded_pack = pack(&directory.path().join("pack"));
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let service = ManualRunService::new(repository.clone(), directory.path().join("artifacts"));
        let run = service
            .start(
                loaded_pack.clone(),
                TargetSelection {
                    kind: TargetKind::ChatGptClient,
                    reported_model: "test".into(),
                    reasoning_effort: None,
                    model_source: ModelSource::LegacyUnknown,
                    model_verification: ModelVerification::LegacyUnknown,
                },
                RunMode::Quick,
                environment(&loaded_pack),
            )
            .unwrap();
        (directory, repository, service, run)
    }

    #[test]
    fn start_rejects_invalid_artifact_roots_before_creating_run_state() {
        let directory = tempdir().unwrap();
        let loaded_pack = pack(&directory.path().join("pack"));
        let ads_root = directory.path().join("artifacts:stream");
        let dot_root = PathBuf::from(format!(r"{}\.\artifacts", directory.path().display()));
        let roots = [
            PathBuf::from(r"relative\artifacts"),
            PathBuf::from(r"\\server\share\artifacts"),
            PathBuf::from(r"\\.\PhysicalDrive0"),
            PathBuf::from(r"\\?\C:\artifacts"),
            ads_root,
            dot_root,
        ];

        for artifact_root in roots {
            let repository = Arc::new(
                RunRepository::open(&directory.path().join(Uuid::new_v4().to_string())).unwrap(),
            );
            let service = ManualRunService::new(repository.clone(), artifact_root);

            assert!(matches!(
                service.start(
                    loaded_pack.clone(),
                    TargetSelection {
                        kind: TargetKind::ChatGptClient,
                        reported_model: "test".into(),
                        reasoning_effort: None,
                        model_source: ModelSource::LegacyUnknown,
                        model_verification: ModelVerification::LegacyUnknown,
                    },
                    RunMode::Quick,
                    environment(&loaded_pack),
                ),
                Err(RunServiceError::UnsafeArtifactPath)
            ));
            assert!(repository.list_runs().unwrap().is_empty());
            assert!(service.active.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn held_directory_handle_blocks_a_deterministic_junction_replacement() {
        let (directory, repository, service, run) = start_service();
        let run_dir = directory
            .path()
            .join("artifacts/runs")
            .join(run.id.to_string());
        let detached_dir = directory.path().join("detached");
        let outside = tempdir().unwrap();
        let outside_path = outside.path().to_path_buf();
        let replaced = Arc::new(AtomicBool::new(false));
        let observed = replaced.clone();
        service.set_before_artifact_write_hook_for_test(Arc::new(move |_| {
            if fs::rename(&run_dir, &detached_dir).is_ok() {
                let status = Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        run_dir.to_str().unwrap(),
                        outside_path.to_str().unwrap(),
                    ])
                    .status()
                    .unwrap();
                assert!(status.success());
                observed.store(true, Ordering::SeqCst);
            }
        }));

        service.submit_answer(run.id, "one", "4").unwrap();

        assert!(!replaced.load(Ordering::SeqCst));
        assert!(repository.get_task_results(run.id).unwrap().len() == 1);
        assert!(!outside.path().join("one.txt").exists());
    }

    #[test]
    fn a_deterministic_final_name_race_cannot_overwrite_or_checkpoint() {
        let (directory, repository, service, run) = start_service();
        let answer_path = directory
            .path()
            .join("artifacts/runs")
            .join(run.id.to_string())
            .join("one.txt");
        let attacker_path = answer_path.clone();
        service.set_before_publish_hook_for_test(Arc::new(move |_| {
            fs::write(&attacker_path, "attacker-owned").unwrap();
        }));

        assert!(matches!(
            service.submit_answer(run.id, "one", "4"),
            Err(RunServiceError::ArtifactConflict)
        ));
        assert_eq!(fs::read_to_string(answer_path).unwrap(), "attacker-owned");
        assert!(repository.get_task_results(run.id).unwrap().is_empty());
    }

    #[test]
    fn checkpoint_failure_removes_only_the_service_created_artifact() {
        let (directory, repository, service, run) = start_service();
        let database = directory.path().join("runs.db");
        let run_id = run.id;
        service.set_after_publish_hook_for_test(Arc::new(move |_| {
            let connection = Connection::open(&database).unwrap();
            connection.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            connection
                .execute("DELETE FROM runs WHERE id=?1", [run_id.to_string()])
                .unwrap();
        }));

        assert!(matches!(
            service.submit_answer(run.id, "one", "4"),
            Err(RunServiceError::Storage(_))
        ));
        let answer_path = directory
            .path()
            .join("artifacts/runs")
            .join(run.id.to_string())
            .join("one.txt");
        assert!(!answer_path.exists());
        assert!(repository.get_task_results(run.id).unwrap().is_empty());
        assert!(matches!(
            service.next_step(run.id),
            Err(RunServiceError::RunNotFound(id)) if id == run.id
        ));
    }

    #[test]
    fn root_component_handle_blocks_a_deterministic_ancestor_replacement() {
        let directory = tempdir().unwrap();
        let middle = directory.path().join("middle");
        fs::create_dir(&middle).unwrap();
        let loaded_pack = pack(&directory.path().join("pack"));
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let service = ManualRunService::new(repository, middle.join("artifacts"));
        let run = service
            .start(
                loaded_pack.clone(),
                TargetSelection {
                    kind: TargetKind::ChatGptClient,
                    reported_model: "test".into(),
                    reasoning_effort: None,
                    model_source: ModelSource::LegacyUnknown,
                    model_verification: ModelVerification::LegacyUnknown,
                },
                RunMode::Quick,
                environment(&loaded_pack),
            )
            .unwrap();
        let detached = directory.path().join("detached");
        let outside = tempdir().unwrap();
        let outside_path = outside.path().to_path_buf();
        let replaced = Arc::new(AtomicBool::new(false));
        let observed = replaced.clone();
        service.set_after_root_component_open_hook_for_test(Arc::new(move |component| {
            if component == middle && fs::rename(&middle, &detached).is_ok() {
                let status = Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        middle.to_str().unwrap(),
                        outside_path.to_str().unwrap(),
                    ])
                    .status()
                    .unwrap();
                assert!(status.success());
                observed.store(true, Ordering::SeqCst);
            }
        }));

        service.submit_answer(run.id, "one", "4").unwrap();

        assert!(!replaced.load(Ordering::SeqCst));
        assert!(!outside.path().join("artifacts").exists());
    }

    #[test]
    fn answer_cannot_be_opened_for_read_or_write_before_checkpoint() {
        let (directory, repository, service, run) = start_service();
        let read_opened = Arc::new(AtomicBool::new(false));
        let write_opened = Arc::new(AtomicBool::new(false));
        let observed_read = read_opened.clone();
        let observed_write = write_opened.clone();
        service.set_after_publish_hook_for_test(Arc::new(move |run_dir| {
            let answer = run_dir.join("one.txt");
            if std::fs::File::open(&answer).is_ok() {
                observed_read.store(true, Ordering::SeqCst);
            }
            if std::fs::OpenOptions::new()
                .write(true)
                .open(&answer)
                .is_ok()
            {
                observed_write.store(true, Ordering::SeqCst);
            }
        }));

        service.submit_answer(run.id, "one", "4").unwrap();

        assert!(!read_opened.load(Ordering::SeqCst));
        assert!(!write_opened.load(Ordering::SeqCst));
        assert_eq!(repository.get_task_results(run.id).unwrap().len(), 1);
        let answer_path = directory
            .path()
            .join("artifacts/runs")
            .join(run.id.to_string())
            .join("one.txt");
        assert_eq!(fs::read_to_string(answer_path).unwrap(), "4");
    }

    #[test]
    fn cleanup_failure_interrupts_the_run_and_removes_active_state() {
        let (directory, repository, service, run) = start_service();
        service.force_checkpoint_and_cleanup_failure_for_test();

        assert!(matches!(
            service.submit_answer(run.id, "one", "4"),
            Err(RunServiceError::CheckpointCleanup { .. })
        ));
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );
        assert!(matches!(
            service.next_step(run.id),
            Err(RunServiceError::RunNotFound(id)) if id == run.id
        ));
        assert!(matches!(
            service.submit_answer(run.id, "one", "4"),
            Err(RunServiceError::RunNotFound(id)) if id == run.id
        ));
        let answer_path = directory
            .path()
            .join("artifacts/runs")
            .join(run.id.to_string())
            .join("one.txt");
        assert!(answer_path.exists());
        assert!(repository.get_task_results(run.id).unwrap().is_empty());
    }

    #[test]
    fn cleanup_failure_reports_a_terminal_status_update_failure() {
        let (_directory, _repository, service, run) = start_service();
        service.force_checkpoint_cleanup_and_status_failure_for_test();

        assert!(matches!(
            service.submit_answer(run.id, "one", "4"),
            Err(RunServiceError::CheckpointCleanupTerminal {
                storage,
                cleanup: _,
                status,
            }) if matches!(storage.as_ref(), StorageError::InvalidData(_))
                && matches!(status.as_ref(), StorageError::RunNotFound(id) if *id == run.id)
        ));
        assert!(matches!(
            service.next_step(run.id),
            Err(RunServiceError::RunNotFound(id)) if id == run.id
        ));
    }
}
