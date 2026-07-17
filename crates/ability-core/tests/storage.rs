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
