mod classify;
mod claude;
mod codex;
mod process;
mod verifier;

pub use classify::*;
pub use claude::*;
pub use codex::*;
pub use process::*;
pub use verifier::*;

use ability_core::{FailureKind, TargetKind};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetAvailability {
    pub kind: TargetKind,
    pub installed: bool,
    pub version: Option<String>,
    pub auth_state: AuthState,
    pub prerequisites: Vec<PrerequisiteStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Unknown,
    Ready,
    NeedsLogin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrerequisiteStatus {
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub prompt: String,
    pub workspace: PathBuf,
    pub time_budget_secs: u64,
    pub max_turns: u32,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterCompletion {
    Completed {
        duration_ms: u64,
        stdout: String,
        stderr: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("target is unavailable")]
    Unavailable,
    #[error("task failed before verification: {kind:?}: {detail}")]
    Infrastructure { kind: FailureKind, detail: String },
    #[error("agent budget was exhausted")]
    AgentBudgetExceeded,
    #[error("user cancelled the task")]
    Cancelled,
}

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn kind(&self) -> TargetKind;
    async fn detect(&self) -> TargetAvailability;
    async fn execute(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<AdapterCompletion, AdapterError>;
}
