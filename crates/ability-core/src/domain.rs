use crate::ExecutionAdapterIdentity;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    ChatGptClient,
    ClaudeClient,
    CodexCli,
    ClaudeCode,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    Manual,
    WindowsAccessibility,
    CliRequested,
    CliReported,
    DefaultRoute,
    #[default]
    LegacyUnknown,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelVerification {
    UserConfirmed,
    ProviderReported,
    Unverified,
    #[default]
    LegacyUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Quick,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Running,
    Completed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Passed,
    Failed,
    Invalid,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    CliMissing,
    RuntimeMissing,
    AuthExpired,
    QuotaExhausted,
    Network,
    UserCancelled,
    AppInterrupted,
    InfrastructureTimeout,
    AgentBudgetExceeded,
    VerifierError,
    WrongAnswer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    InstructionFollowing,
    Logic,
    CodeReview,
    CliCoding,
}

impl Display for Category {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::InstructionFollowing => "instruction_following",
            Self::Logic => "logic",
            Self::CodeReview => "code_review",
            Self::CliCoding => "cli_coding",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSelection {
    pub kind: TargetKind,
    pub reported_model: String,
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub model_source: ModelSource,
    #[serde(default)]
    pub model_verification: ModelVerification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentFingerprint {
    pub os_family: String,
    pub os_version: String,
    pub app_version: String,
    pub cli_version: Option<String>,
    pub verifier_runtime_version: Option<String>,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_content_sha256: String,
    pub scoring_rule_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_adapter_identity: Option<ExecutionAdapterIdentity>,
    pub resumed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreSummary {
    pub ability_score: f64,
    pub passed_tasks: u32,
    pub valid_tasks: u32,
    pub total_tasks: u32,
    pub category_scores: BTreeMap<Category, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: Uuid,
    pub target: TargetSelection,
    pub mode: RunMode,
    pub suite_id: String,
    pub suite_version: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub total_tasks: u32,
    pub completed_tasks: u32,
    pub environment: EnvironmentFingerprint,
    pub score: Option<ScoreSummary>,
}

impl RunRecord {
    pub fn new(
        target: TargetSelection,
        mode: RunMode,
        suite_id: String,
        suite_version: String,
        total_tasks: u32,
        environment: EnvironmentFingerprint,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            target,
            mode,
            suite_id,
            suite_version,
            status: RunStatus::Created,
            started_at: Utc::now(),
            finished_at: None,
            total_tasks,
            completed_tasks: 0,
            environment,
            score: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub run_id: Uuid,
    pub task_id: String,
    pub category: Category,
    pub outcome: TaskOutcome,
    pub score: Option<f64>,
    pub failure_kind: Option<FailureKind>,
    pub duration_ms: u64,
    pub answer_rel_path: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionStatus {
    Completed,
    ProviderError,
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentExitCodeCount {
    pub code: i32,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTokenSummary {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentModelSummary {
    pub requested_model: String,
    pub observed_model: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentExecutionSummary {
    pub run_id: Uuid,
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
    pub evidence_rel_path: Option<String>,
}
