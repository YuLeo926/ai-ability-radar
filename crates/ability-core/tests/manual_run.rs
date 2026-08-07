use ability_core::{
    AdapterLaunchKind, BatchExecutionSurface, BatchMemberSeed, BatchMemberStatus, BatchMode,
    EnvironmentFingerprint, ExecutionAdapterIdentity, FailureKind, IsolationAttestation,
    IsolationEnforcement, ManualRunService, ModelSource, ModelVerification, PackLoader, RunMode,
    RunRecord, RunRepository, RunServiceError, RunStatus, ScanBatchPlan, ScanBatchTarget,
    ScanExecutionAuthorization, TargetKind, TargetSelection, build_batch_schedule,
};
use chrono::{Duration, Utc};
use rusqlite::Connection;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;
use uuid::Uuid;

fn write_pack(root: &std::path::Path, target_kinds: &str, grader: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("one.txt"), "Only output the number 4").unwrap();
    fs::write(
        root.join("manifest.json"),
        format!(
            r#"{{
              "schema_version":1,
              "id":"manual-smoke",
              "version":"1.0.0",
              "title":"Manual Smoke",
              "target_kinds":{target_kinds},
              "tasks":[{{
                "id":"one",
                "category":"logic",
                "prompt_file":"one.txt",
                "starter_dir":null,
                "time_budget_secs":60,
                "max_turns":1,
                "grader":{grader}
              }}]
            }}"#
        ),
    )
    .unwrap();
}

fn write_two_task_pack(root: &std::path::Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("one.txt"), "one").unwrap();
    fs::write(root.join("two.txt"), "two").unwrap();
    fs::write(
        root.join("manifest.json"),
        r#"{
          "schema_version":1,"id":"two-step","version":"1.0.0","title":"Two Step",
          "target_kinds":["chat_gpt_client"],"tasks":[
            {"id":"one","category":"logic","prompt_file":"one.txt","starter_dir":null,"time_budget_secs":60,"max_turns":1,"grader":{"type":"exact_text","expected":"1"}},
            {"id":"two","category":"logic","prompt_file":"two.txt","starter_dir":null,"time_budget_secs":60,"max_turns":1,"grader":{"type":"exact_text","expected":"2"}}
          ]
        }"#,
    )
    .unwrap();
}

fn environment(pack: &ability_core::LoadedPack) -> EnvironmentFingerprint {
    EnvironmentFingerprint {
        os_family: "windows".into(),
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

fn chatgpt_target() -> TargetSelection {
    TargetSelection {
        kind: TargetKind::ChatGptClient,
        reported_model: "user-selected".into(),
        reasoning_effort: None,
        model_source: ModelSource::Manual,
        model_verification: ModelVerification::UserConfirmed,
    }
}

fn guided_target(kind: TargetKind, model: &str) -> ScanBatchTarget {
    let provider = match kind {
        TargetKind::ChatGptClient => "openai",
        TargetKind::ClaudeClient => "anthropic",
        _ => panic!("guided client target required"),
    };
    ScanBatchTarget::new(
        TargetSelection {
            kind,
            reported_model: model.into(),
            reasoning_effort: Some("high".into()),
            model_source: ModelSource::Manual,
            model_verification: ModelVerification::UserConfirmed,
        },
        BatchExecutionSurface::GuidedClient,
        ExecutionAdapterIdentity::new(
            BatchExecutionSurface::GuidedClient,
            provider,
            AdapterLaunchKind::GuidedClient,
            None,
            "guided-client-v1",
        )
        .unwrap(),
    )
    .unwrap()
}

fn client_pack() -> Arc<ability_core::LoadedPack> {
    Arc::new(
        PackLoader::load(
            &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("benchmark-packs/client-quick-v1"),
        )
        .unwrap(),
    )
}

fn create_guided_batch(
    repository: &RunRepository,
    pack: &ability_core::LoadedPack,
) -> (Uuid, ScanBatchPlan) {
    let issued_at = Utc::now();
    let plan = ScanBatchPlan::new(
        pack,
        "ability-v1",
        BatchMode::QuickComparison,
        17,
        vec![
            guided_target(TargetKind::ChatGptClient, "GPT-5.6"),
            guided_target(TargetKind::ClaudeClient, "Claude Sonnet 4.5"),
        ],
        issued_at,
    )
    .unwrap();
    let members = build_batch_schedule(&plan)
        .unwrap()
        .members
        .into_iter()
        .map(|member| BatchMemberSeed {
            ordinal: member.ordinal,
            target_position: member.target_position,
            repetition_index: member.repetition_index,
        })
        .collect::<Vec<_>>();
    let batch_id = Uuid::new_v4();
    repository
        .insert_batch_plan(batch_id, pack, &plan, &members, issued_at)
        .unwrap();
    repository
        .append_execution_authorization(&ScanExecutionAuthorization {
            batch_id,
            member_ordinal: None,
            attempt_number: 1,
            max_provider_turns: plan.cost_estimate.max_provider_turns,
            max_task_budget_secs: plan.cost_estimate.summed_task_budget_secs,
            acknowledgement_hash: plan.acknowledgement_hash.clone(),
            allowed_failure_kind: None,
            expires_at: issued_at + Duration::hours(4),
            created_at: issued_at,
        })
        .unwrap();
    (batch_id, plan)
}

fn owned_run(plan: &ScanBatchPlan, target_position: usize, at: chrono::DateTime<Utc>) -> RunRecord {
    let target = &plan.targets[target_position];
    RunRecord {
        id: Uuid::new_v4(),
        target: target.target.clone(),
        mode: RunMode::Quick,
        suite_id: plan.suite_id.clone(),
        suite_version: plan.suite_version.clone(),
        status: RunStatus::Created,
        started_at: at,
        finished_at: None,
        total_tasks: u32::try_from(plan.sealed_task_budgets.len()).unwrap(),
        completed_tasks: 0,
        environment: EnvironmentFingerprint {
            os_family: "windows".into(),
            os_version: "11".into(),
            app_version: "0.3.0-test".into(),
            cli_version: None,
            verifier_runtime_version: None,
            suite_id: plan.suite_id.clone(),
            suite_version: plan.suite_version.clone(),
            suite_content_sha256: plan.suite_content_sha256.clone(),
            scoring_rule_version: plan.scoring_rule_version.clone(),
            execution_adapter_identity: Some(target.execution_adapter_identity.clone()),
            resumed: false,
        },
        score: None,
    }
}

fn owned_run_for_ordinal(
    plan: &ScanBatchPlan,
    ordinal: usize,
    at: chrono::DateTime<Utc>,
) -> RunRecord {
    let target_position =
        usize::try_from(build_batch_schedule(plan).unwrap().members[ordinal].target_position)
            .unwrap();
    owned_run(plan, target_position, at)
}

fn attestation(at: chrono::DateTime<Utc>, accepted: bool) -> IsolationAttestation {
    IsolationAttestation {
        policy_version: 1,
        enforcement: IsolationEnforcement::UserAttested,
        user_attested: accepted,
        recorded_at: at,
    }
}

#[test]
fn owned_guided_batch_run_reuses_reservation_persists_attestations_and_advances() {
    let dir = tempdir().unwrap();
    let database = dir.path().join("runs.db");
    let pack = client_pack();
    let repository = Arc::new(RunRepository::open(&database).unwrap());
    let (batch_id, plan) = create_guided_batch(&repository, &pack);
    let service = ManualRunService::new(repository.clone(), dir.path().join("artifacts"));
    let started_at = plan.cost_estimate.issued_at + Duration::seconds(1);
    let reserved = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            started_at,
            &owned_run_for_ordinal(&plan, 0, started_at),
        )
        .unwrap()
        .unwrap();
    let run_id = reserved.run.id;

    let running = service
        .start_owned_guided_batch_run(pack.clone(), reserved, started_at)
        .unwrap();
    assert_eq!(running.id, run_id);
    assert_eq!(repository.list_runs().unwrap().len(), 1);

    let mut submitted = 0_i64;
    while let Some(step) = service.next_step(run_id).unwrap() {
        submitted += 1;
        let recorded_at = started_at + Duration::seconds(submitted);
        service
            .submit_guided_batch_answer(
                batch_id,
                0,
                run_id,
                &step.task_id,
                "synthetic local answer",
                attestation(recorded_at, true),
            )
            .unwrap();
    }
    assert_eq!(submitted, i64::try_from(pack.tasks.len()).unwrap());
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Completed);
    assert_eq!(batch.members[1].status, BatchMemberStatus::Planned);
    assert_eq!(repository.list_runs().unwrap().len(), 1);

    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM scan_batch_task_isolation WHERE run_id=?1 AND policy_version=1 AND enforcement_json='\"user_attested\"' AND user_attested=1",
                [run_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        submitted
    );

    let second = owned_run_for_ordinal(&plan, 1, started_at + Duration::minutes(1));
    let next = repository
        .reserve_next_runnable_member_and_run(batch_id, started_at + Duration::minutes(1), &second)
        .unwrap()
        .unwrap();
    assert_eq!(next.member.ordinal, 1);
}

#[test]
fn declined_guided_attestation_terminalizes_without_evidence() {
    let dir = tempdir().unwrap();
    let database = dir.path().join("runs.db");
    let pack = client_pack();
    let repository = Arc::new(RunRepository::open(&database).unwrap());
    let (batch_id, plan) = create_guided_batch(&repository, &pack);
    let service = ManualRunService::new(repository.clone(), dir.path().join("artifacts"));
    let started_at = plan.cost_estimate.issued_at + Duration::seconds(1);
    let reserved = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            started_at,
            &owned_run_for_ordinal(&plan, 0, started_at),
        )
        .unwrap()
        .unwrap();
    let run_id = reserved.run.id;
    service
        .start_owned_guided_batch_run(pack, reserved, started_at)
        .unwrap();
    let step = service.next_step(run_id).unwrap().unwrap();

    service
        .decline_guided_batch_attestation(
            batch_id,
            0,
            run_id,
            &step.task_id,
            started_at + Duration::seconds(2),
        )
        .unwrap();

    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Invalid);
    assert_eq!(
        batch.members[0].failure_kind,
        Some(FailureKind::UserCancelled)
    );
    assert!(repository.get_task_results(run_id).unwrap().is_empty());
    assert_eq!(
        Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM scan_batch_task_isolation WHERE run_id=?1",
                [run_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert!(
        !dir.path()
            .join("artifacts/runs")
            .join(run_id.to_string())
            .exists()
    );
}

#[test]
fn guided_checkpoint_failure_removes_artifact_and_defers_without_half_evidence() {
    let dir = tempdir().unwrap();
    let database = dir.path().join("runs.db");
    let pack = client_pack();
    let repository = Arc::new(RunRepository::open(&database).unwrap());
    let (batch_id, plan) = create_guided_batch(&repository, &pack);
    let service = ManualRunService::new(repository.clone(), dir.path().join("artifacts"));
    let started_at = plan.cost_estimate.issued_at + Duration::seconds(1);
    let reserved = repository
        .reserve_next_runnable_member_and_run(
            batch_id,
            started_at,
            &owned_run_for_ordinal(&plan, 0, started_at),
        )
        .unwrap()
        .unwrap();
    let run_id = reserved.run.id;
    service
        .start_owned_guided_batch_run(pack, reserved, started_at)
        .unwrap();
    let step = service.next_step(run_id).unwrap().unwrap();
    Connection::open(&database)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_guided_result BEFORE INSERT ON task_results
             BEGIN SELECT RAISE(ABORT, 'forced guided checkpoint failure'); END;",
        )
        .unwrap();

    assert!(matches!(
        service.submit_guided_batch_answer(
            batch_id,
            0,
            run_id,
            &step.task_id,
            "synthetic local answer",
            attestation(started_at + Duration::seconds(2), true),
        ),
        Err(RunServiceError::Storage(_))
    ));
    assert!(repository.get_task_results(run_id).unwrap().is_empty());
    let connection = Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM scan_batch_task_isolation WHERE run_id=?1",
                [run_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert!(
        !dir.path()
            .join("artifacts/runs")
            .join(run_id.to_string())
            .join(format!("{}.txt", step.task_id))
            .exists()
    );
    let batch = repository.get_batch(batch_id).unwrap().unwrap();
    assert_eq!(batch.members[0].status, BatchMemberStatus::Deferred);
    assert_eq!(
        batch.members[0].failure_kind,
        Some(FailureKind::AppInterrupted)
    );
    assert_eq!(
        repository.get_run(run_id).unwrap().unwrap().status,
        RunStatus::Interrupted
    );
    assert!(matches!(
        service.next_step(run_id),
        Err(RunServiceError::RunNotFound(id)) if id == run_id
    ));
}

#[test]
fn manual_answers_checkpoint_and_complete_the_run() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();

    let step = service.next_step(run.id).unwrap().unwrap();
    assert_eq!(step.task_id, "one");
    assert!(matches!(
        service.submit_answer(run.id, "one", &"x".repeat(256 * 1024 + 1)),
        Err(RunServiceError::AnswerTooLarge)
    ));
    assert!(repo.get_task_results(run.id).unwrap().is_empty());
    assert!(!dir.path().join("artifacts").join("runs").exists());

    service.submit_answer(run.id, "one", "4").unwrap();

    assert!(service.next_step(run.id).unwrap().is_none());
    let completed = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(completed.status, RunStatus::Completed);
    assert_eq!(completed.score.unwrap().ability_score, 100.0);
    let answer_path = dir
        .path()
        .join("artifacts")
        .join("runs")
        .join(run.id.to_string())
        .join("one.txt");
    assert_eq!(fs::read_to_string(answer_path).unwrap(), "4");
}

#[test]
fn start_rejects_a_client_not_supported_by_the_pack_before_persisting() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["claude_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));

    assert!(matches!(
        service.start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack)
        ),
        Err(RunServiceError::UnsupportedTarget)
    ));
    assert!(repo.list_runs().unwrap().is_empty());
}

#[test]
fn start_rejects_a_mismatched_environment_or_external_verifier_before_persisting() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let mut mismatched_environment = environment(&pack);
    mismatched_environment.suite_content_sha256 = "0".repeat(64);

    assert!(matches!(
        service.start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            mismatched_environment,
        ),
        Err(RunServiceError::EnvironmentMismatch)
    ));
    assert!(repo.list_runs().unwrap().is_empty());

    let external_pack_dir = dir.path().join("external-pack");
    write_pack(
        &external_pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"external_verifier","verifier_id":"approved-v1"}"#,
    );
    let external_pack = Arc::new(PackLoader::load(&external_pack_dir).unwrap());
    assert!(matches!(
        service.start(
            external_pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&external_pack),
        ),
        Err(RunServiceError::UnsupportedGrader { .. })
    ));
    assert!(repo.list_runs().unwrap().is_empty());
}

#[test]
fn submissions_must_follow_the_pack_order_and_cannot_follow_completion() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_two_task_pack(&pack_dir);
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();

    assert!(matches!(
        service.submit_answer(run.id, "two", "2"),
        Err(RunServiceError::OutOfOrder)
    ));
    service.submit_answer(run.id, "one", "1").unwrap();
    service.submit_answer(run.id, "two", "2").unwrap();
    assert!(matches!(
        service.submit_answer(run.id, "two", "2"),
        Err(RunServiceError::RunNotFound(id)) if id == run.id
    ));
    assert_eq!(repo.get_task_results(run.id).unwrap().len(), 2);
}

#[test]
fn cancelling_one_manual_run_is_exact_idempotent_and_does_not_touch_another_run() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let first = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();
    let second = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();

    assert!(service.cancel(first.id).unwrap());
    assert!(!service.cancel(first.id).unwrap());
    assert_eq!(
        repo.get_run(first.id).unwrap().unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(
        repo.get_run(second.id).unwrap().unwrap().status,
        RunStatus::Running
    );
    assert!(matches!(
        service.next_step(first.id),
        Err(RunServiceError::RunNotFound(id)) if id == first.id
    ));
    assert_eq!(
        service.next_step(second.id).unwrap().unwrap().task_id,
        "one"
    );
}

#[test]
fn manual_cancel_preserves_a_committed_prefix_but_never_overwrites_completion() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_two_task_pack(&pack_dir);
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));

    let partial = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();
    service.submit_answer(partial.id, "one", "1").unwrap();
    assert!(service.cancel(partial.id).unwrap());
    assert_eq!(
        repo.get_run(partial.id).unwrap().unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(repo.get_task_results(partial.id).unwrap().len(), 1);
    assert!(
        !repo.has_running_runs().unwrap(),
        "a terminal manual cancellation must release the full-backup precondition"
    );
    assert!(matches!(
        service.resume(
            partial.id,
            chatgpt_target(),
            pack.clone(),
            environment(&pack),
        ),
        Err(RunServiceError::NotResumable(_))
    ));
    assert!(matches!(
        service.submit_answer(partial.id, "two", "2"),
        Err(RunServiceError::RunNotFound(id)) if id == partial.id
    ));
    assert_eq!(
        repo.get_run(partial.id).unwrap().unwrap().status,
        RunStatus::Cancelled,
        "resume and submit attempts must not overwrite the terminal status"
    );

    let completed = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();
    service.submit_answer(completed.id, "one", "1").unwrap();
    service.submit_answer(completed.id, "two", "2").unwrap();
    assert!(!service.cancel(completed.id).unwrap());
    assert_eq!(
        repo.get_run(completed.id).unwrap().unwrap().status,
        RunStatus::Completed
    );
}

#[test]
fn manual_interrupt_preserves_a_committed_prefix_and_remains_resumable() {
    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_two_task_pack(&pack_dir);
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let service = ManualRunService::new(repo.clone(), dir.path().join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();
    service.submit_answer(run.id, "one", "1").unwrap();

    assert!(service.interrupt(run.id).unwrap());
    assert!(!service.interrupt(run.id).unwrap());
    let interrupted = repo.get_run(run.id).unwrap().unwrap();
    assert_eq!(interrupted.status, RunStatus::Interrupted);
    assert_eq!(interrupted.completed_tasks, 1);
    assert_eq!(repo.get_task_results(run.id).unwrap().len(), 1);
    assert!(!repo.has_running_runs().unwrap());
    assert!(matches!(
        service.next_step(run.id),
        Err(RunServiceError::RunNotFound(id)) if id == run.id
    ));

    let resumed = service
        .resume(run.id, run.target, pack.clone(), environment(&pack))
        .unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    assert!(resumed.environment.resumed);
    assert_eq!(service.next_step(run.id).unwrap().unwrap().task_id, "two");
}

#[cfg(windows)]
#[test]
fn artifact_root_cannot_be_reached_through_a_directory_junction() {
    use std::process::Command;

    let dir = tempdir().unwrap();
    let pack_dir = dir.path().join("pack");
    write_pack(
        &pack_dir,
        r#"["chat_gpt_client"]"#,
        r#"{"type":"exact_text","expected":"4"}"#,
    );
    let pack = Arc::new(PackLoader::load(&pack_dir).unwrap());
    let repo = Arc::new(RunRepository::open(&dir.path().join("runs.db")).unwrap());
    let outside = tempdir().unwrap();
    let junction = dir.path().join("artifact-junction");
    let status = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            outside.path().to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let service = ManualRunService::new(repo.clone(), junction.join("artifacts"));
    let run = service
        .start(
            pack.clone(),
            chatgpt_target(),
            RunMode::Quick,
            environment(&pack),
        )
        .unwrap();

    assert!(matches!(
        service.submit_answer(run.id, "one", "4"),
        Err(RunServiceError::UnsafeArtifactPath)
    ));
    assert!(repo.get_task_results(run.id).unwrap().is_empty());
    assert!(!outside.path().join("artifacts").exists());
}
