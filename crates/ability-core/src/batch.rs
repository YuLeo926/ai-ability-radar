use crate::{
    EnvironmentFingerprint, ModelSource, ModelVerification, TargetKind, TargetSelection,
    contains_forbidden_display_character, is_valid_reported_model,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

const POLICY_VERSION: u32 = 1;
const INITIAL_ACKNOWLEDGEMENT_MINUTES: i64 = 15;
const MEMBER_OVERHEAD_SECS: u64 = 300;
const CLIENT_QUICK_V1_SHA256: &str =
    "cfd2b36af1688432626ee80e453d60cd1d8cb4f87371df5f53def6b551e06f8f";
const CLI_QUICK_V1_SHA256: &str =
    "c52c76d1b562812909e88dd71a2f3c70ef874fd795f84c91017b94ad3bb01fea";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchMode {
    QuickComparison,
    Standard,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Created,
    Running,
    Paused,
    Completed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchExecutionSurface {
    GuidedClient,
    AutomatedCli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchFeatureLevel {
    GuidedQuickV1,
    CliStandardV1,
    ReliableFullV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterLaunchKind {
    GuidedClient,
    NativeExe,
    ReviewedNpm,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetRouteIdentity {
    pub kind: TargetKind,
    pub model_or_route: String,
    pub reasoning_effort: Option<String>,
    pub execution_surface: BatchExecutionSurface,
    pub is_default_route: bool,
}

impl TargetRouteIdentity {
    pub fn new(
        kind: TargetKind,
        model_or_route: &str,
        reasoning_effort: Option<&str>,
        execution_surface: BatchExecutionSurface,
        is_default_route: bool,
    ) -> Result<Self, BatchContractError> {
        ensure_kind_surface(kind, execution_surface)?;
        let model_or_route = normalize_model(model_or_route)?;
        let reasoning_effort = reasoning_effort.map(normalize_reasoning).transpose()?;
        if is_default_route {
            if execution_surface != BatchExecutionSurface::AutomatedCli
                || !matches!(model_or_route.as_str(), "default" | "default_route")
                || reasoning_effort.is_some()
            {
                return Err(BatchContractError::InvalidRouteIdentity);
            }
        } else if matches!(model_or_route.as_str(), "default" | "default_route") {
            return Err(BatchContractError::InvalidRouteIdentity);
        }
        Ok(Self {
            kind,
            model_or_route: if is_default_route {
                "default_route".into()
            } else {
                model_or_route
            },
            reasoning_effort,
            execution_surface,
            is_default_route,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionAdapterIdentity {
    pub execution_surface: BatchExecutionSurface,
    pub provider_family: String,
    pub launch_kind: AdapterLaunchKind,
    pub public_version: Option<String>,
    pub adapter_contract_version: String,
}

impl ExecutionAdapterIdentity {
    pub fn new(
        execution_surface: BatchExecutionSurface,
        provider_family: &str,
        launch_kind: AdapterLaunchKind,
        public_version: Option<&str>,
        adapter_contract_version: &str,
    ) -> Result<Self, BatchContractError> {
        let provider_family = normalize_identifier(provider_family, 32)?;
        let adapter_contract_version = normalize_identifier(adapter_contract_version, 64)?;
        let public_version = public_version.map(normalize_public_version).transpose()?;
        let launch_matches = matches!(
            (execution_surface, launch_kind),
            (
                BatchExecutionSurface::GuidedClient,
                AdapterLaunchKind::GuidedClient
            ) | (
                BatchExecutionSurface::AutomatedCli,
                AdapterLaunchKind::NativeExe | AdapterLaunchKind::ReviewedNpm
            )
        );
        if !launch_matches {
            return Err(BatchContractError::InvalidAdapterIdentity);
        }
        Ok(Self {
            execution_surface,
            provider_family,
            launch_kind,
            public_version,
            adapter_contract_version,
        })
    }

    pub fn compatible_with(&self, other: &Self) -> bool {
        self == other
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanBatchTarget {
    pub target: TargetSelection,
    pub route_identity: TargetRouteIdentity,
    pub execution_adapter_identity: ExecutionAdapterIdentity,
}

impl ScanBatchTarget {
    pub fn new(
        target: TargetSelection,
        execution_surface: BatchExecutionSurface,
        execution_adapter_identity: ExecutionAdapterIdentity,
    ) -> Result<Self, BatchContractError> {
        ensure_kind_surface(target.kind, execution_surface)?;
        if execution_adapter_identity.execution_surface != execution_surface {
            return Err(BatchContractError::MixedExecutionSurface);
        }
        validate_provenance(&target, execution_surface)?;
        let expected_provider = match target.kind {
            TargetKind::ChatGptClient | TargetKind::CodexCli => "openai",
            TargetKind::ClaudeClient | TargetKind::ClaudeCode => "anthropic",
        };
        if execution_adapter_identity.provider_family != expected_provider {
            return Err(BatchContractError::InvalidAdapterIdentity);
        }
        let route_identity = TargetRouteIdentity::new(
            target.kind,
            &target.reported_model,
            target.reasoning_effort.as_deref(),
            execution_surface,
            target.model_source == ModelSource::DefaultRoute,
        )?;
        Ok(Self {
            target,
            route_identity,
            execution_adapter_identity,
        })
    }

    pub fn validate_for_new_batch(&self) -> Result<(), BatchContractError> {
        validate_provenance(&self.target, self.route_identity.execution_surface)?;
        if self.execution_adapter_identity.execution_surface
            != self.route_identity.execution_surface
        {
            return Err(BatchContractError::MixedExecutionSurface);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SealedTaskBudget {
    pub max_turns: u64,
    pub time_budget_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchCostEstimate {
    pub policy_version: u32,
    pub execution_surface: BatchExecutionSurface,
    pub mode: BatchMode,
    pub target_count: u64,
    pub repetitions_per_target: u64,
    pub tasks_per_member_run: u64,
    pub planned_member_runs: u64,
    pub task_launches: u64,
    pub guided_interactions: u64,
    pub max_provider_turns: u64,
    pub summed_task_budget_secs: u64,
    pub expected_elapsed_secs_min: u64,
    pub expected_elapsed_secs_max: u64,
    pub provider_execution_ceiling_secs: u64,
    pub authorization_wall_clock_secs: u64,
    pub initial_acknowledgement_expires_at: DateTime<Utc>,
    pub token_quota_amount: Option<u64>,
    pub automatic_retry_budget: u64,
}

impl BatchCostEstimate {
    pub fn execution_authorization_expires_at(&self, started_at: DateTime<Utc>) -> DateTime<Utc> {
        started_at + Duration::seconds(self.authorization_wall_clock_secs as i64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchCostPolicy {
    pub version: u32,
}

impl BatchCostPolicy {
    pub const fn v1() -> Self {
        Self {
            version: POLICY_VERSION,
        }
    }

    pub fn estimate(
        &self,
        execution_surface: BatchExecutionSurface,
        mode: BatchMode,
        target_count: u64,
        task_budgets: &[SealedTaskBudget],
        issued_at: DateTime<Utc>,
    ) -> Result<BatchCostEstimate, BatchContractError> {
        if self.version != POLICY_VERSION || task_budgets.is_empty() {
            return Err(BatchContractError::UnsupportedCostPolicy);
        }
        let limits = PolicyLimits::for_row(execution_surface, mode)?;
        if !(2..=limits.max_targets).contains(&target_count) {
            return Err(BatchContractError::CohortLimitExceeded);
        }
        if task_budgets
            .iter()
            .any(|task| task.max_turns == 0 || task.time_budget_secs == 0)
        {
            return Err(BatchContractError::InvalidTaskBudget);
        }

        let tasks_per_member_run = u64::try_from(task_budgets.len())
            .map_err(|_| BatchContractError::ArithmeticOverflow)?;
        let per_member_turns = checked_sum(task_budgets.iter().map(|task| task.max_turns))?;
        let per_member_time = checked_sum(task_budgets.iter().map(|task| task.time_budget_secs))?;
        let planned_member_runs = target_count
            .checked_mul(limits.repetitions)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let task_launches = planned_member_runs
            .checked_mul(tasks_per_member_run)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let max_provider_turns = planned_member_runs
            .checked_mul(per_member_turns)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let summed_task_budget_secs = planned_member_runs
            .checked_mul(per_member_time)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let guided_interactions = if execution_surface == BatchExecutionSurface::GuidedClient {
            task_launches
        } else {
            0
        };
        let within_limits = planned_member_runs <= limits.member_cap
            && task_launches <= limits.launch_or_interaction_cap
            && guided_interactions <= limits.launch_or_interaction_cap
            && max_provider_turns <= limits.turn_cap
            && summed_task_budget_secs <= limits.task_budget_cap_secs;
        if !within_limits {
            return Err(BatchContractError::CohortLimitExceeded);
        }

        let (expected_member_min, expected_member_max) = match execution_surface {
            BatchExecutionSurface::GuidedClient => (10 * 60_u64, 15 * 60_u64),
            BatchExecutionSurface::AutomatedCli => (30 * 60_u64, 60 * 60_u64),
        };
        let expected_elapsed_secs_min = planned_member_runs
            .checked_mul(expected_member_min)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let expected_elapsed_secs_max = planned_member_runs
            .checked_mul(expected_member_max)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let overhead = planned_member_runs
            .checked_mul(MEMBER_OVERHEAD_SECS)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let provider_execution_ceiling_secs = summed_task_budget_secs
            .checked_add(overhead)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let initial_acknowledgement_expires_at = issued_at
            .checked_add_signed(Duration::minutes(INITIAL_ACKNOWLEDGEMENT_MINUTES))
            .ok_or(BatchContractError::ArithmeticOverflow)?;

        Ok(BatchCostEstimate {
            policy_version: self.version,
            execution_surface,
            mode,
            target_count,
            repetitions_per_target: limits.repetitions,
            tasks_per_member_run,
            planned_member_runs,
            task_launches,
            guided_interactions,
            max_provider_turns,
            summed_task_budget_secs,
            expected_elapsed_secs_min,
            expected_elapsed_secs_max,
            provider_execution_ceiling_secs,
            authorization_wall_clock_secs: limits.window_hours * 60 * 60,
            initial_acknowledgement_expires_at,
            token_quota_amount: None,
            automatic_retry_budget: 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanBatchPlan {
    pub suite_id: String,
    pub suite_version: String,
    pub suite_content_sha256: String,
    pub scoring_rule_version: String,
    pub mode: BatchMode,
    pub seed: u64,
    pub status: BatchStatus,
    pub targets: Vec<ScanBatchTarget>,
    pub sealed_task_budgets: Vec<SealedTaskBudget>,
    pub cost_estimate: BatchCostEstimate,
    pub acknowledgement_hash: String,
}

impl ScanBatchPlan {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        suite_id: &str,
        suite_version: &str,
        suite_content_sha256: &str,
        scoring_rule_version: &str,
        mode: BatchMode,
        seed: u64,
        targets: Vec<ScanBatchTarget>,
        sealed_task_budgets: &[SealedTaskBudget],
        issued_at: DateTime<Utc>,
    ) -> Result<Self, BatchContractError> {
        let suite_id = validated_plan_text(suite_id, 128)?;
        let suite_version = validated_plan_text(suite_version, 64)?;
        let scoring_rule_version = validated_plan_text(scoring_rule_version, 64)?;
        if suite_content_sha256.len() != 64
            || !suite_content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(BatchContractError::InvalidPlanIdentity);
        }
        let surface = targets
            .first()
            .ok_or(BatchContractError::CohortLimitExceeded)?
            .route_identity
            .execution_surface;
        let mut route_identities = BTreeSet::new();
        for target in &targets {
            target.validate_for_new_batch()?;
            if target.route_identity.execution_surface != surface {
                return Err(BatchContractError::MixedExecutionSurface);
            }
            if !route_identities.insert(target.route_identity.clone()) {
                return Err(BatchContractError::DuplicateRouteIdentity);
            }
        }
        let expected_pack = match surface {
            BatchExecutionSurface::GuidedClient => {
                ("client-quick", "1.0.0", CLIENT_QUICK_V1_SHA256)
            }
            BatchExecutionSurface::AutomatedCli => ("cli-quick", "1.0.0", CLI_QUICK_V1_SHA256),
        };
        if (
            suite_id.as_str(),
            suite_version.as_str(),
            suite_content_sha256,
        ) != expected_pack
        {
            return Err(BatchContractError::UnsupportedCostPolicy);
        }
        let target_count =
            u64::try_from(targets.len()).map_err(|_| BatchContractError::ArithmeticOverflow)?;
        let cost_estimate = BatchCostPolicy::v1().estimate(
            surface,
            mode,
            target_count,
            sealed_task_budgets,
            issued_at,
        )?;
        let hash_payload = PlanHashPayload {
            suite_id: &suite_id,
            suite_version: &suite_version,
            suite_content_sha256,
            scoring_rule_version: &scoring_rule_version,
            mode,
            seed,
            targets: &targets,
            sealed_task_budgets,
            cost_estimate: &cost_estimate,
        };
        let bytes = serde_json::to_vec(&hash_payload)
            .map_err(|_| BatchContractError::InvalidPlanIdentity)?;
        let acknowledgement_hash = format!("{:x}", Sha256::digest(bytes));
        Ok(Self {
            suite_id,
            suite_version,
            suite_content_sha256: suite_content_sha256.into(),
            scoring_rule_version,
            mode,
            seed,
            status: BatchStatus::Created,
            targets,
            sealed_task_budgets: sealed_task_budgets.to_vec(),
            cost_estimate,
            acknowledgement_hash,
        })
    }
}

impl EnvironmentFingerprint {
    pub fn require_batch_adapter(
        &self,
        expected: &ExecutionAdapterIdentity,
    ) -> Result<(), BatchContractError> {
        match &self.execution_adapter_identity {
            Some(stored) if stored.compatible_with(expected) => Ok(()),
            Some(_) => Err(BatchContractError::IncompatibleAdapterIdentity),
            None => Err(BatchContractError::MissingAdapterIdentity),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanHashPayload<'a> {
    suite_id: &'a str,
    suite_version: &'a str,
    suite_content_sha256: &'a str,
    scoring_rule_version: &'a str,
    mode: BatchMode,
    seed: u64,
    targets: &'a [ScanBatchTarget],
    sealed_task_budgets: &'a [SealedTaskBudget],
    cost_estimate: &'a BatchCostEstimate,
}

struct PolicyLimits {
    max_targets: u64,
    repetitions: u64,
    member_cap: u64,
    launch_or_interaction_cap: u64,
    turn_cap: u64,
    task_budget_cap_secs: u64,
    window_hours: u64,
}

impl PolicyLimits {
    fn for_row(
        surface: BatchExecutionSurface,
        mode: BatchMode,
    ) -> Result<Self, BatchContractError> {
        let limits = match (surface, mode) {
            (BatchExecutionSurface::GuidedClient, BatchMode::QuickComparison) => Self {
                max_targets: 4,
                repetitions: 1,
                member_cap: 4,
                launch_or_interaction_cap: 32,
                turn_cap: 32,
                task_budget_cap_secs: 4_320,
                window_hours: 4,
            },
            (BatchExecutionSurface::AutomatedCli, BatchMode::QuickComparison) => Self {
                max_targets: 4,
                repetitions: 1,
                member_cap: 4,
                launch_or_interaction_cap: 8,
                turn_cap: 160,
                task_budget_cap_secs: 14_400,
                window_hours: 8,
            },
            (BatchExecutionSurface::AutomatedCli, BatchMode::Standard) => Self {
                max_targets: 4,
                repetitions: 3,
                member_cap: 12,
                launch_or_interaction_cap: 24,
                turn_cap: 480,
                task_budget_cap_secs: 43_200,
                window_hours: 24,
            },
            (BatchExecutionSurface::AutomatedCli, BatchMode::Full) => Self {
                max_targets: 5,
                repetitions: 5,
                member_cap: 25,
                launch_or_interaction_cap: 50,
                turn_cap: 1_000,
                task_budget_cap_secs: 90_000,
                window_hours: 72,
            },
            (BatchExecutionSurface::GuidedClient, BatchMode::Standard | BatchMode::Full) => {
                return Err(BatchContractError::UnsupportedSurfaceMode);
            }
        };
        Ok(limits)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BatchContractError {
    #[error("batch execution surface does not match the target or cohort")]
    MixedExecutionSurface,
    #[error("batch target route identity is invalid")]
    InvalidRouteIdentity,
    #[error("batch execution adapter identity is invalid")]
    InvalidAdapterIdentity,
    #[error("batch target provenance is incoherent")]
    InvalidProvenance,
    #[error("batch mode is unsupported for this execution surface")]
    UnsupportedSurfaceMode,
    #[error("batch cost policy version is unsupported")]
    UnsupportedCostPolicy,
    #[error("batch cohort exceeds cost-policy limits")]
    CohortLimitExceeded,
    #[error("sealed task budget is invalid")]
    InvalidTaskBudget,
    #[error("checked batch cost arithmetic overflowed")]
    ArithmeticOverflow,
    #[error("batch contains a duplicate route identity")]
    DuplicateRouteIdentity,
    #[error("batch plan identity is invalid")]
    InvalidPlanIdentity,
    #[error("batch environment is missing execution adapter identity")]
    MissingAdapterIdentity,
    #[error("batch execution adapter identity is incompatible")]
    IncompatibleAdapterIdentity,
}

fn ensure_kind_surface(
    kind: TargetKind,
    surface: BatchExecutionSurface,
) -> Result<(), BatchContractError> {
    let matches = match surface {
        BatchExecutionSurface::GuidedClient => {
            matches!(kind, TargetKind::ChatGptClient | TargetKind::ClaudeClient)
        }
        BatchExecutionSurface::AutomatedCli => {
            matches!(kind, TargetKind::CodexCli | TargetKind::ClaudeCode)
        }
    };
    matches
        .then_some(())
        .ok_or(BatchContractError::MixedExecutionSurface)
}

fn validate_provenance(
    target: &TargetSelection,
    surface: BatchExecutionSurface,
) -> Result<(), BatchContractError> {
    let accepted = match surface {
        BatchExecutionSurface::GuidedClient => matches!(
            (target.model_source, target.model_verification),
            (ModelSource::Manual, ModelVerification::UserConfirmed)
                | (
                    ModelSource::WindowsAccessibility,
                    ModelVerification::UserConfirmed
                )
        ),
        BatchExecutionSurface::AutomatedCli if target.reported_model.trim() == "default" => {
            matches!(
                (target.model_source, target.model_verification),
                (ModelSource::DefaultRoute, ModelVerification::Unverified)
            )
        }
        BatchExecutionSurface::AutomatedCli => matches!(
            (target.model_source, target.model_verification),
            (ModelSource::CliRequested, ModelVerification::UserConfirmed)
        ),
    };
    accepted
        .then_some(())
        .ok_or(BatchContractError::InvalidProvenance)
}

fn normalize_model(value: &str) -> Result<String, BatchContractError> {
    let trimmed = value.trim();
    if !is_valid_reported_model(trimmed) {
        return Err(BatchContractError::InvalidRouteIdentity);
    }
    Ok(trimmed.to_lowercase())
}

fn normalize_reasoning(value: &str) -> Result<String, BatchContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > 40
        || contains_forbidden_display_character(trimmed)
    {
        return Err(BatchContractError::InvalidRouteIdentity);
    }
    Ok(trimmed.to_lowercase())
}

fn normalize_identifier(value: &str, max_len: usize) -> Result<String, BatchContractError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.len() > max_len
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(BatchContractError::InvalidAdapterIdentity);
    }
    Ok(normalized)
}

fn normalize_public_version(value: &str) -> Result<String, BatchContractError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.len() > 120
        || contains_forbidden_display_character(&normalized)
        || normalized.contains(['/', '\\', ':', '@'])
        || !normalized.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '.' | '-' | '_' | '+' | '(' | ')')
        })
    {
        return Err(BatchContractError::InvalidAdapterIdentity);
    }
    Ok(normalized)
}

fn checked_sum(mut values: impl Iterator<Item = u64>) -> Result<u64, BatchContractError> {
    values.try_fold(0_u64, |sum, value| {
        sum.checked_add(value)
            .ok_or(BatchContractError::ArithmeticOverflow)
    })
}

fn validated_plan_text(value: &str, max_chars: usize) -> Result<String, BatchContractError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || contains_forbidden_display_character(trimmed)
    {
        return Err(BatchContractError::InvalidPlanIdentity);
    }
    Ok(trimmed.to_owned())
}
