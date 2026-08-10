use crate::app_state::{public_adapter_version, RunOperationRegistry};
use ability_adapters::{
    AgentAdapter, AuthState, AvailabilityStatus, CliBatchExecutionBinding, CliRunError,
    CliRunService, LaunchSource, RunEvent, TargetAvailability, WorkspaceVerifier,
};
use ability_core::{
    AdapterLaunchKind, BatchExecutionSurface, BatchMemberStatus, BatchMode, BatchStatus,
    EnvironmentFingerprint, ExecutionAdapterIdentity, FailureKind, LoadedPack, RunMode, RunRecord,
    RunRepository, RunStatus, ScanBatchMemberRecord, ScanBatchRecord, ScanBatchTarget,
    StorageError, TargetKind, TaskOutcome,
};
use chrono::Utc;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchRunEventKind {
    MemberStarted,
    MemberFinished,
    BatchStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchRunEvent {
    pub batch_id: Uuid,
    pub kind: BatchRunEventKind,
    pub member_ordinal: Option<u32>,
    pub run_id: Option<Uuid>,
    pub terminal_members: u32,
    pub planned_members: u32,
    pub status: BatchStatus,
}

#[derive(Debug, Error)]
pub enum BatchRunnerError {
    #[error("batch storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("CLI execution failed: {0}")]
    Cli(#[from] CliRunError),
    #[error("batch is not a reviewed Quick Comparison or Standard CLI batch")]
    UnsupportedBatch,
    #[error("batch contains an ambiguous active member and will not be replayed")]
    AmbiguousActive,
    #[error("batch target or retained adapter identity is inconsistent")]
    TargetMismatch,
    #[error("batch run already has an active local-data operation")]
    OperationBusy,
}

pub struct BatchRunner {
    repository: Arc<RunRepository>,
    cli_runs: Arc<CliRunService>,
    pack: Arc<LoadedPack>,
    verifier: Arc<dyn WorkspaceVerifier>,
    operations: RunOperationRegistry,
}

impl BatchRunner {
    pub fn new(
        repository: Arc<RunRepository>,
        cli_runs: Arc<CliRunService>,
        pack: Arc<LoadedPack>,
        verifier: Arc<dyn WorkspaceVerifier>,
        operations: RunOperationRegistry,
    ) -> Self {
        Self {
            repository,
            cli_runs,
            pack,
            verifier,
            operations,
        }
    }

    pub async fn run(
        &self,
        batch_id: Uuid,
        adapters: BTreeMap<TargetKind, Arc<dyn AgentAdapter>>,
        verifier_runtime_version: String,
        cancellation: CancellationToken,
        events: UnboundedSender<BatchRunEvent>,
    ) -> Result<ScanBatchRecord, BatchRunnerError> {
        let initial = self.required_batch(batch_id)?;
        self.validate_cli_batch(&initial)?;
        let mut detected = BTreeMap::<TargetKind, TargetAvailability>::new();

        loop {
            if cancellation.is_cancelled() {
                self.repository.cancel_batch(batch_id, Utc::now())?;
            }
            self.repository.derive_batch_status(batch_id, Utc::now())?;
            let batch = self.required_batch(batch_id)?;
            self.validate_cli_batch(&batch)?;

            if batch.cancel_requested || cancellation.is_cancelled() {
                self.repository.cancel_batch(batch_id, Utc::now())?;
                let cancelled = self.required_batch(batch_id)?;
                emit_snapshot(
                    &events,
                    &cancelled,
                    BatchRunEventKind::BatchStatus,
                    None,
                    None,
                );
                return Ok(cancelled);
            }
            if matches!(
                batch.status,
                BatchStatus::Completed | BatchStatus::Cancelled
            ) {
                emit_snapshot(&events, &batch, BatchRunEventKind::BatchStatus, None, None);
                return Ok(batch);
            }
            if batch.members.iter().any(|member| {
                matches!(
                    member.status,
                    BatchMemberStatus::Reserved
                        | BatchMemberStatus::Launching
                        | BatchMemberStatus::Running
                )
            }) {
                return Err(BatchRunnerError::AmbiguousActive);
            }

            let Some(member) = batch
                .members
                .iter()
                .find(|member| member.status == BatchMemberStatus::Planned)
                .cloned()
            else {
                emit_snapshot(&events, &batch, BatchRunEventKind::BatchStatus, None, None);
                return Ok(batch);
            };
            let target = target_for_member(&batch, &member)?.clone();
            let Some(adapter) = adapters.get(&target.target.kind).cloned() else {
                self.defer_without_run(&batch, &member, FailureKind::CliMissing, &events)?;
                continue;
            };
            if adapter.contract_version()
                != target.execution_adapter_identity.adapter_contract_version
            {
                return Err(BatchRunnerError::TargetMismatch);
            }

            let availability = if let Some(availability) = detected.get(&target.target.kind) {
                availability.clone()
            } else {
                let availability = adapter.detect().await;
                detected.insert(target.target.kind, availability.clone());
                availability
            };
            let environment = match self.environment_for_target(
                &target,
                &availability,
                &verifier_runtime_version,
            ) {
                Ok(environment) => environment,
                Err(failure) => {
                    self.defer_without_run(&batch, &member, failure, &events)?;
                    continue;
                }
            };

            let run_id = member.run_id.unwrap_or_else(Uuid::new_v4);
            let _operation = self
                .operations
                .claim([run_id])
                .map_err(|_| BatchRunnerError::OperationBusy)?;
            let proposed = proposed_run(&batch, &target, run_id, environment);
            let Some(reservation) = self.repository.reserve_next_runnable_member_and_run(
                batch_id,
                Utc::now(),
                &proposed,
            )?
            else {
                continue;
            };
            if reservation.member.ordinal != member.ordinal
                || reservation.member.target_position != member.target_position
                || reservation.run.id != run_id
            {
                return Err(BatchRunnerError::TargetMismatch);
            }

            let retry_marker = match self.cli_runs.prepare_owned_batch_run(
                &self.pack,
                &reservation,
                &proposed.environment,
            ) {
                Ok(marker) => marker,
                Err(_) => {
                    self.repository.defer_batch_member(
                        batch_id,
                        member.ordinal,
                        Some(run_id),
                        FailureKind::AppInterrupted,
                        Utc::now(),
                    )?;
                    let deferred = self.required_batch(batch_id)?;
                    emit_snapshot(
                        &events,
                        &deferred,
                        BatchRunEventKind::MemberFinished,
                        Some(member.ordinal),
                        Some(run_id),
                    );
                    continue;
                }
            };

            self.repository
                .mark_member_launching(batch_id, member.ordinal, run_id, Utc::now())?;
            if cancellation.is_cancelled() {
                self.cancel_active(batch_id, &member, run_id, &events)?;
                return self.required_batch(batch_id);
            }
            self.repository.mark_member_running_retrying_exact_marker(
                batch_id,
                member.ordinal,
                run_id,
                retry_marker.as_ref(),
                Utc::now(),
            )?;
            let running = self.required_batch(batch_id)?;
            emit_snapshot(
                &events,
                &running,
                BatchRunEventKind::MemberStarted,
                Some(member.ordinal),
                Some(run_id),
            );

            let (run_events, _run_event_receiver) =
                tokio::sync::mpsc::unbounded_channel::<RunEvent>();
            let execution = self
                .cli_runs
                .execute_owned_batch_member(
                    CliBatchExecutionBinding {
                        batch_id,
                        member_ordinal: member.ordinal,
                        run_id,
                    },
                    self.pack.clone(),
                    adapter,
                    self.verifier.clone(),
                    cancellation.child_token(),
                    run_events,
                )
                .await;

            if cancellation.is_cancelled() {
                self.cancel_active(batch_id, &member, run_id, &events)?;
                return self.required_batch(batch_id);
            }
            self.commit_member_outcome(batch_id, &member, run_id, execution, &events)?;
        }
    }

    fn required_batch(&self, batch_id: Uuid) -> Result<ScanBatchRecord, BatchRunnerError> {
        self.repository
            .get_batch(batch_id)?
            .ok_or(BatchRunnerError::UnsupportedBatch)
    }

    fn validate_cli_batch(&self, batch: &ScanBatchRecord) -> Result<(), BatchRunnerError> {
        if !matches!(
            batch.plan.mode,
            BatchMode::QuickComparison | BatchMode::Standard | BatchMode::Full
        ) || (batch.plan.mode == BatchMode::Full && batch.baseline_snapshot.is_none())
            || batch.plan.suite_id != self.pack.manifest.id
            || batch.plan.suite_version != self.pack.manifest.version
            || batch.plan.suite_content_sha256 != self.pack.content_sha256
            || batch.plan.targets.is_empty()
            || batch.plan.targets.iter().any(|target| {
                target.route_identity.execution_surface != BatchExecutionSurface::AutomatedCli
                    || target.execution_adapter_identity.execution_surface
                        != BatchExecutionSurface::AutomatedCli
                    || target.execution_adapter_identity.public_version.is_none()
            })
        {
            return Err(BatchRunnerError::UnsupportedBatch);
        }
        Ok(())
    }

    fn environment_for_target(
        &self,
        target: &ScanBatchTarget,
        availability: &TargetAvailability,
        verifier_runtime_version: &str,
    ) -> Result<EnvironmentFingerprint, FailureKind> {
        if availability.kind != target.target.kind {
            return Err(FailureKind::AppInterrupted);
        }
        let failure = match availability.status {
            AvailabilityStatus::Ready if availability.installed => None,
            AvailabilityStatus::NeedsLogin => Some(FailureKind::AuthExpired),
            AvailabilityStatus::RuntimeMissing => Some(FailureKind::RuntimeMissing),
            AvailabilityStatus::NotFound | AvailabilityStatus::EntryInaccessible => {
                Some(FailureKind::CliMissing)
            }
            AvailabilityStatus::VersionProbeFailed | AvailabilityStatus::Ready => {
                Some(FailureKind::AppInterrupted)
            }
            AvailabilityStatus::VersionUnsupported => Some(FailureKind::RuntimeMissing),
        };
        if let Some(failure) = failure {
            return Err(failure);
        }
        if availability.auth_state == AuthState::NeedsLogin {
            return Err(FailureKind::AuthExpired);
        }
        let detected_launch = match availability.source {
            Some(LaunchSource::NativeExe) => AdapterLaunchKind::NativeExe,
            Some(LaunchSource::ReviewedNpm) => AdapterLaunchKind::ReviewedNpm,
            None => return Err(FailureKind::CliMissing),
        };
        let detected_version = public_adapter_version(
            target.target.kind,
            &target.execution_adapter_identity.adapter_contract_version,
            availability.version.clone(),
        )
        .ok_or(FailureKind::AppInterrupted)?;
        let detected_identity = ExecutionAdapterIdentity::new(
            BatchExecutionSurface::AutomatedCli,
            &target.execution_adapter_identity.provider_family,
            detected_launch,
            Some(&detected_version),
            &target.execution_adapter_identity.adapter_contract_version,
        )
        .map_err(|_| FailureKind::AppInterrupted)?;
        if detected_identity != target.execution_adapter_identity {
            return Err(FailureKind::AppInterrupted);
        }
        let os = os_info::get();
        Ok(EnvironmentFingerprint {
            os_family: os.os_type().to_string(),
            os_version: os.version().to_string(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            cli_version: Some(detected_version),
            verifier_runtime_version: Some(verifier_runtime_version.to_owned()),
            suite_id: self.pack.manifest.id.clone(),
            suite_version: self.pack.manifest.version.clone(),
            suite_content_sha256: self.pack.content_sha256.clone(),
            scoring_rule_version: batch_scoring_rule(),
            execution_adapter_identity: Some(target.execution_adapter_identity.clone()),
            resumed: false,
        })
    }

    fn defer_without_run(
        &self,
        batch: &ScanBatchRecord,
        member: &ScanBatchMemberRecord,
        failure: FailureKind,
        events: &UnboundedSender<BatchRunEvent>,
    ) -> Result<(), BatchRunnerError> {
        self.repository.defer_batch_member(
            batch.id,
            member.ordinal,
            member.run_id,
            failure,
            Utc::now(),
        )?;
        let deferred = self.required_batch(batch.id)?;
        emit_snapshot(
            events,
            &deferred,
            BatchRunEventKind::MemberFinished,
            Some(member.ordinal),
            member.run_id,
        );
        Ok(())
    }

    fn cancel_active(
        &self,
        batch_id: Uuid,
        member: &ScanBatchMemberRecord,
        run_id: Uuid,
        events: &UnboundedSender<BatchRunEvent>,
    ) -> Result<(), BatchRunnerError> {
        self.repository.cancel_batch(batch_id, Utc::now())?;
        let run = self
            .repository
            .get_run(run_id)?
            .ok_or(BatchRunnerError::TargetMismatch)?;
        let (terminal_status, failure_kind) = if run.status == RunStatus::Completed {
            (BatchMemberStatus::Completed, None)
        } else {
            (
                BatchMemberStatus::Cancelled,
                Some(FailureKind::UserCancelled),
            )
        };
        self.repository.finish_batch_member(
            batch_id,
            member.ordinal,
            run_id,
            terminal_status,
            failure_kind,
            Utc::now(),
        )?;
        let cancelled = self.required_batch(batch_id)?;
        emit_snapshot(
            events,
            &cancelled,
            BatchRunEventKind::MemberFinished,
            Some(member.ordinal),
            Some(run_id),
        );
        Ok(())
    }

    fn commit_member_outcome(
        &self,
        batch_id: Uuid,
        member: &ScanBatchMemberRecord,
        run_id: Uuid,
        _execution: Result<(), CliRunError>,
        events: &UnboundedSender<BatchRunEvent>,
    ) -> Result<(), BatchRunnerError> {
        let run = self
            .repository
            .get_run(run_id)?
            .ok_or(BatchRunnerError::TargetMismatch)?;
        let results = self.repository.get_task_results(run_id)?;
        if run.status == RunStatus::Completed {
            self.repository.finish_batch_member(
                batch_id,
                member.ordinal,
                run_id,
                BatchMemberStatus::Completed,
                None,
                Utc::now(),
            )?;
        } else if run.status == RunStatus::Cancelled
            || results
                .iter()
                .any(|result| result.outcome == TaskOutcome::Cancelled)
        {
            self.repository.cancel_batch(batch_id, Utc::now())?;
            self.repository.finish_batch_member(
                batch_id,
                member.ordinal,
                run_id,
                BatchMemberStatus::Cancelled,
                Some(FailureKind::UserCancelled),
                Utc::now(),
            )?;
        } else {
            let failure = results
                .iter()
                .find_map(|result| {
                    (result.outcome == TaskOutcome::Invalid)
                        .then_some(result.failure_kind)
                        .flatten()
                })
                .filter(|failure| retryable_failure(*failure))
                .unwrap_or(FailureKind::AppInterrupted);
            self.repository.defer_batch_member(
                batch_id,
                member.ordinal,
                Some(run_id),
                failure,
                Utc::now(),
            )?;
        }
        let committed = self.required_batch(batch_id)?;
        emit_snapshot(
            events,
            &committed,
            BatchRunEventKind::MemberFinished,
            Some(member.ordinal),
            Some(run_id),
        );
        Ok(())
    }
}

fn target_for_member<'a>(
    batch: &'a ScanBatchRecord,
    member: &ScanBatchMemberRecord,
) -> Result<&'a ScanBatchTarget, BatchRunnerError> {
    batch
        .plan
        .targets
        .get(
            usize::try_from(member.target_position)
                .map_err(|_| BatchRunnerError::TargetMismatch)?,
        )
        .ok_or(BatchRunnerError::TargetMismatch)
}

fn proposed_run(
    batch: &ScanBatchRecord,
    target: &ScanBatchTarget,
    run_id: Uuid,
    mut environment: EnvironmentFingerprint,
) -> RunRecord {
    environment.scoring_rule_version = batch.plan.scoring_rule_version.clone();
    RunRecord {
        id: run_id,
        target: target.target.clone(),
        mode: match batch.plan.mode {
            BatchMode::QuickComparison => RunMode::Quick,
            BatchMode::Standard | BatchMode::Full => RunMode::Deep,
        },
        suite_id: batch.plan.suite_id.clone(),
        suite_version: batch.plan.suite_version.clone(),
        status: RunStatus::Created,
        started_at: Utc::now(),
        finished_at: None,
        total_tasks: u32::try_from(batch.plan.sealed_task_budgets.len()).unwrap_or(u32::MAX),
        completed_tasks: 0,
        environment,
        score: None,
    }
}

fn retryable_failure(failure: FailureKind) -> bool {
    matches!(
        failure,
        FailureKind::CliMissing
            | FailureKind::RuntimeMissing
            | FailureKind::AuthExpired
            | FailureKind::QuotaExhausted
            | FailureKind::Network
            | FailureKind::AppInterrupted
            | FailureKind::InfrastructureTimeout
            | FailureKind::VerifierError
    )
}

fn batch_scoring_rule() -> String {
    "ability-v1".into()
}

fn emit_snapshot(
    events: &UnboundedSender<BatchRunEvent>,
    batch: &ScanBatchRecord,
    kind: BatchRunEventKind,
    member_ordinal: Option<u32>,
    run_id: Option<Uuid>,
) {
    let _ = events.send(BatchRunEvent {
        batch_id: batch.id,
        kind,
        member_ordinal,
        run_id,
        terminal_members: batch.terminal_member_count,
        planned_members: batch.planned_member_count,
        status: batch.status,
    });
}

#[cfg(test)]
use ability_adapters::{AdapterCompletion, AdapterError, ExecutionRequest, VerificationGrade};
#[cfg(test)]
use ability_core::{
    build_batch_schedule, BatchMemberSeed, ModelSource, ModelVerification, PackLoader,
    ScanBatchPlan, ScanExecutionAuthorization, TargetSelection,
};
#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use chrono::Duration;
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
#[derive(Default)]
struct AdapterMetrics {
    detects: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    calls: AtomicUsize,
    max_turns: Mutex<u64>,
    time_budget_secs: Mutex<u64>,
    workspaces: Mutex<Vec<PathBuf>>,
}

#[cfg(test)]
struct FakeAdapter {
    kind: TargetKind,
    contract_version: &'static str,
    availability: TargetAvailability,
    metrics: Arc<AdapterMetrics>,
    execute_failure: Option<FailureKind>,
    wait_for_cancellation: bool,
}

#[cfg(test)]
#[async_trait]
impl AgentAdapter for FakeAdapter {
    fn kind(&self) -> TargetKind {
        self.kind
    }

    fn contract_version(&self) -> &'static str {
        self.contract_version
    }

    async fn detect(&self) -> TargetAvailability {
        self.metrics.detects.fetch_add(1, Ordering::SeqCst);
        self.availability.clone()
    }

    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError> {
        let active = self.metrics.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.metrics.max_active.fetch_max(active, Ordering::SeqCst);
        self.metrics.calls.fetch_add(1, Ordering::SeqCst);
        *self.metrics.max_turns.lock().unwrap() += u64::from(request.max_turns);
        *self.metrics.time_budget_secs.lock().unwrap() += request.time_budget_secs;
        self.metrics
            .workspaces
            .lock()
            .unwrap()
            .push(request.workspace);
        if self.wait_for_cancellation {
            cancellation.cancelled().await;
            self.metrics.active.fetch_sub(1, Ordering::SeqCst);
            return Err(AdapterError::Cancelled);
        }
        tokio::task::yield_now().await;
        self.metrics.active.fetch_sub(1, Ordering::SeqCst);
        if let Some(kind) = self.execute_failure {
            return Err(AdapterError::Infrastructure {
                kind,
                detail: "synthetic infrastructure failure".into(),
            });
        }
        Ok(AdapterCompletion::Completed {
            duration_ms: 1,
            stdout: "synthetic completion".into(),
            stderr: String::new(),
            evidence: None,
        })
    }
}

#[cfg(test)]
struct PassingVerifier;

#[cfg(test)]
#[async_trait]
impl WorkspaceVerifier for PassingVerifier {
    async fn verify(
        &self,
        _verifier_id: &str,
        _workspace: &Path,
        _cancellation: CancellationToken,
    ) -> VerificationGrade {
        VerificationGrade {
            outcome: TaskOutcome::Passed,
            score: Some(100.0),
            failure_kind: None,
            detail: "synthetic verifier pass".into(),
            duration_ms: 1,
        }
    }
}

#[cfg(test)]
struct RunnerFixture {
    _temp: tempfile::TempDir,
    repository: Arc<RunRepository>,
    pack: Arc<LoadedPack>,
    runner: BatchRunner,
}

#[cfg(test)]
impl RunnerFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repository = Arc::new(RunRepository::open(&temp.path().join("radar.db")).unwrap());
        let pack = Arc::new(
            PackLoader::load(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../benchmark-packs/cli-quick-v1"),
            )
            .unwrap(),
        );
        let cli_runs = Arc::new(CliRunService::new(
            repository.clone(),
            temp.path().join("artifacts"),
        ));
        std::fs::create_dir_all(temp.path().join("artifacts")).unwrap();
        let runner = BatchRunner::new(
            repository.clone(),
            cli_runs,
            pack.clone(),
            Arc::new(PassingVerifier),
            RunOperationRegistry::default(),
        );
        Self {
            _temp: temp,
            repository,
            pack,
            runner,
        }
    }

    fn create_batch(&self, mode: BatchMode, targets: Vec<ScanBatchTarget>) -> ScanBatchRecord {
        let now = Utc::now();
        let plan = ScanBatchPlan::new(&self.pack, "ability-v1", mode, 19, targets, now).unwrap();
        let members = build_batch_schedule(&plan)
            .unwrap()
            .members
            .into_iter()
            .map(|member| BatchMemberSeed {
                ordinal: member.ordinal,
                target_position: member.target_position,
                repetition_index: member.repetition_index,
            })
            .collect::<Vec<_>>();
        let batch_id = Uuid::new_v4();
        if mode == BatchMode::Full {
            self.repository
                .create_full_batch_with_baseline_snapshot(
                    batch_id,
                    &self.pack,
                    &plan,
                    &members,
                    now,
                    &ability_core::CalibrationPolicy::production_v1(),
                )
                .unwrap();
        } else {
            self.repository
                .insert_batch_plan(batch_id, &self.pack, &plan, &members, now)
                .unwrap();
        }
        self.repository
            .append_execution_authorization(&ScanExecutionAuthorization {
                batch_id,
                member_ordinal: None,
                attempt_number: 1,
                max_provider_turns: plan.cost_estimate.max_provider_turns,
                max_task_budget_secs: plan.cost_estimate.summed_task_budget_secs,
                acknowledgement_hash: plan.acknowledgement_hash.clone(),
                allowed_failure_kind: None,
                expires_at: plan
                    .cost_estimate
                    .execution_authorization_expires_at(now)
                    .unwrap(),
                created_at: now,
            })
            .unwrap();
        self.repository.resume_batch(batch_id, now).unwrap();
        self.repository.get_batch(batch_id).unwrap().unwrap()
    }

    fn authorize_retry(&self, batch: &ScanBatchRecord, member: &ScanBatchMemberRecord) {
        let now = Utc::now();
        let failure = member.failure_kind.unwrap();
        let planned_runs = batch.plan.cost_estimate.planned_member_runs;
        let mut authorization = ScanExecutionAuthorization {
            batch_id: batch.id,
            member_ordinal: Some(member.ordinal),
            attempt_number: member.attempt_number + 1,
            max_provider_turns: batch.plan.cost_estimate.max_provider_turns / planned_runs,
            max_task_budget_secs: batch.plan.cost_estimate.summed_task_budget_secs / planned_runs,
            acknowledgement_hash: String::new(),
            allowed_failure_kind: Some(failure),
            expires_at: now + Duration::hours(8),
            created_at: now,
        };
        authorization.acknowledgement_hash = authorization
            .expected_retry_acknowledgement_hash(&batch.plan)
            .unwrap();
        self.repository
            .append_execution_authorization(&authorization)
            .unwrap();
    }
}

#[cfg(test)]
fn cli_target(kind: TargetKind, model: &str) -> ScanBatchTarget {
    let (provider, version, contract) = match kind {
        TargetKind::CodexCli => ("openai", "codex-cli 1.2.3", "codex-cli-v1"),
        TargetKind::ClaudeCode => ("anthropic", "1.2.3 (Claude Code)", "claude-code-v1"),
        _ => unreachable!(),
    };
    ScanBatchTarget::new(
        TargetSelection {
            kind,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
            model_source: ModelSource::CliRequested,
            model_verification: ModelVerification::UserConfirmed,
        },
        BatchExecutionSurface::AutomatedCli,
        ExecutionAdapterIdentity::new(
            BatchExecutionSurface::AutomatedCli,
            provider,
            AdapterLaunchKind::NativeExe,
            Some(version),
            contract,
        )
        .unwrap(),
    )
    .unwrap()
}

#[cfg(test)]
fn fake_adapter(
    kind: TargetKind,
    status: AvailabilityStatus,
    metrics: Arc<AdapterMetrics>,
) -> Arc<dyn AgentAdapter> {
    let version = match kind {
        TargetKind::CodexCli => "codex-cli 1.2.3",
        TargetKind::ClaudeCode => "1.2.3 (Claude Code)",
        _ => unreachable!(),
    };
    Arc::new(FakeAdapter {
        kind,
        contract_version: match kind {
            TargetKind::CodexCli => "codex-cli-v1",
            TargetKind::ClaudeCode => "claude-code-v1",
            _ => unreachable!(),
        },
        availability: TargetAvailability {
            kind,
            installed: status == AvailabilityStatus::Ready,
            version: Some(version.into()),
            auth_state: AuthState::Ready,
            status,
            source: Some(LaunchSource::NativeExe),
            prerequisites: Vec::new(),
        },
        metrics,
        execute_failure: None,
        wait_for_cancellation: false,
    })
}

#[cfg(test)]
fn failing_adapter(
    kind: TargetKind,
    failure: FailureKind,
    metrics: Arc<AdapterMetrics>,
) -> Arc<dyn AgentAdapter> {
    let version = match kind {
        TargetKind::CodexCli => "codex-cli 1.2.3",
        TargetKind::ClaudeCode => "1.2.3 (Claude Code)",
        _ => unreachable!(),
    };
    Arc::new(FakeAdapter {
        kind,
        contract_version: match kind {
            TargetKind::CodexCli => "codex-cli-v1",
            TargetKind::ClaudeCode => "claude-code-v1",
            _ => unreachable!(),
        },
        availability: TargetAvailability {
            kind,
            installed: true,
            version: Some(version.into()),
            auth_state: AuthState::Ready,
            status: AvailabilityStatus::Ready,
            source: Some(LaunchSource::NativeExe),
            prerequisites: Vec::new(),
        },
        metrics,
        execute_failure: Some(failure),
        wait_for_cancellation: false,
    })
}

#[cfg(test)]
fn cancellation_adapter(kind: TargetKind, metrics: Arc<AdapterMetrics>) -> Arc<dyn AgentAdapter> {
    let version = match kind {
        TargetKind::CodexCli => "codex-cli 1.2.3",
        TargetKind::ClaudeCode => "1.2.3 (Claude Code)",
        _ => unreachable!(),
    };
    Arc::new(FakeAdapter {
        kind,
        contract_version: match kind {
            TargetKind::CodexCli => "codex-cli-v1",
            TargetKind::ClaudeCode => "claude-code-v1",
            _ => unreachable!(),
        },
        availability: TargetAvailability {
            kind,
            installed: true,
            version: Some(version.into()),
            auth_state: AuthState::Ready,
            status: AvailabilityStatus::Ready,
            source: Some(LaunchSource::NativeExe),
            prerequisites: Vec::new(),
        },
        metrics,
        execute_failure: None,
        wait_for_cancellation: true,
    })
}

#[cfg(test)]
#[tokio::test]
async fn executes_at_most_one_cli_member() {
    let fixture = RunnerFixture::new();
    let batch = fixture.create_batch(
        BatchMode::Standard,
        vec![
            cli_target(TargetKind::CodexCli, "gpt-5.6-sol"),
            cli_target(TargetKind::CodexCli, "gpt-5.6-terra"),
        ],
    );
    let metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([(
        TargetKind::CodexCli,
        fake_adapter(
            TargetKind::CodexCli,
            AvailabilityStatus::Ready,
            metrics.clone(),
        ),
    )]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();

    let completed = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();

    assert_eq!(completed.status, BatchStatus::Completed);
    assert_eq!(metrics.max_active.load(Ordering::SeqCst), 1);
    assert_eq!(metrics.detects.load(Ordering::SeqCst), 1);
    assert_eq!(
        u64::try_from(metrics.calls.load(Ordering::SeqCst)).unwrap(),
        completed.plan.cost_estimate.task_launches
    );
    assert_eq!(
        *metrics.max_turns.lock().unwrap(),
        completed.plan.cost_estimate.max_provider_turns
    );
    assert_eq!(
        *metrics.time_budget_secs.lock().unwrap(),
        completed.plan.cost_estimate.summed_task_budget_secs
    );
    let workspaces = metrics.workspaces.lock().unwrap();
    assert_eq!(workspaces.len(), metrics.calls.load(Ordering::SeqCst));
    assert_eq!(
        workspaces
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        workspaces.len()
    );
}

#[cfg(test)]
#[tokio::test]
async fn rejects_an_adapter_whose_execution_contract_does_not_match_the_plan() {
    let fixture = RunnerFixture::new();
    let batch = fixture.create_batch(
        BatchMode::Standard,
        vec![
            cli_target(TargetKind::CodexCli, "gpt-5.6-terra"),
            cli_target(TargetKind::CodexCli, "gpt-5.6-sol"),
        ],
    );
    let metrics = Arc::new(AdapterMetrics::default());
    let adapters: BTreeMap<TargetKind, Arc<dyn AgentAdapter>> = BTreeMap::from([(
        TargetKind::CodexCli,
        Arc::new(FakeAdapter {
            kind: TargetKind::CodexCli,
            contract_version: "promptfoo-agent-v1",
            availability: TargetAvailability {
                kind: TargetKind::CodexCli,
                installed: true,
                version: Some("promptfoo 0.122.0 codex-sdk 0.147.0 openai-codex-sdk".into()),
                auth_state: AuthState::Ready,
                status: AvailabilityStatus::Ready,
                source: Some(LaunchSource::ReviewedNpm),
                prerequisites: Vec::new(),
            },
            metrics: metrics.clone(),
            execute_failure: None,
            wait_for_cancellation: false,
        }) as Arc<dyn AgentAdapter>,
    )]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();

    let result = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await;

    assert!(matches!(result, Err(BatchRunnerError::TargetMismatch)));
    assert_eq!(metrics.detects.load(Ordering::SeqCst), 0);
    assert_eq!(metrics.calls.load(Ordering::SeqCst), 0);
}

#[cfg(test)]
#[tokio::test]
async fn ambiguous_launch_is_not_replayed() {
    let fixture = RunnerFixture::new();
    let batch = fixture.create_batch(
        BatchMode::QuickComparison,
        vec![
            cli_target(TargetKind::ClaudeCode, "claude-sonnet-4-5"),
            cli_target(TargetKind::CodexCli, "gpt-5.6-sol"),
        ],
    );
    let member = batch.members[0].clone();
    let target = target_for_member(&batch, &member).unwrap();
    let availability_kind = target.target.kind;
    let availability = TargetAvailability {
        kind: availability_kind,
        installed: true,
        version: Some(
            match availability_kind {
                TargetKind::CodexCli => "codex-cli 1.2.3",
                TargetKind::ClaudeCode => "1.2.3 (Claude Code)",
                _ => unreachable!(),
            }
            .into(),
        ),
        auth_state: AuthState::Ready,
        status: AvailabilityStatus::Ready,
        source: Some(LaunchSource::NativeExe),
        prerequisites: Vec::new(),
    };
    let environment = fixture
        .runner
        .environment_for_target(target, &availability, "v22.0.0")
        .unwrap();
    let run_id = Uuid::new_v4();
    let proposed = proposed_run(&batch, target, run_id, environment);
    fixture
        .repository
        .reserve_next_runnable_member_and_run(batch.id, Utc::now(), &proposed)
        .unwrap()
        .unwrap();
    fixture
        .repository
        .mark_member_launching(batch.id, member.ordinal, run_id, Utc::now())
        .unwrap();
    let metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([(
        TargetKind::CodexCli,
        fake_adapter(
            TargetKind::CodexCli,
            AvailabilityStatus::Ready,
            metrics.clone(),
        ),
    )]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();

    let error = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, BatchRunnerError::AmbiguousActive));
    assert_eq!(metrics.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.repository.get_run(run_id).unwrap().unwrap().status,
        RunStatus::Created
    );
}

#[cfg(test)]
#[tokio::test]
async fn deferred_target_does_not_block_runnable_target() {
    let fixture = RunnerFixture::new();
    let batch = fixture.create_batch(
        BatchMode::QuickComparison,
        vec![
            cli_target(TargetKind::ClaudeCode, "claude-sonnet-4-5"),
            cli_target(TargetKind::CodexCli, "gpt-5.6-sol"),
        ],
    );
    let codex_metrics = Arc::new(AdapterMetrics::default());
    let claude_metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([
        (
            TargetKind::CodexCli,
            fake_adapter(
                TargetKind::CodexCli,
                AvailabilityStatus::NotFound,
                codex_metrics.clone(),
            ),
        ),
        (
            TargetKind::ClaudeCode,
            fake_adapter(
                TargetKind::ClaudeCode,
                AvailabilityStatus::Ready,
                claude_metrics.clone(),
            ),
        ),
    ]);
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();

    let paused = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();

    assert_eq!(paused.status, BatchStatus::Paused);
    let codex_member = paused
        .members
        .iter()
        .find(|member| {
            target_for_member(&paused, member).unwrap().target.kind == TargetKind::CodexCli
        })
        .unwrap();
    let claude_member = paused
        .members
        .iter()
        .find(|member| {
            target_for_member(&paused, member).unwrap().target.kind == TargetKind::ClaudeCode
        })
        .unwrap();
    assert_eq!(codex_member.status, BatchMemberStatus::Deferred);
    assert_eq!(codex_member.failure_kind, Some(FailureKind::CliMissing));
    assert_eq!(claude_member.status, BatchMemberStatus::Completed);
    assert_eq!(codex_metrics.calls.load(Ordering::SeqCst), 0);
    assert_eq!(claude_metrics.calls.load(Ordering::SeqCst), 2);
    let emitted = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
    assert!(emitted.iter().any(|event| {
        event.member_ordinal == Some(codex_member.ordinal)
            && event.kind == BatchRunEventKind::MemberFinished
            && event.status == BatchStatus::Running
    }));

    let original_schedule = paused
        .members
        .iter()
        .map(|member| {
            (
                member.ordinal,
                member.target_position,
                member.repetition_index,
            )
        })
        .collect::<Vec<_>>();
    fixture.authorize_retry(&paused, codex_member);
    let retry_metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([(
        TargetKind::CodexCli,
        fake_adapter(
            TargetKind::CodexCli,
            AvailabilityStatus::Ready,
            retry_metrics.clone(),
        ),
    )]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let completed = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();

    assert_eq!(completed.status, BatchStatus::Completed);
    assert_eq!(
        completed
            .members
            .iter()
            .map(|member| (
                member.ordinal,
                member.target_position,
                member.repetition_index
            ))
            .collect::<Vec<_>>(),
        original_schedule
    );
    assert_eq!(retry_metrics.calls.load(Ordering::SeqCst), 2);
    assert_eq!(claude_metrics.calls.load(Ordering::SeqCst), 2);
}

#[cfg(test)]
#[tokio::test]
async fn infrastructure_retry_reuses_the_exact_run_and_failure_marker() {
    let fixture = RunnerFixture::new();
    let batch = fixture.create_batch(
        BatchMode::QuickComparison,
        vec![
            cli_target(TargetKind::ClaudeCode, "claude-sonnet-4-5"),
            cli_target(TargetKind::CodexCli, "gpt-5.6-sol"),
        ],
    );
    let failing_metrics = Arc::new(AdapterMetrics::default());
    let claude_metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([
        (
            TargetKind::CodexCli,
            failing_adapter(
                TargetKind::CodexCli,
                FailureKind::Network,
                failing_metrics.clone(),
            ),
        ),
        (
            TargetKind::ClaudeCode,
            fake_adapter(
                TargetKind::ClaudeCode,
                AvailabilityStatus::Ready,
                claude_metrics,
            ),
        ),
    ]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let paused = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();
    let failed_member = paused
        .members
        .iter()
        .find(|member| member.failure_kind == Some(FailureKind::Network))
        .unwrap();
    let original_run_id = failed_member.run_id.unwrap();
    let marker = fixture
        .repository
        .get_task_results(original_run_id)
        .unwrap();
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].outcome, TaskOutcome::Invalid);
    assert_eq!(failing_metrics.calls.load(Ordering::SeqCst), 1);

    fixture.authorize_retry(&paused, failed_member);
    let unavailable_metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([(
        TargetKind::CodexCli,
        fake_adapter(
            TargetKind::CodexCli,
            AvailabilityStatus::NotFound,
            unavailable_metrics.clone(),
        ),
    )]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let paused_again = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();
    let deferred_again = paused_again
        .members
        .iter()
        .find(|member| member.run_id == Some(original_run_id))
        .unwrap();
    assert_eq!(deferred_again.status, BatchMemberStatus::Deferred);
    assert_eq!(deferred_again.failure_kind, Some(FailureKind::Network));
    assert_eq!(deferred_again.attempt_number, 2);
    assert_eq!(unavailable_metrics.calls.load(Ordering::SeqCst), 0);

    fixture.authorize_retry(&paused_again, deferred_again);
    let success_metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([(
        TargetKind::CodexCli,
        fake_adapter(
            TargetKind::CodexCli,
            AvailabilityStatus::Ready,
            success_metrics.clone(),
        ),
    )]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let completed = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();
    let resumed_member = completed
        .members
        .iter()
        .find(|member| member.run_id == Some(original_run_id))
        .unwrap();
    assert_eq!(resumed_member.status, BatchMemberStatus::Completed);
    assert_eq!(resumed_member.attempt_number, 3);
    let results = fixture
        .repository
        .get_task_results(original_run_id)
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .all(|result| result.outcome == TaskOutcome::Passed));
    assert_eq!(success_metrics.calls.load(Ordering::SeqCst), 2);
}

#[cfg(test)]
#[tokio::test]
async fn cancellation_stops_the_active_member_and_prevents_future_launches() {
    let fixture = RunnerFixture::new();
    let batch = fixture.create_batch(
        BatchMode::QuickComparison,
        vec![
            cli_target(TargetKind::CodexCli, "gpt-5.6-sol"),
            cli_target(TargetKind::CodexCli, "gpt-5.6-terra"),
        ],
    );
    let metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([(
        TargetKind::CodexCli,
        cancellation_adapter(TargetKind::CodexCli, metrics.clone()),
    )]);
    let cancellation = CancellationToken::new();
    let cancel_after_launch = cancellation.clone();
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();

    let run = fixture
        .runner
        .run(batch.id, adapters, "v22.0.0".into(), cancellation, events);
    let cancel = async {
        while metrics.calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        cancel_after_launch.cancel();
    };
    let (result, ()) = tokio::join!(run, cancel);
    let cancelled = result.unwrap();

    assert_eq!(cancelled.status, BatchStatus::Cancelled);
    assert!(cancelled.cancel_requested);
    assert_eq!(metrics.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        cancelled
            .members
            .iter()
            .filter(|member| member.run_id.is_some())
            .count(),
        1
    );
    assert!(cancelled
        .members
        .iter()
        .all(|member| member.status == BatchMemberStatus::Cancelled));
}

#[cfg(test)]
#[tokio::test]
async fn full_batch_runs_only_after_atomic_snapshot_creation() {
    let fixture = RunnerFixture::new();
    let batch = fixture.create_batch(
        BatchMode::Full,
        vec![
            cli_target(TargetKind::CodexCli, "gpt-5.6-sol"),
            cli_target(TargetKind::ClaudeCode, "claude-sonnet-4-5"),
        ],
    );
    let metrics = Arc::new(AdapterMetrics::default());
    let adapters = BTreeMap::from([
        (
            TargetKind::CodexCli,
            fake_adapter(
                TargetKind::CodexCli,
                AvailabilityStatus::Ready,
                metrics.clone(),
            ),
        ),
        (
            TargetKind::ClaudeCode,
            fake_adapter(
                TargetKind::ClaudeCode,
                AvailabilityStatus::Ready,
                metrics.clone(),
            ),
        ),
    ]);
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();

    let completed = fixture
        .runner
        .run(
            batch.id,
            adapters,
            "v22.0.0".into(),
            CancellationToken::new(),
            events,
        )
        .await
        .unwrap();

    assert_eq!(completed.status, BatchStatus::Completed);
    assert!(completed.baseline_snapshot.is_some());
    assert!(metrics.detects.load(Ordering::SeqCst) > 0);
    assert!(metrics.calls.load(Ordering::SeqCst) > 0);
    assert!(fixture
        .repository
        .get_batch(batch.id)
        .unwrap()
        .unwrap()
        .members
        .iter()
        .all(|member| member.status == BatchMemberStatus::Completed));
    let analysis = fixture
        .repository
        .analyze_batch(batch.id, &ability_core::CalibrationPolicy::production_v1())
        .unwrap();
    assert_eq!(
        analysis.signal,
        ability_core::RegressionSignal::InsufficientData
    );
    assert_eq!(analysis.targets.len(), 2);
    assert!(analysis
        .targets
        .iter()
        .all(|target| target.candidate_member_count == 5));
}
