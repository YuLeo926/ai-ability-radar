use crate::{RunRecord, RunStatus, ScoreSummary, TaskResult};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
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
}

const RUN_SELECT_SQL: &str = "SELECT id,target_json,mode_json,suite_id,suite_version,status_json,
    started_at,finished_at,total_tasks,completed_tasks,environment_json,score_json FROM runs";

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
    if let Some(path) = &result.answer_rel_path {
        if !is_safe_relative_path(path) {
            return Err(StorageError::InvalidData(
                "answer path must be relative".into(),
            ));
        }
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
