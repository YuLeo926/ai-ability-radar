use crate::{
    EnvironmentFingerprint, LoadedPack, ModelSource, ModelVerification, TargetKind,
    TargetSelection, contains_forbidden_display_character, is_valid_reported_model,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

const POLICY_VERSION: u32 = 1;
const SESSION_POLICY_VERSION: u32 = 1;
pub(crate) const BATCH_SCHEDULE_POLICY_VERSION: u32 = 1;
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
pub enum SessionIsolationPolicy {
    UserAttestedFreshConversationPerTask,
    MachineEnforcedFreshSessionAndWorkspacePerTask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchTaskSessionBinding {
    pub policy_version: u32,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_content_sha256: String,
    pub task_count: u32,
    pub isolation_policy: SessionIsolationPolicy,
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
        let model_or_route = normalize_model(model_or_route, execution_surface)?;
        let reasoning_effort = reasoning_effort
            .map(|value| normalize_reasoning(value, execution_surface))
            .transpose()?;
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
        reject_forbidden_original(provider_family, BatchContractError::InvalidAdapterIdentity)?;
        reject_forbidden_original(
            adapter_contract_version,
            BatchContractError::InvalidAdapterIdentity,
        )?;
        if let Some(public_version) = public_version {
            reject_forbidden_original(public_version, BatchContractError::InvalidAdapterIdentity)?;
        }
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
        mut target: TargetSelection,
        execution_surface: BatchExecutionSurface,
        execution_adapter_identity: ExecutionAdapterIdentity,
    ) -> Result<Self, BatchContractError> {
        ensure_kind_surface(target.kind, execution_surface)?;
        let canonical_adapter = ExecutionAdapterIdentity::new(
            execution_adapter_identity.execution_surface,
            &execution_adapter_identity.provider_family,
            execution_adapter_identity.launch_kind,
            execution_adapter_identity.public_version.as_deref(),
            &execution_adapter_identity.adapter_contract_version,
        )?;
        if canonical_adapter != execution_adapter_identity
            || execution_adapter_identity.execution_surface != execution_surface
        {
            return Err(BatchContractError::MixedExecutionSurface);
        }
        target.reported_model = canonical_target_model(&target.reported_model, execution_surface)?;
        target.reasoning_effort = target
            .reasoning_effort
            .as_deref()
            .map(|value| normalize_reasoning(value, execution_surface))
            .transpose()?;
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
        let canonical_adapter = ExecutionAdapterIdentity::new(
            self.execution_adapter_identity.execution_surface,
            &self.execution_adapter_identity.provider_family,
            self.execution_adapter_identity.launch_kind,
            self.execution_adapter_identity.public_version.as_deref(),
            &self.execution_adapter_identity.adapter_contract_version,
        )?;
        if canonical_adapter != self.execution_adapter_identity {
            return Err(BatchContractError::InvalidAdapterIdentity);
        }
        let reconstructed = Self::new(
            self.target.clone(),
            self.route_identity.execution_surface,
            canonical_adapter,
        )?;
        (reconstructed == *self)
            .then_some(())
            .ok_or(BatchContractError::InvalidRouteIdentity)
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
    pub issued_at: DateTime<Utc>,
    pub initial_acknowledgement_expires_at: DateTime<Utc>,
    pub token_quota_amount: Option<u64>,
    pub automatic_retry_budget: u64,
}

impl BatchCostEstimate {
    pub fn execution_authorization_expires_at(
        &self,
        started_at: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, BatchContractError> {
        self.validate_intrinsic_v1()?;
        let seconds = i64::try_from(self.authorization_wall_clock_secs)
            .map_err(|_| BatchContractError::ArithmeticOverflow)?;
        started_at
            .checked_add_signed(Duration::seconds(seconds))
            .ok_or(BatchContractError::ArithmeticOverflow)
    }

    pub fn validate_against_pack(&self, pack: &LoadedPack) -> Result<(), BatchContractError> {
        self.validate_intrinsic_v1()?;
        let expected = BatchCostPolicy::v1().estimate(
            pack,
            self.execution_surface,
            self.mode,
            self.target_count,
            self.issued_at,
        )?;
        (expected == *self)
            .then_some(())
            .ok_or(BatchContractError::InvalidCostEstimate)
    }

    fn validate_intrinsic_v1(&self) -> Result<(), BatchContractError> {
        if self.policy_version != POLICY_VERSION
            || self.tasks_per_member_run == 0
            || self.token_quota_amount.is_some()
            || self.automatic_retry_budget != 0
        {
            return Err(BatchContractError::InvalidCostEstimate);
        }
        let limits = PolicyLimits::for_row(self.execution_surface, self.mode)?;
        let members = self
            .target_count
            .checked_mul(limits.repetitions)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let launches = members
            .checked_mul(self.tasks_per_member_run)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let guided = if self.execution_surface == BatchExecutionSurface::GuidedClient {
            launches
        } else {
            0
        };
        let expected_band = match self.execution_surface {
            BatchExecutionSurface::GuidedClient => (10 * 60_u64, 15 * 60_u64),
            BatchExecutionSurface::AutomatedCli => (30 * 60_u64, 60 * 60_u64),
        };
        let expected_min = members
            .checked_mul(expected_band.0)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let expected_max = members
            .checked_mul(expected_band.1)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let ceiling = self
            .summed_task_budget_secs
            .checked_add(
                members
                    .checked_mul(MEMBER_OVERHEAD_SECS)
                    .ok_or(BatchContractError::ArithmeticOverflow)?,
            )
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let acknowledgement_expiry = self
            .issued_at
            .checked_add_signed(Duration::minutes(INITIAL_ACKNOWLEDGEMENT_MINUTES))
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let within_limits = (2..=limits.max_targets).contains(&self.target_count)
            && members <= limits.member_cap
            && launches <= limits.launch_or_interaction_cap
            && guided <= limits.launch_or_interaction_cap
            && self.max_provider_turns <= limits.turn_cap
            && self.summed_task_budget_secs <= limits.task_budget_cap_secs;
        if !within_limits
            || self.repetitions_per_target != limits.repetitions
            || self.planned_member_runs != members
            || self.task_launches != launches
            || self.guided_interactions != guided
            || self.expected_elapsed_secs_min != expected_min
            || self.expected_elapsed_secs_max != expected_max
            || self.provider_execution_ceiling_secs != ceiling
            || self.authorization_wall_clock_secs != limits.window_hours * 60 * 60
            || self.initial_acknowledgement_expires_at != acknowledgement_expiry
        {
            return Err(BatchContractError::InvalidCostEstimate);
        }
        Ok(())
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
        pack: &LoadedPack,
        execution_surface: BatchExecutionSurface,
        mode: BatchMode,
        target_count: u64,
        issued_at: DateTime<Utc>,
    ) -> Result<BatchCostEstimate, BatchContractError> {
        if self.version != POLICY_VERSION {
            return Err(BatchContractError::UnsupportedCostPolicy);
        }
        let task_budgets = verified_pack_budgets(pack, execution_surface)?;
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
            issued_at,
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
    pub schedule_policy_version: u32,
    pub task_session_policy_version: u32,
    pub session_isolation_policy: SessionIsolationPolicy,
    pub targets: Vec<ScanBatchTarget>,
    pub sealed_task_budgets: Vec<SealedTaskBudget>,
    pub cost_estimate: BatchCostEstimate,
    pub acknowledgement_hash: String,
}

impl ScanBatchPlan {
    pub fn new(
        pack: &LoadedPack,
        scoring_rule_version: &str,
        mode: BatchMode,
        seed: u64,
        targets: Vec<ScanBatchTarget>,
        issued_at: DateTime<Utc>,
    ) -> Result<Self, BatchContractError> {
        let suite_id = validated_plan_text(&pack.manifest.id, 128)?;
        let suite_version = validated_plan_text(&pack.manifest.version, 64)?;
        let suite_content_sha256 = pack.content_sha256.as_str();
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
        for target in &targets {
            if !pack.manifest.target_kinds.contains(&target.target.kind) {
                return Err(BatchContractError::UnsupportedCostPolicy);
            }
        }
        let sealed_task_budgets = verified_pack_budgets(pack, surface)?;
        let target_count =
            u64::try_from(targets.len()).map_err(|_| BatchContractError::ArithmeticOverflow)?;
        let cost_estimate =
            BatchCostPolicy::v1().estimate(pack, surface, mode, target_count, issued_at)?;
        let session_isolation_policy = match surface {
            BatchExecutionSurface::GuidedClient => {
                SessionIsolationPolicy::UserAttestedFreshConversationPerTask
            }
            BatchExecutionSurface::AutomatedCli => {
                SessionIsolationPolicy::MachineEnforcedFreshSessionAndWorkspacePerTask
            }
        };
        let mut plan = Self {
            suite_id,
            suite_version,
            suite_content_sha256: suite_content_sha256.into(),
            scoring_rule_version,
            mode,
            seed,
            status: BatchStatus::Created,
            schedule_policy_version: BATCH_SCHEDULE_POLICY_VERSION,
            task_session_policy_version: SESSION_POLICY_VERSION,
            session_isolation_policy,
            targets,
            sealed_task_budgets,
            cost_estimate,
            acknowledgement_hash: String::new(),
        };
        plan.acknowledgement_hash = plan.calculated_acknowledgement_hash()?;
        Ok(plan)
    }

    pub fn validate_acknowledgement_hash(&self) -> Result<(), BatchContractError> {
        if self.acknowledgement_hash == self.calculated_acknowledgement_hash()? {
            Ok(())
        } else {
            Err(BatchContractError::InvalidPlanIdentity)
        }
    }

    pub(crate) fn validated_schedule_contract(
        &self,
    ) -> Result<(u32, BatchTaskSessionBinding), BatchContractError> {
        self.validate_acknowledgement_hash()?;
        self.cost_estimate.validate_intrinsic_v1()?;
        if self.status != BatchStatus::Created
            || self.schedule_policy_version != BATCH_SCHEDULE_POLICY_VERSION
        {
            return Err(BatchContractError::InvalidPlanIdentity);
        }
        let surface = self
            .targets
            .first()
            .ok_or(BatchContractError::CohortLimitExceeded)?
            .route_identity
            .execution_surface;
        let mut route_identities = BTreeSet::new();
        for target in &self.targets {
            target.validate_for_new_batch()?;
            if target.route_identity.execution_surface != surface {
                return Err(BatchContractError::MixedExecutionSurface);
            }
            if !route_identities.insert(target.route_identity.clone()) {
                return Err(BatchContractError::DuplicateRouteIdentity);
            }
        }
        let expected_isolation_policy = match surface {
            BatchExecutionSurface::GuidedClient => {
                SessionIsolationPolicy::UserAttestedFreshConversationPerTask
            }
            BatchExecutionSurface::AutomatedCli => {
                SessionIsolationPolicy::MachineEnforcedFreshSessionAndWorkspacePerTask
            }
        };
        if self.task_session_policy_version != SESSION_POLICY_VERSION
            || self.session_isolation_policy != expected_isolation_policy
        {
            return Err(BatchContractError::InvalidPlanIdentity);
        }
        let target_count = u64::try_from(self.targets.len())
            .map_err(|_| BatchContractError::ArithmeticOverflow)?;
        let task_count = u64::try_from(self.sealed_task_budgets.len())
            .map_err(|_| BatchContractError::ArithmeticOverflow)?;
        if self
            .sealed_task_budgets
            .iter()
            .any(|task| task.max_turns == 0 || task.time_budget_secs == 0)
        {
            return Err(BatchContractError::InvalidTaskBudget);
        }
        let per_member_turns =
            checked_sum(self.sealed_task_budgets.iter().map(|task| task.max_turns))?;
        let per_member_secs = checked_sum(
            self.sealed_task_budgets
                .iter()
                .map(|task| task.time_budget_secs),
        )?;
        let expected_turns = self
            .cost_estimate
            .planned_member_runs
            .checked_mul(per_member_turns)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        let expected_secs = self
            .cost_estimate
            .planned_member_runs
            .checked_mul(per_member_secs)
            .ok_or(BatchContractError::ArithmeticOverflow)?;
        if self.cost_estimate.execution_surface != surface
            || self.cost_estimate.target_count != target_count
            || self.cost_estimate.tasks_per_member_run != task_count
            || self.cost_estimate.max_provider_turns != expected_turns
            || self.cost_estimate.summed_task_budget_secs != expected_secs
        {
            return Err(BatchContractError::InvalidCostEstimate);
        }
        let repetitions = u32::try_from(self.cost_estimate.repetitions_per_target)
            .map_err(|_| BatchContractError::ArithmeticOverflow)?;
        let task_count =
            u32::try_from(task_count).map_err(|_| BatchContractError::ArithmeticOverflow)?;
        let binding = BatchTaskSessionBinding {
            policy_version: self.task_session_policy_version,
            suite_id: self.suite_id.clone(),
            suite_version: self.suite_version.clone(),
            suite_content_sha256: self.suite_content_sha256.clone(),
            task_count,
            isolation_policy: self.session_isolation_policy,
        };
        Ok((repetitions, binding))
    }

    fn calculated_acknowledgement_hash(&self) -> Result<String, BatchContractError> {
        let hash_payload = PlanHashPayload {
            suite_id: &self.suite_id,
            suite_version: &self.suite_version,
            suite_content_sha256: &self.suite_content_sha256,
            scoring_rule_version: &self.scoring_rule_version,
            mode: self.mode,
            seed: self.seed,
            schedule_policy_version: self.schedule_policy_version,
            task_session_policy_version: self.task_session_policy_version,
            session_isolation_policy: self.session_isolation_policy,
            targets: &self.targets,
            sealed_task_budgets: &self.sealed_task_budgets,
            cost_estimate: &self.cost_estimate,
        };
        let bytes = serde_json::to_vec(&hash_payload)
            .map_err(|_| BatchContractError::InvalidPlanIdentity)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
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
    schedule_policy_version: u32,
    task_session_policy_version: u32,
    session_isolation_policy: SessionIsolationPolicy,
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
    #[error("decoded batch cost estimate does not match cost policy v1")]
    InvalidCostEstimate,
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

fn canonical_target_model(
    value: &str,
    surface: BatchExecutionSurface,
) -> Result<String, BatchContractError> {
    reject_forbidden_original(value, BatchContractError::InvalidRouteIdentity)?;
    let trimmed = value.trim();
    if !is_valid_reported_model(trimmed) || looks_like_local_path(trimmed) {
        return Err(BatchContractError::InvalidRouteIdentity);
    }
    if surface == BatchExecutionSurface::AutomatedCli && !safe_cli_model(trimmed) {
        return Err(BatchContractError::InvalidRouteIdentity);
    }
    Ok(trimmed.to_owned())
}

fn normalize_model(
    value: &str,
    surface: BatchExecutionSurface,
) -> Result<String, BatchContractError> {
    Ok(canonical_target_model(value, surface)?.to_lowercase())
}

fn normalize_reasoning(
    value: &str,
    surface: BatchExecutionSurface,
) -> Result<String, BatchContractError> {
    reject_forbidden_original(value, BatchContractError::InvalidRouteIdentity)?;
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 40 || looks_like_local_path(trimmed) {
        return Err(BatchContractError::InvalidRouteIdentity);
    }
    let canonical = trimmed.to_ascii_lowercase();
    const KNOWN: &[&str] = &[
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ];
    if KNOWN.contains(&canonical.as_str()) {
        return Ok(canonical);
    }
    match surface {
        BatchExecutionSurface::GuidedClient => Ok(trimmed.to_owned()),
        BatchExecutionSurface::AutomatedCli
            if canonical.len() <= 32
                && canonical
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) =>
        {
            Ok(canonical)
        }
        BatchExecutionSurface::AutomatedCli => Err(BatchContractError::InvalidRouteIdentity),
    }
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

fn safe_cli_model(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '/' | '-')
        })
}

fn looks_like_local_path(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    let bytes = value.as_bytes();
    value.contains('\\')
        || lower.starts_with('/')
        || lower.starts_with("~/")
        || lower.starts_with("./")
        || lower.starts_with("../")
        || lower.contains("/home/")
        || lower.contains("/users/")
        || lower.contains("/appdata/")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

fn reject_forbidden_original(
    value: &str,
    error: BatchContractError,
) -> Result<(), BatchContractError> {
    if contains_forbidden_display_character(value) {
        Err(error)
    } else {
        Ok(())
    }
}

struct CostPolicyPackSpec {
    id: &'static str,
    content_sha256: &'static str,
    target_kinds: &'static [TargetKind],
    tasks: &'static [(&'static str, u64, u64)],
}

fn verified_pack_budgets(
    pack: &LoadedPack,
    surface: BatchExecutionSurface,
) -> Result<Vec<SealedTaskBudget>, BatchContractError> {
    let expected = match surface {
        BatchExecutionSurface::GuidedClient => CostPolicyPackSpec {
            id: "client-quick",
            content_sha256: CLIENT_QUICK_V1_SHA256,
            target_kinds: &[TargetKind::ChatGptClient, TargetKind::ClaudeClient],
            tasks: &[
                ("instruction-filter", 1, 120),
                ("instruction-csv", 1, 120),
                ("instruction-inventory", 1, 120),
                ("logic-schedule", 1, 120),
                ("logic-truth", 1, 120),
                ("logic-capacity", 1, 120),
                ("review-python", 1, 180),
                ("review-typescript", 1, 180),
            ],
        },
        BatchExecutionSurface::AutomatedCli => CostPolicyPackSpec {
            id: "cli-quick",
            content_sha256: CLI_QUICK_V1_SHA256,
            target_kinds: &[TargetKind::CodexCli, TargetKind::ClaudeCode],
            tasks: &[("dedupe-events", 20, 1_800), ("retry-schedule", 20, 1_800)],
        },
    };
    if pack.manifest.id != expected.id
        || pack.manifest.version != "1.0.0"
        || pack.content_sha256 != expected.content_sha256
        || pack.manifest.target_kinds != expected.target_kinds
        || pack.manifest.tasks.len() != expected.tasks.len()
        || pack.tasks.len() != expected.tasks.len()
    {
        return Err(BatchContractError::UnsupportedCostPolicy);
    }

    let mut budgets = Vec::with_capacity(expected.tasks.len());
    for ((manifest_task, loaded_task), (id, max_turns, time_budget_secs)) in pack
        .manifest
        .tasks
        .iter()
        .zip(&pack.tasks)
        .zip(expected.tasks)
    {
        if manifest_task.id != *id
            || loaded_task.definition.id != *id
            || u64::from(manifest_task.max_turns) != *max_turns
            || u64::from(loaded_task.definition.max_turns) != *max_turns
            || manifest_task.time_budget_secs != *time_budget_secs
            || loaded_task.definition.time_budget_secs != *time_budget_secs
        {
            return Err(BatchContractError::InvalidTaskBudget);
        }
        budgets.push(SealedTaskBudget {
            max_turns: *max_turns,
            time_budget_secs: *time_budget_secs,
        });
    }
    Ok(budgets)
}

fn checked_sum(mut values: impl Iterator<Item = u64>) -> Result<u64, BatchContractError> {
    values.try_fold(0_u64, |sum, value| {
        sum.checked_add(value)
            .ok_or(BatchContractError::ArithmeticOverflow)
    })
}

fn validated_plan_text(value: &str, max_chars: usize) -> Result<String, BatchContractError> {
    reject_forbidden_original(value, BatchContractError::InvalidPlanIdentity)?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > max_chars
        || contains_forbidden_display_character(trimmed)
    {
        return Err(BatchContractError::InvalidPlanIdentity);
    }
    Ok(trimmed.to_owned())
}
