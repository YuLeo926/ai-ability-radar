use crate::app_state::AppState;
use crate::dto::{
    AuthorizeBatchExecutionInput, AuthorizeBatchRetryInput, BatchEstimateDto,
    BatchExecutionAuthorizationDto, BatchIdInput, BatchPlanInput, BatchRetryEstimateDto,
    BatchTargetInput, CreateAcknowledgedBatchInput, EstimateBatchRetryInput, GuidedMemberDecision,
    NextGuidedMemberDto,
};
use ability_core::{
    build_batch_schedule, select_next_scheduled_member, AdapterLaunchKind, BatchExecutionSurface,
    BatchFeatureLevel, BatchMemberSeed, BatchMemberStatus, BatchMode, LoadedPack,
    NextScheduledMember, RunRepository, ScanBatchPlan, ScanBatchRecord, ScanBatchTarget,
    ScanExecutionAuthorization, ScheduledMemberLifecycle, ScheduledMemberState, TargetKind,
    TargetSelection,
};
use chrono::{DateTime, Utc};
use tauri::State;
use uuid::Uuid;

pub(crate) const BATCH_CAPABILITIES: [BatchFeatureLevel; 2] = [
    BatchFeatureLevel::GuidedQuickV1,
    BatchFeatureLevel::CliStandardV1,
];
const SCORING_RULE_VERSION: &str = "ability-v1";
const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_BATCH_LIST_ITEMS: usize = 256;

pub(crate) struct BatchCommandContext<'a> {
    pub(crate) repository: &'a RunRepository,
    pub(crate) client_pack: &'a LoadedPack,
    pub(crate) cli_pack: &'a LoadedPack,
}

impl<'a> BatchCommandContext<'a> {
    fn from_state(state: &'a AppState) -> Self {
        Self {
            repository: &state.repository,
            client_pack: &state.client_pack,
            cli_pack: &state.cli_pack,
        }
    }
}

fn has_capability(capability: BatchFeatureLevel) -> bool {
    BATCH_CAPABILITIES.contains(&capability)
}

fn ensure_supported(surface: BatchExecutionSurface, mode: BatchMode) -> Result<(), String> {
    let supported = match (surface, mode) {
        (BatchExecutionSurface::GuidedClient, BatchMode::QuickComparison) => {
            has_capability(BatchFeatureLevel::GuidedQuickV1)
        }
        (BatchExecutionSurface::AutomatedCli, BatchMode::QuickComparison | BatchMode::Standard) => {
            has_capability(BatchFeatureLevel::CliStandardV1)
        }
        (BatchExecutionSurface::AutomatedCli, BatchMode::Full) => {
            has_capability(BatchFeatureLevel::ReliableFullV1)
        }
        _ => false,
    };
    supported
        .then_some(())
        .ok_or_else(|| "batch mode is not enabled by the reviewed backend capabilities".into())
}

fn expected_adapter_contract(kind: TargetKind) -> (&'static str, &'static str) {
    match kind {
        TargetKind::ChatGptClient => ("openai", "guided-client-v1"),
        TargetKind::ClaudeClient => ("anthropic", "guided-client-v1"),
        TargetKind::CodexCli => ("openai", "codex-cli-v1"),
        TargetKind::ClaudeCode => ("anthropic", "claude-code-v1"),
    }
}

fn validate_reviewed_adapter(input: &BatchTargetInput) -> Result<(), String> {
    let adapter = &input.execution_adapter_identity;
    let (provider, contract) = expected_adapter_contract(input.target.kind);
    let launch_is_reviewed = match input.target.kind {
        TargetKind::ChatGptClient | TargetKind::ClaudeClient => {
            input.execution_surface == BatchExecutionSurface::GuidedClient
                && adapter.launch_kind == AdapterLaunchKind::GuidedClient
                && adapter.public_version.is_none()
        }
        TargetKind::CodexCli | TargetKind::ClaudeCode => {
            input.execution_surface == BatchExecutionSurface::AutomatedCli
                && matches!(
                    adapter.launch_kind,
                    AdapterLaunchKind::NativeExe | AdapterLaunchKind::ReviewedNpm
                )
        }
    };
    if adapter.execution_surface != input.execution_surface
        || adapter.provider_family != provider
        || adapter.adapter_contract_version != contract
        || !launch_is_reviewed
    {
        return Err("batch target uses an unknown or mismatched execution adapter".into());
    }
    Ok(())
}

fn validated_target(input: BatchTargetInput) -> Result<ScanBatchTarget, String> {
    validate_reviewed_adapter(&input)?;
    ScanBatchTarget::new(
        TargetSelection {
            kind: input.target.kind,
            reported_model: input.target.reported_model,
            reasoning_effort: input.target.reasoning_effort,
            model_source: input.target.model_source,
            model_verification: input.target.model_verification,
        },
        input.execution_surface,
        input.execution_adapter_identity,
    )
    .map_err(|error| error.to_string())
}

fn build_plan_at<'a>(
    context: &'a BatchCommandContext<'a>,
    input: BatchPlanInput,
    issued_at: DateTime<Utc>,
) -> Result<(ScanBatchPlan, &'a LoadedPack), String> {
    if input.seed > MAX_SAFE_JSON_INTEGER {
        return Err("batch seed exceeds the exact JSON integer range".into());
    }
    let surface = input
        .targets
        .first()
        .map(|target| target.execution_surface)
        .ok_or_else(|| "batch target cohort is empty".to_string())?;
    ensure_supported(surface, input.mode)?;
    let pack = match surface {
        BatchExecutionSurface::GuidedClient => context.client_pack,
        BatchExecutionSurface::AutomatedCli => context.cli_pack,
    };
    let targets = input
        .targets
        .into_iter()
        .map(validated_target)
        .collect::<Result<Vec<_>, _>>()?;
    let plan = ScanBatchPlan::new(
        pack,
        SCORING_RULE_VERSION,
        input.mode,
        input.seed,
        targets,
        issued_at,
    )
    .map_err(|error| error.to_string())?;
    Ok((plan, pack))
}

fn required_batch(
    context: &BatchCommandContext<'_>,
    batch_id: Uuid,
) -> Result<ScanBatchRecord, String> {
    context
        .repository
        .get_batch(batch_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "batch does not exist".into())
}

fn batch_surface(record: &ScanBatchRecord) -> Result<BatchExecutionSurface, String> {
    let surface = record
        .plan
        .targets
        .first()
        .map(|target| target.route_identity.execution_surface)
        .ok_or_else(|| "stored batch target cohort is empty".to_string())?;
    if record
        .plan
        .targets
        .iter()
        .any(|target| target.route_identity.execution_surface != surface)
    {
        return Err("stored batch mixes execution surfaces".into());
    }
    Ok(surface)
}

fn ensure_record_supported(record: &ScanBatchRecord) -> Result<BatchExecutionSurface, String> {
    let surface = batch_surface(record)?;
    ensure_supported(surface, record.plan.mode)?;
    build_batch_schedule(&record.plan).map_err(|error| error.to_string())?;
    Ok(surface)
}

pub(crate) fn estimate_batch_at(
    context: &BatchCommandContext<'_>,
    input: BatchPlanInput,
    now: DateTime<Utc>,
) -> Result<BatchEstimateDto, String> {
    let (plan, _) = build_plan_at(context, input, now)?;
    Ok(BatchEstimateDto {
        plan,
        capabilities: BATCH_CAPABILITIES.to_vec(),
    })
}

pub(crate) fn create_acknowledged_batch_at(
    context: &BatchCommandContext<'_>,
    input: CreateAcknowledgedBatchInput,
    now: DateTime<Utc>,
    batch_id: Uuid,
) -> Result<ScanBatchRecord, String> {
    let (plan, pack) = build_plan_at(context, input.plan, input.estimate_issued_at)?;
    if input.acknowledgement_hash != plan.acknowledgement_hash {
        return Err("batch acknowledgement is stale or does not match the plan".into());
    }
    if now < plan.cost_estimate.issued_at
        || now > plan.cost_estimate.initial_acknowledgement_expires_at
    {
        return Err("batch acknowledgement has expired".into());
    }
    let schedule = build_batch_schedule(&plan).map_err(|error| error.to_string())?;
    let members = schedule
        .members
        .into_iter()
        .map(|member| BatchMemberSeed {
            ordinal: member.ordinal,
            target_position: member.target_position,
            repetition_index: member.repetition_index,
        })
        .collect::<Vec<_>>();
    context
        .repository
        .insert_batch_plan(batch_id, pack, &plan, &members, now)
        .map_err(|error| error.to_string())?;
    required_batch(context, batch_id)
}

pub(crate) fn get_batch_record(
    context: &BatchCommandContext<'_>,
    input: BatchIdInput,
) -> Result<Option<ScanBatchRecord>, String> {
    context
        .repository
        .get_batch(input.batch_id)
        .map_err(|error| error.to_string())
}

pub(crate) fn list_batch_records(
    context: &BatchCommandContext<'_>,
) -> Result<Vec<ScanBatchRecord>, String> {
    let records = context
        .repository
        .list_batches()
        .map_err(|error| error.to_string())?;
    if records.len() > MAX_BATCH_LIST_ITEMS {
        return Err("batch list exceeds the reviewed response bound".into());
    }
    Ok(records)
}

pub(crate) fn authorize_batch_execution_at(
    context: &BatchCommandContext<'_>,
    input: AuthorizeBatchExecutionInput,
    now: DateTime<Utc>,
) -> Result<BatchExecutionAuthorizationDto, String> {
    let batch = required_batch(context, input.batch_id)?;
    ensure_record_supported(&batch)?;
    if input.acknowledgement_hash != batch.plan.acknowledgement_hash {
        return Err("batch acknowledgement is stale or does not match the stored plan".into());
    }
    if now < batch.created_at || now > batch.plan.cost_estimate.initial_acknowledgement_expires_at {
        return Err("initial batch execution acknowledgement has expired".into());
    }
    let authorization = ScanExecutionAuthorization {
        batch_id: input.batch_id,
        member_ordinal: None,
        attempt_number: 1,
        max_provider_turns: batch.plan.cost_estimate.max_provider_turns,
        max_task_budget_secs: batch.plan.cost_estimate.summed_task_budget_secs,
        acknowledgement_hash: batch.plan.acknowledgement_hash.clone(),
        allowed_failure_kind: None,
        expires_at: batch
            .plan
            .cost_estimate
            .execution_authorization_expires_at(now)
            .map_err(|error| error.to_string())?,
        created_at: now,
    };
    let response = execution_authorization_dto(&batch.plan, authorization.clone())?;
    context
        .repository
        .append_execution_authorization(&authorization)
        .map_err(|error| error.to_string())?;
    Ok(response)
}

fn execution_authorization_dto(
    plan: &ScanBatchPlan,
    authorization: ScanExecutionAuthorization,
) -> Result<BatchExecutionAuthorizationDto, String> {
    let cost = &plan.cost_estimate;
    let max_task_launches = if authorization.member_ordinal.is_some() {
        cost.tasks_per_member_run
    } else {
        cost.task_launches
    };
    let max_guided_interactions = match cost.execution_surface {
        BatchExecutionSurface::GuidedClient => max_task_launches,
        BatchExecutionSurface::AutomatedCli => 0,
    };

    if max_task_launches == 0
        || max_task_launches > cost.task_launches
        || authorization.max_provider_turns == 0
        || authorization.max_provider_turns > cost.max_provider_turns
        || authorization.max_task_budget_secs == 0
        || authorization.max_task_budget_secs > cost.summed_task_budget_secs
        || max_guided_interactions > cost.guided_interactions
    {
        return Err("execution authorization exceeds the immutable batch plan".into());
    }

    Ok(BatchExecutionAuthorizationDto {
        batch_id: authorization.batch_id,
        member_ordinal: authorization.member_ordinal,
        attempt_number: authorization.attempt_number,
        max_task_launches,
        max_provider_turns: authorization.max_provider_turns,
        max_task_budget_secs: authorization.max_task_budget_secs,
        max_guided_interactions,
        acknowledgement_hash: authorization.acknowledgement_hash,
        allowed_failure_kind: authorization.allowed_failure_kind,
        expires_at: authorization.expires_at,
        created_at: authorization.created_at,
    })
}

fn is_retryable_failure(failure: ability_core::FailureKind) -> bool {
    matches!(
        failure,
        ability_core::FailureKind::CliMissing
            | ability_core::FailureKind::RuntimeMissing
            | ability_core::FailureKind::AuthExpired
            | ability_core::FailureKind::QuotaExhausted
            | ability_core::FailureKind::Network
            | ability_core::FailureKind::AppInterrupted
            | ability_core::FailureKind::InfrastructureTimeout
            | ability_core::FailureKind::VerifierError
    )
}

fn retry_authorization_at(
    context: &BatchCommandContext<'_>,
    batch_id: Uuid,
    member_ordinal: u32,
    expected_failure_kind: ability_core::FailureKind,
    created_at: DateTime<Utc>,
) -> Result<ScanExecutionAuthorization, String> {
    if !is_retryable_failure(expected_failure_kind) {
        return Err("failure class is not eligible for explicit batch resume".into());
    }
    let batch = required_batch(context, batch_id)?;
    ensure_record_supported(&batch)?;
    let member = batch
        .members
        .iter()
        .find(|member| member.ordinal == member_ordinal)
        .ok_or_else(|| "batch member does not exist".to_string())?;
    if member.status != BatchMemberStatus::Deferred
        || member.failure_kind != Some(expected_failure_kind)
        || created_at < member.updated_at
    {
        return Err("retry estimate does not match the durable deferred member".into());
    }
    let attempt_number = member
        .attempt_number
        .checked_add(1)
        .ok_or_else(|| "batch member attempt number overflowed".to_string())?;
    let planned_runs = batch.plan.cost_estimate.planned_member_runs;
    if planned_runs == 0 {
        return Err("stored batch has no planned member runs".into());
    }
    let mut authorization = ScanExecutionAuthorization {
        batch_id,
        member_ordinal: Some(member_ordinal),
        attempt_number,
        max_provider_turns: batch.plan.cost_estimate.max_provider_turns / planned_runs,
        max_task_budget_secs: batch.plan.cost_estimate.summed_task_budget_secs / planned_runs,
        acknowledgement_hash: String::new(),
        allowed_failure_kind: Some(expected_failure_kind),
        expires_at: batch
            .plan
            .cost_estimate
            .execution_authorization_expires_at(created_at)
            .map_err(|error| error.to_string())?,
        created_at,
    };
    authorization.acknowledgement_hash = authorization
        .expected_retry_acknowledgement_hash(&batch.plan)
        .map_err(|error| error.to_string())?;
    Ok(authorization)
}

pub(crate) fn estimate_batch_retry_at(
    context: &BatchCommandContext<'_>,
    input: EstimateBatchRetryInput,
    now: DateTime<Utc>,
) -> Result<BatchRetryEstimateDto, String> {
    let authorization = retry_authorization_at(
        context,
        input.batch_id,
        input.member_ordinal,
        input.expected_failure_kind,
        now,
    )?;
    let batch = required_batch(context, input.batch_id)?;
    Ok(BatchRetryEstimateDto {
        authorization: execution_authorization_dto(&batch.plan, authorization)?,
    })
}

pub(crate) fn authorize_batch_retry_at(
    context: &BatchCommandContext<'_>,
    input: AuthorizeBatchRetryInput,
    now: DateTime<Utc>,
) -> Result<BatchExecutionAuthorizationDto, String> {
    let authorization = retry_authorization_at(
        context,
        input.batch_id,
        input.member_ordinal,
        input.allowed_failure_kind,
        input.estimate_created_at,
    )?;
    if input.acknowledgement_hash != authorization.acknowledgement_hash {
        return Err("retry acknowledgement is stale or does not match the durable failure".into());
    }
    if now < authorization.created_at || now > authorization.expires_at {
        return Err("retry execution acknowledgement has expired".into());
    }
    let batch = required_batch(context, input.batch_id)?;
    let response = execution_authorization_dto(&batch.plan, authorization.clone())?;
    context
        .repository
        .append_execution_authorization(&authorization)
        .map_err(|error| error.to_string())?;
    Ok(response)
}

fn get_supported_batch(
    context: &BatchCommandContext<'_>,
    batch_id: Uuid,
) -> Result<ScanBatchRecord, String> {
    let batch = required_batch(context, batch_id)?;
    ensure_record_supported(&batch)?;
    Ok(batch)
}

pub(crate) fn start_batch_at(
    context: &BatchCommandContext<'_>,
    input: BatchIdInput,
    now: DateTime<Utc>,
) -> Result<ScanBatchRecord, String> {
    get_supported_batch(context, input.batch_id)?;
    context
        .repository
        .resume_batch(input.batch_id, now)
        .map_err(|error| error.to_string())?;
    required_batch(context, input.batch_id)
}

pub(crate) fn resume_batch_at(
    context: &BatchCommandContext<'_>,
    input: BatchIdInput,
    now: DateTime<Utc>,
) -> Result<ScanBatchRecord, String> {
    start_batch_at(context, input, now)
}

pub(crate) fn pause_batch_at(
    context: &BatchCommandContext<'_>,
    input: BatchIdInput,
    now: DateTime<Utc>,
) -> Result<ScanBatchRecord, String> {
    get_supported_batch(context, input.batch_id)?;
    context
        .repository
        .pause_batch(input.batch_id, now)
        .map_err(|error| error.to_string())?;
    required_batch(context, input.batch_id)
}

pub(crate) fn cancel_batch_at(
    context: &BatchCommandContext<'_>,
    input: BatchIdInput,
    now: DateTime<Utc>,
) -> Result<ScanBatchRecord, String> {
    required_batch(context, input.batch_id)?;
    context
        .repository
        .cancel_batch(input.batch_id, now)
        .map_err(|error| error.to_string())?;
    required_batch(context, input.batch_id)
}

fn scheduled_lifecycle(status: BatchMemberStatus) -> ScheduledMemberLifecycle {
    match status {
        BatchMemberStatus::Planned => ScheduledMemberLifecycle::Runnable,
        BatchMemberStatus::Deferred => ScheduledMemberLifecycle::Deferred,
        BatchMemberStatus::Reserved => ScheduledMemberLifecycle::Reserved,
        BatchMemberStatus::Launching => ScheduledMemberLifecycle::Launching,
        BatchMemberStatus::Running => ScheduledMemberLifecycle::Running,
        BatchMemberStatus::Completed
        | BatchMemberStatus::Invalid
        | BatchMemberStatus::Unavailable
        | BatchMemberStatus::Cancelled => ScheduledMemberLifecycle::Terminal,
    }
}

fn guided_member_dto(
    batch: &ScanBatchRecord,
    decision: GuidedMemberDecision,
    ordinal: u32,
) -> Result<NextGuidedMemberDto, String> {
    let member = batch
        .members
        .iter()
        .find(|member| member.ordinal == ordinal)
        .cloned()
        .ok_or_else(|| "scheduled batch member is missing".to_string())?;
    let target = batch
        .plan
        .targets
        .get(
            usize::try_from(member.target_position)
                .map_err(|_| "batch target position is out of range".to_string())?,
        )
        .cloned()
        .ok_or_else(|| "batch target position is out of range".to_string())?;
    Ok(NextGuidedMemberDto {
        decision,
        member: Some(member),
        target: Some(target),
    })
}

pub(crate) fn get_next_guided_member_record(
    context: &BatchCommandContext<'_>,
    input: BatchIdInput,
) -> Result<NextGuidedMemberDto, String> {
    let batch = get_supported_batch(context, input.batch_id)?;
    if batch_surface(&batch)? != BatchExecutionSurface::GuidedClient {
        return Err("next guided member is available only for guided client batches".into());
    }
    let schedule = build_batch_schedule(&batch.plan).map_err(|error| error.to_string())?;
    let states = batch
        .members
        .iter()
        .map(|member| ScheduledMemberState {
            ordinal: member.ordinal,
            lifecycle: scheduled_lifecycle(member.status),
        })
        .collect::<Vec<_>>();
    match select_next_scheduled_member(&schedule, &states).map_err(|error| error.to_string())? {
        NextScheduledMember::Runnable(member) => {
            guided_member_dto(&batch, GuidedMemberDecision::Runnable, member.ordinal)
        }
        NextScheduledMember::BlockedByActive { ordinal } => {
            guided_member_dto(&batch, GuidedMemberDecision::BlockedByActive, ordinal)
        }
        NextScheduledMember::Exhausted => Ok(NextGuidedMemberDto {
            decision: GuidedMemberDecision::Exhausted,
            member: None,
            target: None,
        }),
    }
}

fn claim_mutation(state: &AppState) -> Result<crate::app_state::LocalDataMutationClaim, String> {
    state
        .local_data_gate
        .claim_mutating()
        .map_err(|_| "local data is busy; retry after the current backup or mutation".into())
}

#[tauri::command]
pub fn estimate_batch(
    state: State<'_, AppState>,
    input: BatchPlanInput,
) -> Result<BatchEstimateDto, String> {
    estimate_batch_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn create_acknowledged_batch(
    state: State<'_, AppState>,
    input: CreateAcknowledgedBatchInput,
) -> Result<ScanBatchRecord, String> {
    let _claim = claim_mutation(&state)?;
    create_acknowledged_batch_at(
        &BatchCommandContext::from_state(&state),
        input,
        Utc::now(),
        Uuid::new_v4(),
    )
}

#[tauri::command]
pub fn get_batch(
    state: State<'_, AppState>,
    input: BatchIdInput,
) -> Result<Option<ScanBatchRecord>, String> {
    get_batch_record(&BatchCommandContext::from_state(&state), input)
}

#[tauri::command]
pub fn list_batches(state: State<'_, AppState>) -> Result<Vec<ScanBatchRecord>, String> {
    list_batch_records(&BatchCommandContext::from_state(&state))
}

#[tauri::command]
pub fn authorize_batch_execution(
    state: State<'_, AppState>,
    input: AuthorizeBatchExecutionInput,
) -> Result<BatchExecutionAuthorizationDto, String> {
    let _claim = claim_mutation(&state)?;
    authorize_batch_execution_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn estimate_batch_retry(
    state: State<'_, AppState>,
    input: EstimateBatchRetryInput,
) -> Result<BatchRetryEstimateDto, String> {
    estimate_batch_retry_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn authorize_batch_retry(
    state: State<'_, AppState>,
    input: AuthorizeBatchRetryInput,
) -> Result<BatchExecutionAuthorizationDto, String> {
    let _claim = claim_mutation(&state)?;
    authorize_batch_retry_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn start_batch(
    state: State<'_, AppState>,
    input: BatchIdInput,
) -> Result<ScanBatchRecord, String> {
    let _claim = claim_mutation(&state)?;
    start_batch_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn resume_batch(
    state: State<'_, AppState>,
    input: BatchIdInput,
) -> Result<ScanBatchRecord, String> {
    let _claim = claim_mutation(&state)?;
    resume_batch_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn pause_batch(
    state: State<'_, AppState>,
    input: BatchIdInput,
) -> Result<ScanBatchRecord, String> {
    let _claim = claim_mutation(&state)?;
    pause_batch_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn cancel_batch(
    state: State<'_, AppState>,
    input: BatchIdInput,
) -> Result<ScanBatchRecord, String> {
    let _claim = claim_mutation(&state)?;
    cancel_batch_at(&BatchCommandContext::from_state(&state), input, Utc::now())
}

#[tauri::command]
pub fn get_next_guided_member(
    state: State<'_, AppState>,
    input: BatchIdInput,
) -> Result<NextGuidedMemberDto, String> {
    get_next_guided_member_record(&BatchCommandContext::from_state(&state), input)
}
