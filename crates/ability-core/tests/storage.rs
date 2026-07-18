use ability_core::{
    Category, EnvironmentFingerprint, RunMode, RunRecord, RunRepository, RunStatus, ScoreSummary,
    StorageError, TargetKind, TargetSelection, TaskOutcome, TaskResult,
};
use chrono::{Duration, Utc};
use std::collections::BTreeMap;
use tempfile::tempdir;
use uuid::Uuid;

fn sample_run() -> RunRecord {
    RunRecord::new(
        TargetSelection {
            kind: TargetKind::ChatGptClient,
            reported_model: "user-selected".into(),
            reasoning_effort: None,
        },
        RunMode::Quick,
        "client-quick".into(),
        "1.0.0".into(),
        8,
        EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "11".into(),
            app_version: "0.2.0".into(),
            cli_version: None,
            verifier_runtime_version: None,
            suite_id: "client-quick".into(),
            suite_version: "1.0.0".into(),
            suite_content_sha256: "b".repeat(64),
            scoring_rule_version: "ability-v1".into(),
            resumed: false,
        },
    )
}

fn passing_result(run_id: Uuid, task_id: &str) -> TaskResult {
    TaskResult {
        run_id,
        task_id: task_id.into(),
        category: Category::InstructionFollowing,
        outcome: TaskOutcome::Passed,
        score: Some(100.0),
        failure_kind: None,
        duration_ms: 250,
        answer_rel_path: Some("runs/a/answer.txt".into()),
        detail: "exact_json:pass".into(),
    }
}

#[test]
fn checkpoints_survive_repository_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ability.db");
    let run = sample_run();
    {
        let repo = RunRepository::open(&db_path).unwrap();
        repo.insert_run(&run).unwrap();
        repo.save_task_result(&passing_result(run.id, "instruction-1"))
            .unwrap();
    }

    let reopened = RunRepository::open(&db_path).unwrap();
    assert_eq!(reopened.get_task_results(run.id).unwrap().len(), 1);
    assert_eq!(
        reopened.get_run(run.id).unwrap().unwrap().completed_tasks,
        1
    );
}

#[test]
fn startup_marks_only_abandoned_running_runs_interrupted() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut running = sample_run();
    running.status = RunStatus::Running;
    let completed = sample_run();
    repo.insert_run(&running).unwrap();
    repo.insert_run(&completed).unwrap();

    assert_eq!(repo.mark_running_as_interrupted().unwrap(), 1);
    assert_eq!(
        repo.get_run(running.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(
        repo.get_run(completed.id).unwrap().unwrap().status,
        RunStatus::Created
    );
}

#[test]
fn status_updates_reject_unknown_runs() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let unknown_run = Uuid::new_v4();

    assert!(matches!(
        repo.set_run_status(unknown_run, RunStatus::Running),
        Err(StorageError::RunNotFound(id)) if id == unknown_run
    ));
}

#[test]
fn replacing_a_checkpoint_keeps_completed_task_count_unique() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let run = sample_run();
    repo.insert_run(&run).unwrap();

    repo.save_task_result(&passing_result(run.id, "instruction-1"))
        .unwrap();
    let mut replacement = passing_result(run.id, "instruction-1");
    replacement.detail = "exact_json:updated".into();
    replacement.duration_ms = 500;
    repo.save_task_result(&replacement).unwrap();

    assert_eq!(repo.get_run(run.id).unwrap().unwrap().completed_tasks, 1);
    assert_eq!(repo.get_task_results(run.id).unwrap(), vec![replacement]);
}

#[test]
fn checkpoints_cannot_exceed_a_run_total_but_can_replace_an_existing_task() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.total_tasks = 1;
    repo.insert_run(&run).unwrap();

    let first = passing_result(run.id, "instruction-1");
    repo.save_task_result(&first).unwrap();

    let error = repo
        .save_task_result(&passing_result(run.id, "instruction-2"))
        .unwrap_err();
    assert!(matches!(error, StorageError::InvalidData(message) if message.contains("checkpoint")));
    assert_eq!(repo.get_task_results(run.id).unwrap(), vec![first]);
    assert_eq!(repo.get_run(run.id).unwrap().unwrap().completed_tasks, 1);

    let mut replacement = passing_result(run.id, "instruction-1");
    replacement.detail = "exact_json:updated".into();
    repo.save_task_result(&replacement).unwrap();
    assert_eq!(repo.get_task_results(run.id).unwrap(), vec![replacement]);
    assert_eq!(repo.get_run(run.id).unwrap().unwrap().completed_tasks, 1);
}

#[test]
fn inserting_a_run_cannot_claim_completed_tasks_without_checkpoints() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.completed_tasks = 1;

    assert!(repo.insert_run(&run).is_err());
    assert!(repo.get_run(run.id).unwrap().is_none());
}

#[test]
fn completed_run_retains_score_and_finished_time_after_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("ability.db");
    let run = sample_run();
    let score = ScoreSummary {
        ability_score: 87.5,
        passed_tasks: 7,
        valid_tasks: 8,
        total_tasks: 8,
        category_scores: BTreeMap::from([(Category::InstructionFollowing, 87.5)]),
    };
    {
        let repo = RunRepository::open(&db_path).unwrap();
        repo.insert_run(&run).unwrap();
        repo.complete_run(run.id, Some(&score)).unwrap();
    }

    let restored = RunRepository::open(&db_path)
        .unwrap()
        .get_run(run.id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.status, RunStatus::Completed);
    assert!(restored.finished_at.is_some());
    assert_eq!(restored.score, Some(score));
}

#[test]
fn saving_a_result_for_an_unknown_run_is_rejected_without_rows() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let unknown_run = Uuid::new_v4();

    assert!(
        repo.save_task_result(&passing_result(unknown_run, "instruction-1"))
            .is_err()
    );
    assert!(repo.get_task_results(unknown_run).unwrap().is_empty());
}

#[test]
fn runs_are_listed_newest_first_with_a_stable_tiebreaker() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut older = sample_run();
    older.started_at = Utc::now() - Duration::seconds(1);
    let mut newer = sample_run();
    newer.started_at = Utc::now();
    repo.insert_run(&older).unwrap();
    repo.insert_run(&newer).unwrap();

    assert_eq!(
        repo.list_runs()
            .unwrap()
            .into_iter()
            .map(|run| run.id)
            .collect::<Vec<_>>(),
        vec![newer.id, older.id]
    );
}

#[test]
fn invalid_numeric_or_path_values_are_rejected_before_persistence() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let run = sample_run();
    repo.insert_run(&run).unwrap();
    let mut invalid = passing_result(run.id, "instruction-1");
    invalid.score = Some(f64::NAN);
    assert!(repo.save_task_result(&invalid).is_err());
    invalid.score = Some(100.0);
    invalid.answer_rel_path = Some("../answer.txt".into());
    assert!(repo.save_task_result(&invalid).is_err());
    assert!(repo.get_task_results(run.id).unwrap().is_empty());
}

#[test]
fn finish_without_score_terminalizes_only_running_runs_and_clears_score() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    run.score = Some(ScoreSummary {
        ability_score: 100.0,
        passed_tasks: 1,
        valid_tasks: 1,
        total_tasks: 8,
        category_scores: BTreeMap::from([(Category::InstructionFollowing, 100.0)]),
    });
    repo.insert_run(&run).unwrap();

    repo.finish_without_score(run.id, RunStatus::Cancelled)
        .unwrap();

    let stored = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Cancelled);
    assert!(stored.finished_at.is_some());
    assert_eq!(stored.score, None);
}

#[test]
fn finish_without_score_rejects_invalid_status_missing_and_non_running_runs() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut running = sample_run();
    running.status = RunStatus::Running;
    repo.insert_run(&running).unwrap();

    assert!(matches!(
        repo.finish_without_score(running.id, RunStatus::Completed),
        Err(StorageError::InvalidData(message)) if message.contains("cancelled or interrupted")
    ));
    let unchanged = repo.get_run(running.id).unwrap().unwrap();
    assert_eq!(unchanged.status, RunStatus::Running);
    assert!(unchanged.finished_at.is_none());

    let missing = Uuid::new_v4();
    assert!(matches!(
        repo.finish_without_score(missing, RunStatus::Interrupted),
        Err(StorageError::RunNotFound(id)) if id == missing
    ));

    repo.set_run_status(running.id, RunStatus::Completed)
        .unwrap();
    assert!(matches!(
        repo.finish_without_score(running.id, RunStatus::Interrupted),
        Err(StorageError::InvalidData(message)) if message.contains("running")
    ));
    assert_eq!(
        repo.get_run(running.id).unwrap().unwrap().status,
        RunStatus::Completed
    );
}

#[test]
fn raw_reference_cleanup_is_score_preserving_idempotent_and_rejects_active_runs() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    repository.insert_run(&run).unwrap();
    repository
        .save_task_result(&passing_result(run.id, "instruction-1"))
        .unwrap();

    assert!(matches!(
        repository.clear_artifact_references(run.id),
        Err(StorageError::InvalidData(message)) if message.contains("active")
    ));
    assert!(
        repository.get_task_results(run.id).unwrap()[0]
            .answer_rel_path
            .is_some()
    );

    repository
        .finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();
    assert_eq!(repository.clear_artifact_references(run.id).unwrap(), 1);
    assert_eq!(repository.clear_artifact_references(run.id).unwrap(), 0);
    let retained = repository.get_task_results(run.id).unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].score, Some(100.0));
    assert_eq!(retained[0].answer_rel_path, None);
    assert!(repository.get_run(run.id).unwrap().is_some());
}

#[test]
fn delete_one_is_transactional_idempotent_and_cleans_orphan_identity_rows() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Completed;
    repository.insert_run(&run).unwrap();
    repository
        .save_task_result(&passing_result(run.id, "instruction-1"))
        .unwrap();

    assert!(repository.delete_run(run.id).unwrap());
    assert!(!repository.delete_run(run.id).unwrap());
    assert!(repository.get_run(run.id).unwrap().is_none());
    assert!(repository.get_task_results(run.id).unwrap().is_empty());

    let connection = rusqlite::Connection::open(database).unwrap();
    let target_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM targets", [], |row| row.get(0))
        .unwrap();
    let suite_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM suite_versions", [], |row| row.get(0))
        .unwrap();
    assert_eq!((target_count, suite_count), (0, 0));
}

#[test]
fn destructive_repository_operations_reject_running_runs_without_partial_changes() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    repository.insert_run(&run).unwrap();
    repository
        .save_task_result(&passing_result(run.id, "instruction-1"))
        .unwrap();

    assert!(matches!(
        repository.delete_run(run.id),
        Err(StorageError::InvalidData(message)) if message.contains("active")
    ));
    assert!(matches!(
        repository.delete_target_history(TargetKind::ChatGptClient, &[run.id]),
        Err(StorageError::InvalidData(message)) if message.contains("active")
    ));
    assert!(repository.get_run(run.id).unwrap().is_some());
    assert_eq!(repository.get_task_results(run.id).unwrap().len(), 1);
}

#[test]
fn target_history_deletion_binds_the_exact_reviewed_run_snapshot() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let mut reviewed = sample_run();
    reviewed.status = RunStatus::Interrupted;
    repository.insert_run(&reviewed).unwrap();
    let reviewed_ids = vec![reviewed.id];

    let mut newly_created = sample_run();
    newly_created.status = RunStatus::Interrupted;
    repository.insert_run(&newly_created).unwrap();

    assert!(matches!(
        repository.delete_target_history(TargetKind::ChatGptClient, &reviewed_ids),
        Err(StorageError::InvalidData(message)) if message.contains("changed")
    ));
    assert!(repository.get_run(reviewed.id).unwrap().is_some());
    assert!(repository.get_run(newly_created.id).unwrap().is_some());

    let mut exact = vec![reviewed.id, newly_created.id];
    exact.sort_unstable();
    assert_eq!(
        repository
            .delete_target_history(TargetKind::ChatGptClient, &exact)
            .unwrap(),
        2
    );
    assert!(repository.list_runs().unwrap().is_empty());
    assert_eq!(
        repository
            .delete_target_history(TargetKind::ChatGptClient, &[])
            .unwrap(),
        0
    );
}

#[test]
fn injected_target_delete_failure_rolls_back_every_database_row() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let mut first = sample_run();
    first.status = RunStatus::Interrupted;
    let mut second = sample_run();
    second.status = RunStatus::Interrupted;
    repository.insert_run(&first).unwrap();
    repository.insert_run(&second).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(&format!(
            "CREATE TRIGGER fail_target_delete BEFORE DELETE ON runs
             WHEN OLD.id='{}'
             BEGIN SELECT RAISE(ABORT, 'injected delete failure'); END;",
            second.id
        ))
        .unwrap();
    drop(connection);
    let mut ids = vec![first.id, second.id];
    ids.sort_unstable();

    assert!(
        repository
            .delete_target_history(TargetKind::ChatGptClient, &ids)
            .is_err()
    );
    assert!(repository.get_run(first.id).unwrap().is_some());
    assert!(repository.get_run(second.id).unwrap().is_some());
}

#[test]
fn repository_keeps_sqlite_secure_delete_enabled() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let sentinel = "SECURE_DELETE_SENTINEL_7f1d2f977fbc4c63".repeat(8);
    let mut run = sample_run();
    run.status = RunStatus::Interrupted;
    run.target.reported_model = sentinel.clone();
    repository.insert_run(&run).unwrap();
    assert!(repository.delete_run(run.id).unwrap());
    drop(repository);

    let bytes = std::fs::read(database).unwrap();
    assert!(
        !bytes
            .windows(sentinel.len())
            .any(|window| window == sentinel.as_bytes())
    );
}
