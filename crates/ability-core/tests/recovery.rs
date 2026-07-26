use ability_core::{
    Category, EnvironmentFingerprint, FailureKind, ManualRunService, ModelSource,
    ModelVerification, PackLoader, RunMode, RunRecord, RunRepository, RunServiceError, RunStatus,
    TargetKind, TargetSelection, TaskOutcome, TaskResult,
};
use rusqlite::params;
use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};
use tempfile::tempdir;

fn write_pack(root: &Path) -> Arc<ability_core::LoadedPack> {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("one.txt"), "one").unwrap();
    fs::write(root.join("two.txt"), "two").unwrap();
    fs::write(
        root.join("manifest.json"),
        r#"{
          "schema_version":1,
          "id":"resume-pack",
          "version":"1.0.0",
          "title":"Resume",
          "target_kinds":["chat_gpt_client"],
          "tasks":[
            {"id":"one","category":"logic","prompt_file":"one.txt",
             "starter_dir":null,"time_budget_secs":60,"max_turns":1,
             "grader":{"type":"exact_text","expected":"one"}},
            {"id":"two","category":"instruction_following","prompt_file":"two.txt",
             "starter_dir":null,"time_budget_secs":60,"max_turns":1,
             "grader":{"type":"exact_text","expected":"two"}}
          ]
        }"#,
    )
    .unwrap();
    Arc::new(PackLoader::load(root).unwrap())
}

fn write_nonlexicographic_pack(root: &Path) -> Arc<ability_core::LoadedPack> {
    fs::create_dir_all(root).unwrap();
    for task_id in ["zeta", "alpha", "omega"] {
        fs::write(root.join(format!("{task_id}.txt")), task_id).unwrap();
    }
    fs::write(
        root.join("manifest.json"),
        r#"{
          "schema_version":1,
          "id":"resume-nonlex-pack",
          "version":"1.0.0",
          "title":"Resume nonlexicographic",
          "target_kinds":["chat_gpt_client"],
          "tasks":[
            {"id":"zeta","category":"logic","prompt_file":"zeta.txt",
             "starter_dir":null,"time_budget_secs":60,"max_turns":1,
             "grader":{"type":"exact_text","expected":"zeta"}},
            {"id":"alpha","category":"instruction_following","prompt_file":"alpha.txt",
             "starter_dir":null,"time_budget_secs":60,"max_turns":1,
             "grader":{"type":"exact_text","expected":"alpha"}},
            {"id":"omega","category":"code_review","prompt_file":"omega.txt",
             "starter_dir":null,"time_budget_secs":60,"max_turns":1,
             "grader":{"type":"exact_text","expected":"omega"}}
          ]
        }"#,
    )
    .unwrap();
    Arc::new(PackLoader::load(root).unwrap())
}

fn environment(pack: &ability_core::LoadedPack) -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os_family: "Windows".into(),
        os_version: "11".into(),
        app_version: "0.2.0".into(),
        cli_version: None,
        verifier_runtime_version: None,
        suite_id: pack.manifest.id.clone(),
        suite_version: pack.manifest.version.clone(),
        suite_content_sha256: pack.content_sha256.clone(),
        scoring_rule_version: "ability-v1".into(),
        execution_adapter_identity: None,
        resumed: false,
    }
}

fn target() -> TargetSelection {
    TargetSelection {
        kind: TargetKind::ChatGptClient,
        reported_model: "Model-X".into(),
        reasoning_effort: Some("high".into()),
        model_source: ModelSource::LegacyUnknown,
        model_verification: ModelVerification::LegacyUnknown,
    }
}

fn interrupted_run(
    repository: &Arc<RunRepository>,
    artifact_root: &Path,
    pack: &Arc<ability_core::LoadedPack>,
) -> RunRecord {
    let service = ManualRunService::new(repository.clone(), artifact_root.to_path_buf());
    let run = service
        .start(pack.clone(), target(), RunMode::Quick, environment(pack))
        .unwrap();
    service.submit_answer(run.id, "one", "one").unwrap();
    repository.mark_running_as_interrupted().unwrap();
    run
}

fn checkpoint(run: &RunRecord, task_id: &str, category: Category) -> TaskResult {
    TaskResult {
        run_id: run.id,
        task_id: task_id.into(),
        category,
        outcome: TaskOutcome::Passed,
        score: Some(100.0),
        failure_kind: None,
        duration_ms: 1,
        answer_rel_path: Some(format!("runs/{}/{task_id}.txt", run.id)),
        detail: "exact_text:pass".into(),
    }
}

#[test]
fn manual_run_resumes_at_the_next_validated_checkpoint_and_is_marked_resumed() {
    let directory = tempdir().unwrap();
    let pack = write_pack(&directory.path().join("pack"));
    let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    let run = interrupted_run(&repository, &artifact_root, &pack);

    let restarted = ManualRunService::new(repository.clone(), artifact_root);
    let resumed = restarted
        .resume(run.id, target(), pack.clone(), environment(&pack))
        .unwrap();

    assert_eq!(resumed.id, run.id);
    assert_eq!(resumed.status, RunStatus::Running);
    assert!(resumed.environment.resumed);
    assert_eq!(restarted.next_step(run.id).unwrap().unwrap().task_id, "two");
    restarted.submit_answer(run.id, "two", "two").unwrap();
    assert_eq!(
        repository.get_run(run.id).unwrap().unwrap().status,
        RunStatus::Completed
    );
}

#[test]
fn an_already_resumed_run_can_resume_again_after_another_app_interruption() {
    let directory = tempdir().unwrap();
    let pack = write_pack(&directory.path().join("pack"));
    let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    let run = interrupted_run(&repository, &artifact_root, &pack);

    {
        let first_restart = ManualRunService::new(repository.clone(), artifact_root.clone());
        let resumed = first_restart
            .resume(run.id, target(), pack.clone(), environment(&pack))
            .unwrap();
        assert!(resumed.environment.resumed);
        assert_eq!(
            first_restart.next_step(run.id).unwrap().unwrap().task_id,
            "two"
        );
    }

    repository.mark_running_as_interrupted().unwrap();
    let second_restart = ManualRunService::new(repository.clone(), artifact_root);
    let current_environment = environment(&pack);
    let resumed_again = second_restart
        .resume(run.id, target(), pack, current_environment)
        .unwrap();

    assert!(resumed_again.environment.resumed);
    assert_eq!(
        second_restart.next_step(run.id).unwrap().unwrap().task_id,
        "two"
    );
    let checkpoints = repository.get_task_results(run.id).unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].task_id, "one");
}

#[test]
fn resume_removes_only_an_uncheckpointed_published_manual_answer() {
    let directory = tempdir().unwrap();
    let pack = write_pack(&directory.path().join("pack"));
    let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    let run = interrupted_run(&repository, &artifact_root, &pack);
    let orphan = artifact_root
        .join("runs")
        .join(run.id.to_string())
        .join("two.txt");
    fs::write(&orphan, "published before interruption").unwrap();
    let checkpoint = orphan.with_file_name("one.txt");
    assert!(checkpoint.exists());

    let restarted = ManualRunService::new(repository.clone(), artifact_root);
    let current_environment = environment(&pack);
    restarted
        .resume(run.id, target(), pack, current_environment)
        .unwrap();

    assert!(!orphan.exists());
    assert!(checkpoint.exists());
    restarted.submit_answer(run.id, "two", "two").unwrap();
    assert_eq!(
        repository.get_task_results(run.id).unwrap().len(),
        2,
        "the orphan must not become a duplicate checkpoint"
    );
}

#[test]
fn recovery_uses_manifest_order_instead_of_database_task_id_order() {
    let directory = tempdir().unwrap();
    let pack = write_nonlexicographic_pack(&directory.path().join("pack"));
    let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    let service = ManualRunService::new(repository.clone(), artifact_root.clone());
    let run = service
        .start(pack.clone(), target(), RunMode::Quick, environment(&pack))
        .unwrap();
    service.submit_answer(run.id, "zeta", "zeta").unwrap();
    service.submit_answer(run.id, "alpha", "alpha").unwrap();
    repository.mark_running_as_interrupted().unwrap();

    let restarted = ManualRunService::new(repository, artifact_root);
    let current_environment = environment(&pack);
    restarted
        .resume(run.id, target(), pack, current_environment)
        .unwrap();

    assert_eq!(
        restarted.next_step(run.id).unwrap().unwrap().task_id,
        "omega"
    );
}

#[test]
fn only_one_concurrent_second_resume_can_activate_an_already_resumed_run() {
    let directory = tempdir().unwrap();
    let pack = write_pack(&directory.path().join("pack"));
    let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    let run = interrupted_run(&repository, &artifact_root, &pack);
    {
        let first_restart = ManualRunService::new(repository.clone(), artifact_root.clone());
        first_restart
            .resume(run.id, target(), pack.clone(), environment(&pack))
            .unwrap();
    }
    repository.mark_running_as_interrupted().unwrap();
    assert!(
        repository
            .get_run(run.id)
            .unwrap()
            .unwrap()
            .environment
            .resumed
    );
    let barrier = Arc::new(Barrier::new(3));

    let attempts = (0..2)
        .map(|_| {
            let repository = repository.clone();
            let artifact_root = artifact_root.clone();
            let pack = pack.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let service = ManualRunService::new(repository, artifact_root);
                barrier.wait();
                service.resume(run.id, target(), pack.clone(), environment(&pack))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let outcomes = attempts
        .into_iter()
        .map(|attempt| attempt.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);
}

#[test]
fn manual_resume_rejects_every_target_snapshot_mismatch_without_activating_the_run() {
    let mismatches: [fn(&mut TargetSelection); 3] = [
        |value| value.kind = TargetKind::ClaudeClient,
        |value| value.reported_model = "changed-model".into(),
        |value| value.reasoning_effort = Some("low".into()),
    ];

    for mutate in mismatches {
        let directory = tempdir().unwrap();
        let pack = write_pack(&directory.path().join("pack"));
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let artifact_root = directory.path().join("artifacts");
        let run = interrupted_run(&repository, &artifact_root, &pack);
        let service = ManualRunService::new(repository.clone(), artifact_root);
        let mut expected_target = target();
        mutate(&mut expected_target);

        assert!(matches!(
            service.resume(run.id, expected_target, pack.clone(), environment(&pack),),
            Err(RunServiceError::NotResumable(_))
        ));
        assert_eq!(
            repository.get_run(run.id).unwrap().unwrap().status,
            RunStatus::Interrupted
        );
        assert!(
            service.next_step(run.id).is_err(),
            "a rejected target snapshot must not enter the manual active map"
        );
    }
}

#[test]
fn repository_rejects_a_stale_target_inside_the_resume_transaction_before_validation() {
    let directory = tempdir().unwrap();
    let pack = write_pack(&directory.path().join("pack"));
    let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
    let artifact_root = directory.path().join("artifacts");
    let run = interrupted_run(&repository, &artifact_root, &pack);
    let orphan = artifact_root
        .join("runs")
        .join(run.id.to_string())
        .join("two.txt");
    fs::write(&orphan, "uncheckpointed").unwrap();
    let mut stale_target = target();
    stale_target.reported_model = "changed-model".into();
    let validation_called = Cell::new(false);

    assert!(
        repository
            .resume_run(run.id, &stale_target, |_, _| {
                validation_called.set(true);
                Ok(())
            })
            .is_err()
    );

    let stored = repository.get_run(run.id).unwrap().unwrap();
    assert_eq!(stored.status, RunStatus::Interrupted);
    assert!(!stored.environment.resumed);
    assert_eq!(stored.target, target());
    assert!(!validation_called.get());
    assert!(orphan.exists());
}

#[test]
fn resume_rejects_every_reproducibility_environment_change_without_mutating_the_run() {
    let fields: [fn(&mut EnvironmentFingerprint); 9] = [
        |value| value.os_family = "Linux".into(),
        |value| value.os_version = "10".into(),
        |value| value.app_version = "0.2.1".into(),
        |value| value.cli_version = Some("unexpected-cli".into()),
        |value| value.verifier_runtime_version = Some("unexpected-node".into()),
        |value| value.suite_id = "other-suite".into(),
        |value| value.suite_version = "2.0.0".into(),
        |value| value.suite_content_sha256 = "0".repeat(64),
        |value| value.scoring_rule_version = "ability-v2".into(),
    ];

    for mutate in fields {
        let directory = tempdir().unwrap();
        let pack = write_pack(&directory.path().join("pack"));
        let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
        let artifact_root = directory.path().join("artifacts");
        let run = interrupted_run(&repository, &artifact_root, &pack);
        let service = ManualRunService::new(repository.clone(), artifact_root);
        let mut current = environment(&pack);
        mutate(&mut current);

        assert!(matches!(
            service.resume(run.id, target(), pack.clone(), current),
            Err(RunServiceError::NotResumable(_))
        ));
        let stored = repository.get_run(run.id).unwrap().unwrap();
        assert_eq!(stored.status, RunStatus::Interrupted);
        assert!(!stored.environment.resumed);
    }
}

#[test]
fn resume_rejects_pack_and_checkpoint_corruption_fail_closed() {
    enum Corruption {
        RunTotal,
        CompletedCount,
        UnknownTask,
        WrongCategory,
        InvalidOutcome,
        InvalidScore,
        ImpossibleManualFailure,
        PersistedScore,
        ScoringRule,
    }

    for corruption in [
        Corruption::RunTotal,
        Corruption::CompletedCount,
        Corruption::UnknownTask,
        Corruption::WrongCategory,
        Corruption::InvalidOutcome,
        Corruption::InvalidScore,
        Corruption::ImpossibleManualFailure,
        Corruption::PersistedScore,
        Corruption::ScoringRule,
    ] {
        let directory = tempdir().unwrap();
        let pack = write_pack(&directory.path().join("pack"));
        let database = directory.path().join("runs.db");
        let repository = Arc::new(RunRepository::open(&database).unwrap());
        let artifact_root = directory.path().join("artifacts");
        let run = interrupted_run(&repository, &artifact_root, &pack);
        let connection = rusqlite::Connection::open(&database).unwrap();
        match corruption {
            Corruption::RunTotal => {
                connection
                    .execute(
                        "UPDATE runs SET total_tasks=3 WHERE id=?1",
                        [run.id.to_string()],
                    )
                    .unwrap();
            }
            Corruption::CompletedCount => {
                connection
                    .execute(
                        "UPDATE runs SET completed_tasks=0 WHERE id=?1",
                        [run.id.to_string()],
                    )
                    .unwrap();
            }
            Corruption::UnknownTask => {
                connection
                    .execute(
                        "UPDATE task_results SET task_id='unknown' WHERE run_id=?1",
                        [run.id.to_string()],
                    )
                    .unwrap();
            }
            Corruption::WrongCategory => {
                connection
                    .execute(
                        "UPDATE task_results SET category_json='\"code_review\"' WHERE run_id=?1",
                        [run.id.to_string()],
                    )
                    .unwrap();
            }
            Corruption::InvalidOutcome => {
                connection
                    .execute(
                        "UPDATE task_results SET outcome_json='\"invalid\"',
                         score=NULL,failure_kind_json='\"network\"' WHERE run_id=?1",
                        [run.id.to_string()],
                    )
                    .unwrap();
            }
            Corruption::InvalidScore => {
                connection
                    .execute(
                        "UPDATE task_results SET score=42 WHERE run_id=?1",
                        [run.id.to_string()],
                    )
                    .unwrap();
            }
            Corruption::ImpossibleManualFailure => {
                connection
                    .execute(
                        "UPDATE task_results SET outcome_json='\"failed\"',score=0,
                         failure_kind_json='\"agent_budget_exceeded\"' WHERE run_id=?1",
                        [run.id.to_string()],
                    )
                    .unwrap();
            }
            Corruption::PersistedScore => {
                connection
                    .execute(
                        "UPDATE runs SET score_json=?2 WHERE id=?1",
                        params![
                            run.id.to_string(),
                            r#"{"abilityScore":100.0,"passedTasks":1,"validTasks":1,
                               "totalTasks":2,"categoryScores":{"logic":100.0}}"#
                        ],
                    )
                    .unwrap();
            }
            Corruption::ScoringRule => {
                let mut fingerprint = environment(&pack);
                fingerprint.scoring_rule_version = "ability-v2".into();
                connection
                    .execute(
                        "UPDATE runs SET environment_json=?2 WHERE id=?1",
                        params![
                            run.id.to_string(),
                            serde_json::to_string(&fingerprint).unwrap()
                        ],
                    )
                    .unwrap();
            }
        }
        drop(connection);

        let service = ManualRunService::new(repository.clone(), artifact_root);
        assert!(matches!(
            service.resume(run.id, target(), pack.clone(), environment(&pack)),
            Err(RunServiceError::NotResumable(_)) | Err(RunServiceError::Storage(_))
        ));
        let connection = rusqlite::Connection::open(&database).unwrap();
        let (status, resumed): (String, String) = connection
            .query_row(
                "SELECT status_json,environment_json FROM runs WHERE id=?1",
                [run.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            status,
            serde_json::to_string(&RunStatus::Interrupted).unwrap()
        );
        assert!(!resumed.contains(r#""resumed":true"#));
    }
}

#[test]
fn resume_rejects_an_already_active_or_terminal_run() {
    let directory = tempdir().unwrap();
    let pack = write_pack(&directory.path().join("pack"));
    let repository = Arc::new(RunRepository::open(&directory.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repository.clone(), directory.path().join("artifacts"));
    let running = service
        .start(pack.clone(), target(), RunMode::Quick, environment(&pack))
        .unwrap();

    assert!(matches!(
        service.resume(running.id, target(), pack.clone(), environment(&pack)),
        Err(RunServiceError::NotResumable(_))
    ));
    repository
        .finish_without_score(running.id, RunStatus::Cancelled)
        .unwrap();
    assert!(matches!(
        service.resume(running.id, target(), pack.clone(), environment(&pack)),
        Err(RunServiceError::NotResumable(_))
    ));
}

#[test]
fn checkpoint_helper_documents_the_only_scoreable_terminal_shape() {
    let pack_root = tempdir().unwrap();
    let pack = write_pack(pack_root.path());
    let mut run = RunRecord::new(
        target(),
        RunMode::Quick,
        pack.manifest.id.clone(),
        pack.manifest.version.clone(),
        2,
        environment(&pack),
    );
    run.status = RunStatus::Interrupted;
    let result = checkpoint(&run, "one", Category::Logic);
    assert_eq!(result.outcome, TaskOutcome::Passed);
    assert_eq!(result.score, Some(100.0));
    assert_eq!(result.failure_kind, None::<FailureKind>);
}
