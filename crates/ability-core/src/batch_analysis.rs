use crate::{
    AdapterLaunchKind, BatchExecutionSurface, BatchMemberStatus, BatchMode, BatchStatus, Category,
    ExecutionAdapterIdentity, FailureKind, ModelSource, ModelVerification, RunStatus,
    ScanBatchPlan, ScoreSummary, TargetRouteIdentity, TaskOutcome,
};
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

pub const BATCH_ANALYSIS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionSignal {
    InsufficientData,
    Stable,
    Watch,
    LikelyRegression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptedProvenanceClass {
    GuidedManualConfirmed,
    GuidedAccessibilityConfirmed,
    CliRequestedConfirmed,
    CliDefaultUnverified,
}

impl AcceptedProvenanceClass {
    pub fn from_plan_target(
        source: ModelSource,
        verification: ModelVerification,
        surface: BatchExecutionSurface,
    ) -> Option<Self> {
        match (surface, source, verification) {
            (
                BatchExecutionSurface::GuidedClient,
                ModelSource::Manual,
                ModelVerification::UserConfirmed,
            ) => Some(Self::GuidedManualConfirmed),
            (
                BatchExecutionSurface::GuidedClient,
                ModelSource::WindowsAccessibility,
                ModelVerification::UserConfirmed,
            ) => Some(Self::GuidedAccessibilityConfirmed),
            (
                BatchExecutionSurface::AutomatedCli,
                ModelSource::CliRequested,
                ModelVerification::UserConfirmed,
            ) => Some(Self::CliRequestedConfirmed),
            (
                BatchExecutionSurface::AutomatedCli,
                ModelSource::DefaultRoute,
                ModelVerification::Unverified,
            ) => Some(Self::CliDefaultUnverified),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnalysisTargetIdentity {
    pub route_identity: TargetRouteIdentity,
    pub provenance_class: AcceptedProvenanceClass,
    pub provider_family: String,
    pub launch_kind: AdapterLaunchKind,
    pub adapter_contract_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchAnalysisIdentity {
    pub analysis_version: u32,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_content_sha256: String,
    pub scoring_rule_version: String,
    pub execution_surface: BatchExecutionSurface,
    pub targets: Vec<AnalysisTargetIdentity>,
}

impl BatchAnalysisIdentity {
    pub fn from_plan(plan: &ScanBatchPlan) -> Result<Self, AnalysisError> {
        let surface = plan
            .targets
            .first()
            .ok_or(AnalysisError::IncompatibleIdentity)?
            .route_identity
            .execution_surface;
        let mut targets = Vec::with_capacity(plan.targets.len());
        for target in &plan.targets {
            if target.route_identity.execution_surface != surface
                || target.execution_adapter_identity.execution_surface != surface
            {
                return Err(AnalysisError::IncompatibleIdentity);
            }
            let provenance_class = AcceptedProvenanceClass::from_plan_target(
                target.target.model_source,
                target.target.model_verification,
                surface,
            )
            .ok_or(AnalysisError::UnacceptedProvenance)?;
            targets.push(AnalysisTargetIdentity {
                route_identity: target.route_identity.clone(),
                provenance_class,
                provider_family: target.execution_adapter_identity.provider_family.clone(),
                launch_kind: target.execution_adapter_identity.launch_kind,
                adapter_contract_version: target
                    .execution_adapter_identity
                    .adapter_contract_version
                    .clone(),
            });
        }
        Ok(Self {
            analysis_version: BATCH_ANALYSIS_VERSION,
            suite_id: plan.suite_id.clone(),
            suite_version: plan.suite_version.clone(),
            suite_content_sha256: plan.suite_content_sha256.clone(),
            scoring_rule_version: plan.scoring_rule_version.clone(),
            execution_surface: surface,
            targets,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalibrationPolicy {
    pub version: u32,
    pub history_window_days: u32,
    pub maximum_historical_batches: u32,
    pub minimum_historical_batches: u32,
    pub minimum_historical_utc_days: u32,
    pub minimum_candidate_members: u32,
    pub bootstrap_seed: u64,
    pub bootstrap_resamples: u32,
    pub confidence_level: f64,
    pub tolerated_absolute_drop: f64,
    pub tolerated_relative_drop: f64,
    pub likely_regression_enabled: bool,
}

impl CalibrationPolicy {
    pub fn production_v1() -> Self {
        Self {
            version: 1,
            history_window_days: 90,
            maximum_historical_batches: 12,
            minimum_historical_batches: 5,
            minimum_historical_utc_days: 3,
            minimum_candidate_members: 5,
            // Kept inside JavaScript's exact integer range for the desktop bridge.
            bootstrap_seed: 4_149_524_144_152,
            bootstrap_resamples: 2_000,
            confidence_level: 0.95,
            tolerated_absolute_drop: 5.0,
            tolerated_relative_drop: 0.08,
            // This remains false until the real-user calibration and independent review in the plan.
            likely_regression_enabled: false,
        }
    }

    pub fn validate(&self) -> Result<(), AnalysisError> {
        let finite = self.confidence_level.is_finite()
            && self.tolerated_absolute_drop.is_finite()
            && self.tolerated_relative_drop.is_finite();
        if self.version == 0
            || self.history_window_days == 0
            || self.maximum_historical_batches == 0
            || self.minimum_historical_batches == 0
            || self.minimum_historical_utc_days == 0
            || self.minimum_candidate_members == 0
            || self.bootstrap_resamples < 100
            || !finite
            || !(0.5..1.0).contains(&self.confidence_level)
            || self.tolerated_absolute_drop <= 0.0
            || self.tolerated_relative_drop <= 0.0
        {
            return Err(AnalysisError::InvalidPolicy);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineExclusionReason {
    CandidateBatch,
    DuplicateEvidenceId,
    NotCompletedFull,
    MissingOrInvalidSnapshot,
    NotStrictlyBeforeCutoff,
    OutsideHistoryWindow,
    IncompatibleIdentity,
    OlderBatchOnSameUtcDay,
    BeyondMaximumHistoricalBatches,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaselineExclusion {
    pub batch_id: Uuid,
    pub reason: BaselineExclusionReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineEvidenceCandidate {
    pub batch_id: Uuid,
    pub mode: BatchMode,
    pub status: BatchStatus,
    pub finished_at: DateTime<Utc>,
    pub identity: BatchAnalysisIdentity,
    pub has_valid_snapshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaselineSnapshot {
    pub candidate_batch_id: Uuid,
    pub baseline_as_of: DateTime<Utc>,
    pub analysis_version: u32,
    pub calibration_policy_version: u32,
    pub history_window_days: u32,
    pub maximum_historical_batches: u32,
    pub bootstrap_seed: u64,
    pub bootstrap_resamples: u32,
    pub identity: BatchAnalysisIdentity,
    pub selected_batch_ids: Vec<Uuid>,
    pub exclusions: Vec<BaselineExclusion>,
    pub content_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotDigestPayload<'a> {
    candidate_batch_id: Uuid,
    baseline_as_of: DateTime<Utc>,
    analysis_version: u32,
    calibration_policy_version: u32,
    history_window_days: u32,
    maximum_historical_batches: u32,
    bootstrap_seed: u64,
    bootstrap_resamples: u32,
    identity: &'a BatchAnalysisIdentity,
    selected_batch_ids: &'a [Uuid],
    exclusions: &'a [BaselineExclusion],
}

impl BaselineSnapshot {
    pub fn freeze(
        candidate_batch_id: Uuid,
        candidate_plan: &ScanBatchPlan,
        baseline_as_of: DateTime<Utc>,
        policy: &CalibrationPolicy,
        evidence: &[BaselineEvidenceCandidate],
    ) -> Result<Self, AnalysisError> {
        policy.validate()?;
        if candidate_plan.mode != BatchMode::Full {
            return Err(AnalysisError::FullBatchRequired);
        }
        let identity = BatchAnalysisIdentity::from_plan(candidate_plan)?;
        let mut counts = BTreeMap::<Uuid, usize>::new();
        for item in evidence {
            *counts.entry(item.batch_id).or_default() += 1;
        }
        let oldest = baseline_as_of
            .checked_sub_signed(Duration::days(i64::from(policy.history_window_days)))
            .ok_or(AnalysisError::InvalidPolicy)?;
        let mut exclusions = Vec::new();
        let mut eligible = Vec::new();
        let mut seen = BTreeSet::new();
        let mut ordered = evidence.to_vec();
        ordered.sort_by(|left, right| {
            right
                .finished_at
                .cmp(&left.finished_at)
                .then_with(|| left.batch_id.cmp(&right.batch_id))
        });
        for item in ordered {
            if !seen.insert(item.batch_id) {
                continue;
            }
            let reason = if item.batch_id == candidate_batch_id {
                Some(BaselineExclusionReason::CandidateBatch)
            } else if counts.get(&item.batch_id).copied().unwrap_or_default() > 1 {
                Some(BaselineExclusionReason::DuplicateEvidenceId)
            } else if item.mode != BatchMode::Full || item.status != BatchStatus::Completed {
                Some(BaselineExclusionReason::NotCompletedFull)
            } else if !item.has_valid_snapshot {
                Some(BaselineExclusionReason::MissingOrInvalidSnapshot)
            } else if item.finished_at >= baseline_as_of {
                Some(BaselineExclusionReason::NotStrictlyBeforeCutoff)
            } else if item.finished_at < oldest {
                Some(BaselineExclusionReason::OutsideHistoryWindow)
            } else if item.identity != identity {
                Some(BaselineExclusionReason::IncompatibleIdentity)
            } else {
                None
            };
            if let Some(reason) = reason {
                exclusions.push(BaselineExclusion {
                    batch_id: item.batch_id,
                    reason,
                });
            } else {
                eligible.push(item);
            }
        }

        let mut selected_days = BTreeSet::new();
        let mut selected = Vec::new();
        for item in eligible {
            let day = (
                item.finished_at.year(),
                item.finished_at.month(),
                item.finished_at.day(),
            );
            if !selected_days.insert(day) {
                exclusions.push(BaselineExclusion {
                    batch_id: item.batch_id,
                    reason: BaselineExclusionReason::OlderBatchOnSameUtcDay,
                });
            } else if selected.len()
                >= usize::try_from(policy.maximum_historical_batches).unwrap_or(usize::MAX)
            {
                exclusions.push(BaselineExclusion {
                    batch_id: item.batch_id,
                    reason: BaselineExclusionReason::BeyondMaximumHistoricalBatches,
                });
            } else {
                selected.push(item.batch_id);
            }
        }
        exclusions.sort_by(|left, right| {
            left.batch_id
                .cmp(&right.batch_id)
                .then_with(|| (left.reason as u8).cmp(&(right.reason as u8)))
        });
        let mut snapshot = Self {
            candidate_batch_id,
            baseline_as_of,
            analysis_version: BATCH_ANALYSIS_VERSION,
            calibration_policy_version: policy.version,
            history_window_days: policy.history_window_days,
            maximum_historical_batches: policy.maximum_historical_batches,
            bootstrap_seed: policy.bootstrap_seed,
            bootstrap_resamples: policy.bootstrap_resamples,
            identity,
            selected_batch_ids: selected,
            exclusions,
            content_sha256: String::new(),
        };
        snapshot.content_sha256 = snapshot.calculated_digest()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), AnalysisError> {
        let selected = self
            .selected_batch_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let excluded = self
            .exclusions
            .iter()
            .map(|entry| entry.batch_id)
            .collect::<BTreeSet<_>>();
        if self.analysis_version != BATCH_ANALYSIS_VERSION
            || self.identity.analysis_version != self.analysis_version
            || self.calibration_policy_version == 0
            || self.history_window_days == 0
            || self.maximum_historical_batches == 0
            || self.bootstrap_resamples < 100
            || self.bootstrap_seed > 9_007_199_254_740_991
            || self.selected_batch_ids.len()
                > usize::try_from(self.maximum_historical_batches).unwrap_or(usize::MAX)
            || selected.len() != self.selected_batch_ids.len()
            || excluded.len() != self.exclusions.len()
            || selected.contains(&self.candidate_batch_id)
            || !selected.is_disjoint(&excluded)
            || self.exclusions.iter().any(|entry| {
                entry.batch_id == self.candidate_batch_id
                    && entry.reason != BaselineExclusionReason::CandidateBatch
            })
            || self.content_sha256.len() != 64
            || !self
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || self.content_sha256 != self.calculated_digest()?
        {
            return Err(AnalysisError::InvalidSnapshot);
        }
        Ok(())
    }

    fn calculated_digest(&self) -> Result<String, AnalysisError> {
        let payload = SnapshotDigestPayload {
            candidate_batch_id: self.candidate_batch_id,
            baseline_as_of: self.baseline_as_of,
            analysis_version: self.analysis_version,
            calibration_policy_version: self.calibration_policy_version,
            history_window_days: self.history_window_days,
            maximum_historical_batches: self.maximum_historical_batches,
            bootstrap_seed: self.bootstrap_seed,
            bootstrap_resamples: self.bootstrap_resamples,
            identity: &self.identity,
            selected_batch_ids: &self.selected_batch_ids,
            exclusions: &self.exclusions,
        };
        let bytes = serde_json::to_vec(&payload).map_err(|_| AnalysisError::InvalidSnapshot)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskEvidence {
    pub task_id: String,
    pub category: Category,
    pub outcome: TaskOutcome,
    pub score: Option<f64>,
    pub failure_kind: Option<FailureKind>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberEvidence {
    pub member_ordinal: u32,
    pub target_position: u32,
    pub status: BatchMemberStatus,
    pub run_status: Option<RunStatus>,
    pub score: Option<ScoreSummary>,
    pub task_results: Vec<TaskEvidence>,
    pub isolation_complete: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedBatchEvidence {
    pub batch_id: Uuid,
    pub finished_at: DateTime<Utc>,
    pub members: Vec<MemberEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DistributionSummary {
    pub count: u32,
    pub median: f64,
    pub median_absolute_deviation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfidenceInterval {
    pub lower: f64,
    pub upper: f64,
    pub confidence_level: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatchedTaskDelta {
    pub task_id: String,
    pub category: Category,
    pub candidate_median: f64,
    pub baseline_median: f64,
    pub delta: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetBatchAnalysis {
    pub target_position: u32,
    pub signal: RegressionSignal,
    pub candidate: Option<DistributionSummary>,
    pub baseline: Option<DistributionSummary>,
    pub baseline_batch_count: u32,
    pub baseline_utc_day_count: u32,
    pub candidate_member_count: u32,
    pub delta: Option<f64>,
    pub absolute_drop: Option<f64>,
    pub relative_drop: Option<f64>,
    pub delta_confidence_interval: Option<ConfidenceInterval>,
    pub category_candidate: BTreeMap<Category, DistributionSummary>,
    pub category_baseline: BTreeMap<Category, DistributionSummary>,
    pub matched_task_deltas: Vec<MatchedTaskDelta>,
    pub excluded_candidate_member_ordinals: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchAnalysis {
    pub candidate_batch_id: Uuid,
    pub analysis_version: u32,
    pub calibration_policy_version: u32,
    pub baseline_snapshot_sha256: Option<String>,
    pub signal: RegressionSignal,
    pub targets: Vec<TargetBatchAnalysis>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    #[error("full batch analysis is required")]
    FullBatchRequired,
    #[error("analysis identity is incompatible")]
    IncompatibleIdentity,
    #[error("model provenance is not accepted for matched analysis")]
    UnacceptedProvenance,
    #[error("calibration policy is invalid")]
    InvalidPolicy,
    #[error("baseline snapshot is invalid")]
    InvalidSnapshot,
    #[error("analysis evidence is malformed")]
    MalformedEvidence,
}

pub fn distribution(values: &[f64]) -> Result<Option<DistributionSummary>, AnalysisError> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(AnalysisError::MalformedEvidence);
    }
    if values.is_empty() {
        return Ok(None);
    }
    let median_value = median(values)?;
    let deviations = values
        .iter()
        .map(|value| (value - median_value).abs())
        .collect::<Vec<_>>();
    Ok(Some(DistributionSummary {
        count: u32::try_from(values.len()).map_err(|_| AnalysisError::MalformedEvidence)?,
        median: median_value,
        median_absolute_deviation: median(&deviations)?,
    }))
}

pub fn analyze_matched_batch(
    candidate_mode: BatchMode,
    candidate_batch_id: Uuid,
    candidate_members: &[MemberEvidence],
    snapshot: Option<&BaselineSnapshot>,
    historical: &[CompletedBatchEvidence],
    policy: &CalibrationPolicy,
) -> Result<BatchAnalysis, AnalysisError> {
    policy.validate()?;
    if candidate_mode != BatchMode::Full {
        return Ok(BatchAnalysis {
            candidate_batch_id,
            analysis_version: BATCH_ANALYSIS_VERSION,
            calibration_policy_version: policy.version,
            baseline_snapshot_sha256: None,
            signal: RegressionSignal::InsufficientData,
            targets: Vec::new(),
        });
    }
    let snapshot = snapshot.ok_or(AnalysisError::InvalidSnapshot)?;
    snapshot.validate()?;
    if snapshot.candidate_batch_id != candidate_batch_id
        || snapshot.analysis_version != BATCH_ANALYSIS_VERSION
        || snapshot.calibration_policy_version != policy.version
    {
        return Err(AnalysisError::InvalidSnapshot);
    }
    let historical_by_id = historical
        .iter()
        .map(|batch| (batch.batch_id, batch))
        .collect::<BTreeMap<_, _>>();
    if historical_by_id.len() != historical.len() {
        return Err(AnalysisError::MalformedEvidence);
    }
    let selected = snapshot
        .selected_batch_ids
        .iter()
        .map(|id| {
            historical_by_id
                .get(id)
                .copied()
                .ok_or(AnalysisError::MalformedEvidence)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if selected
        .iter()
        .any(|batch| batch.finished_at >= snapshot.baseline_as_of)
    {
        return Err(AnalysisError::MalformedEvidence);
    }

    let target_count = snapshot.identity.targets.len();
    let mut targets = Vec::with_capacity(target_count);
    for target_position in 0..target_count {
        let target_position_u32 =
            u32::try_from(target_position).map_err(|_| AnalysisError::MalformedEvidence)?;
        let mut candidate_scores = Vec::new();
        let mut candidate_categories = BTreeMap::<Category, Vec<f64>>::new();
        let mut candidate_tasks = BTreeMap::<(String, Category), Vec<f64>>::new();
        let mut excluded = Vec::new();
        for member in candidate_members
            .iter()
            .filter(|member| member.target_position == target_position_u32)
        {
            match valid_member_summary(member)? {
                Some(summary) => {
                    candidate_scores.push(summary.ability_score);
                    for (category, score) in summary.category_scores {
                        candidate_categories
                            .entry(category)
                            .or_default()
                            .push(score);
                    }
                    for (identity, score) in summary.task_scores {
                        candidate_tasks.entry(identity).or_default().push(score);
                    }
                }
                None => excluded.push(member.member_ordinal),
            }
        }
        excluded.sort_unstable();

        let mut baseline_scores = Vec::new();
        let mut baseline_categories = BTreeMap::<Category, Vec<f64>>::new();
        let mut baseline_tasks = BTreeMap::<(String, Category), Vec<f64>>::new();
        let mut baseline_days = BTreeSet::new();
        for batch in &selected {
            let summaries = batch
                .members
                .iter()
                .filter(|member| member.target_position == target_position_u32)
                .map(valid_member_summary)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if summaries.is_empty() {
                continue;
            }
            baseline_scores.push(median(
                &summaries
                    .iter()
                    .map(|summary| summary.ability_score)
                    .collect::<Vec<_>>(),
            )?);
            let categories = summaries
                .iter()
                .flat_map(|summary| summary.category_scores.keys().copied())
                .collect::<BTreeSet<_>>();
            for category in categories {
                let values = summaries
                    .iter()
                    .filter_map(|summary| summary.category_scores.get(&category).copied())
                    .collect::<Vec<_>>();
                if values.len() == summaries.len() {
                    baseline_categories
                        .entry(category)
                        .or_default()
                        .push(median(&values)?);
                }
            }
            let tasks = summaries
                .iter()
                .flat_map(|summary| summary.task_scores.keys().cloned())
                .collect::<BTreeSet<_>>();
            for task in tasks {
                let values = summaries
                    .iter()
                    .filter_map(|summary| summary.task_scores.get(&task).copied())
                    .collect::<Vec<_>>();
                if values.len() == summaries.len() {
                    baseline_tasks
                        .entry(task)
                        .or_default()
                        .push(median(&values)?);
                }
            }
            baseline_days.insert((
                batch.finished_at.year(),
                batch.finished_at.month(),
                batch.finished_at.day(),
            ));
        }

        let candidate = distribution(&candidate_scores)?;
        let baseline = distribution(&baseline_scores)?;
        let candidate_count =
            u32::try_from(candidate_scores.len()).map_err(|_| AnalysisError::MalformedEvidence)?;
        let baseline_count =
            u32::try_from(baseline_scores.len()).map_err(|_| AnalysisError::MalformedEvidence)?;
        let baseline_day_count =
            u32::try_from(baseline_days.len()).map_err(|_| AnalysisError::MalformedEvidence)?;
        let sufficient = candidate_count >= policy.minimum_candidate_members
            && baseline_count >= policy.minimum_historical_batches
            && baseline_day_count >= policy.minimum_historical_utc_days;
        let (delta, absolute_drop, relative_drop, interval, signal) =
            if let (Some(candidate), Some(baseline)) = (&candidate, &baseline) {
                let delta = candidate.median - baseline.median;
                let absolute = baseline.median - candidate.median;
                let relative = (baseline.median > 0.0).then_some(absolute / baseline.median);
                let interval = bootstrap_delta_interval(
                    &candidate_scores,
                    &baseline_scores,
                    policy.bootstrap_seed ^ u64::from(target_position_u32),
                    policy.bootstrap_resamples,
                    policy.confidence_level,
                )?;
                let signal = if !sufficient {
                    RegressionSignal::Watch
                } else if policy.likely_regression_enabled
                    && absolute >= policy.tolerated_absolute_drop
                    && relative.is_some_and(|value| value >= policy.tolerated_relative_drop)
                    && interval.upper < -policy.tolerated_absolute_drop
                {
                    RegressionSignal::LikelyRegression
                } else if absolute <= policy.tolerated_absolute_drop / 2.0
                    && interval.upper >= -policy.tolerated_absolute_drop
                {
                    RegressionSignal::Stable
                } else {
                    RegressionSignal::Watch
                };
                (
                    Some(delta),
                    Some(absolute),
                    relative,
                    Some(interval),
                    signal,
                )
            } else {
                (None, None, None, None, RegressionSignal::InsufficientData)
            };

        let category_candidate = summarize_map(candidate_categories)?;
        let category_baseline = summarize_map(baseline_categories)?;
        let mut matched_task_deltas = Vec::new();
        for (task, candidate_values) in candidate_tasks {
            if let Some(baseline_values) = baseline_tasks.get(&task) {
                let candidate_median = median(&candidate_values)?;
                let baseline_median = median(baseline_values)?;
                matched_task_deltas.push(MatchedTaskDelta {
                    task_id: task.0,
                    category: task.1,
                    candidate_median,
                    baseline_median,
                    delta: candidate_median - baseline_median,
                });
            }
        }
        targets.push(TargetBatchAnalysis {
            target_position: target_position_u32,
            signal,
            candidate,
            baseline,
            baseline_batch_count: baseline_count,
            baseline_utc_day_count: baseline_day_count,
            candidate_member_count: candidate_count,
            delta,
            absolute_drop,
            relative_drop,
            delta_confidence_interval: interval,
            category_candidate,
            category_baseline,
            matched_task_deltas,
            excluded_candidate_member_ordinals: excluded,
        });
    }
    let signal = strongest_signal(targets.iter().map(|target| target.signal));
    Ok(BatchAnalysis {
        candidate_batch_id,
        analysis_version: BATCH_ANALYSIS_VERSION,
        calibration_policy_version: policy.version,
        baseline_snapshot_sha256: Some(snapshot.content_sha256.clone()),
        signal,
        targets,
    })
}

#[derive(Debug)]
struct ValidMemberSummary {
    ability_score: f64,
    category_scores: BTreeMap<Category, f64>,
    task_scores: BTreeMap<(String, Category), f64>,
}

fn valid_member_summary(
    member: &MemberEvidence,
) -> Result<Option<ValidMemberSummary>, AnalysisError> {
    let Some(score) = member.score.as_ref() else {
        return Ok(None);
    };
    if !member.isolation_complete
        || member.status != BatchMemberStatus::Completed
        || member.run_status != Some(RunStatus::Completed)
        || member.task_results.is_empty()
        || score.total_tasks == 0
        || usize::try_from(score.total_tasks).ok() != Some(member.task_results.len())
        || score.valid_tasks != score.total_tasks
        || score.passed_tasks > score.valid_tasks
        || !score.ability_score.is_finite()
        || !(0.0..=100.0).contains(&score.ability_score)
        || score
            .category_scores
            .values()
            .any(|value| !value.is_finite() || !(0.0..=100.0).contains(value))
    {
        return Ok(None);
    }
    let mut task_scores = BTreeMap::new();
    let mut passed = 0_u32;
    for task in &member.task_results {
        let valid_outcome = matches!(task.outcome, TaskOutcome::Passed | TaskOutcome::Failed);
        let valid_failure = match task.outcome {
            TaskOutcome::Passed => task.failure_kind.is_none(),
            TaskOutcome::Failed => task.failure_kind == Some(FailureKind::WrongAnswer),
            TaskOutcome::Invalid | TaskOutcome::Cancelled => false,
        };
        let Some(value) = task.score else {
            return Ok(None);
        };
        if !valid_outcome
            || !valid_failure
            || !value.is_finite()
            || !(0.0..=100.0).contains(&value)
            || task_scores
                .insert((task.task_id.clone(), task.category), value)
                .is_some()
        {
            return Ok(None);
        }
        if task.outcome == TaskOutcome::Passed {
            passed = passed
                .checked_add(1)
                .ok_or(AnalysisError::MalformedEvidence)?;
        }
    }
    if passed != score.passed_tasks {
        return Ok(None);
    }
    Ok(Some(ValidMemberSummary {
        ability_score: score.ability_score,
        category_scores: score.category_scores.clone(),
        task_scores,
    }))
}

fn summarize_map<K: Ord>(
    values: BTreeMap<K, Vec<f64>>,
) -> Result<BTreeMap<K, DistributionSummary>, AnalysisError> {
    values
        .into_iter()
        .map(|(key, values)| {
            distribution(&values)?
                .map(|summary| (key, summary))
                .ok_or(AnalysisError::MalformedEvidence)
        })
        .collect()
}

fn median(values: &[f64]) -> Result<f64, AnalysisError> {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err(AnalysisError::MalformedEvidence);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Ok((sorted[middle - 1] + sorted[middle]) / 2.0)
    } else {
        Ok(sorted[middle])
    }
}

fn bootstrap_delta_interval(
    candidate: &[f64],
    baseline: &[f64],
    seed: u64,
    resamples: u32,
    confidence_level: f64,
) -> Result<ConfidenceInterval, AnalysisError> {
    if candidate.is_empty() || baseline.is_empty() || resamples < 2 {
        return Err(AnalysisError::MalformedEvidence);
    }
    let mut rng = DeterministicRng(seed);
    let mut deltas = Vec::with_capacity(
        usize::try_from(resamples).map_err(|_| AnalysisError::MalformedEvidence)?,
    );
    for _ in 0..resamples {
        let candidate_sample = (0..candidate.len())
            .map(|_| candidate[rng.index(candidate.len())])
            .collect::<Vec<_>>();
        let baseline_sample = (0..baseline.len())
            .map(|_| baseline[rng.index(baseline.len())])
            .collect::<Vec<_>>();
        deltas.push(median(&candidate_sample)? - median(&baseline_sample)?);
    }
    deltas.sort_by(f64::total_cmp);
    let tail = (1.0 - confidence_level) / 2.0;
    let last = deltas.len() - 1;
    let lower_index = ((last as f64) * tail).floor() as usize;
    let upper_index = ((last as f64) * (1.0 - tail)).ceil() as usize;
    Ok(ConfidenceInterval {
        lower: deltas[lower_index.min(last)],
        upper: deltas[upper_index.min(last)],
        confidence_level,
    })
}

struct DeterministicRng(u64);

impl DeterministicRng {
    fn index(&mut self, upper: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 % u64::try_from(upper).unwrap_or(u64::MAX)).unwrap_or(0)
    }
}

fn strongest_signal(values: impl Iterator<Item = RegressionSignal>) -> RegressionSignal {
    let mut insufficient = false;
    let mut stable = false;
    let mut watch = false;
    let mut likely = false;
    for signal in values {
        match signal {
            RegressionSignal::InsufficientData => insufficient = true,
            RegressionSignal::Stable => stable = true,
            RegressionSignal::Watch => watch = true,
            RegressionSignal::LikelyRegression => likely = true,
        }
    }
    if likely {
        RegressionSignal::LikelyRegression
    } else if watch || (stable && insufficient) {
        RegressionSignal::Watch
    } else if stable {
        RegressionSignal::Stable
    } else {
        RegressionSignal::InsufficientData
    }
}

pub fn adapter_contract_compatible(
    left: &ExecutionAdapterIdentity,
    right: &ExecutionAdapterIdentity,
) -> bool {
    left.execution_surface == right.execution_surface
        && left.provider_family == right.provider_family
        && left.launch_kind == right.launch_kind
        && left.adapter_contract_version == right.adapter_contract_version
}
