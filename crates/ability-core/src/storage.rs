use crate::{RunRecord, RunStatus, ScoreSummary, TargetKind, TargetSelection, TaskResult};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
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

impl RunRepository {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        connection.execute_batch(include_str!("../migrations/0001_init.sql"))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn insert_run(&self, run: &RunRecord) -> Result<(), StorageError> {
        validate_run(run)?;
        if run.completed_tasks != 0 {
            return Err(StorageError::InvalidData(
                "a new run cannot have completed tasks before checkpoints exist".into(),
            ));
        }
        let target_json = serde_json::to_string(&run.target)?;
        let mode_json = serde_json::to_string(&run.mode)?;
        let status_json = serde_json::to_string(&run.status)?;
        let environment_json = serde_json::to_string(&run.environment)?;
        let score_json = run.score.as_ref().map(serde_json::to_string).transpose()?;
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
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
        let transaction = connection.transaction()?;
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
        let total_tasks: i64 = transaction.query_row(
            "SELECT total_tasks FROM runs WHERE id=?1",
            [result.run_id.to_string()],
            |row| row.get(0),
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
        transaction.execute(
            "UPDATE runs SET completed_tasks=(
              SELECT COUNT(*) FROM task_results WHERE run_id=?1
            ) WHERE id=?1",
            [result.run_id.to_string()],
        )?;
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
        let changed = self.connection.lock().execute(
            "UPDATE runs SET status_json=?2, finished_at=?3, score_json=?4 WHERE id=?1",
            params![
                run_id.to_string(),
                serde_json::to_string(&RunStatus::Completed)?,
                Utc::now().to_rfc3339(),
                score.map(serde_json::to_string).transpose()?,
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::RunNotFound(run_id));
        }
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

        let connection = self.connection.lock();
        let changed = connection.execute(
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
            return Ok(());
        }

        let existing_status: Option<String> = connection
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

    pub fn set_run_status(&self, run_id: Uuid, status: RunStatus) -> Result<(), StorageError> {
        let changed = self.connection.lock().execute(
            "UPDATE runs SET status_json=?2 WHERE id=?1",
            params![run_id.to_string(), serde_json::to_string(&status)?],
        )?;
        if changed == 0 {
            return Err(StorageError::RunNotFound(run_id));
        }
        Ok(())
    }

    pub fn get_run(&self, run_id: Uuid) -> Result<Option<RunRecord>, StorageError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(&format!("{RUN_SELECT_SQL} WHERE id=?1"))?;
        statement
            .query_row([run_id.to_string()], row_to_run)
            .optional()
            .map_err(StorageError::from)
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
        let results = task_results_in_transaction(&transaction, run_id)?;
        validate(&run, &results)?;

        run.status = RunStatus::Running;
        run.finished_at = None;
        run.score = None;
        run.environment.resumed = true;
        let changed = transaction.execute(
            "UPDATE runs
             SET status_json=?2,finished_at=NULL,score_json=NULL,environment_json=?3
             WHERE id=?1 AND status_json=?4",
            params![
                run_id.to_string(),
                serde_json::to_string(&RunStatus::Running)?,
                serde_json::to_string(&run.environment)?,
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

fn clean_orphan_identities(transaction: &rusqlite::Transaction<'_>) -> Result<(), StorageError> {
    transaction.execute(
        "DELETE FROM targets
         WHERE NOT EXISTS (
           SELECT 1 FROM runs WHERE runs.target_json=targets.target_json
         )",
        [],
    )?;
    transaction.execute(
        "DELETE FROM suite_versions
         WHERE NOT EXISTS (
           SELECT 1 FROM runs
           WHERE runs.suite_id=suite_versions.suite_id
             AND runs.suite_version=suite_versions.suite_version
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
