use ability_core::{
    Category, EnvironmentFingerprint, ModelSource, ModelVerification, RunMode, RunRecord,
    RunRepository, RunStatus, ScoreSummary, StorageError, TargetKind, TargetSelection, TaskOutcome,
    TaskResult, summarize_scores,
};
use chrono::{Duration, Utc};
use std::collections::BTreeMap;
use tempfile::tempdir;
use uuid::Uuid;

fn sample_run() -> RunRecord {
    let mut run = RunRecord::new(
        TargetSelection {
            kind: TargetKind::ChatGptClient,
            reported_model: "user-selected".into(),
            reasoning_effort: None,
            model_source: ModelSource::LegacyUnknown,
            model_verification: ModelVerification::LegacyUnknown,
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
            execution_adapter_identity: None,
            resumed: false,
        },
    );
    run.status = RunStatus::Running;
    run
}

#[test]
fn legacy_target_json_defaults_provenance_without_rewriting_model() {
    let target: TargetSelection = serde_json::from_str(
        r#"{"kind":"chat_gpt_client","reportedModel":"GPT-X","reasoningEffort":"high"}"#,
    )
    .unwrap();

    assert_eq!(target.reported_model, "GPT-X");
    assert_eq!(target.model_source, ModelSource::LegacyUnknown);
    assert_eq!(target.model_verification, ModelVerification::LegacyUnknown);
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

fn invalid_infrastructure_result(run_id: Uuid, task_id: &str) -> TaskResult {
    TaskResult {
        run_id,
        task_id: task_id.into(),
        category: Category::InstructionFollowing,
        outcome: TaskOutcome::Invalid,
        score: None,
        failure_kind: Some(ability_core::FailureKind::Network),
        duration_ms: 250,
        answer_rel_path: None,
        detail: "network unavailable".into(),
    }
}

fn complete_one_result_run(repository: &RunRepository, mut run: RunRecord) -> RunRecord {
    run.status = RunStatus::Running;
    run.total_tasks = 1;
    run.completed_tasks = 0;
    run.finished_at = None;
    run.score = None;
    repository.insert_run(&run).unwrap();
    let result = passing_result(run.id, "instruction-1");
    repository.save_task_result(&result).unwrap();
    let score = summarize_scores(&[result], 1).unwrap();
    repository.complete_run(run.id, Some(&score)).unwrap();
    repository.get_run(run.id).unwrap().unwrap()
}

fn finish_run_without_score(
    repository: &RunRepository,
    mut run: RunRecord,
    status: RunStatus,
) -> RunRecord {
    run.status = RunStatus::Running;
    run.completed_tasks = 0;
    run.finished_at = None;
    run.score = None;
    repository.insert_run(&run).unwrap();
    repository.finish_without_score(run.id, status).unwrap();
    repository.get_run(run.id).unwrap().unwrap()
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
    let mut created = sample_run();
    created.status = RunStatus::Created;
    repo.insert_run(&running).unwrap();
    repo.insert_run(&created).unwrap();

    assert_eq!(repo.mark_running_as_interrupted().unwrap(), 1);
    assert_eq!(
        repo.get_run(running.id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(
        repo.get_run(created.id).unwrap().unwrap().status,
        RunStatus::Created
    );
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
    let mut run = sample_run();
    run.status = RunStatus::Running;
    let score = ScoreSummary {
        ability_score: 100.0,
        passed_tasks: 8,
        valid_tasks: 8,
        total_tasks: 8,
        category_scores: BTreeMap::from([(Category::InstructionFollowing, 100.0)]),
    };
    {
        let repo = RunRepository::open(&db_path).unwrap();
        repo.insert_run(&run).unwrap();
        for index in 1..=run.total_tasks {
            repo.save_task_result(&passing_result(run.id, &format!("instruction-{index}")))
                .unwrap();
        }
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
fn complete_run_rejects_partial_evidence_without_mutating_the_running_run() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    run.total_tasks = 2;
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&passing_result(run.id, "instruction-1"))
        .unwrap();

    assert!(matches!(
        repo.complete_run(run.id, None),
        Err(StorageError::InvalidData(message)) if message.contains("complete")
    ));

    let unchanged = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(unchanged.status, RunStatus::Running);
    assert_eq!(unchanged.completed_tasks, 1);
    assert_eq!(unchanged.finished_at, None);
    assert_eq!(unchanged.score, None);
}

#[test]
fn complete_run_rejects_a_score_bound_to_a_different_task_total() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    run.total_tasks = 1;
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&passing_result(run.id, "instruction-1"))
        .unwrap();
    let score = ScoreSummary {
        ability_score: 100.0,
        passed_tasks: 1,
        valid_tasks: 1,
        total_tasks: 2,
        category_scores: BTreeMap::from([(Category::InstructionFollowing, 100.0)]),
    };

    assert!(matches!(
        repo.complete_run(run.id, Some(&score)),
        Err(StorageError::InvalidData(message)) if message.contains("total")
    ));

    let unchanged = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(unchanged.status, RunStatus::Running);
    assert_eq!(unchanged.finished_at, None);
    assert_eq!(unchanged.score, None);
}

#[test]
fn complete_run_rejects_missing_or_mismatched_canonical_score_evidence() {
    for supplied_score in [
        None,
        Some(ScoreSummary {
            ability_score: 50.0,
            passed_tasks: 0,
            valid_tasks: 1,
            total_tasks: 1,
            category_scores: BTreeMap::from([(Category::InstructionFollowing, 50.0)]),
        }),
    ] {
        let dir = tempdir().unwrap();
        let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
        let mut run = sample_run();
        run.status = RunStatus::Running;
        run.total_tasks = 1;
        repo.insert_run(&run).unwrap();
        repo.save_task_result(&passing_result(run.id, "instruction-1"))
            .unwrap();

        assert!(matches!(
            repo.complete_run(run.id, supplied_score.as_ref()),
            Err(StorageError::InvalidData(message)) if message.contains("score")
        ));
        let unchanged = repo.get_run(run.id).unwrap().unwrap();
        assert_eq!(unchanged.status, RunStatus::Running);
        assert_eq!(unchanged.finished_at, None);
        assert_eq!(unchanged.score, None);
    }
}

#[test]
fn complete_run_accepts_full_all_invalid_evidence_with_no_score() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    run.total_tasks = 1;
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&invalid_infrastructure_result(run.id, "instruction-1"))
        .unwrap();

    repo.complete_run(run.id, None).unwrap();

    let completed = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(completed.status, RunStatus::Completed);
    assert_eq!(completed.completed_tasks, 1);
    assert_eq!(completed.score, None);
}

#[test]
fn complete_run_rejects_incoherent_full_evidence_without_mutation() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    run.total_tasks = 1;
    repo.insert_run(&run).unwrap();
    let mut incoherent = passing_result(run.id, "instruction-1");
    incoherent.score = Some(50.0);
    repo.save_task_result(&incoherent).unwrap();
    let before_run = repo.get_run(run.id).unwrap().unwrap();
    let before_results = repo.get_task_results(run.id).unwrap();

    assert!(matches!(
        repo.complete_run(run.id, None),
        Err(StorageError::InvalidData(message)) if message.contains("evidence")
    ));

    assert_eq!(repo.get_run(run.id).unwrap().unwrap(), before_run);
    assert_eq!(repo.get_task_results(run.id).unwrap(), before_results);
}

#[test]
fn completed_run_rejects_checkpoint_replacement_without_mutation() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    run.total_tasks = 1;
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&passing_result(run.id, "instruction-1"))
        .unwrap();
    let score = ScoreSummary {
        ability_score: 100.0,
        passed_tasks: 1,
        valid_tasks: 1,
        total_tasks: 1,
        category_scores: BTreeMap::from([(Category::InstructionFollowing, 100.0)]),
    };
    repo.complete_run(run.id, Some(&score)).unwrap();
    let before_run = repo.get_run(run.id).unwrap().unwrap();
    let before_results = repo.get_task_results(run.id).unwrap();
    let mut replacement = passing_result(run.id, "instruction-1");
    replacement.detail = "forbidden replacement".into();

    assert!(matches!(
        repo.save_task_result(&replacement),
        Err(StorageError::InvalidData(message)) if message.contains("running")
    ));

    assert_eq!(repo.get_run(run.id).unwrap().unwrap(), before_run);
    assert_eq!(repo.get_task_results(run.id).unwrap(), before_results);
}

#[test]
fn insert_run_rejects_every_terminal_state() {
    for status in [
        RunStatus::Completed,
        RunStatus::Cancelled,
        RunStatus::Interrupted,
    ] {
        let dir = tempdir().unwrap();
        let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
        let mut run = sample_run();
        run.status = status;
        run.finished_at = Some(Utc::now());

        assert!(matches!(
            repo.insert_run(&run),
            Err(StorageError::InvalidData(message)) if message.contains("new run")
        ));
        assert!(repo.get_run(run.id).unwrap().is_none());
    }
}

#[test]
fn retry_marker_removal_rejects_a_valid_completed_checkpoint() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
    let checkpoint = passing_result(run.id, "instruction-1");
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&checkpoint).unwrap();
    repo.finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();

    assert!(matches!(
        repo.resume_run_retrying_exact_marker(
            run.id,
            &run.target,
            &checkpoint,
            |_, _| panic!("a valid checkpoint must not be exposed as a retry candidate"),
        ),
        Err(StorageError::InvalidData(message)) if message.contains("invalid")
    ));
    assert_eq!(repo.get_task_results(run.id).unwrap(), vec![checkpoint]);
    assert_eq!(repo.get_run(run.id).unwrap().unwrap().completed_tasks, 1);
}

#[test]
fn retry_resume_rolls_back_marker_removal_when_candidate_validation_fails() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.total_tasks = 2;
    let checkpoint = passing_result(run.id, "instruction-1");
    let marker = invalid_infrastructure_result(run.id, "instruction-2");
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&checkpoint).unwrap();
    repo.save_task_result(&marker).unwrap();
    repo.finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();
    let before_run = repo.get_run(run.id).unwrap().unwrap();
    let before_results = repo.get_task_results(run.id).unwrap();

    assert!(matches!(
        repo.resume_run_retrying_exact_marker(
            run.id,
            &run.target,
            &marker,
            |candidate, results| {
                assert_eq!(candidate.completed_tasks, 1);
                assert_eq!(results, std::slice::from_ref(&checkpoint));
                Err(StorageError::InvalidData(
                    "injected recovery validation failure".into(),
                ))
            },
        ),
        Err(StorageError::InvalidData(message))
            if message == "injected recovery validation failure"
    ));

    assert_eq!(repo.get_run(run.id).unwrap().unwrap(), before_run);
    assert_eq!(repo.get_task_results(run.id).unwrap(), before_results);
}

#[test]
fn retry_resume_atomically_deletes_exact_marker_and_restores_running() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.total_tasks = 2;
    let checkpoint = passing_result(run.id, "instruction-1");
    let marker = invalid_infrastructure_result(run.id, "instruction-2");
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&checkpoint).unwrap();
    repo.save_task_result(&marker).unwrap();
    repo.finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();

    let resumed = repo
        .resume_run_retrying_exact_marker(run.id, &run.target, &marker, |candidate, results| {
            assert_eq!(candidate.completed_tasks, 1);
            assert_eq!(results, std::slice::from_ref(&checkpoint));
            Ok(())
        })
        .unwrap();

    assert_eq!(resumed.status, RunStatus::Running);
    assert_eq!(resumed.completed_tasks, 1);
    assert!(resumed.finished_at.is_none());
    assert!(resumed.score.is_none());
    assert!(resumed.environment.resumed);
    assert_eq!(repo.get_run(run.id).unwrap().unwrap(), resumed);
    assert_eq!(repo.get_task_results(run.id).unwrap(), vec![checkpoint]);
}

#[test]
fn retry_resume_rolls_back_exact_marker_when_post_delete_run_update_fails() {
    let dir = tempdir().unwrap();
    let database = dir.path().join("ability.db");
    let repo = RunRepository::open(&database).unwrap();
    let mut run = sample_run();
    run.total_tasks = 2;
    let checkpoint = passing_result(run.id, "instruction-1");
    let marker = invalid_infrastructure_result(run.id, "instruction-2");
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&checkpoint).unwrap();
    repo.save_task_result(&marker).unwrap();
    repo.finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();
    let before_run = repo.get_run(run.id).unwrap().unwrap();
    let before_results = repo.get_task_results(run.id).unwrap();

    let trigger_connection = rusqlite::Connection::open(&database).unwrap();
    trigger_connection
        .execute_batch(&format!(
            "CREATE TRIGGER fail_retry_resume_update
             BEFORE UPDATE OF status_json, completed_tasks ON runs
             WHEN OLD.id = '{}'
             BEGIN
               SELECT RAISE(ABORT, 'injected post-delete run update failure');
             END;",
            run.id
        ))
        .unwrap();

    assert!(
        repo.resume_run_retrying_exact_marker(
            run.id,
            &run.target,
            &marker,
            |candidate, results| {
                assert_eq!(candidate.completed_tasks, 1);
                assert_eq!(results, std::slice::from_ref(&checkpoint));
                Ok(())
            },
        )
        .is_err()
    );
    assert_eq!(repo.get_run(run.id).unwrap().unwrap(), before_run);
    assert_eq!(repo.get_task_results(run.id).unwrap(), before_results);

    trigger_connection
        .execute_batch("DROP TRIGGER fail_retry_resume_update;")
        .unwrap();
    let resumed = repo
        .resume_run_retrying_exact_marker(run.id, &run.target, &marker, |_, _| Ok(()))
        .unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    assert_eq!(resumed.completed_tasks, 1);
    assert_eq!(repo.get_task_results(run.id).unwrap(), vec![checkpoint]);
}

#[test]
fn marker_free_resume_exposes_persisted_count_mismatch_to_validation() {
    let dir = tempdir().unwrap();
    let database = dir.path().join("ability.db");
    let repo = RunRepository::open(&database).unwrap();
    let mut run = sample_run();
    run.total_tasks = 2;
    let checkpoint = passing_result(run.id, "instruction-1");
    repo.insert_run(&run).unwrap();
    repo.save_task_result(&checkpoint).unwrap();
    repo.finish_without_score(run.id, RunStatus::Interrupted)
        .unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE runs SET completed_tasks=2 WHERE id=?1",
            [run.id.to_string()],
        )
        .unwrap();
    let before = repo.get_run(run.id).unwrap().unwrap();

    assert!(matches!(
        repo.resume_run(run.id, &run.target, |candidate, results| {
            if usize::try_from(candidate.completed_tasks).ok() != Some(results.len()) {
                return Err(StorageError::InvalidData(
                    "persisted completed count mismatch".into(),
                ));
            }
            Ok(())
        }),
        Err(StorageError::InvalidData(message))
            if message == "persisted completed count mismatch"
    ));
    assert_eq!(repo.get_run(run.id).unwrap().unwrap(), before);
    assert_eq!(repo.get_task_results(run.id).unwrap(), vec![checkpoint]);
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
fn finish_without_score_terminalizes_only_running_runs_without_inventing_score() {
    let dir = tempdir().unwrap();
    let repo = RunRepository::open(&dir.path().join("ability.db")).unwrap();
    let mut run = sample_run();
    run.status = RunStatus::Running;
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

    repo.finish_without_score(running.id, RunStatus::Cancelled)
        .unwrap();
    assert!(matches!(
        repo.finish_without_score(running.id, RunStatus::Interrupted),
        Err(StorageError::InvalidData(message)) if message.contains("running")
    ));
    assert_eq!(
        repo.get_run(running.id).unwrap().unwrap().status,
        RunStatus::Cancelled
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
    let run = complete_one_result_run(&repository, sample_run());

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
    let reviewed = finish_run_without_score(&repository, sample_run(), RunStatus::Interrupted);
    let reviewed_ids = vec![reviewed.id];

    let newly_created = finish_run_without_score(&repository, sample_run(), RunStatus::Interrupted);

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
    let first = finish_run_without_score(&repository, sample_run(), RunStatus::Interrupted);
    let second = finish_run_without_score(&repository, sample_run(), RunStatus::Interrupted);
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
    run.target.reported_model = sentinel.clone();
    let run = finish_run_without_score(&repository, run, RunStatus::Interrupted);
    assert!(repository.delete_run(run.id).unwrap());
    drop(repository);

    let bytes = std::fs::read(database).unwrap();
    assert!(
        !bytes
            .windows(sentinel.len())
            .any(|window| window == sentinel.as_bytes())
    );
}

#[test]
fn retention_policy_defaults_to_forever_and_rejects_every_unsupported_value() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();

    assert_eq!(repository.raw_retention_days().unwrap(), None);
    for accepted in [None, Some(7), Some(30), Some(90)] {
        repository.set_raw_retention_days(accepted).unwrap();
        assert_eq!(repository.raw_retention_days().unwrap(), accepted);
    }
    for rejected in [
        Some(0),
        Some(1),
        Some(8),
        Some(89),
        Some(91),
        Some(u32::MAX),
    ] {
        assert!(matches!(
            repository.set_raw_retention_days(rejected),
            Err(StorageError::InvalidData(_))
        ));
    }

    let connection = rusqlite::Connection::open(&database).unwrap();
    for corrupt in ["{}", "\"7\"", "7.0", "4294967296", "91"] {
        connection
            .execute(
                "UPDATE settings SET value_json=?1 WHERE key='raw_retention_days'",
                [corrupt],
            )
            .unwrap();
        assert!(
            repository.raw_retention_days().is_err(),
            "{corrupt:?} must fail closed"
        );
    }
}

#[test]
fn retention_candidates_require_terminal_status_real_finished_time_and_inclusive_boundary() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    repository.set_raw_retention_days(Some(7)).unwrap();

    let older_cancelled = finish_run_without_score(&repository, sample_run(), RunStatus::Cancelled);
    let boundary = complete_one_result_run(&repository, sample_run());
    let cutoff = boundary.finished_at.unwrap();
    let now = cutoff + Duration::days(7);

    let mut created = sample_run();
    created.status = RunStatus::Created;
    repository.insert_run(&created).unwrap();
    let running = sample_run();
    repository.insert_run(&running).unwrap();
    let _interrupted = finish_run_without_score(&repository, sample_run(), RunStatus::Interrupted);

    // Deliberately corrupt only read-path fixtures that cannot arise through the
    // lifecycle API: a completed row without finished_at and a future terminal time.
    let completed_without_time = complete_one_result_run(&repository, sample_run());
    let cancelled_in_future =
        finish_run_without_score(&repository, sample_run(), RunStatus::Cancelled);
    let connection = rusqlite::Connection::open(database).unwrap();
    connection
        .execute(
            "UPDATE runs SET finished_at=NULL WHERE id=?1",
            [completed_without_time.id.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE runs SET finished_at=?2 WHERE id=?1",
            rusqlite::params![
                cancelled_in_future.id.to_string(),
                (now + Duration::days(1)).to_rfc3339()
            ],
        )
        .unwrap();

    let candidates = repository.retention_candidates(now).unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
        vec![older_cancelled.id, boundary.id]
    );
    assert_eq!(candidates[0].target.kind, older_cancelled.target.kind);
    assert_eq!(candidates[1].finished_at, cutoff);

    repository.set_raw_retention_days(None).unwrap();
    assert!(repository.retention_candidates(now).unwrap().is_empty());
}

#[test]
fn retention_reference_cleanup_rechecks_the_exact_candidate_and_current_policy_atomically() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    repository.set_raw_retention_days(Some(7)).unwrap();
    let run = complete_one_result_run(&repository, sample_run());
    let now = run.finished_at.unwrap() + Duration::days(8);
    let candidate = repository.retention_candidates(now).unwrap().remove(0);

    repository.set_raw_retention_days(Some(30)).unwrap();
    assert!(
        repository
            .clear_retention_candidate(&candidate, now)
            .is_err()
    );
    assert!(
        repository.get_task_results(run.id).unwrap()[0]
            .answer_rel_path
            .is_some()
    );

    repository.set_raw_retention_days(Some(7)).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_retention_clear BEFORE UPDATE OF answer_rel_path
             ON task_results BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    assert!(
        repository
            .clear_retention_candidate(&candidate, now)
            .is_err()
    );
    assert!(
        repository.get_task_results(run.id).unwrap()[0]
            .answer_rel_path
            .is_some()
    );
    connection
        .execute_batch("DROP TRIGGER fail_retention_clear;")
        .unwrap();

    assert_eq!(
        repository
            .clear_retention_candidate(&candidate, now)
            .unwrap(),
        1
    );
    let retained_run = repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(retained_run.status, RunStatus::Completed);
    assert_eq!(retained_run.finished_at, run.finished_at);
    assert_eq!(retained_run.score, run.score);
    let retained_result = repository.get_task_results(run.id).unwrap().remove(0);
    assert_eq!(retained_result.score, Some(100.0));
    assert_eq!(retained_result.answer_rel_path, None);
}

#[test]
fn backup_snapshot_binds_exact_run_identities_and_is_a_readable_sqlite_database() {
    let directory = tempdir().unwrap();
    let repository = RunRepository::open(&directory.path().join("ability.db")).unwrap();
    let first = sample_run();
    let mut second = sample_run();
    second.target.kind = TargetKind::CodexCli;
    repository.insert_run(&first).unwrap();
    repository.insert_run(&second).unwrap();

    let snapshot_path = directory.path().join("snapshot.sqlite");
    let runs = repository.snapshot_to_backup_file(&snapshot_path).unwrap();
    assert_eq!(
        runs.iter()
            .map(|run| (run.id, run.target))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            (first.id, first.target.kind),
            (second.id, second.target.kind),
        ])
    );
    assert!(
        std::fs::read(&snapshot_path)
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    let connection = rusqlite::Connection::open(snapshot_path).unwrap();
    let run_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
        .unwrap();
    let policy: String = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key='raw_retention_days'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(run_count, 2);
    assert_eq!(policy, "null");
}

#[test]
fn publication_rows_accept_only_canonical_safe_fixed_metadata() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    let run = sample_run();
    repository.insert_run(&run).unwrap();
    let report_id = Uuid::new_v4();
    let exported_at = chrono::DateTime::parse_from_rfc3339("2026-07-19T12:34:56Z")
        .unwrap()
        .with_timezone(&Utc);
    let hash = "a".repeat(64);

    repository
        .record_publication_at(report_id, run.id, &hash, "local_html", exported_at)
        .unwrap();
    for (bad_hash, bad_kind) in [
        ("A".repeat(64), "local_html"),
        ("a".repeat(63), "local_html"),
        ("g".repeat(64), "local_html"),
        ("a".repeat(64), "C:\\private\\report.html"),
        ("a".repeat(64), "local_zip"),
    ] {
        assert!(matches!(
            repository.record_publication_at(
                Uuid::new_v4(),
                run.id,
                &bad_hash,
                bad_kind,
                exported_at,
            ),
            Err(StorageError::InvalidData(_))
        ));
    }

    drop(repository);
    let connection = rusqlite::Connection::open(database).unwrap();
    let row: (String, String, String, String, String) = connection
        .query_row(
            "SELECT report_id,run_id,exported_at,report_sha256,destination_kind
             FROM publications",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, report_id.to_string());
    assert_eq!(row.1, run.id.to_string());
    assert_eq!(row.2, exported_at.to_rfc3339());
    assert_eq!(row.3, hash);
    assert_eq!(row.4, "local_html");
    assert!(!row.4.contains(['\\', '/']));
}

#[test]
fn opening_repository_applies_v3_without_changing_legacy_defaults() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("ability.db");
    let repository = RunRepository::open(&database).unwrap();
    assert_eq!(repository.raw_retention_days().unwrap(), None);
    drop(repository);

    let connection = rusqlite::Connection::open(database).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        3
    );
    for table in [
        "scan_batches",
        "scan_batch_targets",
        "scan_batch_members",
        "scan_batch_task_isolation",
        "scan_execution_authorizations",
        "baseline_snapshots",
        "scan_deletion_intents",
    ] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "missing v3 table {table}"
        );
    }
}
