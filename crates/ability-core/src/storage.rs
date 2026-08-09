use crate::{
    BaselineEvidenceCandidate, BaselineSnapshot, BatchAnalysis, BatchAnalysisIdentity, BatchMode,
    BatchStatus, CalibrationPolicy, CompletedBatchEvidence, FailureKind, MemberEvidence, RunMode,
    RunRecord, RunStatus, ScanBatchPlan, ScoreSummary, TargetKind, TargetSelection, TaskEvidence,
    TaskOutcome, TaskResult, analyze_matched_batch, grading::has_coherent_task_evidence,
    summarize_scores,
};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("JSON encoding or decoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stored timestamp is invalid: {0}")]
    Time(#[from] chrono::ParseError),
    #[error("stored value is invalid: {0}")]
    InvalidData(String),
    #[error("run does not exist: {0}")]
    RunNotFound(Uuid),
}

pub struct RunRepository {
    connection: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionCandidate {
    pub id: Uuid,
    pub target: TargetSelection,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BackupRunBinding {
    pub id: Uuid,
    pub target: TargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchMemberStatus {
    Planned,
    Reserved,
    Launching,
    Running,
    Deferred,
    Completed,
    Invalid,
    Unavailable,
    Cancelled,
}

impl BatchMemberStatus {
    fn is_active(self) -> bool {
        matches!(self, Self::Reserved | Self::Launching | Self::Running)
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Invalid | Self::Unavailable | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchMemberSeed {
    pub ordinal: u32,
    pub target_position: u32,
    pub repetition_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanBatchMemberRecord {
    pub ordinal: u32,
    pub target_position: u32,
    pub repetition_index: u32,
    pub run_id: Option<Uuid>,
    pub status: BatchMemberStatus,
    pub failure_kind: Option<FailureKind>,
    pub attempt_number: u32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanBatchRecord {
    pub id: Uuid,
    pub plan: ScanBatchPlan,
    pub baseline_snapshot: Option<BaselineSnapshot>,
    pub status: BatchStatus,
    pub cancel_requested: bool,
    pub planned_member_count: u32,
    pub terminal_member_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub members: Vec<ScanBatchMemberRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchReservation {
    pub batch_id: Uuid,
    pub member: ScanBatchMemberRecord,
    pub run: RunRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScanExecutionAuthorization {
    pub batch_id: Uuid,
    pub member_ordinal: Option<u32>,
    pub attempt_number: u32,
    pub max_provider_turns: u64,
    pub max_task_budget_secs: u64,
    pub acknowledgement_hash: String,
    pub allowed_failure_kind: Option<FailureKind>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl ScanExecutionAuthorization {
    pub fn expected_retry_acknowledgement_hash(
        &self,
        plan: &ScanBatchPlan,
    ) -> Result<String, StorageError> {
        validate_plan_acknowledgement_hash(plan)?;
        let member_ordinal = self.member_ordinal.ok_or_else(|| {
            StorageError::InvalidData(
                "retry acknowledgement requires a member-scoped authorization".into(),
            )
        })?;
        let allowed_failure_kind = self.allowed_failure_kind.ok_or_else(|| {
            StorageError::InvalidData(
                "retry acknowledgement requires an allowed failure kind".into(),
            )
        })?;
        if self.attempt_number == 0
            || self.max_provider_turns == 0
            || self.max_task_budget_secs == 0
            || self.expires_at <= self.created_at
            || !is_retryable_batch_failure(allowed_failure_kind)
        {
            return Err(StorageError::InvalidData(
                "retry acknowledgement payload is invalid".into(),
            ));
        }
        let payload = RetryAuthorizationHashPayload {
            policy_version: 1,
            plan_acknowledgement_hash: &plan.acknowledgement_hash,
            batch_id: self.batch_id,
            member_ordinal,
            attempt_number: self.attempt_number,
            max_provider_turns: self.max_provider_turns,
            max_task_budget_secs: self.max_task_budget_secs,
            allowed_failure_kind,
            expires_at: self.expires_at,
            created_at: self.created_at,
        };
        let bytes = serde_json::to_vec(&payload)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RetryAuthorizationHashPayload<'a> {
    policy_version: u32,
    plan_acknowledgement_hash: &'a str,
    batch_id: Uuid,
    member_ordinal: u32,
    attempt_number: u32,
    max_provider_turns: u64,
    max_task_budget_secs: u64,
    allowed_failure_kind: FailureKind,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationEnforcement {
    UserAttested,
    MachineEnforced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IsolationAttestation {
    pub policy_version: u32,
    pub enforcement: IsolationEnforcement,
    pub user_attested: bool,
    pub recorded_at: DateTime<Utc>,
}

struct StoredBatchRow {
    plan_json: String,
    mode_json: String,
    suite_id: String,
    suite_version: String,
    content_sha256: String,
    scoring_rule_version: String,
    seed: i64,
    status_json: String,
    acknowledgement_hash: String,
    acknowledgement_expires_at: String,
    planned_member_count: i64,
    terminal_member_count: i64,
    cancel_requested: i64,
    created_at: String,
    updated_at: String,
}

struct StoredBatchIdentityRow {
    plan_json: String,
    mode_json: String,
    suite_id: String,
    suite_version: String,
    content_sha256: String,
    scoring_rule_version: String,
    seed: i64,
    acknowledgement_hash: String,
    acknowledgement_expires_at: String,
}

impl RunRepository {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        connection.execute_batch(include_str!("../migrations/0001_init.sql"))?;
        connection.execute_batch(include_str!("../migrations/0002_settings.sql"))?;
        connection.execute_batch(include_str!("../migrations/0003_scan_batches.sql"))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn insert_run(&self, run: &RunRecord) -> Result<(), StorageError> {
        validate_run(run)?;
        if !matches!(run.status, RunStatus::Created | RunStatus::Running)
            || run.completed_tasks != 0
            || run.finished_at.is_some()
            || run.score.is_some()
        {
            return Err(StorageError::InvalidData(
                "a new run must be created or running with no terminal evidence".into(),
            ));
        }
        let target_json = serde_json::to_string(&run.target)?;
        let mode_json = serde_json::to_string(&run.mode)?;
        let status_json = serde_json::to_string(&run.status)?;
        let environment_json = serde_json::to_string(&run.environment)?;
        let score_json = run.score.as_ref().map(serde_json::to_string).transpose()?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT OR IGNORE INTO targets(target_json) VALUES (?1)",
            [&target_json],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO suite_versions(
               suite_id,suite_version,content_sha256,scoring_rule_version
             ) VALUES (?1,?2,?3,?4)",
            params![
                &run.suite_id,
                &run.suite_version,
                &run.environment.suite_content_sha256,
                &run.environment.scoring_rule_version,
            ],
        )?;
        transaction.execute(
            "INSERT INTO runs(
              id,target_json,mode_json,suite_id,suite_version,status_json,started_at,
              finished_at,total_tasks,completed_tasks,environment_json,score_json
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                run.id.to_string(),
                target_json,
                mode_json,
                &run.suite_id,
                &run.suite_version,
                status_json,
                run.started_at.to_rfc3339(),
                run.finished_at.as_ref().map(|value| value.to_rfc3339()),
                i64::from(run.total_tasks),
                i64::from(run.completed_tasks),
                environment_json,
                score_json,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_task_result(&self, result: &TaskResult) -> Result<(), StorageError> {
        validate_task_result(result)?;
        let duration_ms = i64::try_from(result.duration_ms)
            .map_err(|_| StorageError::InvalidData("duration_ms exceeds SQLite range".into()))?;
        let category_json = serde_json::to_string(&result.category)?;
        let outcome_json = serde_json::to_string(&result.outcome)?;
        let failure_kind_json = result
            .failure_kind
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch_owned: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM scan_batch_members WHERE run_id=?1",
            [result.run_id.to_string()],
            |row| row.get(0),
        )?;
        if batch_owned != 0 {
            return Err(StorageError::InvalidData(
                "batch-owned task evidence requires the atomic isolation checkpoint".into(),
            ));
        }
        let (status_json, total_tasks): (String, i64) = transaction
            .query_row(
                "SELECT status_json,total_tasks FROM runs WHERE id=?1",
                [result.run_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(StorageError::RunNotFound(result.run_id))?;
        let status: RunStatus = serde_json::from_str(&status_json)?;
        if status != RunStatus::Running {
            return Err(StorageError::InvalidData(
                "task results can be saved only while the run is running".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO task_results(
              run_id,task_id,category_json,outcome_json,score,failure_kind_json,
              duration_ms,answer_rel_path,detail
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
            ON CONFLICT(run_id,task_id) DO UPDATE SET
              category_json=excluded.category_json,
              outcome_json=excluded.outcome_json,
              score=excluded.score,
              failure_kind_json=excluded.failure_kind_json,
              duration_ms=excluded.duration_ms,
              answer_rel_path=excluded.answer_rel_path,
              detail=excluded.detail",
            params![
                result.run_id.to_string(),
                &result.task_id,
                category_json,
                outcome_json,
                result.score,
                failure_kind_json,
                duration_ms,
                &result.answer_rel_path,
                &result.detail,
            ],
        )?;
        let checkpoint_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM task_results WHERE run_id=?1",
            [result.run_id.to_string()],
            |row| row.get(0),
        )?;
        if checkpoint_count > total_tasks {
            return Err(StorageError::InvalidData(
                "checkpoint count exceeds the run total_tasks".into(),
            ));
        }
        let changed = transaction.execute(
            "UPDATE runs SET completed_tasks=(
              SELECT COUNT(*) FROM task_results WHERE run_id=?1
            ) WHERE id=?1 AND status_json=?2",
            params![
                result.run_id.to_string(),
                serde_json::to_string(&RunStatus::Running)?,
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(
                "run changed while checkpoint evidence was being saved".into(),
            ));
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn complete_run(
        &self,
        run_id: Uuid,
        score: Option<&ScoreSummary>,
    ) -> Result<(), StorageError> {
        if let Some(score) = score {
            validate_score_summary(score)?;
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (status_json, total_tasks, completed_tasks): (String, i64, i64) = transaction
            .query_row(
                "SELECT status_json,total_tasks,completed_tasks FROM runs WHERE id=?1",
                [run_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(StorageError::RunNotFound(run_id))?;
        let status: RunStatus = serde_json::from_str(&status_json)?;
        let result_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM task_results WHERE run_id=?1",
            [run_id.to_string()],
            |row| row.get(0),
        )?;
        if status != RunStatus::Running
            || completed_tasks != total_tasks
            || result_count != total_tasks
        {
            return Err(StorageError::InvalidData(
                "complete_run requires a running run with complete task evidence".into(),
            ));
        }
        if score.is_some_and(|value| i64::from(value.total_tasks) != total_tasks) {
            return Err(StorageError::InvalidData(
                "score total_tasks does not match the run total_tasks".into(),
            ));
        }
        let total_tasks_u32 = u32::try_from(total_tasks)
            .map_err(|_| StorageError::InvalidData("run total_tasks is out of range".into()))?;
        let results = task_results_in_transaction(&transaction, run_id)?;
        if results
            .iter()
            .any(|result| !has_coherent_task_evidence(result))
        {
            return Err(StorageError::InvalidData(
                "complete_run requires coherent task evidence".into(),
            ));
        }
        let canonical_score = summarize_scores(&results, total_tasks_u32);
        if score != canonical_score.as_ref() {
            return Err(StorageError::InvalidData(
                "score does not match canonical task evidence".into(),
            ));
        }
        let changed = transaction.execute(
            "UPDATE runs
             SET status_json=?2, finished_at=?3, score_json=?4
             WHERE id=?1 AND status_json=?5 AND completed_tasks=total_tasks",
            params![
                run_id.to_string(),
                serde_json::to_string(&RunStatus::Completed)?,
                Utc::now().to_rfc3339(),
                score.map(serde_json::to_string).transpose()?,
                serde_json::to_string(&RunStatus::Running)?,
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(
                "run changed while completion was being validated".into(),
            ));
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn finish_without_score(
        &self,
        run_id: Uuid,
        status: RunStatus,
    ) -> Result<(), StorageError> {
        if !matches!(status, RunStatus::Cancelled | RunStatus::Interrupted) {
            return Err(StorageError::InvalidData(
                "finish_without_score requires cancelled or interrupted status".into(),
            ));
        }

        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch_owned: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM scan_batch_members WHERE run_id=?1",
            [run_id.to_string()],
            |row| row.get(0),
        )?;
        if batch_owned != 0 {
            return Err(StorageError::InvalidData(
                "batch-owned run terminalization requires a member transition".into(),
            ));
        }
        let changed = transaction.execute(
            "UPDATE runs
             SET status_json=?2, finished_at=?3, score_json=NULL
             WHERE id=?1 AND status_json=?4",
            params![
                run_id.to_string(),
                serde_json::to_string(&status)?,
                Utc::now().to_rfc3339(),
                serde_json::to_string(&RunStatus::Running)?,
            ],
        )?;
        if changed == 1 {
            transaction.commit()?;
            return Ok(());
        }

        let existing_status: Option<String> = transaction
            .query_row(
                "SELECT status_json FROM runs WHERE id=?1",
                [run_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        match existing_status {
            None => Err(StorageError::RunNotFound(run_id)),
            Some(existing_status) => Err(StorageError::InvalidData(format!(
                "finish_without_score requires a running run, found {}",
                serde_json::from_str::<RunStatus>(&existing_status)
                    .map(|value| format!("{value:?}"))
                    .unwrap_or(existing_status)
            ))),
        }
    }

    pub(crate) fn interrupt_running_after_checkpoint_cleanup(
        &self,
        run_id: Uuid,
    ) -> Result<(), StorageError> {
        self.finish_without_score(run_id, RunStatus::Interrupted)
    }

    pub fn get_run(&self, run_id: Uuid) -> Result<Option<RunRecord>, StorageError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(&format!("{RUN_SELECT_SQL} WHERE id=?1"))?;
        statement
            .query_row([run_id.to_string()], row_to_run)
            .optional()
            .map_err(StorageError::from)
    }

    pub fn is_batch_owned_run(&self, run_id: Uuid) -> Result<bool, StorageError> {
        let count: i64 = self.connection.lock().query_row(
            "SELECT COUNT(*) FROM scan_batch_members WHERE run_id=?1",
            [run_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(count != 0)
    }

    pub fn is_active_batch_member_run(
        &self,
        batch_id: Uuid,
        member_ordinal: u32,
        run_id: Uuid,
    ) -> Result<bool, StorageError> {
        let count: i64 = self.connection.lock().query_row(
            "SELECT COUNT(*)
             FROM scan_batch_members m
             JOIN scan_batches b ON b.id=m.batch_id
             JOIN runs r ON r.id=m.run_id
             WHERE m.batch_id=?1 AND m.ordinal=?2 AND m.run_id=?3
               AND m.status_json=?4 AND r.status_json=?5 AND b.cancel_requested=0",
            params![
                batch_id.to_string(),
                i64::from(member_ordinal),
                run_id.to_string(),
                serde_json::to_string(&BatchMemberStatus::Running)?,
                serde_json::to_string(&RunStatus::Running)?,
            ],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }

    pub fn get_task_results(&self, run_id: Uuid) -> Result<Vec<TaskResult>, StorageError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT run_id,task_id,category_json,outcome_json,score,failure_kind_json,
             duration_ms,answer_rel_path,detail
             FROM task_results WHERE run_id=?1 ORDER BY task_id ASC",
        )?;
        let rows = statement.query_map([run_id.to_string()], row_to_task_result)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn get_run_task_counts(&self, run_id: Uuid) -> Result<Option<(u32, u32)>, StorageError> {
        let counts: Option<(i64, i64)> = self
            .connection
            .lock()
            .query_row(
                "SELECT completed_tasks,total_tasks FROM runs WHERE id=?1",
                [run_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        counts
            .map(|(completed_tasks, total_tasks)| {
                let completed_tasks = u32::try_from(completed_tasks).map_err(|_| {
                    StorageError::InvalidData("stored completed_tasks is out of range".into())
                })?;
                let total_tasks = u32::try_from(total_tasks).map_err(|_| {
                    StorageError::InvalidData("stored total_tasks is out of range".into())
                })?;
                if completed_tasks > total_tasks {
                    return Err(StorageError::InvalidData(
                        "stored completed_tasks exceeds total_tasks".into(),
                    ));
                }
                Ok((completed_tasks, total_tasks))
            })
            .transpose()
    }

    pub fn list_runs(&self) -> Result<Vec<RunRecord>, StorageError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(&format!(
            "{RUN_SELECT_SQL} ORDER BY started_at DESC, id ASC"
        ))?;
        let rows = statement.query_map([], row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    pub fn has_running_runs(&self) -> Result<bool, StorageError> {
        let running = serde_json::to_string(&RunStatus::Running)?;
        let count: i64 = self.connection.lock().query_row(
            "SELECT
               (SELECT COUNT(*) FROM runs WHERE status_json=?1) +
               (SELECT COUNT(*) FROM scan_batch_members
                WHERE status_json IN (?2,?3,?4))",
            params![
                running,
                serde_json::to_string(&BatchMemberStatus::Reserved)?,
                serde_json::to_string(&BatchMemberStatus::Launching)?,
                serde_json::to_string(&BatchMemberStatus::Running)?,
            ],
            |row| row.get(0),
        )?;
        Ok(count != 0)
    }

    pub fn raw_retention_days(&self) -> Result<Option<u32>, StorageError> {
        raw_retention_days_from(&self.connection.lock())
    }

    pub fn set_raw_retention_days(&self, days: Option<u32>) -> Result<(), StorageError> {
        validate_raw_retention_days(days)?;
        self.connection.lock().execute(
            "INSERT INTO settings(key,value_json)
             VALUES ('raw_retention_days',?1)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            [serde_json::to_string(&days)?],
        )?;
        Ok(())
    }

    pub fn retention_candidates(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<RetentionCandidate>, StorageError> {
        let connection = self.connection.lock();
        let Some(days) = raw_retention_days_from(&connection)? else {
            return Ok(Vec::new());
        };
        let cutoff = now - chrono::Duration::days(i64::from(days));
        let completed = serde_json::to_string(&RunStatus::Completed)?;
        let cancelled = serde_json::to_string(&RunStatus::Cancelled)?;
        let mut statement = connection.prepare(
            "SELECT id,target_json,finished_at FROM runs
             WHERE status_json IN (?1,?2) AND finished_at IS NOT NULL",
        )?;
        let rows = statement.query_map(params![completed, cancelled], |row| {
            let id: String = row.get(0)?;
            let target: String = row.get(1)?;
            let finished_at: String = row.get(2)?;
            Ok(RetentionCandidate {
                id: Uuid::parse_str(&id).map_err(to_sql_error)?,
                target: serde_json::from_str(&target).map_err(to_sql_error)?,
                finished_at: DateTime::parse_from_rfc3339(&finished_at)
                    .map_err(to_sql_error)?
                    .with_timezone(&Utc),
            })
        })?;
        let mut candidates = rows.collect::<Result<Vec<_>, _>>()?;
        candidates.retain(|candidate| candidate.finished_at <= cutoff);
        candidates.sort_by(|left, right| {
            left.finished_at
                .cmp(&right.finished_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(candidates)
    }

    pub fn clear_retention_candidate(
        &self,
        candidate: &RetentionCandidate,
        now: DateTime<Utc>,
    ) -> Result<usize, StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let days = raw_retention_days_from(&transaction)?.ok_or_else(|| {
            StorageError::InvalidData("raw retention policy no longer expires data".into())
        })?;
        let current: Option<(String, String, Option<String>)> = transaction
            .query_row(
                "SELECT target_json,status_json,finished_at FROM runs WHERE id=?1",
                [candidate.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (target_json, status_json, finished_at) =
            current.ok_or(StorageError::RunNotFound(candidate.id))?;
        let target: TargetSelection = serde_json::from_str(&target_json)?;
        let status: RunStatus = serde_json::from_str(&status_json)?;
        let finished_at = finished_at
            .ok_or_else(|| StorageError::InvalidData("retention candidate is unfinished".into()))
            .and_then(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .map(|value| value.with_timezone(&Utc))
                    .map_err(StorageError::from)
            })?;
        let cutoff = now - chrono::Duration::days(i64::from(days));
        if target != candidate.target
            || finished_at != candidate.finished_at
            || !matches!(status, RunStatus::Completed | RunStatus::Cancelled)
            || finished_at > cutoff
        {
            return Err(StorageError::InvalidData(
                "retention candidate changed before cleanup".into(),
            ));
        }
        let changed = transaction.execute(
            "UPDATE task_results SET answer_rel_path=NULL
             WHERE run_id=?1 AND answer_rel_path IS NOT NULL",
            [candidate.id.to_string()],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn snapshot_to_backup_file(
        &self,
        snapshot_path: &Path,
    ) -> Result<Vec<BackupRunBinding>, StorageError> {
        let source = self.connection.lock();
        let mut snapshot = Connection::open(snapshot_path)?;
        snapshot.execute_batch("PRAGMA journal_mode=OFF; PRAGMA temp_store=MEMORY;")?;
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut snapshot)?;
            backup.run_to_completion(128, std::time::Duration::from_millis(1), None)?;
        }
        let mut statement = snapshot.prepare("SELECT id,target_json FROM runs ORDER BY id ASC")?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let parsed = Uuid::parse_str(&id).map_err(to_sql_error)?;
            if parsed.to_string() != id {
                return Err(to_sql_error(StorageError::InvalidData(
                    "stored run UUID is not canonical".into(),
                )));
            }
            let target_json: String = row.get(1)?;
            let target =
                serde_json::from_str::<TargetSelection>(&target_json).map_err(to_sql_error)?;
            Ok(BackupRunBinding {
                id: parsed,
                target: target.kind,
            })
        })?;
        let runs = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        Ok(runs)
    }

    pub fn record_publication(
        &self,
        report_id: Uuid,
        run_id: Uuid,
        report_sha256: &str,
        destination_kind: &str,
    ) -> Result<(), StorageError> {
        self.record_publication_at(
            report_id,
            run_id,
            report_sha256,
            destination_kind,
            Utc::now(),
        )
    }

    pub fn record_publication_at(
        &self,
        report_id: Uuid,
        run_id: Uuid,
        report_sha256: &str,
        destination_kind: &str,
        exported_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        if report_sha256.len() != 64
            || !report_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(StorageError::InvalidData(
                "publication report hash must be lowercase SHA-256 hex".into(),
            ));
        }
        if destination_kind != "local_html" {
            return Err(StorageError::InvalidData(
                "publication destination kind is unsupported".into(),
            ));
        }
        self.connection.lock().execute(
            "INSERT INTO publications(
               report_id,run_id,exported_at,report_sha256,destination_kind
             ) VALUES (?1,?2,?3,?4,?5)",
            params![
                report_id.to_string(),
                run_id.to_string(),
                exported_at.to_rfc3339(),
                report_sha256,
                destination_kind,
            ],
        )?;
        Ok(())
    }

    pub fn mark_running_as_interrupted(&self) -> Result<usize, StorageError> {
        self.connection
            .lock()
            .execute(
                "UPDATE runs SET status_json=?1 WHERE status_json=?2",
                params![
                    serde_json::to_string(&RunStatus::Interrupted)?,
                    serde_json::to_string(&RunStatus::Running)?,
                ],
            )
            .map_err(StorageError::from)
    }

    pub fn resume_run<F>(
        &self,
        run_id: Uuid,
        expected_target: &TargetSelection,
        validate: F,
    ) -> Result<RunRecord, StorageError>
    where
        F: FnOnce(&RunRecord, &[TaskResult]) -> Result<(), StorageError>,
    {
        self.resume_run_inner(run_id, expected_target, None, validate)
    }

    pub fn resume_run_retrying_exact_marker<F>(
        &self,
        run_id: Uuid,
        expected_target: &TargetSelection,
        expected_marker: &TaskResult,
        validate: F,
    ) -> Result<RunRecord, StorageError>
    where
        F: FnOnce(&RunRecord, &[TaskResult]) -> Result<(), StorageError>,
    {
        validate_retry_marker(expected_marker)?;
        if expected_marker.run_id != run_id {
            return Err(StorageError::InvalidData(
                "retry marker belongs to a different run".into(),
            ));
        }
        self.resume_run_inner(run_id, expected_target, Some(expected_marker), validate)
    }

    fn resume_run_inner<F>(
        &self,
        run_id: Uuid,
        expected_target: &TargetSelection,
        expected_marker: Option<&TaskResult>,
        validate: F,
    ) -> Result<RunRecord, StorageError>
    where
        F: FnOnce(&RunRecord, &[TaskResult]) -> Result<(), StorageError>,
    {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut run = transaction
            .query_row(
                &format!("{RUN_SELECT_SQL} WHERE id=?1"),
                [run_id.to_string()],
                row_to_run,
            )
            .optional()?
            .ok_or(StorageError::RunNotFound(run_id))?;
        let batch_owned: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM scan_batch_members WHERE run_id=?1",
            [run_id.to_string()],
            |row| row.get(0),
        )?;
        if batch_owned != 0 {
            return Err(StorageError::InvalidData(
                "batch-owned run resume requires member-scoped reauthorization".into(),
            ));
        }
        if run.status != RunStatus::Interrupted {
            return Err(StorageError::InvalidData(
                "run is not an interrupted resumable run".into(),
            ));
        }
        if run.target != *expected_target {
            return Err(StorageError::InvalidData(
                "run target does not match the reviewed recovery target".into(),
            ));
        }
        let stored_results = task_results_in_transaction(&transaction, run_id)?;
        let candidate_results = if let Some(expected_marker) = expected_marker {
            let stored_count = u32::try_from(stored_results.len()).map_err(|_| {
                StorageError::InvalidData("completed task count is too large".into())
            })?;
            if run.completed_tasks != stored_count {
                return Err(StorageError::InvalidData(
                    "persisted completed count does not match stored results".into(),
                ));
            }
            let stored_marker = stored_results
                .iter()
                .find(|result| result.task_id == expected_marker.task_id)
                .ok_or_else(|| {
                    StorageError::InvalidData("retry marker changed before resume".into())
                })?;
            if stored_marker != expected_marker {
                return Err(StorageError::InvalidData(
                    "retry marker changed before resume".into(),
                ));
            }
            stored_results
                .iter()
                .filter(|result| result.task_id != expected_marker.task_id)
                .cloned()
                .collect::<Vec<_>>()
        } else {
            stored_results
        };
        if expected_marker.is_some() {
            run.completed_tasks = u32::try_from(candidate_results.len()).map_err(|_| {
                StorageError::InvalidData("completed task count is too large".into())
            })?;
        }
        validate(&run, &candidate_results)?;

        if let Some(expected_marker) = expected_marker {
            let changed = transaction.execute(
                "DELETE FROM task_results WHERE run_id=?1 AND task_id=?2",
                params![run_id.to_string(), &expected_marker.task_id],
            )?;
            if changed != 1 {
                return Err(StorageError::InvalidData(
                    "retry marker changed before resume".into(),
                ));
            }
        }

        run.status = RunStatus::Running;
        run.finished_at = None;
        run.score = None;
        run.environment.resumed = true;
        let changed = transaction.execute(
            "UPDATE runs
             SET status_json=?2,finished_at=NULL,score_json=NULL,environment_json=?3,
                 completed_tasks=?4
             WHERE id=?1 AND status_json=?5",
            params![
                run_id.to_string(),
                serde_json::to_string(&RunStatus::Running)?,
                serde_json::to_string(&run.environment)?,
                run.completed_tasks,
                serde_json::to_string(&RunStatus::Interrupted)?,
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(
                "run changed while recovery was being validated".into(),
            ));
        }
        transaction.commit()?;
        Ok(run)
    }

    pub fn insert_batch_plan(
        &self,
        batch_id: Uuid,
        pack: &crate::LoadedPack,
        plan: &ScanBatchPlan,
        members: &[BatchMemberSeed],
        created_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        if plan.mode == BatchMode::Full {
            return Err(StorageError::InvalidData(
                "Full batches require atomic baseline snapshot creation".into(),
            ));
        }
        validate_new_batch_plan(plan, members)?;
        validate_batch_plan_against_pack(plan, pack)?;
        if created_at < plan.cost_estimate.issued_at
            || created_at > plan.cost_estimate.initial_acknowledgement_expires_at
        {
            return Err(StorageError::InvalidData(
                "batch creation time is outside the acknowledged estimate window".into(),
            ));
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_batch_rows(&transaction, batch_id, plan, members, created_at)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn create_full_batch_with_baseline_snapshot(
        &self,
        batch_id: Uuid,
        pack: &crate::LoadedPack,
        plan: &ScanBatchPlan,
        members: &[BatchMemberSeed],
        baseline_as_of: DateTime<Utc>,
        policy: &CalibrationPolicy,
    ) -> Result<BaselineSnapshot, StorageError> {
        if plan.mode != BatchMode::Full {
            return Err(StorageError::InvalidData(
                "atomic baseline creation only accepts Full batches".into(),
            ));
        }
        validate_new_batch_plan(plan, members)?;
        validate_batch_plan_against_pack(plan, pack)?;
        if baseline_as_of < plan.cost_estimate.issued_at
            || baseline_as_of > plan.cost_estimate.initial_acknowledgement_expires_at
        {
            return Err(StorageError::InvalidData(
                "batch creation time is outside the acknowledged estimate window".into(),
            ));
        }
        let candidate_identity = BatchAnalysisIdentity::from_plan(plan)
            .map_err(|error| StorageError::InvalidData(error.to_string()))?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = {
            let mut statement = transaction.prepare(
                "SELECT id,mode_json,status_json,updated_at
                 FROM scan_batches ORDER BY updated_at DESC,id ASC",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut evidence = Vec::with_capacity(rows.len());
        for (stored_id, mode_json, status_json, updated_at) in rows {
            let evidence_id = Uuid::parse_str(&stored_id)
                .map_err(|_| StorageError::InvalidData("stored batch id is invalid".into()))?;
            match load_batch(&transaction, evidence_id) {
                Ok(Some(batch)) => evidence.push(BaselineEvidenceCandidate {
                    batch_id: evidence_id,
                    mode: batch.plan.mode,
                    status: batch.status,
                    finished_at: batch.updated_at,
                    identity: BatchAnalysisIdentity::from_plan(&batch.plan)
                        .map_err(|error| StorageError::InvalidData(error.to_string()))?,
                    has_valid_snapshot: batch.baseline_snapshot.is_some(),
                }),
                Ok(None) | Err(_) => {
                    // Preserve the exact id in the frozen exclusion list, but never let a
                    // malformed stored row borrow the candidate's compatible identity.
                    let mode = serde_json::from_str::<BatchMode>(&mode_json)
                        .unwrap_or(BatchMode::QuickComparison);
                    let status = serde_json::from_str::<BatchStatus>(&status_json)
                        .unwrap_or(BatchStatus::Interrupted);
                    let finished_at = DateTime::parse_from_rfc3339(&updated_at)
                        .map_err(StorageError::Time)?
                        .with_timezone(&Utc);
                    evidence.push(BaselineEvidenceCandidate {
                        batch_id: evidence_id,
                        mode,
                        status,
                        finished_at,
                        identity: candidate_identity.clone(),
                        has_valid_snapshot: false,
                    });
                }
            }
        }
        let snapshot = BaselineSnapshot::freeze(batch_id, plan, baseline_as_of, policy, &evidence)
            .map_err(|error| StorageError::InvalidData(error.to_string()))?;
        insert_batch_rows(&transaction, batch_id, plan, members, baseline_as_of)?;
        transaction.execute(
            "INSERT INTO baseline_snapshots(
               candidate_batch_id,baseline_as_of,snapshot_json,content_sha256,created_at
             ) VALUES (?1,?2,?3,?4,?2)",
            params![
                batch_id.to_string(),
                baseline_as_of.to_rfc3339(),
                serde_json::to_string(&snapshot)?,
                &snapshot.content_sha256,
            ],
        )?;
        transaction.commit()?;
        Ok(snapshot)
    }

    pub fn get_baseline_snapshot(
        &self,
        batch_id: Uuid,
    ) -> Result<Option<BaselineSnapshot>, StorageError> {
        Ok(
            load_batch(&self.connection.lock(), batch_id)?
                .and_then(|batch| batch.baseline_snapshot),
        )
    }

    pub fn analyze_batch(
        &self,
        batch_id: Uuid,
        policy: &CalibrationPolicy,
    ) -> Result<BatchAnalysis, StorageError> {
        let connection = self.connection.lock();
        let batch = load_batch(&connection, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        let candidate_members = load_member_evidence(&connection, &batch)?;
        let mut historical = Vec::new();
        if let Some(snapshot) = batch.baseline_snapshot.as_ref() {
            for evidence_id in &snapshot.selected_batch_ids {
                let evidence_batch = load_batch(&connection, *evidence_id)?.ok_or_else(|| {
                    StorageError::InvalidData(
                        "frozen baseline evidence is no longer available".into(),
                    )
                })?;
                if evidence_batch.plan.mode != BatchMode::Full
                    || evidence_batch.status != BatchStatus::Completed
                    || evidence_batch.updated_at >= snapshot.baseline_as_of
                {
                    return Err(StorageError::InvalidData(
                        "frozen baseline evidence no longer satisfies its cutoff".into(),
                    ));
                }
                historical.push(CompletedBatchEvidence {
                    batch_id: evidence_batch.id,
                    finished_at: evidence_batch.updated_at,
                    members: load_member_evidence(&connection, &evidence_batch)?,
                });
            }
        }
        analyze_matched_batch(
            batch.plan.mode,
            batch.id,
            &candidate_members,
            batch.baseline_snapshot.as_ref(),
            &historical,
            policy,
        )
        .map_err(|error| StorageError::InvalidData(error.to_string()))
    }

    pub fn get_batch(&self, batch_id: Uuid) -> Result<Option<ScanBatchRecord>, StorageError> {
        load_batch(&self.connection.lock(), batch_id)
    }

    pub fn list_batches(&self) -> Result<Vec<ScanBatchRecord>, StorageError> {
        let connection = self.connection.lock();
        let mut statement =
            connection.prepare("SELECT id FROM scan_batches ORDER BY created_at DESC,id ASC")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        ids.into_iter()
            .map(|id| {
                let id = Uuid::parse_str(&id)
                    .map_err(|_| StorageError::InvalidData("stored batch id is invalid".into()))?;
                load_batch(&connection, id)?.ok_or_else(|| {
                    StorageError::InvalidData("stored batch disappeared while listing".into())
                })
            })
            .collect()
    }

    pub fn append_execution_authorization(
        &self,
        authorization: &ScanExecutionAuthorization,
    ) -> Result<(), StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch = load_batch(&transaction, authorization.batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        if batch.cancel_requested {
            return Err(StorageError::InvalidData(
                "cancelled batch cannot receive new execution authorization".into(),
            ));
        }
        if authorization.created_at < batch.created_at {
            return Err(StorageError::InvalidData(
                "execution authorization predates batch creation".into(),
            ));
        }
        let plan = batch.plan;
        validate_authorization_against_plan(authorization, &plan)?;
        let member_scope = authorization.member_ordinal.map(i64::from).unwrap_or(-1);
        if let Some(member_ordinal) = authorization.member_ordinal {
            let row: (String, Option<String>, i64, String) = transaction
                .query_row(
                    "SELECT status_json,failure_kind_json,attempt_number,updated_at
                     FROM scan_batch_members WHERE batch_id=?1 AND ordinal=?2",
                    params![
                        authorization.batch_id.to_string(),
                        i64::from(member_ordinal)
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?
                .ok_or_else(|| StorageError::InvalidData("batch member does not exist".into()))?;
            let status: BatchMemberStatus = serde_json::from_str(&row.0)?;
            let failure = row
                .1
                .map(|value| serde_json::from_str::<FailureKind>(&value))
                .transpose()?;
            let attempt = u32::try_from(row.2).map_err(|_| {
                StorageError::InvalidData("stored member attempt is invalid".into())
            })?;
            let member_updated_at = DateTime::parse_from_rfc3339(&row.3)
                .map_err(StorageError::Time)?
                .with_timezone(&Utc);
            if status != BatchMemberStatus::Deferred
                || authorization.allowed_failure_kind != failure
                || !failure.is_some_and(is_retryable_batch_failure)
                || authorization.attempt_number != attempt.saturating_add(1)
                || authorization.created_at < member_updated_at
            {
                return Err(StorageError::InvalidData(
                    "member retry authorization does not match durable deferred state".into(),
                ));
            }
        }
        transaction.execute(
            "INSERT INTO scan_execution_authorizations(
               batch_id,member_scope,attempt_number,max_provider_turns,
               max_task_budget_secs,acknowledgement_hash,allowed_failure_kind_json,
               expires_at,created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                authorization.batch_id.to_string(),
                member_scope,
                i64::from(authorization.attempt_number),
                to_sqlite_u64(authorization.max_provider_turns, "authorization turns")?,
                to_sqlite_u64(authorization.max_task_budget_secs, "authorization time")?,
                &authorization.acknowledgement_hash,
                authorization
                    .allowed_failure_kind
                    .map(|value| serde_json::to_string(&value))
                    .transpose()?,
                authorization.expires_at.to_rfc3339(),
                authorization.created_at.to_rfc3339(),
            ],
        )?;
        if let Some(member_ordinal) = authorization.member_ordinal {
            let changed = transaction.execute(
                "UPDATE scan_batch_members
                 SET status_json=?3,failure_kind_json=NULL,attempt_number=?4,updated_at=?5
                 WHERE batch_id=?1 AND ordinal=?2 AND status_json=?6",
                params![
                    authorization.batch_id.to_string(),
                    i64::from(member_ordinal),
                    serde_json::to_string(&BatchMemberStatus::Planned)?,
                    i64::from(authorization.attempt_number),
                    authorization.created_at.to_rfc3339(),
                    serde_json::to_string(&BatchMemberStatus::Deferred)?,
                ],
            )?;
            if changed != 1 {
                return Err(StorageError::InvalidData(
                    "deferred member changed while authorization was appended".into(),
                ));
            }
        }
        update_batch_status_in_transaction(
            &transaction,
            authorization.batch_id,
            authorization.created_at,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn reserve_next_runnable_member_and_run(
        &self,
        batch_id: Uuid,
        now: DateTime<Utc>,
        run: &RunRecord,
    ) -> Result<Option<BatchReservation>, StorageError> {
        validate_run(run)?;
        if run.status != RunStatus::Created
            || run.completed_tasks != 0
            || run.finished_at.is_some()
            || run.score.is_some()
        {
            return Err(StorageError::InvalidData(
                "a reserved batch run must be a fresh created run".into(),
            ));
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch = load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        if batch.cancel_requested {
            return Err(StorageError::InvalidData(
                "cancelled batch cannot reserve new members".into(),
            ));
        }
        let active_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM scan_batch_members
             WHERE batch_id=?1 AND status_json IN (?2,?3,?4)",
            params![
                batch_id.to_string(),
                serde_json::to_string(&BatchMemberStatus::Reserved)?,
                serde_json::to_string(&BatchMemberStatus::Launching)?,
                serde_json::to_string(&BatchMemberStatus::Running)?,
            ],
            |row| row.get(0),
        )?;
        if active_count != 0 {
            return Err(StorageError::InvalidData(
                "batch already has an active member".into(),
            ));
        }
        let mut candidate = None;
        for member in &batch.members {
            if member.status == BatchMemberStatus::Planned
                && active_member_authorization(&transaction, &batch.plan, batch_id, member, now)?
            {
                candidate = Some(member.clone());
                break;
            }
        }
        let Some(candidate) = candidate else {
            update_batch_status_in_transaction(&transaction, batch_id, now)?;
            transaction.commit()?;
            return Ok(None);
        };
        let ordinal = i64::from(candidate.ordinal);
        let target_position = i64::from(candidate.target_position);
        let repetition_index = i64::from(candidate.repetition_index);
        let existing_run = candidate.run_id;
        let attempt = i64::from(candidate.attempt_number);
        let plan = batch.plan;
        let target_position_usize = usize::try_from(target_position)
            .map_err(|_| StorageError::InvalidData("stored target position is invalid".into()))?;
        validate_reserved_run(&plan, target_position_usize, run)?;
        let reserved_run = if let Some(existing_run) = existing_run {
            if existing_run != run.id {
                return Err(StorageError::InvalidData(
                    "reconciled member must reuse its preallocated run id".into(),
                ));
            }
            let stored_run = run_in_transaction(&transaction, run.id)?
                .ok_or(StorageError::RunNotFound(run.id))?;
            validate_reserved_run(&plan, target_position_usize, &stored_run)?;
            if !matches!(
                stored_run.status,
                RunStatus::Created | RunStatus::Interrupted
            ) {
                return Err(StorageError::InvalidData(
                    "preallocated batch run is already terminal".into(),
                ));
            }
            stored_run
        } else {
            insert_run_in_transaction(&transaction, run)?;
            run.clone()
        };
        let planned_json = serde_json::to_string(&BatchMemberStatus::Planned)?;
        let next_attempt = if attempt == 0 { 1 } else { attempt };
        let changed = transaction.execute(
            "UPDATE scan_batch_members
             SET run_id=?4,status_json=?5,attempt_number=?6,updated_at=?7
             WHERE batch_id=?1 AND ordinal=?2 AND status_json=?3",
            params![
                batch_id.to_string(),
                ordinal,
                &planned_json,
                run.id.to_string(),
                serde_json::to_string(&BatchMemberStatus::Reserved)?,
                next_attempt,
                now.to_rfc3339(),
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(
                "batch member changed while it was being reserved".into(),
            ));
        }
        update_batch_status_in_transaction(&transaction, batch_id, now)?;
        let member = ScanBatchMemberRecord {
            ordinal: u32::try_from(ordinal)
                .map_err(|_| StorageError::InvalidData("stored ordinal is invalid".into()))?,
            target_position: u32::try_from(target_position).map_err(|_| {
                StorageError::InvalidData("stored target position is invalid".into())
            })?,
            repetition_index: u32::try_from(repetition_index).map_err(|_| {
                StorageError::InvalidData("stored repetition index is invalid".into())
            })?,
            run_id: Some(run.id),
            status: BatchMemberStatus::Reserved,
            failure_kind: None,
            attempt_number: u32::try_from(next_attempt)
                .map_err(|_| StorageError::InvalidData("stored attempt is invalid".into()))?,
            updated_at: now,
        };
        transaction.commit()?;
        Ok(Some(BatchReservation {
            batch_id,
            member,
            run: reserved_run,
        }))
    }

    pub fn mark_member_launching(
        &self,
        batch_id: Uuid,
        ordinal: u32,
        run_id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch = load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        if batch.cancel_requested {
            return Err(StorageError::InvalidData(
                "cancelled batch cannot cross the provider boundary".into(),
            ));
        }
        let member = require_member_state(
            &transaction,
            batch_id,
            ordinal,
            Some(run_id),
            BatchMemberStatus::Reserved,
        )?;
        if !active_member_authorization(&transaction, &batch.plan, batch_id, &member, at)? {
            return Err(StorageError::InvalidData(
                "reserved member execution authorization is no longer active".into(),
            ));
        }
        let run =
            run_in_transaction(&transaction, run_id)?.ok_or(StorageError::RunNotFound(run_id))?;
        if !matches!(run.status, RunStatus::Created | RunStatus::Interrupted) {
            return Err(StorageError::InvalidData(
                "reserved member run is not launchable".into(),
            ));
        }
        update_member_state(
            &transaction,
            batch_id,
            ordinal,
            BatchMemberStatus::Reserved,
            BatchMemberStatus::Launching,
            None,
            at,
        )?;
        update_batch_status_in_transaction(&transaction, batch_id, at)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn mark_member_running(
        &self,
        batch_id: Uuid,
        ordinal: u32,
        run_id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        self.mark_member_running_retrying_exact_marker(batch_id, ordinal, run_id, None, at)
    }

    pub fn mark_member_running_retrying_exact_marker(
        &self,
        batch_id: Uuid,
        ordinal: u32,
        run_id: Uuid,
        expected_marker: Option<&TaskResult>,
        at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        if let Some(marker) = expected_marker {
            validate_retry_marker(marker)?;
            if marker.run_id != run_id {
                return Err(StorageError::InvalidData(
                    "retry marker belongs to a different batch run".into(),
                ));
            }
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch = load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        let member = require_member_state(
            &transaction,
            batch_id,
            ordinal,
            Some(run_id),
            BatchMemberStatus::Launching,
        )?;
        if !active_member_authorization(&transaction, &batch.plan, batch_id, &member, at)? {
            return Err(StorageError::InvalidData(
                "launching member execution authorization is no longer active".into(),
            ));
        }
        let run =
            run_in_transaction(&transaction, run_id)?.ok_or(StorageError::RunNotFound(run_id))?;
        if !matches!(run.status, RunStatus::Created | RunStatus::Interrupted) {
            return Err(StorageError::InvalidData(
                "launching member run is not startable".into(),
            ));
        }
        let mut completed_tasks = run.completed_tasks;
        if let Some(expected_marker) = expected_marker {
            if run.status != RunStatus::Interrupted || member.attempt_number < 2 {
                return Err(StorageError::InvalidData(
                    "retry marker requires an explicitly reauthorized interrupted member".into(),
                ));
            }
            let authorization = authorization_in_transaction(
                &transaction,
                batch_id,
                i64::from(ordinal),
                member.attempt_number,
            )?
            .ok_or_else(|| {
                StorageError::InvalidData(
                    "retry marker has no exact member-scoped authorization".into(),
                )
            })?;
            if authorization.allowed_failure_kind != expected_marker.failure_kind {
                return Err(StorageError::InvalidData(
                    "retry marker no longer matches the authorized durable failure".into(),
                ));
            }
            let stored_marker = task_results_in_transaction(&transaction, run_id)?
                .into_iter()
                .find(|result| result.task_id == expected_marker.task_id)
                .ok_or_else(|| {
                    StorageError::InvalidData("retry marker changed before batch resume".into())
                })?;
            if stored_marker != *expected_marker {
                return Err(StorageError::InvalidData(
                    "retry marker changed before batch resume".into(),
                ));
            }
            let changed = transaction.execute(
                "DELETE FROM task_results WHERE run_id=?1 AND task_id=?2",
                params![run_id.to_string(), &expected_marker.task_id],
            )?;
            if changed != 1 {
                return Err(StorageError::InvalidData(
                    "retry marker changed before batch resume".into(),
                ));
            }
            completed_tasks = completed_tasks.checked_sub(1).ok_or_else(|| {
                StorageError::InvalidData("retry marker count is inconsistent".into())
            })?;
        } else if run.status == RunStatus::Created && run.completed_tasks != 0 {
            return Err(StorageError::InvalidData(
                "fresh batch run contains unexpected checkpoints".into(),
            ));
        }
        let mut environment = run.environment;
        if run.status == RunStatus::Interrupted {
            environment.resumed = true;
        }
        let changed = transaction.execute(
            "UPDATE runs SET status_json=?2,finished_at=NULL,score_json=NULL,environment_json=?3,
                             completed_tasks=?4
             WHERE id=?1 AND status_json=?5",
            params![
                run_id.to_string(),
                serde_json::to_string(&RunStatus::Running)?,
                serde_json::to_string(&environment)?,
                i64::from(completed_tasks),
                serde_json::to_string(&run.status)?,
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::InvalidData(
                "run changed while member entered running state".into(),
            ));
        }
        update_member_state(
            &transaction,
            batch_id,
            ordinal,
            BatchMemberStatus::Launching,
            BatchMemberStatus::Running,
            None,
            at,
        )?;
        update_batch_status_in_transaction(&transaction, batch_id, at)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn defer_batch_member(
        &self,
        batch_id: Uuid,
        ordinal: u32,
        run_id: Option<Uuid>,
        failure_kind: FailureKind,
        at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        if !is_retryable_batch_failure(failure_kind) {
            return Err(StorageError::InvalidData(
                "failure class is not eligible for explicit batch resume".into(),
            ));
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch = load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        if batch.cancel_requested {
            return Err(StorageError::InvalidData(
                "cancelled batch members must be terminalized, not deferred".into(),
            ));
        }
        let member = member_in_transaction(&transaction, batch_id, ordinal)?
            .ok_or_else(|| StorageError::InvalidData("batch member does not exist".into()))?;
        if !matches!(
            member.status,
            BatchMemberStatus::Planned
                | BatchMemberStatus::Reserved
                | BatchMemberStatus::Launching
                | BatchMemberStatus::Running
        ) || member.run_id != run_id
        {
            return Err(StorageError::InvalidData(
                "only the exact runnable or active member can be deferred".into(),
            ));
        }
        let mut effective_failure_kind = failure_kind;
        if let Some(run_id) = run_id {
            let run = run_in_transaction(&transaction, run_id)?
                .ok_or(StorageError::RunNotFound(run_id))?;
            if let Some(marker_failure) =
                durable_retry_failure_in_transaction(&transaction, run_id)?
            {
                effective_failure_kind = marker_failure;
            }
            if matches!(run.status, RunStatus::Created | RunStatus::Running) {
                transaction.execute(
                    "UPDATE runs SET status_json=?2,finished_at=?3,score_json=NULL
                     WHERE id=?1 AND status_json=?4",
                    params![
                        run_id.to_string(),
                        serde_json::to_string(&RunStatus::Interrupted)?,
                        at.to_rfc3339(),
                        serde_json::to_string(&run.status)?,
                    ],
                )?;
            } else if run.status != RunStatus::Interrupted {
                return Err(StorageError::InvalidData(
                    "terminal run cannot be changed into deferred work".into(),
                ));
            }
        }
        update_member_state(
            &transaction,
            batch_id,
            ordinal,
            member.status,
            BatchMemberStatus::Deferred,
            Some(effective_failure_kind),
            at,
        )?;
        update_batch_status_in_transaction(&transaction, batch_id, at)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn finish_batch_member(
        &self,
        batch_id: Uuid,
        ordinal: u32,
        run_id: Uuid,
        terminal_status: BatchMemberStatus,
        failure_kind: Option<FailureKind>,
        at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        if !terminal_status.is_terminal() {
            return Err(StorageError::InvalidData(
                "batch member finish requires a terminal status".into(),
            ));
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        let member = member_in_transaction(&transaction, batch_id, ordinal)?
            .ok_or_else(|| StorageError::InvalidData("batch member does not exist".into()))?;
        if member.run_id != Some(run_id)
            || !matches!(
                member.status,
                BatchMemberStatus::Reserved
                    | BatchMemberStatus::Launching
                    | BatchMemberStatus::Running
                    | BatchMemberStatus::Deferred
            )
        {
            return Err(StorageError::InvalidData(
                "completed or foreign member cannot be rebound or rerun".into(),
            ));
        }
        let run =
            run_in_transaction(&transaction, run_id)?.ok_or(StorageError::RunNotFound(run_id))?;
        let coherent = match terminal_status {
            BatchMemberStatus::Completed => {
                run.status == RunStatus::Completed && failure_kind.is_none()
            }
            BatchMemberStatus::Cancelled => {
                let terminalized = if matches!(
                    run.status,
                    RunStatus::Created | RunStatus::Running | RunStatus::Interrupted
                ) {
                    transaction.execute(
                        "UPDATE runs SET status_json=?2,finished_at=?3,score_json=NULL
                         WHERE id=?1 AND status_json=?4",
                        params![
                            run_id.to_string(),
                            serde_json::to_string(&RunStatus::Cancelled)?,
                            at.to_rfc3339(),
                            serde_json::to_string(&run.status)?,
                        ],
                    )? == 1
                } else {
                    run.status == RunStatus::Cancelled
                };
                terminalized && failure_kind == Some(FailureKind::UserCancelled)
            }
            BatchMemberStatus::Invalid | BatchMemberStatus::Unavailable => {
                let terminalized = if matches!(run.status, RunStatus::Created | RunStatus::Running)
                {
                    transaction.execute(
                        "UPDATE runs SET status_json=?2,finished_at=?3,score_json=NULL
                         WHERE id=?1 AND status_json=?4",
                        params![
                            run_id.to_string(),
                            serde_json::to_string(&RunStatus::Interrupted)?,
                            at.to_rfc3339(),
                            serde_json::to_string(&run.status)?,
                        ],
                    )? == 1
                } else {
                    matches!(run.status, RunStatus::Interrupted | RunStatus::Cancelled)
                };
                terminalized && failure_kind.is_some()
            }
            _ => false,
        };
        if !coherent {
            return Err(StorageError::InvalidData(
                "terminal member state does not match canonical run evidence".into(),
            ));
        }
        update_member_state(
            &transaction,
            batch_id,
            ordinal,
            member.status,
            terminal_status,
            failure_kind,
            at,
        )?;
        update_batch_status_in_transaction(&transaction, batch_id, at)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_guided_task_result_with_isolation(
        &self,
        batch_id: Uuid,
        ordinal: u32,
        result: &TaskResult,
        attestation: &IsolationAttestation,
    ) -> Result<(), StorageError> {
        validate_task_result(result)?;
        if attestation.policy_version != 1
            || attestation.enforcement != IsolationEnforcement::UserAttested
            || !attestation.user_attested
        {
            return Err(StorageError::InvalidData(
                "guided checkpoint requires positive policy-v1 user attestation".into(),
            ));
        }
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let member = require_member_state(
            &transaction,
            batch_id,
            ordinal,
            Some(result.run_id),
            BatchMemberStatus::Running,
        )?;
        if attestation.recorded_at < member.updated_at {
            return Err(StorageError::InvalidData(
                "isolation attestation predates the active member state".into(),
            ));
        }
        let batch = load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        if batch.cancel_requested {
            return Err(StorageError::InvalidData(
                "cancelled batch cannot accept new task evidence".into(),
            ));
        }
        let plan = batch.plan;
        if reviewed_task_category(&plan, &result.task_id) != Some(result.category) {
            return Err(StorageError::InvalidData(
                "task id/category is not owned by the batch sealed pack".into(),
            ));
        }
        let surface_json: String = transaction.query_row(
            "SELECT t.execution_surface_json
             FROM scan_batch_members m
             JOIN scan_batch_targets t
               ON t.batch_id=m.batch_id AND t.position=m.target_position
             WHERE m.batch_id=?1 AND m.ordinal=?2",
            params![batch_id.to_string(), i64::from(ordinal)],
            |row| row.get(0),
        )?;
        let surface: crate::BatchExecutionSurface = serde_json::from_str(&surface_json)?;
        if surface != crate::BatchExecutionSurface::GuidedClient {
            return Err(StorageError::InvalidData(
                "user-attested checkpoint is valid only for guided members".into(),
            ));
        }
        if !active_member_authorization(
            &transaction,
            &plan,
            batch_id,
            &member,
            attestation.recorded_at,
        )? {
            return Err(StorageError::InvalidData(
                "guided checkpoint has no active execution authorization".into(),
            ));
        }

        transaction.execute(
            "INSERT INTO scan_batch_task_isolation(
               batch_id,member_ordinal,run_id,task_id,policy_version,
               enforcement_json,user_attested,recorded_at
             ) VALUES (?1,?2,?3,?4,?5,?6,1,?7)",
            params![
                batch_id.to_string(),
                i64::from(ordinal),
                result.run_id.to_string(),
                &result.task_id,
                i64::from(attestation.policy_version),
                serde_json::to_string(&attestation.enforcement)?,
                attestation.recorded_at.to_rfc3339(),
            ],
        )?;
        save_task_result_in_transaction(&transaction, result)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_cli_batch_task_result(
        &self,
        batch_id: Uuid,
        ordinal: u32,
        result: &TaskResult,
    ) -> Result<(), StorageError> {
        validate_task_result(result)?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let member = require_member_state(
            &transaction,
            batch_id,
            ordinal,
            Some(result.run_id),
            BatchMemberStatus::Running,
        )?;
        let batch = load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        if batch.cancel_requested {
            return Err(StorageError::InvalidData(
                "cancelled batch cannot accept new CLI task evidence".into(),
            ));
        }
        if batch.plan.session_isolation_policy
            != crate::SessionIsolationPolicy::MachineEnforcedFreshSessionAndWorkspacePerTask
            || batch.plan.task_session_policy_version != 1
        {
            return Err(StorageError::InvalidData(
                "CLI checkpoint requires the reviewed machine-isolation policy".into(),
            ));
        }
        let surface_json: String = transaction.query_row(
            "SELECT t.execution_surface_json
             FROM scan_batch_members m
             JOIN scan_batch_targets t
               ON t.batch_id=m.batch_id AND t.position=m.target_position
             WHERE m.batch_id=?1 AND m.ordinal=?2",
            params![batch_id.to_string(), i64::from(ordinal)],
            |row| row.get(0),
        )?;
        let surface: crate::BatchExecutionSurface = serde_json::from_str(&surface_json)?;
        if surface != crate::BatchExecutionSurface::AutomatedCli {
            return Err(StorageError::InvalidData(
                "machine-isolated CLI checkpoint belongs to a guided member".into(),
            ));
        }
        if reviewed_task_category(&batch.plan, &result.task_id) != Some(result.category) {
            return Err(StorageError::InvalidData(
                "task id/category is not owned by the batch sealed pack".into(),
            ));
        }
        if !active_member_authorization(&transaction, &batch.plan, batch_id, &member, Utc::now())? {
            return Err(StorageError::InvalidData(
                "CLI checkpoint has no active execution authorization".into(),
            ));
        }
        save_task_result_in_transaction(&transaction, result)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn reconcile_batches_after_startup(
        &self,
        now: DateTime<Utc>,
    ) -> Result<usize, StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch_ids = {
            let mut statement = transaction.prepare("SELECT id FROM scan_batches ORDER BY id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut changed_members = 0_usize;
        for batch_id_text in batch_ids {
            let batch_id = Uuid::parse_str(&batch_id_text)
                .map_err(|_| StorageError::InvalidData("stored batch id is invalid".into()))?;
            let plan = validate_batch_immutable_identity(&transaction, batch_id)?;
            let cancel_requested: i64 = transaction.query_row(
                "SELECT cancel_requested FROM scan_batches WHERE id=?1",
                [batch_id.to_string()],
                |row| row.get(0),
            )?;
            let members = batch_members_in_transaction(&transaction, batch_id)?;
            let seeds = members
                .iter()
                .map(|member| BatchMemberSeed {
                    ordinal: member.ordinal,
                    target_position: member.target_position,
                    repetition_index: member.repetition_index,
                })
                .collect::<Vec<_>>();
            validate_new_batch_plan(&plan, &seeds)?;
            for member in members {
                let Some(run_id) = member.run_id else {
                    continue;
                };
                let run = run_in_transaction(&transaction, run_id)?
                    .ok_or(StorageError::RunNotFound(run_id))?;
                if cancel_requested != 0 && !member.status.is_terminal() {
                    let (next, failure) = if run.status == RunStatus::Completed {
                        (BatchMemberStatus::Completed, None)
                    } else {
                        if run.status != RunStatus::Cancelled {
                            let changed = transaction.execute(
                                "UPDATE runs SET status_json=?2,finished_at=?3,score_json=NULL
                                 WHERE id=?1 AND status_json=?4",
                                params![
                                    run_id.to_string(),
                                    serde_json::to_string(&RunStatus::Cancelled)?,
                                    now.to_rfc3339(),
                                    serde_json::to_string(&run.status)?,
                                ],
                            )?;
                            if changed != 1 {
                                return Err(StorageError::InvalidData(
                                    "cancelled run changed during startup reconciliation".into(),
                                ));
                            }
                        }
                        (
                            BatchMemberStatus::Cancelled,
                            Some(FailureKind::UserCancelled),
                        )
                    };
                    update_member_state(
                        &transaction,
                        batch_id,
                        member.ordinal,
                        member.status,
                        next,
                        failure,
                        now,
                    )?;
                    changed_members = changed_members.saturating_add(1);
                    continue;
                }
                let mut next = member.status;
                let mut failure = member.failure_kind;
                match (member.status, run.status) {
                    (BatchMemberStatus::Reserved, RunStatus::Created) => {
                        if active_member_authorization(&transaction, &plan, batch_id, &member, now)?
                        {
                            next = BatchMemberStatus::Planned;
                            failure = None;
                        } else {
                            next = BatchMemberStatus::Deferred;
                            failure = Some(FailureKind::AuthExpired);
                        }
                    }
                    (
                        BatchMemberStatus::Launching | BatchMemberStatus::Running,
                        RunStatus::Created | RunStatus::Running,
                    ) => {
                        next = BatchMemberStatus::Deferred;
                        failure = Some(FailureKind::AppInterrupted);
                        interrupt_run_in_transaction(&transaction, run_id, run.status, now)?;
                    }
                    (status, RunStatus::Completed) if status != BatchMemberStatus::Completed => {
                        next = BatchMemberStatus::Completed;
                        failure = None;
                    }
                    (status, RunStatus::Cancelled) if status != BatchMemberStatus::Cancelled => {
                        next = BatchMemberStatus::Cancelled;
                        failure = Some(FailureKind::UserCancelled);
                    }
                    (status, RunStatus::Interrupted)
                        if !matches!(status, BatchMemberStatus::Deferred) =>
                    {
                        next = BatchMemberStatus::Deferred;
                        failure = Some(FailureKind::AppInterrupted);
                    }
                    (status, RunStatus::Created | RunStatus::Running) if status.is_terminal() => {
                        next = BatchMemberStatus::Invalid;
                        failure = Some(FailureKind::AppInterrupted);
                        interrupt_run_in_transaction(&transaction, run_id, run.status, now)?;
                    }
                    _ => {}
                }
                if next == BatchMemberStatus::Deferred
                    && let Some(marker_failure) =
                        durable_retry_failure_in_transaction(&transaction, run_id)?
                {
                    failure = Some(marker_failure);
                }
                if next != member.status || failure != member.failure_kind {
                    update_member_state(
                        &transaction,
                        batch_id,
                        member.ordinal,
                        member.status,
                        next,
                        failure,
                        now,
                    )?;
                    changed_members = changed_members.saturating_add(1);
                }
            }
            update_batch_status_in_transaction(&transaction, batch_id, now)?;
        }
        transaction.commit()?;
        Ok(changed_members)
    }

    pub fn derive_batch_status(
        &self,
        batch_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<BatchStatus, StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        let status = update_batch_status_in_transaction(&transaction, batch_id, now)?;
        transaction.commit()?;
        Ok(status)
    }

    pub fn pause_batch(&self, batch_id: Uuid, now: DateTime<Utc>) -> Result<(), StorageError> {
        let status = self.derive_batch_status(batch_id, now)?;
        if status == BatchStatus::Paused {
            Ok(())
        } else {
            Err(StorageError::InvalidData(
                "batch can be paused only when all remaining members are deferred".into(),
            ))
        }
    }

    pub fn resume_batch(&self, batch_id: Uuid, now: DateTime<Utc>) -> Result<(), StorageError> {
        let status = self.derive_batch_status(batch_id, now)?;
        if status == BatchStatus::Running {
            Ok(())
        } else {
            Err(StorageError::InvalidData(
                "batch resume requires a newly authorized runnable member".into(),
            ))
        }
    }

    pub fn cancel_batch(&self, batch_id: Uuid, now: DateTime<Utc>) -> Result<(), StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        load_batch(&transaction, batch_id)?
            .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
        transaction.execute(
            "UPDATE scan_batches SET cancel_requested=1,updated_at=?2 WHERE id=?1",
            params![batch_id.to_string(), now.to_rfc3339()],
        )?;
        let members = batch_members_in_transaction(&transaction, batch_id)?;
        for member in members {
            if matches!(
                member.status,
                BatchMemberStatus::Planned
                    | BatchMemberStatus::Reserved
                    | BatchMemberStatus::Deferred
            ) {
                if let Some(run_id) = member.run_id {
                    let run = run_in_transaction(&transaction, run_id)?
                        .ok_or(StorageError::RunNotFound(run_id))?;
                    if matches!(run.status, RunStatus::Created | RunStatus::Interrupted) {
                        transaction.execute(
                            "UPDATE runs SET status_json=?2,finished_at=?3,score_json=NULL
                             WHERE id=?1 AND status_json=?4",
                            params![
                                run_id.to_string(),
                                serde_json::to_string(&RunStatus::Cancelled)?,
                                now.to_rfc3339(),
                                serde_json::to_string(&run.status)?,
                            ],
                        )?;
                    }
                }
                update_member_state(
                    &transaction,
                    batch_id,
                    member.ordinal,
                    member.status,
                    BatchMemberStatus::Cancelled,
                    Some(FailureKind::UserCancelled),
                    now,
                )?;
            }
        }
        update_batch_status_in_transaction(&transaction, batch_id, now)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn delete_batch(&self, batch_id: Uuid) -> Result<bool, StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let member_count: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT COUNT(*),SUM(CASE WHEN run_id IS NOT NULL THEN 1 ELSE 0 END)
                 FROM scan_batch_members WHERE batch_id=?1",
                [batch_id.to_string()],
                |row| Ok((row.get(0)?, row.get::<_, Option<i64>>(1)?.unwrap_or(0))),
            )
            .optional()?;
        let Some((_members, owned_runs)) = member_count else {
            transaction.commit()?;
            return Ok(false);
        };
        let exists: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM scan_batches WHERE id=?1",
            [batch_id.to_string()],
            |row| row.get(0),
        )?;
        if exists == 0 {
            transaction.commit()?;
            return Ok(false);
        }
        if owned_runs != 0 {
            return Err(StorageError::InvalidData(
                "batch with owned runs requires the recoverable data-lifecycle delete path".into(),
            ));
        }
        let changed = transaction.execute(
            "DELETE FROM scan_batches WHERE id=?1",
            [batch_id.to_string()],
        )?;
        clean_orphan_identities(&transaction)?;
        transaction.commit()?;
        checkpoint_after_delete(&connection)?;
        Ok(changed == 1)
    }

    pub fn clear_artifact_references(&self, run_id: Uuid) -> Result<usize, StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let status = run_status_in_transaction(&transaction, run_id)?
            .ok_or(StorageError::RunNotFound(run_id))?;
        reject_active_delete(status)?;
        let changed = transaction.execute(
            "UPDATE task_results SET answer_rel_path=NULL
             WHERE run_id=?1 AND answer_rel_path IS NOT NULL",
            [run_id.to_string()],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn delete_run(&self, run_id: Uuid) -> Result<bool, StorageError> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(status) = run_status_in_transaction(&transaction, run_id)? else {
            transaction.commit()?;
            return Ok(false);
        };
        reject_active_delete(status)?;
        let changed = transaction.execute("DELETE FROM runs WHERE id=?1", [run_id.to_string()])?;
        clean_orphan_identities(&transaction)?;
        transaction.commit()?;
        checkpoint_after_delete(&connection)?;
        Ok(changed == 1)
    }

    pub fn delete_target_history(
        &self,
        target: TargetKind,
        expected_run_ids: &[Uuid],
    ) -> Result<u32, StorageError> {
        let mut expected = expected_run_ids.to_vec();
        expected.sort_unstable();
        if expected.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(StorageError::InvalidData(
                "reviewed run snapshot contains duplicates".into(),
            ));
        }

        let mut connection = self.connection.lock();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut statement =
            transaction.prepare("SELECT id,target_json,status_json FROM runs ORDER BY id ASC")?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let target_json: String = row.get(1)?;
            let status_json: String = row.get(2)?;
            Ok((
                Uuid::parse_str(&id).map_err(to_sql_error)?,
                serde_json::from_str::<TargetSelection>(&target_json).map_err(to_sql_error)?,
                serde_json::from_str::<RunStatus>(&status_json).map_err(to_sql_error)?,
            ))
        })?;
        let records = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let current = records
            .iter()
            .filter(|(_, selection, _)| selection.kind == target)
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>();
        if current != expected {
            return Err(StorageError::InvalidData(
                "target history changed after confirmation".into(),
            ));
        }
        if records.iter().any(|(id, _, status)| {
            expected.binary_search(id).is_ok() && *status == RunStatus::Running
        }) {
            return Err(StorageError::InvalidData(
                "active runs cannot be deleted".into(),
            ));
        }
        for run_id in &expected {
            transaction.execute("DELETE FROM runs WHERE id=?1", [run_id.to_string()])?;
        }
        clean_orphan_identities(&transaction)?;
        transaction.commit()?;
        checkpoint_after_delete(&connection)?;
        u32::try_from(expected.len())
            .map_err(|_| StorageError::InvalidData("delete count exceeds supported range".into()))
    }
}

const RUN_SELECT_SQL: &str = "SELECT id,target_json,mode_json,suite_id,suite_version,status_json,
    started_at,finished_at,total_tasks,completed_tasks,environment_json,score_json FROM runs";

fn validate_new_batch_plan(
    plan: &ScanBatchPlan,
    members: &[BatchMemberSeed],
) -> Result<(), StorageError> {
    let (validated_repetitions, _) = plan
        .validated_schedule_contract()
        .map_err(|error| StorageError::InvalidData(error.to_string()))?;
    if plan.status != BatchStatus::Created
        || plan.targets.is_empty()
        || plan.cost_estimate.target_count
            != u64::try_from(plan.targets.len()).map_err(|_| {
                StorageError::InvalidData("batch target count exceeds supported range".into())
            })?
        || plan.cost_estimate.planned_member_runs
            != u64::try_from(members.len()).map_err(|_| {
                StorageError::InvalidData("batch member count exceeds supported range".into())
            })?
        || !is_lower_sha256(&plan.suite_content_sha256)
        || !is_lower_sha256(&plan.acknowledgement_hash)
    {
        return Err(StorageError::InvalidData(
            "batch plan shape or immutable identity is invalid".into(),
        ));
    }
    validate_plan_acknowledgement_hash(plan)?;
    for target in &plan.targets {
        target
            .validate_for_new_batch()
            .map_err(|error| StorageError::InvalidData(error.to_string()))?;
    }
    if !(2..=5).contains(&plan.targets.len()) || members.len() > 25 {
        return Err(StorageError::InvalidData(
            "batch target or member count exceeds the hard policy cap".into(),
        ));
    }
    let repetitions = validated_repetitions;
    let mut pairs = std::collections::BTreeSet::new();
    for (expected_ordinal, member) in members.iter().enumerate() {
        if usize::try_from(member.ordinal).ok() != Some(expected_ordinal)
            || usize::try_from(member.target_position)
                .ok()
                .is_none_or(|position| position >= plan.targets.len())
            || member.repetition_index >= repetitions
            || !pairs.insert((member.target_position, member.repetition_index))
        {
            return Err(StorageError::InvalidData(
                "batch member schedule is not a complete unique ordinal mapping".into(),
            ));
        }
    }
    let expected_pairs = plan
        .targets
        .iter()
        .enumerate()
        .flat_map(|(position, _)| {
            (0..repetitions).map(move |repetition| {
                (
                    u32::try_from(position).expect("batch target cap fits u32"),
                    repetition,
                )
            })
        })
        .collect::<std::collections::BTreeSet<_>>();
    if pairs != expected_pairs {
        return Err(StorageError::InvalidData(
            "batch member schedule does not cover every target repetition exactly once".into(),
        ));
    }
    Ok(())
}

fn validate_plan_acknowledgement_hash(plan: &ScanBatchPlan) -> Result<(), StorageError> {
    plan.validate_acknowledgement_hash()
        .map_err(|error| StorageError::InvalidData(error.to_string()))
}

fn validate_batch_plan_against_pack(
    plan: &ScanBatchPlan,
    pack: &crate::LoadedPack,
) -> Result<(), StorageError> {
    if plan.suite_id != pack.manifest.id
        || plan.suite_version != pack.manifest.version
        || plan.suite_content_sha256 != pack.content_sha256
        || plan
            .targets
            .iter()
            .any(|target| !pack.manifest.target_kinds.contains(&target.target.kind))
    {
        return Err(StorageError::InvalidData(
            "batch plan suite or target identity does not match the verified pack".into(),
        ));
    }
    plan.cost_estimate
        .validate_against_pack(pack)
        .map_err(|error| StorageError::InvalidData(error.to_string()))?;
    let pack_budgets = pack
        .tasks
        .iter()
        .map(|task| crate::SealedTaskBudget {
            max_turns: u64::from(task.definition.max_turns),
            time_budget_secs: task.definition.time_budget_secs,
        })
        .collect::<Vec<_>>();
    if pack_budgets != plan.sealed_task_budgets {
        return Err(StorageError::InvalidData(
            "batch plan budgets do not match the verified pack task order".into(),
        ));
    }
    Ok(())
}

fn validate_authorization_shape(
    authorization: &ScanExecutionAuthorization,
) -> Result<(), StorageError> {
    if authorization.attempt_number == 0
        || authorization.expires_at <= authorization.created_at
        || !is_lower_sha256(&authorization.acknowledgement_hash)
    {
        return Err(StorageError::InvalidData(
            "execution authorization shape is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_authorization_against_plan(
    authorization: &ScanExecutionAuthorization,
    plan: &ScanBatchPlan,
) -> Result<(), StorageError> {
    validate_authorization_shape(authorization)?;
    validate_plan_acknowledgement_hash(plan)?;
    if authorization.created_at < plan.cost_estimate.issued_at {
        return Err(StorageError::InvalidData(
            "execution authorization predates the immutable estimate".into(),
        ));
    }
    let latest_expiry = plan
        .cost_estimate
        .execution_authorization_expires_at(authorization.created_at)
        .map_err(|error| StorageError::InvalidData(error.to_string()))?;
    if authorization.expires_at > latest_expiry {
        return Err(StorageError::InvalidData(
            "execution authorization exceeds the policy wall-clock window".into(),
        ));
    }
    let planned_runs = plan.cost_estimate.planned_member_runs;
    if planned_runs == 0 {
        return Err(StorageError::InvalidData(
            "batch plan has no planned members".into(),
        ));
    }
    let batch_scope = authorization.member_ordinal.is_none();
    let (turn_limit, time_limit) = if batch_scope {
        if authorization.attempt_number != 1
            || authorization.allowed_failure_kind.is_some()
            || authorization.acknowledgement_hash != plan.acknowledgement_hash
            || authorization.created_at > plan.cost_estimate.initial_acknowledgement_expires_at
            || authorization.expires_at != latest_expiry
        {
            return Err(StorageError::InvalidData(
                "initial batch authorization does not match the immutable plan".into(),
            ));
        }
        (
            plan.cost_estimate.max_provider_turns,
            plan.cost_estimate.summed_task_budget_secs,
        )
    } else {
        let expected_hash = authorization.expected_retry_acknowledgement_hash(plan)?;
        if authorization.acknowledgement_hash != expected_hash {
            return Err(StorageError::InvalidData(
                "retry acknowledgement does not match its exact scope and budget".into(),
            ));
        }
        (
            plan.cost_estimate.max_provider_turns / planned_runs,
            plan.cost_estimate.summed_task_budget_secs / planned_runs,
        )
    };
    if authorization.max_provider_turns == 0
        || authorization.max_task_budget_secs == 0
        || authorization.max_provider_turns > turn_limit
        || authorization.max_task_budget_secs > time_limit
        || (batch_scope
            && (authorization.max_provider_turns != turn_limit
                || authorization.max_task_budget_secs != time_limit))
    {
        return Err(StorageError::InvalidData(
            "execution authorization exceeds the reviewed remaining budget".into(),
        ));
    }
    Ok(())
}

fn authorization_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    member_scope: i64,
    attempt_number: u32,
) -> Result<Option<ScanExecutionAuthorization>, StorageError> {
    type StoredAuthorization = (i64, i64, String, Option<String>, String, String);
    let row: Option<StoredAuthorization> = transaction
        .query_row(
            "SELECT max_provider_turns,max_task_budget_secs,acknowledgement_hash,
                    allowed_failure_kind_json,expires_at,created_at
             FROM scan_execution_authorizations
             WHERE batch_id=?1 AND member_scope=?2 AND attempt_number=?3",
            params![
                batch_id.to_string(),
                member_scope,
                i64::from(attempt_number)
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((turns, task_secs, hash, failure_json, expires_at, created_at)) = row else {
        return Ok(None);
    };
    let member_ordinal = if member_scope == -1 {
        None
    } else {
        Some(u32::try_from(member_scope).map_err(|_| {
            StorageError::InvalidData("stored authorization member scope is invalid".into())
        })?)
    };
    Ok(Some(ScanExecutionAuthorization {
        batch_id,
        member_ordinal,
        attempt_number,
        max_provider_turns: u64::try_from(turns).map_err(|_| {
            StorageError::InvalidData("stored authorization turn budget is invalid".into())
        })?,
        max_task_budget_secs: u64::try_from(task_secs).map_err(|_| {
            StorageError::InvalidData("stored authorization time budget is invalid".into())
        })?,
        acknowledgement_hash: hash,
        allowed_failure_kind: failure_json
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        expires_at: DateTime::parse_from_rfc3339(&expires_at)
            .map_err(StorageError::Time)?
            .with_timezone(&Utc),
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map_err(StorageError::Time)?
            .with_timezone(&Utc),
    }))
}

fn active_member_authorization(
    transaction: &rusqlite::Transaction<'_>,
    plan: &ScanBatchPlan,
    batch_id: Uuid,
    member: &ScanBatchMemberRecord,
    at: DateTime<Utc>,
) -> Result<bool, StorageError> {
    Ok(matches!(
        member_authorization_state(transaction, plan, batch_id, member, at)?,
        MemberAuthorizationState::Active
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberAuthorizationState {
    Missing,
    Pending,
    Active,
    Expired,
}

fn member_authorization_state(
    transaction: &rusqlite::Transaction<'_>,
    plan: &ScanBatchPlan,
    batch_id: Uuid,
    member: &ScanBatchMemberRecord,
    at: DateTime<Utc>,
) -> Result<MemberAuthorizationState, StorageError> {
    let effective_attempt = member.attempt_number.max(1);
    let member_authorization = if member.attempt_number == 0 {
        None
    } else {
        authorization_in_transaction(
            transaction,
            batch_id,
            i64::from(member.ordinal),
            effective_attempt,
        )?
    };
    let authorization = if let Some(authorization) = member_authorization {
        Some(authorization)
    } else if effective_attempt == 1 {
        authorization_in_transaction(transaction, batch_id, -1, 1)?
    } else {
        None
    };
    let Some(authorization) = authorization else {
        return Ok(MemberAuthorizationState::Missing);
    };
    validate_authorization_against_plan(&authorization, plan)?;
    if authorization.created_at > at {
        Ok(MemberAuthorizationState::Pending)
    } else if authorization.expires_at <= at {
        Ok(MemberAuthorizationState::Expired)
    } else {
        Ok(MemberAuthorizationState::Active)
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_retryable_batch_failure(value: FailureKind) -> bool {
    matches!(
        value,
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

fn reviewed_task_category(plan: &ScanBatchPlan, task_id: &str) -> Option<crate::Category> {
    match (plan.suite_id.as_str(), plan.suite_version.as_str(), task_id) {
        (
            "client-quick",
            "1.0.0",
            "instruction-filter" | "instruction-csv" | "instruction-inventory",
        ) => Some(crate::Category::InstructionFollowing),
        ("client-quick", "1.0.0", "logic-schedule" | "logic-truth" | "logic-capacity") => {
            Some(crate::Category::Logic)
        }
        ("client-quick", "1.0.0", "review-python" | "review-typescript") => {
            Some(crate::Category::CodeReview)
        }
        ("cli-quick", "1.0.0", "dedupe-events" | "retry-schedule") => {
            Some(crate::Category::CliCoding)
        }
        _ => None,
    }
}

fn to_sqlite_u64(value: u64, label: &str) -> Result<i64, StorageError> {
    i64::try_from(value)
        .map_err(|_| StorageError::InvalidData(format!("{label} exceeds SQLite range")))
}

fn validate_reserved_run(
    plan: &ScanBatchPlan,
    target_position: usize,
    run: &RunRecord,
) -> Result<(), StorageError> {
    let target = plan.targets.get(target_position).ok_or_else(|| {
        StorageError::InvalidData("batch member target position is invalid".into())
    })?;
    let expected_mode = match plan.mode {
        crate::BatchMode::QuickComparison => RunMode::Quick,
        crate::BatchMode::Standard | crate::BatchMode::Full => RunMode::Deep,
    };
    let expected_total = u32::try_from(plan.sealed_task_budgets.len())
        .map_err(|_| StorageError::InvalidData("batch task count is out of range".into()))?;
    if run.target != target.target
        || run.mode != expected_mode
        || run.suite_id != plan.suite_id
        || run.suite_version != plan.suite_version
        || run.total_tasks != expected_total
        || run.environment.suite_id != plan.suite_id
        || run.environment.suite_version != plan.suite_version
        || run.environment.suite_content_sha256 != plan.suite_content_sha256
        || run.environment.scoring_rule_version != plan.scoring_rule_version
        || run.environment.execution_adapter_identity.as_ref()
            != Some(&target.execution_adapter_identity)
    {
        return Err(StorageError::InvalidData(
            "preallocated run does not match its immutable batch target and suite".into(),
        ));
    }
    Ok(())
}

fn insert_run_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    run: &RunRecord,
) -> Result<(), StorageError> {
    validate_run(run)?;
    let target_json = serde_json::to_string(&run.target)?;
    transaction.execute(
        "INSERT OR IGNORE INTO targets(target_json) VALUES (?1)",
        [&target_json],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO suite_versions(
           suite_id,suite_version,content_sha256,scoring_rule_version
         ) VALUES (?1,?2,?3,?4)",
        params![
            &run.suite_id,
            &run.suite_version,
            &run.environment.suite_content_sha256,
            &run.environment.scoring_rule_version,
        ],
    )?;
    let suite: (String, String) = transaction.query_row(
        "SELECT content_sha256,scoring_rule_version FROM suite_versions
         WHERE suite_id=?1 AND suite_version=?2",
        params![&run.suite_id, &run.suite_version],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if suite
        != (
            run.environment.suite_content_sha256.clone(),
            run.environment.scoring_rule_version.clone(),
        )
    {
        return Err(StorageError::InvalidData(
            "run suite identity conflicts with persisted batch suite".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO runs(
          id,target_json,mode_json,suite_id,suite_version,status_json,started_at,
          finished_at,total_tasks,completed_tasks,environment_json,score_json
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,?8,0,?9,NULL)",
        params![
            run.id.to_string(),
            target_json,
            serde_json::to_string(&run.mode)?,
            &run.suite_id,
            &run.suite_version,
            serde_json::to_string(&run.status)?,
            run.started_at.to_rfc3339(),
            i64::from(run.total_tasks),
            serde_json::to_string(&run.environment)?,
        ],
    )?;
    Ok(())
}

fn run_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    run_id: Uuid,
) -> Result<Option<RunRecord>, StorageError> {
    transaction
        .query_row(
            &format!("{RUN_SELECT_SQL} WHERE id=?1"),
            [run_id.to_string()],
            row_to_run,
        )
        .optional()
        .map_err(StorageError::from)
}

fn member_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    ordinal: u32,
) -> Result<Option<ScanBatchMemberRecord>, StorageError> {
    transaction
        .query_row(
            "SELECT ordinal,target_position,repetition_index,run_id,status_json,
                    failure_kind_json,attempt_number,updated_at
             FROM scan_batch_members WHERE batch_id=?1 AND ordinal=?2",
            params![batch_id.to_string(), i64::from(ordinal)],
            row_to_batch_member,
        )
        .optional()
        .map_err(StorageError::from)
}

fn batch_members_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
) -> Result<Vec<ScanBatchMemberRecord>, StorageError> {
    let mut statement = transaction.prepare(
        "SELECT ordinal,target_position,repetition_index,run_id,status_json,
                failure_kind_json,attempt_number,updated_at
         FROM scan_batch_members WHERE batch_id=?1 ORDER BY ordinal ASC",
    )?;
    let rows = statement.query_map([batch_id.to_string()], row_to_batch_member)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
}

fn row_to_batch_member(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScanBatchMemberRecord> {
    let run_id: Option<String> = row.get(3)?;
    let status: String = row.get(4)?;
    let failure: Option<String> = row.get(5)?;
    let updated_at: String = row.get(7)?;
    Ok(ScanBatchMemberRecord {
        ordinal: u32::try_from(row.get::<_, i64>(0)?).map_err(to_sql_error)?,
        target_position: u32::try_from(row.get::<_, i64>(1)?).map_err(to_sql_error)?,
        repetition_index: u32::try_from(row.get::<_, i64>(2)?).map_err(to_sql_error)?,
        run_id: run_id
            .map(|value| Uuid::parse_str(&value).map_err(to_sql_error))
            .transpose()?,
        status: serde_json::from_str(&status).map_err(to_sql_error)?,
        failure_kind: failure
            .map(|value| serde_json::from_str(&value).map_err(to_sql_error))
            .transpose()?,
        attempt_number: u32::try_from(row.get::<_, i64>(6)?).map_err(to_sql_error)?,
        updated_at: DateTime::parse_from_rfc3339(&updated_at)
            .map_err(to_sql_error)?
            .with_timezone(&Utc),
    })
}

fn require_member_state(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    ordinal: u32,
    run_id: Option<Uuid>,
    expected: BatchMemberStatus,
) -> Result<ScanBatchMemberRecord, StorageError> {
    let member = member_in_transaction(transaction, batch_id, ordinal)?
        .ok_or_else(|| StorageError::InvalidData("batch member does not exist".into()))?;
    if member.status != expected || member.run_id != run_id {
        return Err(StorageError::InvalidData(
            "batch member state or run ownership changed".into(),
        ));
    }
    Ok(member)
}

fn update_member_state(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    ordinal: u32,
    from: BatchMemberStatus,
    to: BatchMemberStatus,
    failure: Option<FailureKind>,
    at: DateTime<Utc>,
) -> Result<(), StorageError> {
    let changed = transaction.execute(
        "UPDATE scan_batch_members
         SET status_json=?3,failure_kind_json=?4,updated_at=?5
         WHERE batch_id=?1 AND ordinal=?2 AND status_json=?6",
        params![
            batch_id.to_string(),
            i64::from(ordinal),
            serde_json::to_string(&to)?,
            failure
                .map(|value| serde_json::to_string(&value))
                .transpose()?,
            at.to_rfc3339(),
            serde_json::to_string(&from)?,
        ],
    )?;
    if changed != 1 {
        return Err(StorageError::InvalidData(
            "batch member changed during state transition".into(),
        ));
    }
    Ok(())
}

fn interrupt_run_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    run_id: Uuid,
    from: RunStatus,
    at: DateTime<Utc>,
) -> Result<(), StorageError> {
    let changed = transaction.execute(
        "UPDATE runs SET status_json=?2,finished_at=?3,score_json=NULL
         WHERE id=?1 AND status_json=?4",
        params![
            run_id.to_string(),
            serde_json::to_string(&RunStatus::Interrupted)?,
            at.to_rfc3339(),
            serde_json::to_string(&from)?,
        ],
    )?;
    if changed != 1 {
        return Err(StorageError::InvalidData(
            "run changed during startup reconciliation".into(),
        ));
    }
    Ok(())
}

fn save_task_result_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    result: &TaskResult,
) -> Result<(), StorageError> {
    let (status_json, total_tasks): (String, i64) = transaction
        .query_row(
            "SELECT status_json,total_tasks FROM runs WHERE id=?1",
            [result.run_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(StorageError::RunNotFound(result.run_id))?;
    if serde_json::from_str::<RunStatus>(&status_json)? != RunStatus::Running {
        return Err(StorageError::InvalidData(
            "task results can be saved only while the run is running".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO task_results(
          run_id,task_id,category_json,outcome_json,score,failure_kind_json,
          duration_ms,answer_rel_path,detail
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
        ON CONFLICT(run_id,task_id) DO UPDATE SET
          category_json=excluded.category_json,
          outcome_json=excluded.outcome_json,score=excluded.score,
          failure_kind_json=excluded.failure_kind_json,duration_ms=excluded.duration_ms,
          answer_rel_path=excluded.answer_rel_path,detail=excluded.detail",
        params![
            result.run_id.to_string(),
            &result.task_id,
            serde_json::to_string(&result.category)?,
            serde_json::to_string(&result.outcome)?,
            result.score,
            result
                .failure_kind
                .map(|value| serde_json::to_string(&value))
                .transpose()?,
            to_sqlite_u64(result.duration_ms, "duration_ms")?,
            &result.answer_rel_path,
            &result.detail,
        ],
    )?;
    let checkpoint_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM task_results WHERE run_id=?1",
        [result.run_id.to_string()],
        |row| row.get(0),
    )?;
    if checkpoint_count > total_tasks {
        return Err(StorageError::InvalidData(
            "checkpoint count exceeds the run total_tasks".into(),
        ));
    }
    let changed = transaction.execute(
        "UPDATE runs SET completed_tasks=?2 WHERE id=?1 AND status_json=?3",
        params![
            result.run_id.to_string(),
            checkpoint_count,
            serde_json::to_string(&RunStatus::Running)?,
        ],
    )?;
    if changed != 1 {
        return Err(StorageError::InvalidData(
            "run changed while checkpoint evidence was saved".into(),
        ));
    }
    Ok(())
}

fn derive_batch_status_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(BatchStatus, u32), StorageError> {
    let cancel_requested: i64 = transaction
        .query_row(
            "SELECT cancel_requested FROM scan_batches WHERE id=?1",
            [batch_id.to_string()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
    let plan = validate_batch_immutable_identity(transaction, batch_id)?;
    let mut members = batch_members_in_transaction(transaction, batch_id)?;
    if members.is_empty() {
        return Err(StorageError::InvalidData(
            "stored batch has no members".into(),
        ));
    }
    for member in &mut members {
        if member.status == BatchMemberStatus::Planned {
            match member_authorization_state(transaction, &plan, batch_id, member, now)? {
                MemberAuthorizationState::Expired => {
                    update_member_state(
                        transaction,
                        batch_id,
                        member.ordinal,
                        BatchMemberStatus::Planned,
                        BatchMemberStatus::Deferred,
                        Some(FailureKind::AuthExpired),
                        now,
                    )?;
                    member.status = BatchMemberStatus::Deferred;
                    member.failure_kind = Some(FailureKind::AuthExpired);
                    member.updated_at = now;
                }
                MemberAuthorizationState::Missing if member.attempt_number != 0 => {
                    return Err(StorageError::InvalidData(
                        "planned resumed member has no matching execution authorization".into(),
                    ));
                }
                _ => {}
            }
        }
    }
    let active = members
        .iter()
        .filter(|member| member.status.is_active())
        .count();
    let terminal = members
        .iter()
        .filter(|member| member.status.is_terminal())
        .count();
    let deferred = members
        .iter()
        .filter(|member| member.status == BatchMemberStatus::Deferred)
        .count();
    let startup_interrupted = members.iter().any(|member| {
        member.status == BatchMemberStatus::Deferred
            && member.failure_kind == Some(FailureKind::AppInterrupted)
    });
    let mut runnable = 0_usize;
    for member in &members {
        if member.status == BatchMemberStatus::Planned
            && active_member_authorization(transaction, &plan, batch_id, member, now)?
        {
            runnable = runnable.saturating_add(1);
        }
    }
    let status = if cancel_requested != 0 {
        if active != 0 {
            BatchStatus::Running
        } else if terminal == members.len() {
            BatchStatus::Cancelled
        } else {
            return Err(StorageError::InvalidData(
                "cancelled batch contains non-terminal inactive members".into(),
            ));
        }
    } else if active != 0 || runnable != 0 {
        BatchStatus::Running
    } else if terminal == members.len() {
        BatchStatus::Completed
    } else if startup_interrupted {
        BatchStatus::Interrupted
    } else if deferred != 0 {
        BatchStatus::Paused
    } else {
        BatchStatus::Created
    };
    Ok((
        status,
        u32::try_from(terminal)
            .map_err(|_| StorageError::InvalidData("terminal count is too large".into()))?,
    ))
}

fn update_batch_status_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    now: DateTime<Utc>,
) -> Result<BatchStatus, StorageError> {
    let (status, terminal_count) = derive_batch_status_in_transaction(transaction, batch_id, now)?;
    let changed = transaction.execute(
        "UPDATE scan_batches SET status_json=?2,terminal_member_count=?3,updated_at=?4
         WHERE id=?1",
        params![
            batch_id.to_string(),
            serde_json::to_string(&status)?,
            i64::from(terminal_count),
            now.to_rfc3339(),
        ],
    )?;
    if changed != 1 {
        return Err(StorageError::InvalidData(
            "batch disappeared during status derivation".into(),
        ));
    }
    Ok(status)
}

fn validate_stored_batch_lifecycle(
    status: BatchStatus,
    cancel_requested: bool,
    members: &[ScanBatchMemberRecord],
) -> Result<(), StorageError> {
    let active = members.iter().any(|member| member.status.is_active());
    let all_terminal = members.iter().all(|member| member.status.is_terminal());
    let has_deferred = members
        .iter()
        .any(|member| member.status == BatchMemberStatus::Deferred);
    let has_startup_interruption = members.iter().any(|member| {
        member.status == BatchMemberStatus::Deferred
            && member.failure_kind == Some(FailureKind::AppInterrupted)
    });
    let coherent = match status {
        BatchStatus::Created => members
            .iter()
            .all(|member| member.status == BatchMemberStatus::Planned && member.run_id.is_none()),
        BatchStatus::Running => {
            active
                || members
                    .iter()
                    .any(|member| member.status == BatchMemberStatus::Planned)
        }
        BatchStatus::Paused => !active && has_deferred && !has_startup_interruption,
        BatchStatus::Interrupted => !active && has_startup_interruption,
        BatchStatus::Completed => !cancel_requested && all_terminal,
        BatchStatus::Cancelled => cancel_requested && !active && all_terminal,
    };
    if !coherent {
        return Err(StorageError::InvalidData(
            "stored batch lifecycle does not match member state".into(),
        ));
    }
    Ok(())
}

fn validate_batch_immutable_identity(
    connection: &Connection,
    batch_id: Uuid,
) -> Result<ScanBatchPlan, StorageError> {
    let row: StoredBatchIdentityRow = connection
        .query_row(
            "SELECT plan_json,mode_json,suite_id,suite_version,content_sha256,
                    scoring_rule_version,seed,acknowledgement_hash,
                    acknowledgement_expires_at
             FROM scan_batches WHERE id=?1",
            [batch_id.to_string()],
            |row| {
                Ok(StoredBatchIdentityRow {
                    plan_json: row.get(0)?,
                    mode_json: row.get(1)?,
                    suite_id: row.get(2)?,
                    suite_version: row.get(3)?,
                    content_sha256: row.get(4)?,
                    scoring_rule_version: row.get(5)?,
                    seed: row.get(6)?,
                    acknowledgement_hash: row.get(7)?,
                    acknowledgement_expires_at: row.get(8)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StorageError::InvalidData("batch does not exist".into()))?;
    let plan: ScanBatchPlan = serde_json::from_str(&row.plan_json)?;
    validate_plan_acknowledgement_hash(&plan)?;
    for target in &plan.targets {
        target
            .validate_for_new_batch()
            .map_err(|error| StorageError::InvalidData(error.to_string()))?;
    }
    let mode: crate::BatchMode = serde_json::from_str(&row.mode_json)?;
    let seed = u64::try_from(row.seed)
        .map_err(|_| StorageError::InvalidData("stored batch seed is invalid".into()))?;
    let acknowledgement_expiry = DateTime::parse_from_rfc3339(&row.acknowledgement_expires_at)
        .map_err(StorageError::Time)?
        .with_timezone(&Utc);
    if mode != plan.mode
        || row.suite_id != plan.suite_id
        || row.suite_version != plan.suite_version
        || row.content_sha256 != plan.suite_content_sha256
        || row.scoring_rule_version != plan.scoring_rule_version
        || seed != plan.seed
        || row.acknowledgement_hash != plan.acknowledgement_hash
        || acknowledgement_expiry != plan.cost_estimate.initial_acknowledgement_expires_at
    {
        return Err(StorageError::InvalidData(
            "indexed batch identity does not match immutable plan JSON".into(),
        ));
    }
    let mut target_statement = connection.prepare(
        "SELECT position,target_json,route_identity_json,adapter_identity_json,
                execution_surface_json
         FROM scan_batch_targets WHERE batch_id=?1 ORDER BY position ASC",
    )?;
    let target_rows = target_statement
        .query_map([batch_id.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if target_rows.len() != plan.targets.len() {
        return Err(StorageError::InvalidData(
            "persisted batch target count does not match the plan".into(),
        ));
    }
    for (position, row) in target_rows.iter().enumerate() {
        let target = &plan.targets[position];
        if usize::try_from(row.0).ok() != Some(position)
            || serde_json::from_str::<TargetSelection>(&row.1)? != target.target
            || serde_json::from_str::<crate::TargetRouteIdentity>(&row.2)? != target.route_identity
            || serde_json::from_str::<crate::ExecutionAdapterIdentity>(&row.3)?
                != target.execution_adapter_identity
            || serde_json::from_str::<crate::BatchExecutionSurface>(&row.4)?
                != target.route_identity.execution_surface
        {
            return Err(StorageError::InvalidData(
                "persisted batch target identity does not match the plan".into(),
            ));
        }
    }
    Ok(plan)
}

fn insert_batch_rows(
    transaction: &rusqlite::Transaction<'_>,
    batch_id: Uuid,
    plan: &ScanBatchPlan,
    members: &[BatchMemberSeed],
    created_at: DateTime<Utc>,
) -> Result<(), StorageError> {
    let seed = i64::try_from(plan.seed)
        .map_err(|_| StorageError::InvalidData("batch seed exceeds SQLite range".into()))?;
    let planned_member_count = i64::try_from(members.len())
        .map_err(|_| StorageError::InvalidData("batch member count exceeds SQLite range".into()))?;
    let plan_json = serde_json::to_string(plan)?;
    let mode_json = serde_json::to_string(&plan.mode)?;
    let status_json = serde_json::to_string(&BatchStatus::Created)?;
    let batch_id_text = batch_id.to_string();

    transaction.execute(
        "INSERT OR IGNORE INTO suite_versions(
           suite_id,suite_version,content_sha256,scoring_rule_version
         ) VALUES (?1,?2,?3,?4)",
        params![
            &plan.suite_id,
            &plan.suite_version,
            &plan.suite_content_sha256,
            &plan.scoring_rule_version,
        ],
    )?;
    let stored_suite: (String, String) = transaction.query_row(
        "SELECT content_sha256,scoring_rule_version FROM suite_versions
         WHERE suite_id=?1 AND suite_version=?2",
        params![&plan.suite_id, &plan.suite_version],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if stored_suite
        != (
            plan.suite_content_sha256.clone(),
            plan.scoring_rule_version.clone(),
        )
    {
        return Err(StorageError::InvalidData(
            "suite id/version is already bound to different content or scoring".into(),
        ));
    }

    transaction.execute(
        "INSERT INTO scan_batches(
           id,plan_json,mode_json,suite_id,suite_version,content_sha256,
           scoring_rule_version,seed,status_json,acknowledgement_hash,
           acknowledgement_expires_at,planned_member_count,terminal_member_count,
           cancel_requested,created_at,updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,0,0,?13,?13)",
        params![
            &batch_id_text,
            &plan_json,
            &mode_json,
            &plan.suite_id,
            &plan.suite_version,
            &plan.suite_content_sha256,
            &plan.scoring_rule_version,
            seed,
            &status_json,
            &plan.acknowledgement_hash,
            plan.cost_estimate
                .initial_acknowledgement_expires_at
                .to_rfc3339(),
            planned_member_count,
            created_at.to_rfc3339(),
        ],
    )?;

    for (position, target) in plan.targets.iter().enumerate() {
        let target_json = serde_json::to_string(&target.target)?;
        transaction.execute(
            "INSERT OR IGNORE INTO targets(target_json) VALUES (?1)",
            [&target_json],
        )?;
        transaction.execute(
            "INSERT INTO scan_batch_targets(
               batch_id,position,target_json,route_identity_json,
               adapter_identity_json,execution_surface_json
             ) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                &batch_id_text,
                i64::try_from(position).map_err(|_| StorageError::InvalidData(
                    "batch target position exceeds SQLite range".into()
                ))?,
                target_json,
                serde_json::to_string(&target.route_identity)?,
                serde_json::to_string(&target.execution_adapter_identity)?,
                serde_json::to_string(&target.route_identity.execution_surface)?,
            ],
        )?;
    }

    let planned_json = serde_json::to_string(&BatchMemberStatus::Planned)?;
    for member in members {
        transaction.execute(
            "INSERT INTO scan_batch_members(
               batch_id,ordinal,target_position,repetition_index,run_id,
               status_json,failure_kind_json,attempt_number,updated_at
             ) VALUES (?1,?2,?3,?4,NULL,?5,NULL,0,?6)",
            params![
                &batch_id_text,
                i64::from(member.ordinal),
                i64::from(member.target_position),
                i64::from(member.repetition_index),
                &planned_json,
                created_at.to_rfc3339(),
            ],
        )?;
    }
    Ok(())
}

fn load_valid_baseline_snapshot(
    connection: &Connection,
    batch_id: Uuid,
) -> Result<Option<BaselineSnapshot>, StorageError> {
    let rows = {
        let mut statement = connection.prepare(
            "SELECT baseline_as_of,snapshot_json,content_sha256,created_at
             FROM baseline_snapshots WHERE candidate_batch_id=?1",
        )?;
        statement
            .query_map([batch_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return Err(StorageError::InvalidData(
            "Full batch must have exactly one baseline snapshot".into(),
        ));
    }
    let (baseline_as_of, snapshot_json, content_sha256, created_at) = &rows[0];
    let snapshot: BaselineSnapshot = serde_json::from_str(snapshot_json)?;
    snapshot
        .validate()
        .map_err(|error| StorageError::InvalidData(error.to_string()))?;
    let indexed_cutoff = DateTime::parse_from_rfc3339(baseline_as_of)
        .map_err(StorageError::Time)?
        .with_timezone(&Utc);
    let indexed_created = DateTime::parse_from_rfc3339(created_at)
        .map_err(StorageError::Time)?
        .with_timezone(&Utc);
    if snapshot.candidate_batch_id != batch_id
        || snapshot.baseline_as_of != indexed_cutoff
        || indexed_created != indexed_cutoff
        || snapshot.content_sha256 != *content_sha256
    {
        return Err(StorageError::InvalidData(
            "indexed baseline snapshot does not match immutable snapshot JSON".into(),
        ));
    }
    Ok(Some(snapshot))
}

fn load_member_evidence(
    connection: &Connection,
    batch: &ScanBatchRecord,
) -> Result<Vec<MemberEvidence>, StorageError> {
    let mut evidence = Vec::with_capacity(batch.members.len());
    for member in &batch.members {
        let Some(run_id) = member.run_id else {
            evidence.push(MemberEvidence {
                member_ordinal: member.ordinal,
                target_position: member.target_position,
                status: member.status,
                run_status: None,
                score: None,
                task_results: Vec::new(),
                isolation_complete: false,
            });
            continue;
        };
        let run = {
            let mut statement = connection.prepare(&format!("{RUN_SELECT_SQL} WHERE id=?1"))?;
            statement
                .query_row([run_id.to_string()], row_to_run)
                .optional()?
        };
        let Some(run) = run else {
            return Err(StorageError::InvalidData(
                "batch member references a missing run".into(),
            ));
        };
        let task_results = {
            let mut statement = connection.prepare(
                "SELECT run_id,task_id,category_json,outcome_json,score,failure_kind_json,
                        duration_ms,answer_rel_path,detail
                 FROM task_results WHERE run_id=?1 ORDER BY task_id ASC",
            )?;
            statement
                .query_map([run_id.to_string()], row_to_task_result)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let isolation_complete = match batch.plan.session_isolation_policy {
            crate::SessionIsolationPolicy::MachineEnforcedFreshSessionAndWorkspacePerTask => batch
                .plan
                .targets
                .get(usize::try_from(member.target_position).unwrap_or(usize::MAX))
                .is_some_and(|target| {
                    target.route_identity.execution_surface
                        == crate::BatchExecutionSurface::AutomatedCli
                }),
            crate::SessionIsolationPolicy::UserAttestedFreshConversationPerTask => {
                let rows = {
                    let mut statement = connection.prepare(
                        "SELECT task_id,policy_version,enforcement_json,user_attested
                         FROM scan_batch_task_isolation
                         WHERE batch_id=?1 AND member_ordinal=?2 AND run_id=?3
                         ORDER BY task_id ASC",
                    )?;
                    statement
                        .query_map(
                            params![
                                batch.id.to_string(),
                                i64::from(member.ordinal),
                                run_id.to_string(),
                            ],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, i64>(1)?,
                                    row.get::<_, String>(2)?,
                                    row.get::<_, i64>(3)?,
                                ))
                            },
                        )?
                        .collect::<Result<Vec<_>, _>>()?
                };
                let result_ids = task_results
                    .iter()
                    .map(|result| result.task_id.as_str())
                    .collect::<BTreeSet<_>>();
                rows.len() == task_results.len()
                    && rows.len() == batch.plan.sealed_task_budgets.len()
                    && rows
                        .iter()
                        .all(|(task_id, version, enforcement, attested)| {
                            *version == i64::from(batch.plan.task_session_policy_version)
                                && matches!(
                                    serde_json::from_str::<IsolationEnforcement>(enforcement),
                                    Ok(IsolationEnforcement::UserAttested)
                                )
                                && *attested == 1
                                && result_ids.contains(task_id.as_str())
                        })
            }
        };
        evidence.push(MemberEvidence {
            member_ordinal: member.ordinal,
            target_position: member.target_position,
            status: member.status,
            run_status: Some(run.status),
            score: run.score,
            task_results: task_results
                .into_iter()
                .map(|result| TaskEvidence {
                    task_id: result.task_id,
                    category: result.category,
                    outcome: result.outcome,
                    score: result.score,
                    failure_kind: result.failure_kind,
                })
                .collect(),
            isolation_complete,
        });
    }
    Ok(evidence)
}

fn load_batch(
    connection: &Connection,
    batch_id: Uuid,
) -> Result<Option<ScanBatchRecord>, StorageError> {
    let row: Option<StoredBatchRow> = connection
        .query_row(
            "SELECT plan_json,mode_json,suite_id,suite_version,content_sha256,
                    scoring_rule_version,seed,status_json,acknowledgement_hash,
                    acknowledgement_expires_at,planned_member_count,
                    terminal_member_count,cancel_requested,created_at,updated_at
             FROM scan_batches WHERE id=?1",
            [batch_id.to_string()],
            |row| {
                Ok(StoredBatchRow {
                    plan_json: row.get(0)?,
                    mode_json: row.get(1)?,
                    suite_id: row.get(2)?,
                    suite_version: row.get(3)?,
                    content_sha256: row.get(4)?,
                    scoring_rule_version: row.get(5)?,
                    seed: row.get(6)?,
                    status_json: row.get(7)?,
                    acknowledgement_hash: row.get(8)?,
                    acknowledgement_expires_at: row.get(9)?,
                    planned_member_count: row.get(10)?,
                    terminal_member_count: row.get(11)?,
                    cancel_requested: row.get(12)?,
                    created_at: row.get(13)?,
                    updated_at: row.get(14)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let plan: ScanBatchPlan = serde_json::from_str(&row.plan_json)?;
    validate_plan_acknowledgement_hash(&plan)?;
    for target in &plan.targets {
        target
            .validate_for_new_batch()
            .map_err(|error| StorageError::InvalidData(error.to_string()))?;
    }
    let mode: crate::BatchMode = serde_json::from_str(&row.mode_json)?;
    let status: BatchStatus = serde_json::from_str(&row.status_json)?;
    let seed = u64::try_from(row.seed)
        .map_err(|_| StorageError::InvalidData("stored batch seed is invalid".into()))?;
    let planned_member_count = u32::try_from(row.planned_member_count)
        .map_err(|_| StorageError::InvalidData("stored planned count is invalid".into()))?;
    let terminal_member_count = u32::try_from(row.terminal_member_count)
        .map_err(|_| StorageError::InvalidData("stored terminal count is invalid".into()))?;
    let acknowledgement_expiry = DateTime::parse_from_rfc3339(&row.acknowledgement_expires_at)
        .map_err(StorageError::Time)?
        .with_timezone(&Utc);
    if mode != plan.mode
        || row.suite_id != plan.suite_id
        || row.suite_version != plan.suite_version
        || row.content_sha256 != plan.suite_content_sha256
        || row.scoring_rule_version != plan.scoring_rule_version
        || seed != plan.seed
        || row.acknowledgement_hash != plan.acknowledgement_hash
        || acknowledgement_expiry != plan.cost_estimate.initial_acknowledgement_expires_at
        || u64::from(planned_member_count) != plan.cost_estimate.planned_member_runs
    {
        return Err(StorageError::InvalidData(
            "indexed batch identity does not match immutable plan JSON".into(),
        ));
    }
    let created_at = DateTime::parse_from_rfc3339(&row.created_at)
        .map_err(StorageError::Time)?
        .with_timezone(&Utc);
    let snapshot = load_valid_baseline_snapshot(connection, batch_id)?;
    match mode {
        BatchMode::Full => {
            let snapshot = snapshot.as_ref().ok_or_else(|| {
                StorageError::InvalidData("Full batch is missing its baseline snapshot".into())
            })?;
            let identity = BatchAnalysisIdentity::from_plan(&plan)
                .map_err(|error| StorageError::InvalidData(error.to_string()))?;
            if snapshot.baseline_as_of != created_at || snapshot.identity != identity {
                return Err(StorageError::InvalidData(
                    "Full batch baseline snapshot does not match its immutable plan".into(),
                ));
            }
        }
        BatchMode::QuickComparison | BatchMode::Standard if snapshot.is_some() => {
            return Err(StorageError::InvalidData(
                "Quick and Standard batches cannot carry regression snapshots".into(),
            ));
        }
        BatchMode::QuickComparison | BatchMode::Standard => {}
    }
    let mut target_statement = connection.prepare(
        "SELECT position,target_json,route_identity_json,adapter_identity_json,
                execution_surface_json
         FROM scan_batch_targets WHERE batch_id=?1 ORDER BY position ASC",
    )?;
    let target_rows = target_statement
        .query_map([batch_id.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if target_rows.len() != plan.targets.len() {
        return Err(StorageError::InvalidData(
            "persisted batch target count does not match the plan".into(),
        ));
    }
    for (position, row) in target_rows.iter().enumerate() {
        let target = &plan.targets[position];
        if usize::try_from(row.0).ok() != Some(position)
            || serde_json::from_str::<TargetSelection>(&row.1)? != target.target
            || serde_json::from_str::<crate::TargetRouteIdentity>(&row.2)? != target.route_identity
            || serde_json::from_str::<crate::ExecutionAdapterIdentity>(&row.3)?
                != target.execution_adapter_identity
            || serde_json::from_str::<crate::BatchExecutionSurface>(&row.4)?
                != target.route_identity.execution_surface
        {
            return Err(StorageError::InvalidData(
                "persisted batch target identity does not match the plan".into(),
            ));
        }
    }
    let mut member_statement = connection.prepare(
        "SELECT ordinal,target_position,repetition_index,run_id,status_json,
                failure_kind_json,attempt_number,updated_at
         FROM scan_batch_members WHERE batch_id=?1 ORDER BY ordinal ASC",
    )?;
    let members = member_statement
        .query_map([batch_id.to_string()], row_to_batch_member)?
        .collect::<Result<Vec<_>, _>>()?;
    if members.len() != usize::try_from(planned_member_count).unwrap_or(usize::MAX)
        || members
            .iter()
            .enumerate()
            .any(|(index, member)| usize::try_from(member.ordinal).ok() != Some(index))
        || members
            .iter()
            .filter(|member| member.status.is_terminal())
            .count()
            != usize::try_from(terminal_member_count).unwrap_or(usize::MAX)
    {
        return Err(StorageError::InvalidData(
            "persisted batch member index/count does not match indexed state".into(),
        ));
    }
    let seeds = members
        .iter()
        .map(|member| BatchMemberSeed {
            ordinal: member.ordinal,
            target_position: member.target_position,
            repetition_index: member.repetition_index,
        })
        .collect::<Vec<_>>();
    validate_new_batch_plan(&plan, &seeds)?;
    validate_stored_batch_lifecycle(status, row.cancel_requested != 0, &members)?;
    Ok(Some(ScanBatchRecord {
        id: batch_id,
        plan,
        baseline_snapshot: snapshot,
        status,
        cancel_requested: row.cancel_requested != 0,
        planned_member_count,
        terminal_member_count,
        created_at,
        updated_at: DateTime::parse_from_rfc3339(&row.updated_at)
            .map_err(StorageError::Time)?
            .with_timezone(&Utc),
        members,
    }))
}

fn task_results_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    run_id: Uuid,
) -> Result<Vec<TaskResult>, StorageError> {
    let mut statement = transaction.prepare(
        "SELECT run_id,task_id,category_json,outcome_json,score,failure_kind_json,
         duration_ms,answer_rel_path,detail
         FROM task_results WHERE run_id=?1 ORDER BY task_id ASC",
    )?;
    let rows = statement.query_map([run_id.to_string()], row_to_task_result)?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::from)
}

fn durable_retry_failure_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    run_id: Uuid,
) -> Result<Option<FailureKind>, StorageError> {
    let mut markers = task_results_in_transaction(transaction, run_id)?
        .into_iter()
        .filter(|result| result.outcome == TaskOutcome::Invalid);
    let Some(marker) = markers.next() else {
        return Ok(None);
    };
    if markers.next().is_some() {
        return Ok(None);
    }
    Ok(marker
        .failure_kind
        .filter(|failure| is_retryable_batch_failure(*failure)))
}

fn run_status_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    run_id: Uuid,
) -> Result<Option<RunStatus>, StorageError> {
    let status: Option<String> = transaction
        .query_row(
            "SELECT status_json FROM runs WHERE id=?1",
            [run_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    status
        .map(|value| serde_json::from_str(&value).map_err(StorageError::from))
        .transpose()
}

fn reject_active_delete(status: RunStatus) -> Result<(), StorageError> {
    if status == RunStatus::Running {
        Err(StorageError::InvalidData(
            "active runs cannot be deleted".into(),
        ))
    } else {
        Ok(())
    }
}

fn raw_retention_days_from(connection: &Connection) -> Result<Option<u32>, StorageError> {
    let value: String = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key='raw_retention_days'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| StorageError::InvalidData("raw retention setting is missing".into()))?;
    let days = serde_json::from_str::<Option<u32>>(&value)?;
    validate_raw_retention_days(days)?;
    Ok(days)
}

fn validate_raw_retention_days(days: Option<u32>) -> Result<(), StorageError> {
    if matches!(days, None | Some(7 | 30 | 90)) {
        Ok(())
    } else {
        Err(StorageError::InvalidData(
            "raw retention days must be forever, 7, 30, or 90".into(),
        ))
    }
}

fn clean_orphan_identities(transaction: &rusqlite::Transaction<'_>) -> Result<(), StorageError> {
    transaction.execute(
        "DELETE FROM targets
         WHERE NOT EXISTS (
           SELECT 1 FROM runs WHERE runs.target_json=targets.target_json
         ) AND NOT EXISTS (
           SELECT 1 FROM scan_batch_targets
           WHERE scan_batch_targets.target_json=targets.target_json
         )",
        [],
    )?;
    transaction.execute(
        "DELETE FROM suite_versions
         WHERE NOT EXISTS (
           SELECT 1 FROM runs
           WHERE runs.suite_id=suite_versions.suite_id
             AND runs.suite_version=suite_versions.suite_version
         ) AND NOT EXISTS (
           SELECT 1 FROM scan_batches
           WHERE scan_batches.suite_id=suite_versions.suite_id
             AND scan_batches.suite_version=suite_versions.suite_version
             AND scan_batches.content_sha256=suite_versions.content_sha256
             AND scan_batches.scoring_rule_version=suite_versions.scoring_rule_version
         )",
        [],
    )?;
    Ok(())
}

fn checkpoint_after_delete(connection: &Connection) -> Result<(), StorageError> {
    match connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
        Ok(()) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(code, _))
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(StorageError::Database(error)),
    }
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    let id: String = row.get(0)?;
    let target: String = row.get(1)?;
    let mode: String = row.get(2)?;
    let status: String = row.get(5)?;
    let started_at: String = row.get(6)?;
    let finished_at: Option<String> = row.get(7)?;
    let environment: String = row.get(10)?;
    let score: Option<String> = row.get(11)?;
    let run = RunRecord {
        id: Uuid::parse_str(&id).map_err(to_sql_error)?,
        target: serde_json::from_str(&target).map_err(to_sql_error)?,
        mode: serde_json::from_str(&mode).map_err(to_sql_error)?,
        suite_id: row.get(3)?,
        suite_version: row.get(4)?,
        status: serde_json::from_str(&status).map_err(to_sql_error)?,
        started_at: DateTime::parse_from_rfc3339(&started_at)
            .map_err(to_sql_error)?
            .with_timezone(&Utc),
        finished_at: finished_at
            .map(|value| {
                DateTime::parse_from_rfc3339(&value)
                    .map(|date| date.with_timezone(&Utc))
                    .map_err(to_sql_error)
            })
            .transpose()?,
        total_tasks: row.get(8)?,
        completed_tasks: row.get(9)?,
        environment: serde_json::from_str(&environment).map_err(to_sql_error)?,
        score: score
            .map(|value| serde_json::from_str(&value).map_err(to_sql_error))
            .transpose()?,
    };
    validate_run(&run).map_err(to_sql_error)?;
    Ok(run)
}

fn row_to_task_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskResult> {
    let run_id: String = row.get(0)?;
    let category: String = row.get(2)?;
    let outcome: String = row.get(3)?;
    let failure_kind: Option<String> = row.get(5)?;
    let duration_ms: i64 = row.get(6)?;
    let result = TaskResult {
        run_id: Uuid::parse_str(&run_id).map_err(to_sql_error)?,
        task_id: row.get(1)?,
        category: serde_json::from_str(&category).map_err(to_sql_error)?,
        outcome: serde_json::from_str(&outcome).map_err(to_sql_error)?,
        score: row.get(4)?,
        failure_kind: failure_kind
            .map(|value| serde_json::from_str(&value).map_err(to_sql_error))
            .transpose()?,
        duration_ms: u64::try_from(duration_ms).map_err(to_sql_error)?,
        answer_rel_path: row.get(7)?,
        detail: row.get(8)?,
    };
    validate_task_result(&result).map_err(to_sql_error)?;
    Ok(result)
}

fn validate_run(run: &RunRecord) -> Result<(), StorageError> {
    if run.suite_id.is_empty() || run.suite_version.is_empty() {
        return Err(StorageError::InvalidData(
            "suite id and version must not be empty".into(),
        ));
    }
    if run.completed_tasks > run.total_tasks {
        return Err(StorageError::InvalidData(
            "completed_tasks exceeds total_tasks".into(),
        ));
    }
    if run.environment.suite_id != run.suite_id
        || run.environment.suite_version != run.suite_version
    {
        return Err(StorageError::InvalidData(
            "run suite does not match its environment fingerprint".into(),
        ));
    }
    if let Some(score) = &run.score {
        validate_score_summary(score)?;
    }
    Ok(())
}

fn validate_task_result(result: &TaskResult) -> Result<(), StorageError> {
    if result.task_id.is_empty() || result.detail.is_empty() {
        return Err(StorageError::InvalidData(
            "task id and detail must not be empty".into(),
        ));
    }
    if let Some(score) = result.score {
        validate_score(score, "task score")?;
    }
    if let Some(path) = &result.answer_rel_path
        && !is_safe_relative_path(path)
    {
        return Err(StorageError::InvalidData(
            "answer path must be relative".into(),
        ));
    }
    Ok(())
}

fn validate_retry_marker(result: &TaskResult) -> Result<(), StorageError> {
    validate_task_result(result)?;
    if result.outcome != TaskOutcome::Invalid
        || result.score.is_some()
        || !matches!(
            result.failure_kind,
            Some(
                FailureKind::CliMissing
                    | FailureKind::RuntimeMissing
                    | FailureKind::AuthExpired
                    | FailureKind::QuotaExhausted
                    | FailureKind::Network
                    | FailureKind::AppInterrupted
                    | FailureKind::InfrastructureTimeout
                    | FailureKind::VerifierError
            )
        )
    {
        return Err(StorageError::InvalidData(
            "retry resume requires invalid infrastructure evidence".into(),
        ));
    }
    Ok(())
}

fn validate_score_summary(score: &ScoreSummary) -> Result<(), StorageError> {
    validate_score(score.ability_score, "ability score")?;
    if score.passed_tasks > score.valid_tasks || score.valid_tasks > score.total_tasks {
        return Err(StorageError::InvalidData(
            "score counts are inconsistent".into(),
        ));
    }
    for value in score.category_scores.values() {
        validate_score(*value, "category score")?;
    }
    Ok(())
}

fn validate_score(value: f64, name: &str) -> Result<(), StorageError> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(StorageError::InvalidData(format!(
            "{name} must be finite and between 0 and 100"
        )));
    }
    Ok(())
}

fn is_safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && !value.starts_with(['/', '\\'])
        && !value.contains(':')
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn to_sql_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}
