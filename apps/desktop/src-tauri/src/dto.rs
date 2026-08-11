use ability_adapters::{RunEvent, TargetAvailability};
use ability_core::{
    AgentExecutionStatus, AgentExecutionSummary, AgentExitCodeCount, AgentModelSummary,
    AgentTokenSummary, BatchExecutionSurface, BatchFeatureLevel, BatchMode, Category,
    ExecutionAdapterIdentity, FailureKind, ModelSource, ModelVerification, RunMode, RunRecord,
    ScanBatchMemberRecord, ScanBatchPlan, ScanBatchTarget, TargetKind, TaskOutcome, TaskResult,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn deserialize_canonical_batch_uuid<'de, D>(deserializer: D) -> Result<Uuid, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let parsed = Uuid::parse_str(&value).map_err(serde::de::Error::custom)?;
    if parsed.is_nil() || !parsed.hyphenated().to_string().eq_ignore_ascii_case(&value) {
        return Err(serde::de::Error::custom(
            "batch id must be a non-nil canonical UUID",
        ));
    }
    Ok(parsed)
}

fn deserialize_canonical_run_uuid<'de, D>(deserializer: D) -> Result<Uuid, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let parsed = Uuid::parse_str(&value).map_err(serde::de::Error::custom)?;
    if parsed.is_nil() || !parsed.hyphenated().to_string().eq_ignore_ascii_case(&value) {
        return Err(serde::de::Error::custom(
            "run id must be a non-nil canonical UUID",
        ));
    }
    Ok(parsed)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackSummaryDto {
    pub id: String,
    pub version: String,
    pub title: String,
    pub task_count: u32,
    pub estimated_minutes: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapDto {
    pub targets: Vec<TargetAvailability>,
    pub client_pack: PackSummaryDto,
    pub cli_pack: PackSummaryDto,
    pub batch_capabilities: Vec<BatchFeatureLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetSelectionInput {
    pub kind: TargetKind,
    pub reported_model: String,
    pub reasoning_effort: Option<String>,
    pub model_source: ModelSource,
    pub model_verification: ModelVerification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchTargetInput {
    pub target: TargetSelectionInput,
    pub execution_surface: BatchExecutionSurface,
    pub execution_adapter_identity: ExecutionAdapterIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchPlanInput {
    pub mode: BatchMode,
    pub seed: u64,
    pub targets: Vec<BatchTargetInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAcknowledgedBatchInput {
    pub plan: BatchPlanInput,
    pub estimate_issued_at: DateTime<Utc>,
    pub acknowledgement_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchIdInput {
    #[serde(deserialize_with = "deserialize_canonical_batch_uuid")]
    pub batch_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteBatchInput {
    #[serde(deserialize_with = "deserialize_canonical_batch_uuid")]
    pub batch_id: Uuid,
    pub delete_owned_runs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizeBatchExecutionInput {
    #[serde(deserialize_with = "deserialize_canonical_batch_uuid")]
    pub batch_id: Uuid,
    pub acknowledgement_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EstimateBatchRetryInput {
    #[serde(deserialize_with = "deserialize_canonical_batch_uuid")]
    pub batch_id: Uuid,
    pub member_ordinal: u32,
    pub expected_failure_kind: FailureKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizeBatchRetryInput {
    #[serde(deserialize_with = "deserialize_canonical_batch_uuid")]
    pub batch_id: Uuid,
    pub member_ordinal: u32,
    pub allowed_failure_kind: FailureKind,
    pub estimate_created_at: DateTime<Utc>,
    pub acknowledgement_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchEstimateDto {
    pub plan: ScanBatchPlan,
    pub capabilities: Vec<BatchFeatureLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchExecutionAuthorizationDto {
    pub batch_id: Uuid,
    pub member_ordinal: Option<u32>,
    pub attempt_number: u32,
    pub max_task_launches: u64,
    pub max_provider_turns: u64,
    pub max_task_budget_secs: u64,
    pub max_guided_interactions: u64,
    pub acknowledgement_hash: String,
    pub allowed_failure_kind: Option<FailureKind>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchRetryEstimateDto {
    pub authorization: BatchExecutionAuthorizationDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuidedMemberDecision {
    Runnable,
    BlockedByActive,
    Exhausted,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextGuidedMemberDto {
    pub decision: GuidedMemberDecision,
    pub member: Option<ScanBatchMemberRecord>,
    pub target: Option<ScanBatchTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitGuidedBatchAnswerInput {
    #[serde(deserialize_with = "deserialize_canonical_batch_uuid")]
    pub batch_id: Uuid,
    pub member_ordinal: u32,
    #[serde(deserialize_with = "deserialize_canonical_run_uuid")]
    pub run_id: Uuid,
    pub task_id: String,
    pub answer: String,
    #[serde(deserialize_with = "deserialize_fresh_conversation_true")]
    pub user_attested_new_conversation: bool,
}

fn deserialize_fresh_conversation_true<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match bool::deserialize(deserializer)? {
        true => Ok(true),
        false => Err(serde::de::Error::custom(
            "fresh blank conversation attestation must be true",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclineGuidedBatchAttestationInput {
    #[serde(deserialize_with = "deserialize_canonical_batch_uuid")]
    pub batch_id: Uuid,
    pub member_ordinal: u32,
    #[serde(deserialize_with = "deserialize_canonical_run_uuid")]
    pub run_id: Uuid,
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRunInput {
    pub target: TargetSelectionInput,
    pub mode: RunMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectClientSelectionInput {
    pub target: TargetKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitAnswerInput {
    pub run_id: String,
    pub task_id: String,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportReportInput {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunIdInput {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentEvidenceInput {
    pub run_id: String,
    pub task_id: String,
}

fn deserialize_required_nullable_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeTargetSelectionInput {
    pub kind: TargetKind,
    pub reported_model: String,
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    pub reasoning_effort: Option<String>,
    pub model_source: ModelSource,
    pub model_verification: ModelVerification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeRunInput {
    pub run_id: String,
    pub expected_target: ResumeTargetSelectionInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteTargetHistoryInput {
    pub target: TargetKind,
    pub expected_run_ids: Vec<String>,
}

fn deserialize_required_retention<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<u32>::deserialize(deserializer)?;
    if matches!(value, None | Some(7 | 30 | 90)) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "raw retention must be forever, 7, 30, or 90",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRetentionInput {
    #[serde(deserialize_with = "deserialize_required_retention")]
    pub raw_retention_days: Option<u32>,
}

fn deserialize_true<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match bool::deserialize(deserializer)? {
        true => Ok(true),
        false => Err(serde::de::Error::custom(
            "unencrypted raw-data acknowledgement must be true",
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FullBackupInput {
    #[serde(deserialize_with = "deserialize_true")]
    pub acknowledged_unencrypted_raw_data: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSettingsDto {
    pub raw_retention_days: Option<u32>,
    pub cleanup_pending: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetailDto {
    pub run: RunRecord,
    pub task_results: Vec<TaskResultDto>,
    pub agent_execution_summaries: Vec<AgentExecutionSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentExecutionSummaryDto {
    pub task_id: String,
    pub contract_version: String,
    pub status: AgentExecutionStatus,
    pub command_succeeded: Option<u64>,
    pub command_failed: Option<u64>,
    pub command_unknown: Option<u64>,
    pub exit_codes: Vec<AgentExitCodeCount>,
    pub tool_error_count: Option<u64>,
    pub file_change_count: Option<u64>,
    pub session_present: Option<bool>,
    pub tokens: AgentTokenSummary,
    pub model: Option<AgentModelSummary>,
    pub provider_unknown_field_count: Option<u64>,
    pub agent_duration_ms: Option<u64>,
    pub detail_available: bool,
}

impl From<AgentExecutionSummary> for AgentExecutionSummaryDto {
    fn from(summary: AgentExecutionSummary) -> Self {
        Self {
            task_id: summary.task_id,
            contract_version: summary.contract_version,
            status: summary.status,
            command_succeeded: summary.command_succeeded,
            command_failed: summary.command_failed,
            command_unknown: summary.command_unknown,
            exit_codes: summary.exit_codes,
            tool_error_count: summary.tool_error_count,
            file_change_count: summary.file_change_count,
            session_present: summary.session_present,
            tokens: summary.tokens,
            model: summary.model,
            provider_unknown_field_count: summary.provider_unknown_field_count,
            agent_duration_ms: summary.agent_duration_ms,
            detail_available: summary.evidence_rel_path.is_some(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentToolUsageDto {
    pub name: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentToolErrorDto {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentExecutionDetailDto {
    pub final_text: String,
    pub session_present: bool,
    pub tokens: AgentTokenSummary,
    pub tool_summary: Vec<AgentToolUsageDto>,
    pub command_succeeded: Option<u64>,
    pub command_failed: Option<u64>,
    pub command_unknown: Option<u64>,
    pub exit_codes: Vec<AgentExitCodeCount>,
    pub tool_errors: Vec<AgentToolErrorDto>,
    pub file_change_count: Option<u64>,
    pub model: AgentModelSummary,
    pub provider_unknown_field_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum AgentEvidenceResponseDto {
    Available(AgentExecutionDetailDto),
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResultDto {
    pub run_id: Uuid,
    pub task_id: String,
    pub category: Category,
    pub outcome: TaskOutcome,
    pub score: Option<f64>,
    pub failure_kind: Option<FailureKind>,
    pub duration_ms: u64,
    pub answer_rel_path: Option<String>,
}

impl TryFrom<TaskResult> for TaskResultDto {
    type Error = String;

    fn try_from(result: TaskResult) -> Result<Self, Self::Error> {
        if result
            .answer_rel_path
            .as_deref()
            .is_some_and(|path| !is_safe_relative_artifact(path))
        {
            return Err("stored artifact metadata is not a safe relative path".into());
        }
        Ok(Self {
            run_id: result.run_id,
            task_id: result.task_id,
            category: result.category,
            outcome: result.outcome,
            score: result.score,
            failure_kind: result.failure_kind,
            duration_ms: result.duration_ms,
            answer_rel_path: result.answer_rel_path,
        })
    }
}

fn is_safe_relative_artifact(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['/', '\\'])
        && !value.contains([':', '\\'])
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunErrorEvent {
    pub run_id: String,
    pub message: String,
}

pub type CliRunEventDto = RunEvent;

#[cfg(test)]
mod tests {
    use super::*;
    use ability_core::{ModelSource, ModelVerification, RunMode, TargetKind};
    use serde_json::json;

    #[test]
    fn detect_client_selection_input_accepts_client_and_cli_target_kinds() {
        let input: DetectClientSelectionInput = serde_json::from_value(json!({
            "target": "chat_gpt_client"
        }))
        .unwrap();
        assert_eq!(input.target, TargetKind::ChatGptClient);

        assert!(serde_json::from_value::<DetectClientSelectionInput>(json!({
            "target": "codex_cli"
        }))
        .is_ok());
    }

    #[test]
    fn start_run_input_uses_the_expected_camel_case_wire_shape() {
        let input: StartRunInput = serde_json::from_value(json!({
            "target": {
                "kind": "chat_gpt_client",
                "reportedModel": "GPT-5",
                "reasoningEffort": "high",
                "modelSource": "windows_accessibility",
                "modelVerification": "user_confirmed"
            },
            "mode": "quick"
        }))
        .unwrap();

        assert_eq!(input.target.kind, TargetKind::ChatGptClient);
        assert_eq!(input.target.reported_model, "GPT-5");
        assert_eq!(input.target.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(input.target.model_source, ModelSource::WindowsAccessibility);
        assert_eq!(
            input.target.model_verification,
            ModelVerification::UserConfirmed
        );
        assert_eq!(input.mode, RunMode::Quick);
    }

    #[test]
    fn start_run_input_requires_model_provenance() {
        let missing = serde_json::from_value::<StartRunInput>(json!({
            "target": {
                "kind": "codex_cli",
                "reportedModel": "default",
                "reasoningEffort": null
            },
            "mode": "quick"
        }));

        assert!(missing.is_err());
    }

    #[test]
    fn start_run_input_rejects_unknown_outer_and_nested_fields() {
        let outer = serde_json::from_value::<StartRunInput>(json!({
            "target": {
                "kind": "codex_cli",
                "reportedModel": "default",
                "reasoningEffort": null,
                "modelSource": "default_route",
                "modelVerification": "unverified"
            },
            "mode": "quick",
            "arguments": ["--dangerously-skip-permissions"]
        }));
        assert!(outer.is_err());

        let nested = serde_json::from_value::<StartRunInput>(json!({
            "target": {
                "kind": "codex_cli",
                "reportedModel": "default",
                "reasoningEffort": null,
                "modelSource": "default_route",
                "modelVerification": "unverified",
                "program": "anything"
            },
            "mode": "quick"
        }));
        assert!(nested.is_err());
    }

    #[test]
    fn submit_answer_input_rejects_unknown_fields() {
        let valid: SubmitAnswerInput = serde_json::from_value(json!({
            "runId": "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
            "taskId": "logic-truth",
            "answer": "{}"
        }))
        .unwrap();
        assert_eq!(valid.task_id, "logic-truth");

        let unknown = serde_json::from_value::<SubmitAnswerInput>(json!({
            "runId": "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
            "taskId": "logic-truth",
            "answer": "{}",
            "artifactPath": "C:/Users/example/secrets.txt"
        }));
        assert!(unknown.is_err());
    }

    #[test]
    fn export_report_input_accepts_only_a_run_id_and_never_a_destination() {
        let valid: ExportReportInput = serde_json::from_value(json!({
            "runId": "39d9f772-2e12-4b2d-af13-94c32d36f2d3"
        }))
        .unwrap();
        assert_eq!(valid.run_id, "39d9f772-2e12-4b2d-af13-94c32d36f2d3");

        for forbidden in [
            json!({
                "runId": "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
                "destination": "C:/Users/Alice/report.html"
            }),
            json!({
                "runId": "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
                "filePath": "/home/alice/report.html"
            }),
        ] {
            assert!(serde_json::from_value::<ExportReportInput>(forbidden).is_err());
        }
    }

    #[test]
    fn resume_input_binds_the_exact_target_and_rejects_unknown_nested_fields() {
        let valid: ResumeRunInput = serde_json::from_value(json!({
            "runId": "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
            "expectedTarget": {
                "kind": "codex_cli",
                "reportedModel": "gpt-5.1-codex",
                "reasoningEffort": "high",
                "modelSource": "cli_requested",
                "modelVerification": "user_confirmed"
            }
        }))
        .unwrap();
        assert_eq!(valid.expected_target.kind, TargetKind::CodexCli);
        assert_eq!(valid.expected_target.reported_model, "gpt-5.1-codex");
        assert_eq!(
            valid.expected_target.reasoning_effort.as_deref(),
            Some("high")
        );
        assert_eq!(
            valid.expected_target.model_source,
            ModelSource::CliRequested
        );
        assert_eq!(
            valid.expected_target.model_verification,
            ModelVerification::UserConfirmed
        );

        for forbidden in [
            json!({
                "runId": valid.run_id,
                "expectedTarget": {
                    "kind": "codex_cli",
                    "reportedModel": "gpt-5.1-codex",
                    "reasoningEffort": "high",
                    "modelSource": "cli_requested",
                    "modelVerification": "user_confirmed"
                },
                "path": "C:/Users/Alice"
            }),
            json!({
                "runId": valid.run_id,
                "expectedTarget": {
                    "kind": "codex_cli",
                    "reportedModel": "gpt-5.1-codex",
                    "reasoningEffort": "high",
                    "modelSource": "cli_requested",
                    "modelVerification": "user_confirmed",
                    "program": "cmd.exe"
                }
            }),
            json!({
                "runId": valid.run_id,
                "expectedTarget": {
                    "kind": "codex_cli",
                    "reportedModel": "gpt-5.1-codex",
                    "reasoningEffort": "high",
                    "modelSource": "cli_requested",
                    "modelVerification": "user_confirmed"
                },
                "force": true
            }),
            json!({
                "runId": valid.run_id,
                "expectedTarget": {
                    "kind": "codex_cli",
                    "reportedModel": "gpt-5.1-codex",
                    "modelSource": "cli_requested",
                    "modelVerification": "user_confirmed"
                }
            }),
        ] {
            assert!(serde_json::from_value::<ResumeRunInput>(forbidden).is_err());
        }
    }

    #[test]
    fn single_run_delete_inputs_accept_only_a_run_id() {
        let valid: RunIdInput = serde_json::from_value(json!({
            "runId": "39d9f772-2e12-4b2d-af13-94c32d36f2d3"
        }))
        .unwrap();
        assert_eq!(valid.run_id, "39d9f772-2e12-4b2d-af13-94c32d36f2d3");
        for forbidden in [
            json!({"runId": valid.run_id, "path": "C:/Users/Alice"}),
            json!({"runId": valid.run_id, "force": true}),
            json!({"runId": valid.run_id, "program": "cmd.exe"}),
        ] {
            assert!(serde_json::from_value::<RunIdInput>(forbidden).is_err());
        }
    }

    #[test]
    fn target_history_delete_input_binds_target_and_exact_reviewed_ids() {
        let valid: DeleteTargetHistoryInput = serde_json::from_value(json!({
            "target": "codex_cli",
            "expectedRunIds": [
                "39d9f772-2e12-4b2d-af13-94c32d36f2d3",
                "f1e93ab2-b1ec-4204-a973-b4cdf4ddf81b"
            ]
        }))
        .unwrap();
        assert_eq!(valid.target, TargetKind::CodexCli);
        assert_eq!(valid.expected_run_ids.len(), 2);
        assert!(serde_json::from_value::<DeleteTargetHistoryInput>(json!({
            "target": "codex_cli",
            "expectedRunIds": [],
            "path": "C:/Users/Alice"
        }))
        .is_err());
    }

    #[test]
    fn retention_input_requires_one_nullable_allowlisted_field_and_nothing_else() {
        for (wire, expected) in [
            (json!({"rawRetentionDays": null}), None),
            (json!({"rawRetentionDays": 7}), Some(7)),
            (json!({"rawRetentionDays": 30}), Some(30)),
            (json!({"rawRetentionDays": 90}), Some(90)),
        ] {
            let input: SetRetentionInput = serde_json::from_value(wire).unwrap();
            assert_eq!(input.raw_retention_days, expected);
        }
        for invalid in [
            json!({}),
            json!({"rawRetentionDays": 8}),
            json!({"rawRetentionDays": 4294967296_u64}),
            json!({"rawRetentionDays": "7"}),
            json!({"rawRetentionDays": null, "path": "C:/private"}),
        ] {
            assert!(
                serde_json::from_value::<SetRetentionInput>(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn full_backup_input_requires_exact_true_acknowledgement_and_accepts_no_path() {
        let accepted: FullBackupInput = serde_json::from_value(json!({
            "acknowledgedUnencryptedRawData": true
        }))
        .unwrap();
        assert!(accepted.acknowledged_unencrypted_raw_data);
        for invalid in [
            json!({}),
            json!({"acknowledgedUnencryptedRawData": null}),
            json!({"acknowledgedUnencryptedRawData": "true"}),
            json!({"acknowledgedUnencryptedRawData": false}),
            json!({
                "acknowledgedUnencryptedRawData": true,
                "destination": "C:/private/backup.zip"
            }),
        ] {
            assert!(serde_json::from_value::<FullBackupInput>(invalid).is_err());
        }
    }
}
